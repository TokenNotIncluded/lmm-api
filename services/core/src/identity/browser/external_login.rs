//! All verified external providers share the existing browser family and MFA.
use super::*;
use crate::identity::{BrowserLoginOutcome, ExternalProvider, MfaChallenge, MfaError, MfaResult};

impl BrowserAuth {
    fn external_binding(&self, provider: ExternalProvider, binding: &str) -> Result<String> {
        if binding.len() != 64 || !binding.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(IdentityError::Unauthorized);
        }
        self.derived_secret(b"browser-external-flow-v1", provider.issuer(), binding, "")
    }

    pub async fn begin_external_login(
        &self,
        provider: ExternalProvider,
        binding: &str,
    ) -> BrowserResult<String> {
        Ok(self
            .store
            .begin_external_login(provider, &self.external_binding(provider, binding)?)
            .await?)
    }

    pub async fn finish_external_login(
        &self,
        provider: ExternalProvider,
        state: &str,
        binding: &str,
        code: &str,
        metadata: &BrowserSessionMetadata,
        mfa_binding: &str,
    ) -> MfaResult<BrowserLoginOutcome> {
        let identity = self
            .store
            .exchange_external_code(
                provider,
                state,
                &self.external_binding(provider, binding)?,
                code,
            )
            .await?;
        let source = metadata.ip.parse().ok().map(|address| {
            crate::abuse::registration::RegistrationSource {
                address,
                kind: crate::abuse::registration::RegistrationSourceKind::Socket,
            }
        });
        let mut tx = self.store.pool.begin().await?;
        let user = self.store.external_user(&mut tx, &identity, source).await?;
        let outcome = self
            .provider_login_outcome(
                &mut tx,
                user,
                metadata,
                mfa_binding,
                &format!("session.{}_browser_login", provider.slug()),
            )
            .await?;
        tx.commit().await?;
        Ok(outcome)
    }

    pub(in crate::identity::browser) async fn provider_login_outcome(
        &self,
        tx: &mut PgConnection,
        user: i64,
        metadata: &BrowserSessionMetadata,
        binding: &str,
        action: &str,
    ) -> MfaResult<BrowserLoginOutcome> {
        if metadata.ip.len() > 128
            || metadata.user_agent.len() > 1024
            || binding.len() != 64
            || !binding.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(IdentityError::Invalid.into());
        }
        // Lock the live authority and local MFA policy before deciding whether
        // to issue a family. Provider email and role fields grant no authority.
        sqlx::query("SELECT u.id FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND u.active AND a.active FOR SHARE OF u,a NOWAIT")
            .bind(user).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let required: bool = sqlx::query_scalar("SELECT second_factor_required FROM core_identity.user_profiles WHERE user_id=$1 FOR SHARE NOWAIT")
            .bind(user).fetch_one(&mut *tx).await?;
        if required {
            let enabled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.mfa_factors WHERE user_id=$1 AND enabled)")
                .bind(user).fetch_one(&mut *tx).await?;
            if !enabled {
                return Err(IdentityError::Unauthorized.into());
            }
            // This purpose identifies a browser primary-authentication stage.
            // The same completion path checks binding, generation and factor;
            // using a provider does not bypass or upgrade that assurance.
            Ok(BrowserLoginOutcome::Challenge(
                super::super::mfa::begin_challenge(
                    &self.store,
                    tx,
                    user,
                    "browser_password_login",
                    Some(binding),
                    Some(metadata),
                )
                .await?,
            ))
        } else {
            Ok(BrowserLoginOutcome::Session(
                self.issue_browser_family(tx, user, metadata, action)
                    .await?,
            ))
        }
    }

    /// Recover only a pending cookie-bound challenge after a direct provider
    /// callback. Neither flow token nor code is placed in a redirect URL.
    pub async fn pending_external_mfa(
        &self,
        token: &str,
        binding: &str,
    ) -> MfaResult<MfaChallenge> {
        if !token.starts_with("lmmf_") || token.len() != 69 {
            return Err(MfaError::FlowExpired);
        }
        let expires: Option<i64> = sqlx::query_scalar("SELECT floor(extract(epoch FROM expires_at))::bigint FROM core_identity.mfa_challenges WHERE digest=$1 AND binding_digest=$2 AND purpose='browser_password_login' AND expires_at>clock_timestamp() AND consumed_at IS NULL")
            .bind(digest(token)?).bind(digest(binding)?).fetch_optional(&self.store.pool).await?;
        Ok(MfaChallenge {
            require_2fa: true,
            flow_token: token.to_owned(),
            expires_at: expires.ok_or(MfaError::FlowExpired)?,
        })
    }
}
