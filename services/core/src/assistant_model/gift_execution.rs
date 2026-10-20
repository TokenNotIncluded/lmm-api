//! Fixed promotional tool execution. A provider suggestion cannot select an
//! account, bypass eligibility, or turn a rejected gift into a successful reply.
use crate::{
    gift::{self, GiftDecision, Receipt},
    identity,
    model_http::assistant_gift::{GiftAction, GiftToolCall},
};
use sqlx::{PgConnection, Row};
pub(super) struct GiftOutcome {
    pub answer: String,
    pub receipt: Option<Receipt>,
    pub tool_id: String,
    pub response_sha256: [u8; 32],
}
pub(super) async fn execute(
    c: &mut PgConnection,
    session: &str,
    request_id: &str,
    call: &GiftToolCall,
) -> Result<GiftOutcome, super::Error> {
    let actor = match identity::authorize_in_transaction(c, session).await {
        Ok(actor) => actor,
        Err(identity::IdentityError::Unauthorized | identity::IdentityError::Forbidden) => {
            return Ok(GiftOutcome {
                answer: "会话已失效，本次未发放赠礼。".into(),
                receipt: None,
                tool_id: call.id().into(),
                response_sha256: call.response_sha256(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    if actor.credential_kind != identity::CredentialKind::Session {
        return Err(super::Error::Forbidden);
    }
    let binding=sqlx::query("SELECT g.conversation_id,g.session_id,g.revoked,g.expires_at,v.owner_user_id,v.owner_account_id FROM core_charging.requests r JOIN core_assistant.grants g ON g.id=r.assistant_grant_id JOIN core_assistant.conversations v ON v.id=g.conversation_id JOIN core_assistant.operations o ON o.request_id=r.id AND o.grant_id=g.id WHERE r.id=$1 AND r.state='streaming' AND r.actor_user_id=$2 AND r.owner_account_id=$3 FOR UPDATE OF r,g,o").bind(request_id).bind(actor.user_id).bind(actor.owner_account_id).fetch_optional(&mut *c).await?.ok_or(super::Error::Forbidden)?;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&mut *c)
        .await?;
    if binding.try_get::<i64, _>("session_id")? != actor.credential_id
        || binding.try_get::<bool, _>("revoked")?
        || now >= binding.try_get::<i64, _>("expires_at")?
    {
        return Ok(GiftOutcome {
            answer: "赠礼授权已失效，本次未发放赠礼。".into(),
            receipt: None,
            tool_id: call.id().into(),
            response_sha256: call.response_sha256(),
        });
    }
    let conversation: String = binding.try_get("conversation_id")?;
    sqlx::query("SAVEPOINT fixed_gift_action")
        .execute(&mut *c)
        .await?;
    let result = match call.action() {
        GiftAction::Prepare(a) => {
            gift::execute_in_transaction(
                c,
                actor.user_id,
                &conversation,
                request_id,
                GiftDecision {
                    amount_credits: a.amount_credits,
                    reason: a.reason.clone(),
                },
            )
            .await
        }
        GiftAction::Status => gift::status_in_transaction(c, actor.user_id).await,
    };
    let (receipt, denied) = match result {
        Ok(receipt) => {
            sqlx::query("RELEASE SAVEPOINT fixed_gift_action")
                .execute(&mut *c)
                .await?;
            (receipt, false)
        }
        Err(
            gift::Error::Forbidden
            | gift::Error::Invalid
            | gift::Error::NotFound
            | gift::Error::Conflict,
        ) => {
            sqlx::query("ROLLBACK TO SAVEPOINT fixed_gift_action")
                .execute(&mut *c)
                .await?;
            sqlx::query("RELEASE SAVEPOINT fixed_gift_action")
                .execute(&mut *c)
                .await?;
            (None, true)
        }
        Err(_) => return Err(super::Error::Unavailable),
    };
    let answer = if denied {
        "当前不符合新用户赠礼条件，本次未发放赠礼。".to_owned()
    } else if let Some(r) = &receipt {
        if r.claimed_at_unix.is_some() {
            format!("新用户赠礼已入账：{} 积分。", r.credit_amount)
        } else {
            "已有新用户赠礼记录，请查看当前领取条件。".to_owned()
        }
    } else {
        match call.action() {
            GiftAction::Prepare(_) => "本次没有发放赠礼。".to_owned(),
            GiftAction::Status => "尚无新用户赠礼记录。".to_owned(),
        }
    };
    Ok(GiftOutcome {
        answer,
        receipt,
        tool_id: call.id().to_owned(),
        response_sha256: call.response_sha256(),
    })
}
