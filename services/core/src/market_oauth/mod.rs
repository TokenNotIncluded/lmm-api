//! Resource-specific market authorization. Tokens never acquire model authority.
pub mod http;
mod protocol;
use crate::identity::{self, BrowserAuth, IdentityError, IdentityStore};
use protocol::*;
pub use protocol::{
    AuthorizationRequest, MarketProfile, RegisteredClient, Registration, TokenForm, TokenWire,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, postgres::PgRow};
pub const SCHEMA: &str = include_str!("../../schema/market_oauth.sql");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Conflict,
    Unavailable,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if e.as_database_error()
            .is_some_and(|d| matches!(d.code().as_deref(), Some("23505" | "40001" | "40P01")))
        {
            Self::Conflict
        } else {
            Self::Unavailable
        }
    }
}
impl From<IdentityError> for Error {
    fn from(e: IdentityError) -> Self {
        match e {
            IdentityError::Invalid => Self::Invalid,
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            IdentityError::Conflict => Self::Conflict,
            IdentityError::Storage => Self::Unavailable,
        }
    }
}
#[derive(Clone)]
pub struct MarketIssuer {
    pool: PgPool,
    profile: MarketProfile,
    identity: IdentityStore,
}
/// Only this module can construct a market principal after current token checks.
#[derive(Clone, Serialize)]
pub struct MarketPrincipal {
    user_id: i64,
    account_id: i64,
    client_id: String,
    credential_id: String,
    auth_version: i64,
    expires_at_unix: i64,
    can_invoke: bool,
    can_manage: bool,
    issuer: String,
    resource: String,
    budget_credits: i64,
}
impl MarketPrincipal {
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn account_id(&self) -> i64 {
        self.account_id
    }
    pub fn family_id(&self) -> &str {
        &self.credential_id
    }
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
    pub fn budget_credits(&self) -> i64 {
        self.budget_credits
    }
    pub fn auth_version(&self) -> i64 {
        self.auth_version
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn resource(&self) -> &str {
        &self.resource
    }
    pub fn expires_at_unix(&self) -> i64 {
        self.expires_at_unix
    }
    pub fn can_invoke(&self) -> bool {
        self.can_invoke
    }
    pub fn can_manage(&self) -> bool {
        self.can_manage
    }
}
pub struct ConsentPage {
    pub binding: String,
    pub csrf: String,
    pub account: Option<String>,
    pub client_name: String,
    pub resource: String,
    pub scopes: Vec<String>,
    pub maximum_budget_credits: i64,
}
async fn tx(pool: &PgPool) -> Result<Transaction<'_, Postgres>> {
    let mut t = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout='2s'")
        .execute(&mut *t)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='3s'")
        .execute(&mut *t)
        .await?;
    Ok(t)
}
impl MarketIssuer {
    pub async fn open(pool: PgPool, profile: MarketProfile) -> Result<Self> {
        sqlx::query("SELECT digest FROM core_market_oauth.tokens LIMIT 0")
            .execute(&pool)
            .await?;
        Ok(Self {
            identity: IdentityStore::from_pool(pool.clone()),
            pool,
            profile,
        })
    }
    pub fn profile(&self) -> &MarketProfile {
        &self.profile
    }
    pub async fn rate_limit(&self, peer: &str) -> Result<bool> {
        Ok(self
            .identity
            .auth_rate_limit("market_oauth_peer", peer, 60)
            .await?)
    }
    pub async fn register(&self, mut input: Registration) -> Result<RegisteredClient> {
        let templates = input.validate()?;
        let scopes = scopes(&input.scope)?;
        let id = format!(
            "lmm-mcp-{}",
            hash(
                &serde_json::to_string(&(
                    &self.profile.issuer,
                    &input.client_name,
                    &templates,
                    &input.scope,
                    input.grant_types.len() == 2
                ))
                .map_err(|_| Error::Invalid)?
            )
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
        );
        let mut t = tx(&self.pool).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(72753290313)")
            .execute(&mut *t)
            .await?;
        let existing:Option<i64>=sqlx::query_scalar("SELECT created_at FROM core_market_oauth.clients WHERE id=$1 AND issuer=$2 AND revoked_at IS NULL").bind(&id).bind(&self.profile.issuer).fetch_optional(&mut *t).await?;
        let created = if let Some(at) = existing {
            at
        } else {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM core_market_oauth.clients WHERE issuer=$1",
            )
            .bind(&self.profile.issuer)
            .fetch_one(&mut *t)
            .await?;
            if count >= 4096 {
                return Err(Error::Forbidden);
            }
            sqlx::query_scalar("INSERT INTO core_market_oauth.clients(id,issuer,metadata,redirect_templates,scopes,refresh_enabled,created_at) VALUES($1,$2,$3,$4,$5,$6,extract(epoch FROM clock_timestamp())::bigint) RETURNING created_at").bind(&id).bind(&self.profile.issuer).bind(serde_json::to_value(&input).map_err(|_|Error::Invalid)?).bind(templates).bind(scopes).bind(input.grant_types.len()==2).fetch_one(&mut *t).await?
        };
        t.commit().await?;
        Ok(RegisteredClient {
            client_id: id,
            client_id_issued_at: created,
            metadata: input,
        })
    }
    async fn client(&self, c: &mut PgConnection, id: &str) -> Result<PgRow> {
        canonical(id, "lmm-mcp-")?;
        sqlx::query("SELECT * FROM core_market_oauth.clients WHERE id=$1 AND issuer=$2 AND revoked_at IS NULL FOR SHARE").bind(id).bind(&self.profile.issuer).fetch_optional(c).await?.ok_or(Error::Unauthorized)
    }
    async fn pending(
        &self,
        c: &mut PgConnection,
        request: &AuthorizationRequest,
        actor: Option<(i64, i64, i64, String)>,
        predecessor: Option<&str>,
    ) -> Result<ConsentPage> {
        let requested = request.validate(&self.profile)?;
        let client = self.client(c, &request.client_id).await?;
        let allowed: Vec<String> = client.try_get("scopes")?;
        let redirects: Vec<String> = client.try_get("redirect_templates")?;
        if requested.iter().any(|s| !allowed.contains(s))
            || !redirects.contains(&redirect(&request.redirect_uri)?)
        {
            return Err(Error::Forbidden);
        }
        let binding = secret("");
        let csrf = secret("");
        let inserted=sqlx::query("INSERT INTO core_market_oauth.requests(id,csrf,client_id,issuer,resource,redirect_uri,state,challenge,scopes,user_id,account_id,user_version,sid) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13 WHERE $14::bytea IS NULL OR EXISTS(SELECT 1 FROM core_market_oauth.requests WHERE id=$14 AND consumed_at IS NOT NULL AND expires_at>clock_timestamp())").bind(hash(&binding)).bind(hash(&csrf)).bind(&request.client_id).bind(&self.profile.issuer).bind(&self.profile.resource).bind(&request.redirect_uri).bind(&request.state).bind(&request.code_challenge).bind(&requested).bind(actor.as_ref().map(|a|a.0)).bind(actor.as_ref().map(|a|a.1)).bind(actor.as_ref().map(|a|a.2)).bind(actor.as_ref().map(|a|a.3.as_str())).bind(predecessor.map(hash)).execute(&mut *c).await?;
        if inserted.rows_affected() != 1 {
            return Err(Error::Unauthorized);
        }
        let account = if let Some(a) = actor {
            Some(
                sqlx::query_scalar(
                    "SELECT username FROM core_identity.user_profiles WHERE user_id=$1",
                )
                .bind(a.0)
                .fetch_one(&mut *c)
                .await?,
            )
        } else {
            None
        };
        let metadata: serde_json::Value = client.try_get("metadata")?;
        let client_name = metadata
            .get("client_name")
            .and_then(|v| v.as_str())
            .ok_or(Error::Unavailable)?
            .to_owned();
        Ok(ConsentPage {
            binding,
            csrf,
            account,
            client_name,
            resource: self.profile.resource.clone(),
            scopes: requested,
            maximum_budget_credits: self.profile.max_budget,
        })
    }
    pub async fn begin(&self, request: AuthorizationRequest) -> Result<ConsentPage> {
        let mut t = tx(&self.pool).await?;
        let page = self.pending(&mut t, &request, None, None).await?;
        t.commit().await?;
        Ok(page)
    }
    async fn consume(&self, c: &mut PgConnection, binding: &str, csrf: &str) -> Result<PgRow> {
        canonical(binding, "")?;
        canonical(csrf, "")?;
        sqlx::query("UPDATE core_market_oauth.requests SET consumed_at=clock_timestamp() WHERE id=$1 AND csrf=$2 AND issuer=$3 AND resource=$4 AND consumed_at IS NULL AND expires_at>clock_timestamp() RETURNING *").bind(hash(binding)).bind(hash(csrf)).bind(&self.profile.issuer).bind(&self.profile.resource).fetch_optional(c).await?.ok_or(Error::Unauthorized)
    }
    fn original(r: &PgRow) -> Result<AuthorizationRequest> {
        Ok(AuthorizationRequest {
            response_type: "code".into(),
            client_id: r.try_get("client_id")?,
            redirect_uri: r.try_get("redirect_uri")?,
            scope: r.try_get::<Vec<String>, _>("scopes")?.join(" "),
            resource: r.try_get("resource")?,
            code_challenge: r.try_get("challenge")?,
            code_challenge_method: "S256".into(),
            state: r.try_get("state")?,
        })
    }
    pub async fn continue_authorization(
        &self,
        binding: &str,
        csrf: &str,
        cookie: &str,
    ) -> Result<ConsentPage> {
        let mut t = tx(&self.pool).await?;
        let row = self.consume(&mut t, binding, csrf).await?;
        if row.try_get::<Option<i64>, _>("user_id")?.is_some() {
            t.commit().await?;
            return Err(Error::Unauthorized);
        }
        let actor = match BrowserAuth::oauth_identity_in_transaction(&mut t, cookie).await {
            Ok(a) => a,
            Err(e) => {
                t.commit().await?;
                return Err(e.into());
            }
        };
        let result = self
            .pending(
                &mut t,
                &Self::original(&row)?,
                Some((actor.user, actor.account, actor.version, actor.sid)),
                Some(binding),
            )
            .await;
        let page = match result {
            Ok(p) => p,
            Err(e @ (Error::Unauthorized | Error::Forbidden)) => {
                t.commit().await?;
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        t.commit().await?;
        Ok(page)
    }
    pub async fn consent(
        &self,
        binding: &str,
        csrf: &str,
        cookie: &str,
        allow: bool,
        budget: i64,
    ) -> Result<String> {
        let mut t = tx(&self.pool).await?;
        let r = self.consume(&mut t, binding, csrf).await?;
        let actor = match BrowserAuth::oauth_identity_in_transaction(&mut t, cookie).await {
            Ok(a) => a,
            Err(e) => {
                t.commit().await?;
                return Err(e.into());
            }
        };
        if r.try_get::<Option<i64>, _>("user_id")? != Some(actor.user)
            || r.try_get::<Option<i64>, _>("account_id")? != Some(actor.account)
            || r.try_get::<Option<i64>, _>("user_version")? != Some(actor.version)
            || r.try_get::<Option<String>, _>("sid")?.as_deref() != Some(&actor.sid)
        {
            t.commit().await?;
            return Err(Error::Unauthorized);
        }
        let client = self
            .client(&mut t, &r.try_get::<String, _>("client_id")?)
            .await?;
        let requested: Vec<String> = r.try_get("scopes")?;
        let invokes = requested.iter().any(|s| s == "market:invoke");
        if allow
            && (budget < 0
                || budget > self.profile.max_budget
                || (invokes && budget == 0)
                || (!invokes && budget != 0))
        {
            t.commit().await?;
            return Err(Error::Invalid);
        }
        if allow && (invokes || requested.iter().any(|s| s == "market:manage")) {
            identity::reject_marked_new_action(
                &mut t,
                actor.user,
                identity::NewResourceAction::RemoteExecution,
            )
            .await?
        }
        let mut target = reqwest::Url::parse(&r.try_get::<String, _>("redirect_uri")?)
            .map_err(|_| Error::Unavailable)?;
        let live: bool = sqlx::query_scalar(
            "SELECT expires_at>clock_timestamp() FROM core_market_oauth.requests WHERE id=$1",
        )
        .bind(hash(binding))
        .fetch_one(&mut *t)
        .await?;
        if !live {
            t.commit().await?;
            return Err(Error::Unauthorized);
        }
        if allow {
            let family = secret("lmm_mg_");
            let code = secret("lmm_mc_");
            sqlx::query("INSERT INTO core_market_oauth.families(id,client_id,issuer,resource,user_id,account_id,user_version,scopes,budget_credits,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,clock_timestamp()+interval '30 days')").bind(&family).bind(client.try_get::<String,_>("id")?).bind(&self.profile.issuer).bind(&self.profile.resource).bind(actor.user).bind(actor.account).bind(actor.version).bind(requested).bind(budget).execute(&mut *t).await?;
            sqlx::query("INSERT INTO core_market_oauth.codes(digest,family_id,redirect_uri,challenge,expires_at) VALUES($1,$2,$3,$4,clock_timestamp()+interval '120 seconds')").bind(hash(&code)).bind(family).bind(r.try_get::<String,_>("redirect_uri")?).bind(r.try_get::<String,_>("challenge")?).execute(&mut *t).await?;
            target.query_pairs_mut().append_pair("code", &code);
        } else {
            target
                .query_pairs_mut()
                .append_pair("error", "access_denied");
        }
        target
            .query_pairs_mut()
            .append_pair("state", &r.try_get::<String, _>("state")?)
            .append_pair("iss", &self.profile.issuer);
        t.commit().await?;
        Ok(target.into())
    }
    async fn family(&self, c: &mut PgConnection, id: &str, client: &str) -> Result<PgRow> {
        self.client(c, client).await?;
        sqlx::query("SELECT f.*,extract(epoch FROM f.expires_at)::bigint AS expires FROM core_market_oauth.families f JOIN core_identity.users u ON u.id=f.user_id JOIN core_identity.accounts a ON a.id=f.account_id WHERE f.id=$1 AND f.client_id=$2 AND f.issuer=$3 AND f.resource=$4 AND f.revoked_at IS NULL AND f.expires_at>clock_timestamp() AND u.active AND a.active AND u.auth_version=f.user_version AND u.personal_account_id=f.account_id FOR UPDATE OF f FOR SHARE OF u,a").bind(id).bind(client).bind(&self.profile.issuer).bind(&self.profile.resource).fetch_optional(c).await?.ok_or(Error::Unauthorized)
    }
    async fn issue(&self, c: &mut PgConnection, f: &PgRow, scope: &[String]) -> Result<TokenWire> {
        let access = secret("lmm_at_");
        let refresh = secret("lmm_rt_");
        let expires_in:i64=sqlx::query_scalar("INSERT INTO core_market_oauth.tokens(digest,family_id,kind,scopes,expires_at) SELECT $1,id,'access',$3,LEAST(expires_at,clock_timestamp()+interval '600 seconds') FROM core_market_oauth.families WHERE id=$2 AND expires_at>clock_timestamp() AND revoked_at IS NULL RETURNING greatest(0,floor(extract(epoch FROM expires_at-clock_timestamp())))::bigint").bind(hash(&access)).bind(f.try_get::<String,_>("id")?).bind(scope).fetch_optional(&mut *c).await?.ok_or(Error::Unauthorized)?;
        if expires_in <= 0 {
            return Err(Error::Unauthorized);
        }
        let cl = self
            .client(c, &f.try_get::<String, _>("client_id")?)
            .await?;
        let enabled: bool = cl.try_get("refresh_enabled")?;
        if enabled {
            sqlx::query("INSERT INTO core_market_oauth.tokens(digest,family_id,kind,scopes,expires_at) SELECT $1,id,'refresh',$3,expires_at FROM core_market_oauth.families WHERE id=$2").bind(hash(&refresh)).bind(f.try_get::<String,_>("id")?).bind(scope).execute(c).await?;
        }
        Ok(TokenWire {
            access_token: access,
            refresh_token: enabled.then_some(refresh),
            token_type: "Bearer",
            expires_in,
            scope: scope.join(" "),
        })
    }
    pub async fn token(&self, input: TokenForm) -> Result<TokenWire> {
        if input.resource != self.profile.resource {
            return Err(Error::Unauthorized);
        }
        canonical(&input.client_id, "lmm-mcp-")?;
        let mut t = tx(&self.pool).await?;
        let (family, scope) = match input.grant_type.as_str() {
            "authorization_code" if input.refresh_token.is_none() && input.scope.is_none() => {
                let code = input.code.as_deref().ok_or(Error::Invalid)?;
                canonical(code, "lmm_mc_")?;
                let r=sqlx::query("SELECT * FROM core_market_oauth.codes WHERE digest=$1 AND consumed_at IS NULL AND expires_at>clock_timestamp() FOR UPDATE").bind(hash(code)).fetch_optional(&mut *t).await?.ok_or(Error::Unauthorized)?;
                let f = self
                    .family(
                        &mut t,
                        &r.try_get::<String, _>("family_id")?,
                        &input.client_id,
                    )
                    .await?;
                if input.redirect_uri.as_deref()
                    != Some(r.try_get::<String, _>("redirect_uri")?.as_str())
                {
                    return Err(Error::Unauthorized);
                }
                verify(
                    &r.try_get::<String, _>("challenge")?,
                    input.code_verifier.as_deref().ok_or(Error::Invalid)?,
                )?;
                if sqlx::query("UPDATE core_market_oauth.codes SET consumed_at=clock_timestamp() WHERE digest=$1 AND consumed_at IS NULL AND expires_at>clock_timestamp()").bind(hash(code)).execute(&mut *t).await?.rows_affected()!=1{return Err(Error::Unauthorized)}
                let scope: Vec<String> = f.try_get("scopes")?;
                (f, scope)
            }
            "refresh_token"
                if input.code.is_none()
                    && input.redirect_uri.is_none()
                    && input.code_verifier.is_none() =>
            {
                let token = input.refresh_token.as_deref().ok_or(Error::Invalid)?;
                canonical(token, "lmm_rt_")?;
                let r=sqlx::query("SELECT *,consumed_at IS NOT NULL AS used FROM core_market_oauth.tokens WHERE digest=$1 AND kind='refresh' AND expires_at>clock_timestamp() FOR UPDATE").bind(hash(token)).fetch_optional(&mut *t).await?.ok_or(Error::Unauthorized)?;
                let f = self
                    .family(
                        &mut t,
                        &r.try_get::<String, _>("family_id")?,
                        &input.client_id,
                    )
                    .await?;
                if r.try_get::<bool, _>("used")? {
                    sqlx::query("UPDATE core_market_oauth.families SET revoked_at=clock_timestamp() WHERE id=$1").bind(f.try_get::<String,_>("id")?).execute(&mut *t).await?;
                    t.commit().await?;
                    return Err(Error::Unauthorized);
                }
                let previous: Vec<String> = r.try_get("scopes")?;
                let scope = match input.scope.as_deref() {
                    None => previous.clone(),
                    Some(s) => scopes(s)?,
                };
                if scope.iter().any(|s| !previous.contains(s)) {
                    return Err(Error::Forbidden);
                }
                if sqlx::query("UPDATE core_market_oauth.tokens SET consumed_at=clock_timestamp() WHERE digest=$1 AND consumed_at IS NULL AND expires_at>clock_timestamp()").bind(hash(token)).execute(&mut *t).await?.rows_affected()!=1{return Err(Error::Unauthorized)}
                (f, scope)
            }
            _ => return Err(Error::Invalid),
        };
        let token = self.issue(&mut t, &family, &scope).await?;
        t.commit().await?;
        Ok(token)
    }
    pub async fn revoke(&self, client: &str, token: &str) -> Result<()> {
        canonical(client, "lmm-mcp-")?;
        if token.starts_with("lmm_at_") {
            canonical(token, "lmm_at_")?
        } else {
            canonical(token, "lmm_rt_")?
        }
        let mut t = tx(&self.pool).await?;
        sqlx::query("UPDATE core_market_oauth.families f SET revoked_at=clock_timestamp() FROM core_market_oauth.tokens t WHERE t.digest=$1 AND f.id=t.family_id AND f.client_id=$2 AND f.issuer=$3 AND f.resource=$4 AND f.revoked_at IS NULL").bind(hash(token)).bind(client).bind(&self.profile.issuer).bind(&self.profile.resource).execute(&mut *t).await?;
        t.commit().await?;
        Ok(())
    }
    pub async fn validate(&self, token: &str, required: &[&str]) -> Result<MarketPrincipal> {
        let mut t = tx(&self.pool).await?;
        let a = validate_in_transaction(&mut t, &self.profile, token, required).await?;
        t.commit().await?;
        Ok(a)
    }
}
pub async fn validate_in_transaction(
    c: &mut PgConnection,
    p: &MarketProfile,
    token: &str,
    required: &[&str],
) -> Result<MarketPrincipal> {
    canonical(token, "lmm_at_")?;
    let needs = scopes(&required.join(" "))?;
    let r=sqlx::query("SELECT f.*,t.scopes AS token_scopes,extract(epoch FROM LEAST(t.expires_at,f.expires_at))::bigint AS expires FROM core_market_oauth.tokens t JOIN core_market_oauth.families f ON f.id=t.family_id JOIN core_market_oauth.clients cl ON cl.id=f.client_id JOIN core_identity.users u ON u.id=f.user_id JOIN core_identity.accounts a ON a.id=f.account_id WHERE t.digest=$1 AND t.kind='access' AND t.expires_at>clock_timestamp() AND f.expires_at>clock_timestamp() AND f.revoked_at IS NULL AND cl.revoked_at IS NULL AND f.issuer=$2 AND cl.issuer=$2 AND f.resource=$3 AND u.active AND a.active AND u.auth_version=f.user_version AND u.personal_account_id=f.account_id FOR SHARE OF f,cl,u,a").bind(hash(token)).bind(&p.issuer).bind(&p.resource).fetch_optional(c).await?.ok_or(Error::Unauthorized)?;
    let scope: Vec<String> = r.try_get("token_scopes")?;
    let grant: Vec<String> = r.try_get("scopes")?;
    if scope.iter().any(|s| !grant.contains(s)) || needs.iter().any(|s| !scope.contains(s)) {
        return Err(Error::Forbidden);
    }
    Ok(MarketPrincipal {
        user_id: r.try_get("user_id")?,
        account_id: r.try_get("account_id")?,
        client_id: format!("oauth:{}", r.try_get::<String, _>("client_id")?),
        credential_id: r.try_get("id")?,
        auth_version: r.try_get("user_version")?,
        expires_at_unix: r.try_get("expires")?,
        can_invoke: scope.iter().any(|s| s == "market:invoke"),
        can_manage: scope.iter().any(|s| s == "market:manage"),
        issuer: p.issuer.clone(),
        resource: p.resource.clone(),
        budget_credits: r.try_get("budget_credits")?,
    })
}
/// Caller authenticates the current personal Session and revokes its market
/// delegations/consents in the same transaction. No unrelated family is touched.
pub async fn revoke_client_in_transaction(
    c: &mut PgConnection,
    p: &MarketProfile,
    user: i64,
    client: &str,
) -> Result<Vec<String>> {
    let client = client.strip_prefix("oauth:").ok_or(Error::Invalid)?;
    canonical(client, "lmm-mcp-")?;
    sqlx::query_scalar("UPDATE core_market_oauth.families SET revoked_at=clock_timestamp() WHERE user_id=$1 AND client_id=$2 AND issuer=$3 AND resource=$4 AND revoked_at IS NULL RETURNING id").bind(user).bind(client).bind(&p.issuer).bind(&p.resource).fetch_all(c).await.map_err(Into::into)
}
