use super::{Error, context::VerifiedToolContext};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
/// This projection contains no other person's identifiers, IP, email or text.
/// Missing immutable evidence is unavailable, not a guessed low-risk outcome.
pub async fn risk_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    if actor.developer || actor.role != "user" {
        return Err(Error::Forbidden);
    }
    let r=sqlx::query("SELECT o.source_policy_version,o.source_kind,o.source_ip IS NOT NULL AS network_available,q.evidence_kind,q.registration_hold,q.reward_used FROM core_abuse.registration_observations o LEFT JOIN core_gift.qualifications q ON q.user_id=o.user_id WHERE o.user_id=$1")
        .bind(actor.user).fetch_optional(&mut *c).await?;
    let Some(r) = r else {
        return Ok(json!({"ok":false,"status":"verification_unavailable"}));
    };
    if r.try_get::<Option<i32>, _>("source_policy_version")? != Some(1)
        || !r.try_get::<bool, _>("network_available")?
        || r.try_get::<Option<String>, _>("evidence_kind")?.as_deref()
            != Some("google_verified_email_v1")
    {
        return Ok(json!({"ok":false,"status":"verification_unavailable"}));
    }
    let marked = crate::abuse::is_marked_in_transaction(c, actor.user)
        .await
        .map_err(|_| Error::Unavailable)?;
    Ok(
        json!({"ok":true,"status":"checked","risk":{"verified_email":true,"registration_network_evidence":true,"registration_hold":r.try_get::<Option<bool>,_>("registration_hold")?,"reward_used":r.try_get::<Option<bool>,_>("reward_used")?,"current_abuse_mark":marked,"source":"core verified registration v1"},"untrusted_own_history":[],"own_history_available":false,"message":"Use these as fallible observations, not proof of a person's identity or a tool authorization."}),
    )
}
