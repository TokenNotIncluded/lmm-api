//! Personal assistant admission. Called only inside Core's durable grant transaction.
//! No API key, synthetic OAuth source, caller-selected account or standalone commit.
use super::*;

impl Charging {
    pub(crate) async fn reserve_assistant(
        &self,
        c: &mut PgConnection,
        session: &str,
        grant_id: &str,
        request: &Request,
        worker: &[u8; 32],
    ) -> Result<Charge> {
        request.validate()?;
        if request.price.upstream_cost_group_ppm.is_some() {
            return Err(Error::Forbidden);
        }
        let actor = crate::identity::authorize_in_transaction(c, session)
            .await
            .map_err(authority::identity_error)?;
        if actor.credential_kind != crate::identity::CredentialKind::Session {
            return Err(Error::Forbidden);
        }
        let conversation: String =
            sqlx::query_scalar("SELECT conversation_id FROM core_assistant.grants WHERE id=$1")
                .bind(grant_id)
                .fetch_optional(&mut *c)
                .await?
                .ok_or(Error::Unauthorized)?;
        crate::assistant_model::lock_conversation_id(c, &conversation)
            .await
            .map_err(assistant_error)?;
        let row = sqlx::query("SELECT g.session_id,g.auth_version,g.price,g.policy,v.owner_user_id,v.owner_account_id FROM core_assistant.grants g JOIN core_assistant.conversations v ON v.id=g.conversation_id WHERE g.id=$1 AND NOT g.revoked AND g.expires_at>extract(epoch FROM clock_timestamp())::bigint FOR UPDATE OF g")
            .bind(grant_id).fetch_optional(&mut *c).await?.ok_or(Error::Unauthorized)?;
        let owner: i64 = row.try_get("owner_account_id")?;
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *c)
        .await?;
        if row.try_get::<i64, _>("session_id")? != actor.credential_id
            || row.try_get::<i64, _>("auth_version")? != version
            || row.try_get::<i64, _>("owner_user_id")? != actor.user_id
            || owner != actor.owner_account_id
            || row.try_get::<Value, _>("price")? != json(&request.price)?
        {
            return Err(Error::Forbidden);
        }
        // The account must be the actor's live personal account, never a team or sponsor.
        let preference: String = sqlx::query_scalar("SELECT p.preference FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_billing.account_policies p ON p.account_id=a.id WHERE u.id=$1 AND a.id=$2 AND a.kind='personal' AND a.active FOR SHARE OF u,a,p")
            .bind(actor.user_id).bind(owner).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
        let policy: crate::assistant_model::PersonalTextPolicy =
            serde_json::from_value(row.try_get("policy")?).map_err(|_| Error::Storage)?;
        policy.validate().map_err(assistant_error)?;
        if request.reserve > policy.budget_credits {
            return Err(Error::BudgetExceeded);
        }
        let (payer_id, sponsor_id, preference) = if let Some(sponsor) = &policy.sponsor {
            sponsor
                .admit(c, actor.user_id, owner, request.reserve)
                .await
                .map_err(assistant_error)?;
            (
                sponsor.treasury_account_id,
                Some(sponsor.treasury_account_id),
                "wallet_only".to_owned(),
            )
        } else {
            crate::identity::authorize_assistant_model_in_transaction(
                c,
                session,
                &request.price.group,
            )
            .await
            .map_err(authority::identity_error)?;
            (owner, None, preference)
        };
        lock(c, &request.id).await?;
        // Only the initiating fresh reservation may mint worker authority.
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_charging.requests WHERE id=$1 OR assistant_grant_id=$2)")
            .bind(&request.id).bind(grant_id).fetch_one(&mut *c).await?;
        if exists {
            return Err(Error::Conflict);
        }
        authority::reject_new_model(c, actor.user_id).await?;
        let at = now(c).await?;
        let payer = authority::Payer {
            id: payer_id,
            preference,
        };
        let mut last = Error::NoFunds;
        for source in limits::sources(c, &payer, &request.price, at).await? {
            sqlx::query("SAVEPOINT assistant_funding")
                .execute(&mut *c)
                .await?;
            let (sub, start, end) = match source {
                Source::Wallet => (None, None, None),
                Source::Subscription { id, start, end } => (Some(id), Some(start), Some(end)),
            };
            sqlx::query("INSERT INTO core_charging.requests(id,actor_user_id,key_id,oauth_source,assistant_grant_id,assistant_sponsor_account_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,source_subscription_id,source_start,source_end,authorized_at,lease_until,hard_deadline,state,reserved) VALUES($1,$2,NULL,NULL,$3,$17,$4,$16,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'reserved',$15)")
                .bind(&request.id).bind(actor.user_id).bind(grant_id).bind(owner).bind(request.fingerprint.to_vec()).bind(authority::worker_digest(worker)?).bind(hash(request)?).bind(json(&request.price)?).bind(sub).bind(start).bind(end).bind(at).bind(at+request.lease_seconds).bind(at+request.maximum_seconds).bind(request.reserve).bind(payer_id).bind(sponsor_id).execute(&mut *c).await?;
            let charge = load(c, &request.id).await?.charge;
            match self.hold(c, &charge, at).await {
                Ok(()) => {
                    sqlx::query("RELEASE SAVEPOINT assistant_funding")
                        .execute(&mut *c)
                        .await?;
                    return Ok(charge);
                }
                Err(e @ (Error::NoFunds | Error::BudgetExceeded)) => {
                    last = e;
                    sqlx::query("ROLLBACK TO SAVEPOINT assistant_funding")
                        .execute(&mut *c)
                        .await?;
                    sqlx::query("RELEASE SAVEPOINT assistant_funding")
                        .execute(&mut *c)
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Same transaction as reserve+operation insertion, before any HTTP bytes.
    pub(crate) async fn start_assistant(
        &self,
        c: &mut PgConnection,
        id: &str,
        worker: &[u8; 32],
    ) -> Result<()> {
        lock(c, id).await?;
        let stored = load(c, id).await?;
        worker_matches(&stored, worker)?;
        live(&stored.charge, now(c).await?)?;
        if stored.charge.state != "reserved"
            || stored.charge.key_id.is_some()
            || stored.charge.oauth_source.is_some()
        {
            return Err(Error::WrongState);
        }
        let grant: Option<String> =
            sqlx::query_scalar("SELECT assistant_grant_id FROM core_charging.requests WHERE id=$1")
                .bind(id)
                .fetch_one(&mut *c)
                .await?;
        if grant.is_none() {
            return Err(Error::Forbidden);
        }
        sqlx::query(
            "UPDATE core_charging.requests SET state='streaming',revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *c)
        .await?;
        event(c, id, "start", json!({"kind":"assistant_dispatch_claim"})).await?;
        Ok(())
    }

    pub(crate) async fn unknown_assistant(
        &self,
        c: &mut PgConnection,
        id: &str,
        worker: &[u8; 32],
    ) -> Result<()> {
        lock(c, id).await?;
        let stored = load(c, id).await?;
        worker_matches(&stored, worker)?;
        if stored.charge.state == "streaming" {
            sqlx::query("UPDATE core_charging.requests SET state='reconcile',revision=revision+1 WHERE id=$1").bind(id).execute(&mut *c).await?;
            event(
                c,
                id,
                "assistant_unknown",
                json!({"kind":"assistant_requires_original_evidence"}),
            )
            .await?;
        }
        Ok(())
    }

    /// The answer and wallet/subscription settlement are ONE PostgreSQL commit.
    pub(crate) async fn settle_assistant(
        &self,
        c: &mut PgConnection,
        id: &str,
        worker: &[u8; 32],
        usage: Usage,
        answer: &str,
    ) -> Result<Charge> {
        if answer.is_empty() || answer.len() > 32768 {
            return Err(Error::Invalid);
        }
        lock(c, id).await?;
        let stored = load(c, id).await?;
        worker_matches(&stored, worker)?;
        live(&stored.charge, now(c).await?)?;
        if stored.charge.state != "streaming"
            || stored.charge.key_id.is_some()
            || stored.charge.oauth_source.is_some()
        {
            return Err(Error::WrongState);
        }
        let count=sqlx::query("UPDATE core_assistant.operations o SET answer_text=$2,completed_at=extract(epoch FROM clock_timestamp())::bigint FROM core_charging.requests r WHERE o.request_id=$1 AND r.id=o.request_id AND r.assistant_grant_id=o.grant_id AND o.answer_text IS NULL")
            .bind(id).bind(answer).execute(&mut *c).await?.rows_affected();
        if count != 1 {
            return Err(Error::Conflict);
        }
        self.finish(c, stored.charge, usage, Outcome::Completed, None)
            .await
    }
}

fn assistant_error(e: crate::assistant_model::Error) -> Error {
    match e {
        crate::assistant_model::Error::Invalid => Error::Invalid,
        crate::assistant_model::Error::Unauthorized => Error::Unauthorized,
        crate::assistant_model::Error::Forbidden => Error::Forbidden,
        crate::assistant_model::Error::Conflict => Error::Conflict,
        crate::assistant_model::Error::NoFunds => Error::BudgetExceeded,
        _ => Error::Storage,
    }
}
