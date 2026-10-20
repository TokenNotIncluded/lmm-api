//! Typed actors for legacy market credits. A resource token never becomes a
//! Session principal or a core_identity.credentials foreign key.
use super::{Error, Owner, Result, ToolMarketStore, credit_access::TokenRequest};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Postgres, Row, Transaction, types::Json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActorSource {
    Session {
        credential_id: i64,
    },
    Personal {
        token_id: String,
    },
    Oauth {
        family_id: String,
        issuer: String,
        resource: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketAction {
    Discover,
    Invoke,
    Manage,
    Delegate,
}
/// Constructed only by a live transaction authentication. All fields are
/// private so callers cannot fabricate payer identity or permission bits.
#[derive(Clone, Debug)]
pub struct MarketActor {
    user_id: i64,
    payer_account_id: i64,
    auth_version: i64,
    client_id: Option<String>,
    source: ActorSource,
    expires_at: i64,
    credential_expires_at: i64,
    can_invoke: bool,
    can_manage: bool,
}
impl MarketActor {
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn payer_account_id(&self) -> i64 {
        self.payer_account_id
    }
    pub fn auth_version(&self) -> i64 {
        self.auth_version
    }
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }
    pub fn source(&self) -> &ActorSource {
        &self.source
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn credential_expires_at(&self) -> i64 {
        self.credential_expires_at
    }
    pub fn can_invoke(&self) -> bool {
        self.can_invoke
    }
    pub fn can_manage(&self) -> bool {
        self.can_manage
    }
    pub fn owner(&self) -> Owner {
        Owner {
            user: self.user_id.to_string(),
            account: self.user_id.to_string(),
        }
    }
    pub fn credential_id(&self) -> Option<&str> {
        match &self.source {
            ActorSource::Personal { token_id } => Some(token_id),
            ActorSource::Oauth { family_id, .. } => Some(family_id),
            _ => None,
        }
    }
    pub fn credential_kind(&self) -> &'static str {
        match &self.source {
            ActorSource::Session { .. } => "session",
            ActorSource::Personal { .. } => "personal",
            ActorSource::Oauth { .. } => "oauth",
        }
    }
    pub fn assert_client(&self, client: &str) -> Result<()> {
        if client.is_empty() || client.len() > 128 || client.trim() != client {
            return Err(Error::Invalid);
        }
        if self.client_id.as_deref().is_some_and(|v| v != client) {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
    pub fn assert_owner(&self, owner: &Owner) -> Result<()> {
        if *owner != self.owner() {
            Err(Error::Forbidden)
        } else {
            Ok(())
        }
    }
}
fn oauth_error(e: crate::market_oauth::Error) -> Error {
    match e {
        crate::market_oauth::Error::Invalid => Error::Invalid,
        crate::market_oauth::Error::Unauthorized => Error::Unauthorized,
        crate::market_oauth::Error::Forbidden => Error::Forbidden,
        crate::market_oauth::Error::Conflict => Error::Conflict,
        crate::market_oauth::Error::Unavailable => Error::Storage,
    }
}

pub async fn authenticate(
    tx: &mut Transaction<'_, Postgres>,
    profile: Option<&crate::market_oauth::MarketProfile>,
    secret: &str,
    action: MarketAction,
) -> Result<MarketActor> {
    authenticate_connection(&mut **tx, profile, secret, action).await
}
pub async fn authenticate_connection(
    tx: &mut PgConnection,
    profile: Option<&crate::market_oauth::MarketProfile>,
    secret: &str,
    action: MarketAction,
) -> Result<MarketActor> {
    let now = sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(&mut *tx)
        .await?;
    let actor = if secret.starts_with("lmm_market_") {
        if secret.len() != 54
            || URL_SAFE_NO_PAD.decode(&secret[11..]).map(|b| b.len()).ok() != Some(32)
        {
            return Err(Error::Unauthorized);
        }
        let r=sqlx::query("SELECT t.id,t.actor_user_id,t.payer_account_id,t.auth_version,t.request,u.auth_version AS user_version FROM core_tool_market.credit_tokens t JOIN core_identity.users u ON u.id=t.actor_user_id JOIN core_identity.accounts a ON a.id=t.payer_account_id WHERE t.token_hash=$1 AND t.revoked_at=0 AND u.active AND a.active AND u.personal_account_id=t.payer_account_id FOR SHARE OF t,u,a").bind(Sha256::digest(secret.as_bytes()).to_vec()).fetch_optional(&mut *tx).await?.ok_or(Error::Unauthorized)?;
        let Json(request): Json<TokenRequest> = r.try_get("request")?;
        let user: i64 = r.try_get("actor_user_id")?;
        let version: i64 = r.try_get("auth_version")?;
        if version != r.try_get::<i64, _>("user_version")?
            || request.owner.user != user.to_string()
            || request.owner.account != user.to_string()
            || request.expires_at <= now
            || request.client_id.is_empty()
            || request.client_id.len() > 128
            || request.client_id.starts_with("oauth:")
        {
            return Err(Error::Unauthorized);
        }
        MarketActor {
            user_id: user,
            payer_account_id: r.try_get("payer_account_id")?,
            auth_version: version,
            client_id: Some(request.client_id),
            source: ActorSource::Personal {
                token_id: r.try_get("id")?,
            },
            expires_at: request.expires_at,
            credential_expires_at: request.expires_at,
            can_invoke: request.can_invoke,
            can_manage: request.can_manage,
        }
    } else if secret.starts_with("lmm_at_") {
        let p = profile.ok_or(Error::Storage)?;
        let principal =
            crate::market_oauth::validate_in_transaction(&mut *tx, p, secret, &["market:discover"])
                .await
                .map_err(oauth_error)?;
        // Delegation is bounded by the immutable family lifetime, not by the
        // short-lived access token currently used to read it.
        let family_expiry:i64=sqlx::query_scalar("SELECT floor(extract(epoch FROM expires_at))::bigint FROM core_market_oauth.families WHERE id=$1 FOR SHARE").bind(principal.family_id()).fetch_one(&mut *tx).await?;
        MarketActor {
            user_id: principal.user_id(),
            payer_account_id: principal.account_id(),
            auth_version: principal.auth_version(),
            client_id: Some(principal.client_id().to_owned()),
            source: ActorSource::Oauth {
                family_id: principal.family_id().to_owned(),
                issuer: principal.issuer().to_owned(),
                resource: principal.resource().to_owned(),
            },
            expires_at: principal.expires_at_unix(),
            credential_expires_at: family_expiry,
            can_invoke: principal.can_invoke(),
            can_manage: principal.can_manage(),
        }
    } else {
        let p = crate::identity::authorize_in_transaction(&mut *tx, secret).await?;
        if p.credential_kind != crate::identity::CredentialKind::Session
            || p.owner.kind != crate::accounts::AccountKind::Personal
            || p.owner.id != p.user_id
        {
            return Err(Error::Forbidden);
        }
        MarketActor {
            user_id: p.user_id,
            payer_account_id: p.owner_account_id,
            auth_version: sqlx::query_scalar(
                "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
            )
            .bind(p.user_id)
            .fetch_one(&mut *tx)
            .await?,
            client_id: None,
            source: ActorSource::Session {
                credential_id: p.credential_id,
            },
            expires_at: i64::MAX,
            credential_expires_at: i64::MAX,
            can_invoke: true,
            can_manage: true,
        }
    };
    if matches!(action, MarketAction::Invoke | MarketAction::Delegate) && !actor.can_invoke
        || matches!(action, MarketAction::Manage | MarketAction::Delegate) && !actor.can_manage
    {
        return Err(Error::Forbidden);
    }
    Ok(actor)
}

/// Resolve an owner-selected connection without trusting a family ID supplied
/// by MCP. OAuth setup applies atomically to all current eligible families.
pub async fn owner_targets(
    tx: &mut Transaction<'_, Postgres>,
    profile: Option<&crate::market_oauth::MarketProfile>,
    owner: &MarketActor,
    kind: &str,
    id: &str,
) -> Result<Vec<MarketActor>> {
    if !matches!(owner.source(), ActorSource::Session { .. }) {
        return Err(Error::Forbidden);
    }
    let now = ToolMarketStore::now(tx).await?;
    let mut targets = Vec::new();
    match kind {
        "personal" => {
            let r=sqlx::query("SELECT * FROM core_tool_market.credit_tokens WHERE id=$1 AND actor_user_id=$2 AND revoked_at=0 FOR SHARE").bind(id).bind(owner.user_id()).fetch_optional(&mut **tx).await?.ok_or(Error::Forbidden)?;
            let Json(request): Json<TokenRequest> = r.try_get("request")?;
            if r.try_get::<i64, _>("auth_version")? != owner.auth_version()
                || r.try_get::<i64, _>("payer_account_id")? != owner.payer_account_id()
                || request.expires_at <= now
                || !request.can_invoke
                || !request.can_manage
                || request.client_id.starts_with("oauth:")
            {
                return Err(Error::Forbidden);
            }
            targets.push(MarketActor {
                user_id: owner.user_id(),
                payer_account_id: owner.payer_account_id(),
                auth_version: owner.auth_version(),
                client_id: Some(request.client_id),
                source: ActorSource::Personal {
                    token_id: id.to_owned(),
                },
                expires_at: request.expires_at,
                credential_expires_at: request.expires_at,
                can_invoke: true,
                can_manage: true,
            });
        }
        "oauth" => {
            let p = profile.ok_or(Error::Storage)?;
            let client = id.strip_prefix("oauth:").ok_or(Error::Invalid)?;
            let rows=sqlx::query("SELECT f.id,f.client_id,extract(epoch FROM f.expires_at)::bigint AS expires FROM core_market_oauth.families f JOIN core_market_oauth.clients c ON c.id=f.client_id WHERE f.user_id=$1 AND f.account_id=$2 AND f.user_version=$3 AND f.client_id=$4 AND f.issuer=$5 AND f.resource=$6 AND f.revoked_at IS NULL AND c.revoked_at IS NULL AND c.issuer=f.issuer AND f.expires_at>clock_timestamp() AND f.scopes @> ARRAY['market:discover','market:invoke','market:manage']::text[] ORDER BY f.id LIMIT 101 FOR SHARE OF f,c").bind(owner.user_id()).bind(owner.payer_account_id()).bind(owner.auth_version()).bind(client).bind(&p.issuer).bind(&p.resource).fetch_all(&mut **tx).await?;
            if rows.is_empty() || rows.len() > 100 {
                return Err(Error::Forbidden);
            }
            for r in rows {
                let expiry: i64 = r.try_get("expires")?;
                targets.push(MarketActor {
                    user_id: owner.user_id(),
                    payer_account_id: owner.payer_account_id(),
                    auth_version: owner.auth_version(),
                    client_id: Some(format!("oauth:{}", r.try_get::<String, _>("client_id")?)),
                    source: ActorSource::Oauth {
                        family_id: r.try_get("id")?,
                        issuer: p.issuer.clone(),
                        resource: p.resource.clone(),
                    },
                    expires_at: expiry,
                    credential_expires_at: expiry,
                    can_invoke: true,
                    can_manage: true,
                });
            }
        }
        _ => return Err(Error::Invalid),
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn market_actor_scope_cannot_select_another_client_or_payer() {
        let a = MarketActor {
            user_id: 7,
            payer_account_id: 101,
            auth_version: 3,
            client_id: Some("oauth:exact".into()),
            source: ActorSource::Oauth {
                family_id: "family".into(),
                issuer: "https://example.test/api/oauth2/market".into(),
                resource: "https://example.test/mcp/market".into(),
            },
            expires_at: 100,
            credential_expires_at: 200,
            can_invoke: true,
            can_manage: false,
        };
        assert!(a.assert_client("oauth:exact").is_ok());
        assert!(a.assert_client("oauth:Exact").is_err());
        assert!(a.assert_client("other").is_err());
        assert_eq!(
            a.owner(),
            Owner {
                user: "7".into(),
                account: "7".into()
            }
        );
        assert_eq!(a.payer_account_id(), 101);
        assert!(
            a.assert_owner(&Owner {
                user: "7".into(),
                account: "101".into()
            })
            .is_err()
        );
        assert_eq!(a.credential_kind(), "oauth");
        assert_eq!(a.credential_id(), Some("family"));
        assert_eq!(a.credential_expires_at(), 200);
    }
    #[test]
    fn actor_source_is_a_closed_distinct_foreign_key_union() {
        let sources = [
            ActorSource::Session { credential_id: 9 },
            ActorSource::Personal {
                token_id: "personal".into(),
            },
            ActorSource::Oauth {
                family_id: "family".into(),
                issuer: "issuer".into(),
                resource: "resource".into(),
            },
        ];
        for s in sources {
            assert_eq!(
                serde_json::from_slice::<ActorSource>(&serde_json::to_vec(&s).unwrap()).unwrap(),
                s
            )
        }
        for raw in [
            r#"{"kind":"session","credential_id":9,"family_id":"family"}"#,
            r#"{"kind":"oauth","credential_id":9}"#,
            r#"{"kind":"universal","token_id":"personal"}"#,
        ] {
            assert!(serde_json::from_str::<ActorSource>(raw).is_err())
        }
    }
}
