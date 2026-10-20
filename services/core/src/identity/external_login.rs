//! Provider subjects are identities; email and mutable usernames are not.
mod provider;
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use provider::VerifiedExternalIdentity;
pub use provider::{ExternalOAuth, ExternalProvider};

impl IdentityStore {
    pub fn with_external_oauth(mut self, config: ExternalOAuth) -> Result<Self> {
        if self.external.contains_key(&config.provider()) {
            return Err(IdentityError::Invalid);
        }
        self.external
            .insert(config.provider(), std::sync::Arc::new(config));
        Ok(self)
    }

    /// Public readiness is available only for the configured canonical origin.
    pub fn external_browser_client_id(
        &self,
        provider: ExternalProvider,
        origin: &str,
    ) -> Option<&str> {
        let config = self.external.get(&provider)?;
        let callback = reqwest::Url::parse(config.redirect_uri()).ok()?;
        (callback.origin().ascii_serialization() == origin).then_some(config.client_id())
    }

    pub(super) async fn begin_external_login(
        &self,
        provider: ExternalProvider,
        binding: &str,
    ) -> Result<String> {
        let config = self
            .external
            .get(&provider)
            .ok_or(IdentityError::Forbidden)?;
        let state = new_secret("lmmo_")?;
        let verifier = new_secret("")?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        sqlx::query("DELETE FROM core_identity.oauth_flows WHERE state_digest IN (SELECT state_digest FROM core_identity.oauth_flows WHERE expires_at<=clock_timestamp() LIMIT 100)")
            .execute(&self.pool).await?;
        sqlx::query("INSERT INTO core_identity.oauth_flows(state_digest,issuer,binding_digest,nonce_digest,pkce_verifier) VALUES ($1,$2,$3,$4,$5)")
            .bind(digest(&state)?).bind(provider.issuer()).bind(digest(binding)?).bind(digest(&new_secret("")?)?).bind(verifier).execute(&self.pool).await?;
        config.authorization_url(&state, &challenge)
    }

    pub(super) async fn exchange_external_code(
        &self,
        provider: ExternalProvider,
        state: &str,
        binding: &str,
        code: &str,
    ) -> Result<VerifiedExternalIdentity> {
        let config = self
            .external
            .get(&provider)
            .ok_or(IdentityError::Forbidden)?;
        if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
            return Err(IdentityError::Unauthorized);
        }
        // Consume before network I/O. A provider failure or restart cannot make
        // a callback reusable, and another provider cannot consume this flow.
        let verifier: String = sqlx::query_scalar("DELETE FROM core_identity.oauth_flows WHERE state_digest=$1 AND issuer=$2 AND binding_digest=$3 AND expires_at>clock_timestamp() RETURNING pkce_verifier")
            .bind(digest(state)?).bind(provider.issuer()).bind(digest(binding)?).fetch_optional(&self.pool).await?.ok_or(IdentityError::Unauthorized)?;
        config.exchange(code, &verifier).await
    }

    pub(super) async fn external_user(
        &self,
        tx: &mut PgConnection,
        identity: &VerifiedExternalIdentity,
        source: Option<crate::abuse::registration::RegistrationSource>,
    ) -> Result<i64> {
        auth::lock_registration(tx).await?;
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT user_id FROM core_identity.oauth_identities WHERE issuer=$1 AND subject=$2",
        )
        .bind(identity.issuer())
        .bind(identity.subject())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(user) = existing {
            return Ok(user);
        }
        if !self.registration_enabled {
            return Err(IdentityError::Forbidden);
        }
        let user = auth::create_user(tx).await?;
        let username = format!("{}_{}", identity.provider().slug(), &new_secret("")?[..12]);
        sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,display_name,password_login_allowed) VALUES ($1,$2,$2,FALSE)")
            .bind(user).bind(username).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) VALUES ($1,$2,$3)",
        )
        .bind(identity.issuer())
        .bind(identity.subject())
        .bind(user)
        .execute(&mut *tx)
        .await?;
        crate::abuse::registration::record(tx, user, source)
            .await
            .map_err(|_| IdentityError::Storage)?;
        Ok(user)
    }
}

#[cfg(test)]
mod tests;
