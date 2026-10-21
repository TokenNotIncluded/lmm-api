//! Telegram uses the same browser family and local MFA policy as other logins.
use super::*;
use crate::identity::telegram::{TelegramAuthorization, TelegramFlow, TelegramFlowOwner};
use crate::identity::{BrowserLoginOutcome, MfaResult};

impl BrowserAuth {
    fn telegram_binding(&self, binding: &str, purpose: &str) -> Result<String> {
        if binding.len() != 64
            || !binding.bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(purpose, "login" | "bind")
        {
            return Err(IdentityError::Unauthorized);
        }
        self.derived_secret(b"browser-telegram-flow-v1", purpose, binding, "")
    }

    pub async fn begin_telegram_login(&self, binding: &str) -> BrowserResult<TelegramFlow> {
        Ok(self
            .store
            .begin_telegram_flow(&self.telegram_binding(binding, "login")?, None)
            .await?)
    }

    pub async fn finish_telegram_login(
        &self,
        flow: &str,
        binding: &str,
        input: &TelegramAuthorization,
        metadata: &BrowserSessionMetadata,
        mfa_binding: &str,
    ) -> MfaResult<BrowserLoginOutcome> {
        let proof = self
            .store
            .consume_telegram_flow(
                flow,
                &self.telegram_binding(binding, "login")?,
                "login",
                input,
            )
            .await?;
        let mut tx = self.store.pool.begin().await?;
        // Ordinary Telegram login has never been an email/username or implicit
        // registration path. Imported links must name the exact stable ID.
        let user: i64=sqlx::query_scalar("SELECT user_id FROM core_telegram_auth.bindings WHERE subject=$1 AND revoked_at IS NULL FOR SHARE")
            .bind(&proof.subject).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let outcome = self
            .provider_login_outcome(
                &mut tx,
                user,
                metadata,
                mfa_binding,
                "session.telegram_browser_login",
            )
            .await?;
        tx.commit().await?;
        Ok(outcome)
    }

    pub async fn begin_telegram_bind(
        &self,
        access: &str,
        binding: &str,
        expected: Option<&str>,
    ) -> BrowserResult<TelegramFlow> {
        let mut tx = self.store.pool.begin().await?;
        let actor = session(&mut tx, access).await?;
        let sid:String=sqlx::query_scalar("SELECT sid FROM core_identity.web_sessions WHERE current_credential_id=$1 AND user_id=$2 AND user_version=$3 AND revoked_at IS NULL AND expires_at>clock_timestamp() AND (NOT weekly_logout OR created_at>clock_timestamp()-interval '7 days')")
            .bind(actor.credential_id).bind(actor.user_id).bind(actor.user_version).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        if expected.is_some_and(|expected| expected != sid) {
            return Err(BrowserAuthError::SessionMismatch);
        }
        tx.commit().await?;
        Ok(self
            .store
            .begin_telegram_flow(
                &self.telegram_binding(binding, "bind")?,
                Some(TelegramFlowOwner {
                    user_id: actor.user_id,
                    user_version: actor.user_version,
                    sid,
                }),
            )
            .await?)
    }

    pub async fn finish_telegram_bind(
        &self,
        flow: &str,
        binding: &str,
        input: &TelegramAuthorization,
    ) -> BrowserResult<()> {
        let proof = self
            .store
            .consume_telegram_flow(
                flow,
                &self.telegram_binding(binding, "bind")?,
                "bind",
                input,
            )
            .await?;
        let owner = proof.owner.ok_or(IdentityError::Unauthorized)?;
        let mut tx = self.store.pool.begin().await?;
        // A top-level Widget callback has no bearer. Its authority is limited
        // to this exact cookie-bound flow and the still-live original family.
        // Refresh within that family is allowed; logout and generation changes
        // are terminal. No refresh cookie is read or widened to /api/oauth.
        let credential: i64=sqlx::query_scalar("SELECT c.id FROM core_identity.web_sessions s JOIN core_identity.users u ON u.id=s.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.credentials c ON c.id=s.current_credential_id WHERE s.sid=$1 AND u.id=$2 AND u.auth_version=$3 AND s.user_version=u.auth_version AND c.user_version=u.auth_version AND c.user_id=u.id AND c.kind='session' AND u.active AND a.active AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp() AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days') AND c.revoked_at IS NULL AND c.expires_at>clock_timestamp() FOR UPDATE OF s,u,c NOWAIT FOR SHARE OF a NOWAIT")
            .bind(&owner.sid).bind(owner.user_id).bind(owner.user_version).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        super::super::mfa::require_session_assurance(
            &mut tx,
            credential,
            owner.user_id,
            owner.user_version,
        )
        .await?;
        self.store
            .bind_telegram_subject(&mut tx, owner.user_id, &proof.subject)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
