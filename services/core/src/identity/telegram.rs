//! Ordinary Telegram Login Widget support. Only explicit stable-ID bindings
//! can sign in; names, email and provider profile fields cannot claim a user.
mod protocol;
use super::*;
use protocol::VerifiedTelegramIdentity;
pub use protocol::{TelegramAuthorization, TelegramLogin};

pub(crate) const SCHEMA: &str = include_str!("../../schema/telegram_auth.sql");

#[derive(Serialize)]
pub struct TelegramFlow {
    pub flow_token: String,
    pub expires_at: i64,
}

pub(super) struct TelegramFlowOwner {
    pub user_id: i64,
    pub user_version: i64,
    pub sid: String,
}

pub(super) struct TelegramProof {
    pub subject: String,
    pub owner: Option<TelegramFlowOwner>,
}

impl IdentityStore {
    pub fn with_telegram_login(mut self, config: TelegramLogin) -> Result<Self> {
        if self.telegram.is_some() {
            return Err(IdentityError::Invalid);
        }
        self.telegram = Some(std::sync::Arc::new(config));
        Ok(self)
    }

    pub fn telegram_bot_name(&self) -> Option<&str> {
        self.telegram.as_ref().map(|config| config.bot_name())
    }

    pub(super) async fn begin_telegram_flow(
        &self,
        binding: &str,
        owner: Option<TelegramFlowOwner>,
    ) -> Result<TelegramFlow> {
        self.telegram.as_ref().ok_or(IdentityError::Forbidden)?;
        let token = new_secret("lmmt_")?;
        let purpose = if owner.is_some() { "bind" } else { "login" };
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM core_telegram_auth.flows WHERE token_digest IN (SELECT token_digest FROM core_telegram_auth.flows WHERE expires_at<=clock_timestamp() LIMIT 100)")
            .execute(&mut *tx).await?;
        sqlx::query("DELETE FROM core_telegram_auth.assertions WHERE digest IN (SELECT digest FROM core_telegram_auth.assertions WHERE expires_at<=clock_timestamp() LIMIT 100)")
            .execute(&mut *tx).await?;
        let expires_at = sqlx::query_scalar("INSERT INTO core_telegram_auth.flows(token_digest,purpose,binding_digest,user_id,user_version,sid) VALUES ($1,$2,$3,$4,$5,$6) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(digest(&token)?).bind(purpose).bind(digest(binding)?)
            .bind(owner.as_ref().map(|o|o.user_id)).bind(owner.as_ref().map(|o|o.user_version))
            .bind(owner.as_ref().map(|o|o.sid.as_str())).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(TelegramFlow {
            flow_token: token,
            expires_at,
        })
    }

    pub(super) async fn consume_telegram_flow(
        &self,
        flow: &str,
        binding: &str,
        purpose: &str,
        input: &TelegramAuthorization,
    ) -> Result<TelegramProof> {
        if !flow.starts_with("lmmt_") || flow.len() != 69 || !matches!(purpose, "login" | "bind") {
            return Err(IdentityError::Unauthorized);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
            .ok_or(IdentityError::Storage)?;
        let identity: VerifiedTelegramIdentity = self
            .telegram
            .as_ref()
            .ok_or(IdentityError::Forbidden)?
            .verify(input, now)?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("UPDATE core_telegram_auth.flows SET consumed_at=clock_timestamp() WHERE token_digest=$1 AND purpose=$2 AND binding_digest=$3 AND consumed_at IS NULL AND expires_at>clock_timestamp() AND to_timestamp($4)>=created_at-interval '30 seconds' RETURNING user_id,user_version,sid")
            .bind(digest(flow)?).bind(purpose).bind(digest(binding)?).bind(identity.auth_date() as f64)
            .fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        // An assertion successfully used for either purpose cannot be replayed
        // using another flow, another browser, or the other purpose. Commit the
        // claim before account/session checks so rejected callbacks stay spent.
        let claimed = sqlx::query(
            "INSERT INTO core_telegram_auth.assertions(digest) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(identity.replay_digest().to_vec())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if claimed != 1 {
            return Err(IdentityError::Unauthorized);
        }
        let owner = if purpose == "bind" {
            Some(TelegramFlowOwner {
                user_id: row.try_get("user_id")?,
                user_version: row.try_get("user_version")?,
                sid: row.try_get("sid")?,
            })
        } else {
            None
        };
        tx.commit().await?;
        Ok(TelegramProof {
            subject: identity.subject().to_owned(),
            owner,
        })
    }

    pub(super) async fn bind_telegram_subject(
        &self,
        tx: &mut PgConnection,
        user: i64,
        subject: &str,
    ) -> Result<()> {
        let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_telegram_auth.bindings WHERE user_id=$1 AND revoked_at IS NULL)")
            .bind(user).fetch_one(&mut *tx).await?;
        if active {
            return Err(IdentityError::Conflict);
        }
        let claimed = sqlx::query("INSERT INTO core_telegram_auth.bindings(subject,user_id) VALUES ($1,$2) ON CONFLICT(subject) DO UPDATE SET revoked_at=NULL WHERE core_telegram_auth.bindings.user_id=EXCLUDED.user_id AND core_telegram_auth.bindings.revoked_at IS NOT NULL")
            .bind(subject).bind(user).execute(&mut *tx).await?.rows_affected();
        if claimed != 1 {
            return Err(IdentityError::Conflict);
        }
        sqlx::query("INSERT INTO core_identity.audit(actor_user_id,action,target) VALUES ($1,'identity.telegram_bind',jsonb_build_object('user_id',$1::bigint))")
            .bind(user).execute(tx).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
