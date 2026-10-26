//! Core-owned legacy consent/budget/token authority. Never accepts Go balance,
//! seller, fee, platform payer or successful ledger assertions.
use super::{
    credits::{self, MAX_CREDITS},
    *,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use sqlx::postgres::PgRow;
macro_rules! request {
    ($name:ident {$($field:ident:$ty:ty),* $(,)?})=>{
        #[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
        #[serde(deny_unknown_fields)]
        pub struct $name {pub owner:Owner,pub operation_id:String,$(pub $field:$ty),*}
    }
}
request!(ConsentRequest {
    client_id: String,
    tool_id: String,
    version_id: String,
    max_price_quota: i64,
    max_total_quota: i64,
    max_calls: i64,
    expires_at: i64
});
request!(IdRequest { id: String });
request!(BudgetRequest {
    scope: String,
    scope_id: String,
    limit_quota: i64
});
request!(TokenRequest {
    client_id: String,
    can_invoke: bool,
    can_manage: bool,
    expires_at: i64
});
request!(ClientRequest { client_id: String });
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApproveRequest {
    pub operation_id: String,
    pub quote_id: String,
    pub visibility: String,
    pub approved: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApproveReceipt {
    pub operation_id: String,
    pub quote_id: String,
    pub visibility: String,
    pub approved: bool,
    pub approved_by: String,
    pub approved_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsentReceipt {
    #[serde(flatten)]
    pub request: ConsentRequest,
    pub quote_id: String,
    pub frozen_price_quota: i64,
    pub rules_hash: String,
    pub id: String,
    pub created_at: i64,
    pub revoked_at: i64,
    pub reserved_quota: i64,
    pub spent_quota: i64,
    pub reserved_calls: i64,
    pub successful_calls: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetReceipt {
    #[serde(flatten)]
    pub request: BudgetRequest,
    pub reserved_quota: i64,
    pub spent_quota: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenReceipt {
    #[serde(flatten)]
    pub request: TokenRequest,
    pub id: String,
    pub auth_version: i64,
    pub created_at: i64,
    pub revoked_at: i64,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub token: String,
    pub replayed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevocationReceipt {
    #[serde(flatten)]
    pub request: IdRequest,
    pub revoked_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisconnectReceipt {
    #[serde(flatten)]
    pub request: ClientRequest,
    pub tokens_revoked: i64,
    pub grants_revoked: i64,
    pub revoked_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    pub owner: Owner,
    pub offset: i64,
    pub limit: i64,
}
#[derive(Serialize)]
pub struct ListReceipt<T: Serialize> {
    pub owner: Owner,
    pub rows: Vec<T>,
}
#[derive(Clone)]
pub struct CreditStore {
    pub(super) base: ToolMarketStore,
    pub(super) market_profile: Option<std::sync::Arc<crate::market_oauth::MarketProfile>>,
    pub(super) verifier: Option<std::sync::Arc<super::receipt_verifier::ReceiptVerifier>>,
}
fn client(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && v.trim() == v
}
fn quota(v: i64) -> bool {
    (0..=MAX_CREDITS).contains(&v)
}
impl CreditStore {
    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            base: ToolMarketStore::from_pool(pool),
            verifier: None,
            market_profile: None,
        }
    }
    pub fn with_market_oauth_profile(
        mut self,
        profile: std::sync::Arc<crate::market_oauth::MarketProfile>,
    ) -> Self {
        self.market_profile = Some(profile);
        self
    }
    pub fn with_receipt_verifier(
        mut self,
        verifier: std::sync::Arc<super::receipt_verifier::ReceiptVerifier>,
    ) -> Self {
        self.verifier = Some(verifier);
        self
    }
    pub(super) async fn owned(
        tx: &mut Transaction<'_, Postgres>,
        secret: &str,
        owner: &Owner,
    ) -> Result<Principal> {
        let p = ToolMarketStore::actor(tx, secret).await?;
        if owner.user != p.user_id.to_string() || owner.account != p.user_id.to_string() {
            return Err(Error::Forbidden);
        }
        Ok(p)
    }
    async fn admit(
        &self,
        secret: &str,
        owner: &Owner,
        operation: &str,
    ) -> Result<(Transaction<'static, Postgres>, Principal)> {
        if !name(operation) {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = Self::owned(&mut tx, secret, owner).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("credit-access:{}:{operation}", actor.user_id))
            .execute(&mut *tx)
            .await?;
        Ok((tx, actor))
    }
    pub(super) async fn prior<T: Serialize>(
        tx: &mut Transaction<'_, Postgres>,
        actor: i64,
        op: &str,
        kind: &str,
        req: &T,
    ) -> Result<Option<PgRow>> {
        let row=sqlx::query("SELECT * FROM core_tool_market.credit_access_operations WHERE actor_user_id=$1 AND operation_id=$2").bind(actor).bind(op).fetch_optional(&mut **tx).await?;
        if let Some(r) = &row
            && (r.try_get::<String, _>("kind")? != kind
                || r.try_get::<String, _>("request_hash")? != hash(req)?)
        {
            return Err(Error::Conflict);
        }
        Ok(row)
    }
    // Actor, operation, kind, target and details are separate ledger columns.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record<T: Serialize>(
        tx: &mut Transaction<'_, Postgres>,
        actor: i64,
        op: &str,
        kind: &str,
        req: &T,
        target: &str,
        now: i64,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO core_tool_market.credit_access_operations(actor_user_id,operation_id,kind,request_hash,target_id,created_at,details) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(actor).bind(op).bind(kind).bind(hash(req)?).bind(target).bind(now).bind(details).execute(&mut **tx).await?;
        Ok(())
    }
    pub async fn register_quote(&self, secret: &str, q: &credits::Quote) -> Result<credits::Quote> {
        if !digest(&q.id)
            || !digest(&q.remote_digest)
            || !digest(&q.pricing_digest)
            || q.pricing_digest != hash(&q.price)?
            || ![&q.service_id, &q.tool_id, &q.version_id]
                .into_iter()
                .all(|v| name(v))
        {
            return Err(Error::Invalid);
        }
        if q.id
            != hash(&[
                "legacy-credit-quote-v2",
                &q.version_id,
                &q.tool_id,
                &q.remote_digest,
                &q.pricing_digest,
            ])?
        {
            return Err(Error::Invalid);
        }
        let ceiling = q.price.ceiling()?;
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("credit-service:{}", q.service_id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO core_tool_market.credit_services(service_id,owner_user_id,updated_by,updated_at) VALUES($1,$2,$2,extract(epoch FROM clock_timestamp())::BIGINT) ON CONFLICT(owner_user_id,service_id) DO NOTHING").bind(&q.service_id).bind(actor.user_id).execute(&mut *tx).await?;
        let service_owner:i64=sqlx::query_scalar("SELECT owner_user_id FROM core_tool_market.credit_services WHERE service_id=$1 AND owner_user_id=$2 FOR SHARE").bind(&q.service_id).bind(actor.user_id).fetch_one(&mut *tx).await?;
        if service_owner != actor.user_id {
            return Err(Error::Forbidden);
        }
        Self::check_service(&mut tx, actor.user_id, &q.service_id).await?;
        let now = ToolMarketStore::now(&mut tx).await?;
        if q.expires <= now {
            return Err(Error::Invalid);
        }
        let enabled: Option<bool> =
            sqlx::query_scalar("SELECT active FROM core_tool_market.publishers WHERE user_id=$1")
                .bind(actor.user_id)
                .fetch_optional(&mut *tx)
                .await?;
        if enabled != Some(true) {
            return Err(Error::Forbidden);
        }
        let policy =
            sqlx::query("SELECT * FROM core_tool_market.credit_policy WHERE singleton FOR SHARE")
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::Missing)?;
        sqlx::query("INSERT INTO core_tool_market.credit_quotes(id,seller_user_id,seller_account_id,platform_account_id,fee_bps,policy_version,snapshot,ceiling_credits) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(id) DO NOTHING").bind(&q.id).bind(actor.user_id).bind(actor.owner_account_id).bind(policy.try_get::<Option<i64>,_>("platform_account_id")?).bind(policy.try_get::<i64,_>("fee_bps")?).bind(policy.try_get::<i64,_>("version")?).bind(Json(q)).bind(ceiling).execute(&mut *tx).await?;
        let row = sqlx::query(
            "SELECT snapshot,seller_user_id FROM core_tool_market.credit_quotes WHERE id=$1",
        )
        .bind(&q.id)
        .fetch_one(&mut *tx)
        .await?;
        let Json(saved): Json<credits::Quote> = row.try_get("snapshot")?;
        if saved != *q || row.try_get::<Option<i64>, _>("seller_user_id")? != Some(actor.user_id) {
            return Err(Error::Conflict);
        }
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn approve_quote(&self, secret: &str, r: &ApproveRequest) -> Result<ApproveReceipt> {
        if !name(&r.operation_id)
            || !digest(&r.quote_id)
            || !matches!(r.visibility.as_str(), "public" | "private")
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "credit-access:{}:{}",
                actor.user_id, r.operation_id
            ))
            .execute(&mut *tx)
            .await?;
        let prior =
            Self::prior(&mut tx, actor.user_id, &r.operation_id, "quote-approve", r).await?;
        let row =
            sqlx::query("SELECT * FROM core_tool_market.credit_quotes WHERE id=$1 FOR UPDATE")
                .bind(&r.quote_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::Missing)?;
        let seller: i64 = row.try_get("seller_user_id")?;
        let role: String = sqlx::query_scalar(
            "SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *tx)
        .await?;
        let admin = matches!(role.as_str(), "admin" | "superadmin");
        if r.approved && r.visibility == "private" && row.try_get::<bool, _>("suspended")? {
            return Err(Error::Forbidden);
        }
        if (r.approved && r.visibility == "public" && !admin)
            || (r.approved
                && r.visibility == "private"
                && (seller != actor.user_id || row.try_get::<i64, _>("ceiling_credits")? != 0))
            || (!r.approved && seller != actor.user_id && !admin)
        {
            return Err(Error::Forbidden);
        }
        if admin
            && crate::identity::has_privileged_import_facts(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Storage)?
        {
            crate::identity::load_complete_policy(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Forbidden)?;
        }
        let now = ToolMarketStore::now(&mut tx).await?;

        let out = if prior.is_some() {
            if r.approved {
                let Json(q): Json<credits::Quote> = row.try_get("snapshot")?;
                Self::check_version(&mut tx, seller, &q.service_id, &q.version_id).await?;
            }
            if row.try_get::<bool, _>("approved")? != r.approved
                || row.try_get::<String, _>("visibility")? != r.visibility
            {
                return Err(Error::Conflict);
            }
            ApproveReceipt {
                operation_id: r.operation_id.clone(),
                quote_id: r.quote_id.clone(),
                visibility: r.visibility.clone(),
                approved: r.approved,
                approved_by: row.try_get::<i64, _>("approved_by")?.to_string(),
                approved_at: row.try_get("approved_at")?,
            }
        } else {
            if r.approved {
                let Json(q): Json<credits::Quote> = row.try_get("snapshot")?;
                Self::check_service(&mut tx, seller, &q.service_id).await?;
                sqlx::query("UPDATE core_tool_market.credit_services SET active_version_id=$3,revision=revision+1,updated_by=$4,updated_at=$5 WHERE owner_user_id=$1 AND service_id=$2 AND active_version_id<>$3").bind(seller).bind(&q.service_id).bind(&q.version_id).bind(actor.user_id).bind(now).execute(&mut *tx).await?;
            }

            sqlx::query("UPDATE core_tool_market.credit_quotes SET approved=$2,visibility=$3,approved_by=$4,approved_at=$5,suspended=CASE WHEN $6 AND $2 THEN FALSE WHEN $6 AND NOT $2 AND seller_user_id<>$4 THEN TRUE ELSE suspended END WHERE id=$1").bind(&r.quote_id).bind(r.approved).bind(&r.visibility).bind(actor.user_id).bind(now).bind(admin).execute(&mut *tx).await?;
            Self::record(
                &mut tx,
                actor.user_id,
                &r.operation_id,
                "quote-approve",
                r,
                &r.quote_id,
                now,
                None,
            )
            .await?;
            ApproveReceipt {
                operation_id: r.operation_id.clone(),
                quote_id: r.quote_id.clone(),
                visibility: r.visibility.clone(),
                approved: r.approved,
                approved_by: actor.user_id.to_string(),
                approved_at: now,
            }
        };
        tx.commit().await?;
        Ok(out)
    }
    pub async fn create_consent(&self, secret: &str, r: &ConsentRequest) -> Result<ConsentReceipt> {
        if matches!(r.client_id.as_str(), "assistant" | "builtin-assistant")
            || !client(&r.client_id)
            || !name(&r.tool_id)
            || !name(&r.version_id)
            || !quota(r.max_price_quota)
            || !quota(r.max_total_quota)
            || !(1..=1_000_000).contains(&r.max_calls)
        {
            return Err(Error::Invalid);
        }
        let (mut tx, actor) = self.admit(secret, &r.owner, &r.operation_id).await?;
        if let Some(prior) =
            Self::prior(&mut tx, actor.user_id, &r.operation_id, "consent-create", r).await?
        {
            let out = Self::consent(
                &mut tx,
                &prior.try_get::<String, _>("target_id")?,
                actor.user_id,
            )
            .await?;
            tx.commit().await?;
            return Ok(out);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        let row=sqlx::query("SELECT q.* FROM core_tool_market.credit_quotes q LEFT JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id WHERE q.snapshot->>'tool_id'=$1 AND q.snapshot->>'version_id'=$2 AND q.approved AND (p.active OR EXISTS(SELECT FROM core_tool_market.credit_builtin_bindings b WHERE b.quote_id=q.id)) FOR SHARE OF q").bind(&r.tool_id).bind(&r.version_id).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        let Json(q): Json<credits::Quote> = row.try_get("snapshot")?;
        if let Some(seller) = row.try_get::<Option<i64>, _>("seller_user_id")? {
            Self::check_version(&mut tx, seller, &q.service_id, &q.version_id).await?;
        }

        if r.expires_at <= now
            || r.expires_at > q.expires
            || r.max_price_quota < q.price.ceiling()?
            || r.max_total_quota < q.price.ceiling()?
            || (row.try_get::<String, _>("visibility")? == "private"
                && row.try_get::<Option<i64>, _>("seller_user_id")? != Some(actor.user_id))
        {
            return Err(Error::Forbidden);
        }
        let id = hash(&["credit-consent-v2", &r.owner.user, &r.operation_id])?;
        sqlx::query("INSERT INTO core_tool_market.credit_consents(id,actor_user_id,payer_account_id,quote_id,request,created_at) VALUES($1,$2,$3,$4,$5,$6)").bind(&id).bind(actor.user_id).bind(actor.owner_account_id).bind(&q.id).bind(Json(r)).bind(now).execute(&mut *tx).await?;
        Self::record(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "consent-create",
            r,
            &id,
            now,
            None,
        )
        .await?;
        let out = Self::consent(&mut tx, &id, actor.user_id).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub(super) async fn consent(
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        user: i64,
    ) -> Result<ConsentReceipt> {
        let row=sqlx::query("SELECT c.*,q.snapshot,q.ceiling_credits FROM core_tool_market.credit_consents c JOIN core_tool_market.credit_quotes q ON q.id=c.quote_id WHERE c.id=$1 AND c.actor_user_id=$2").bind(id).bind(user).fetch_optional(&mut **tx).await?.ok_or(Error::Missing)?;
        let Json(q): Json<credits::Quote> = row.try_get("snapshot")?;
        let Json(request) = row.try_get("request")?;
        Ok(ConsentReceipt {
            request,
            quote_id: q.id,
            frozen_price_quota: row.try_get("ceiling_credits")?,
            rules_hash: q.pricing_digest,
            id: id.into(),
            created_at: row.try_get("created_at")?,
            revoked_at: row.try_get("revoked_at")?,
            reserved_quota: row.try_get("reserved_credits")?,
            spent_quota: row.try_get("spent_credits")?,
            reserved_calls: row.try_get("reserved_calls")?,
            successful_calls: row.try_get("successful_calls")?,
        })
    }
    pub async fn revoke(
        &self,
        secret: &str,
        r: &IdRequest,
        token: bool,
    ) -> Result<RevocationReceipt> {
        if !name(&r.id) {
            return Err(Error::Invalid);
        }
        let kind = if token {
            "token-revoke"
        } else {
            "consent-revoke"
        };
        let table = if token {
            "credit_tokens"
        } else {
            "credit_consents"
        };
        let (mut tx, actor) = self.admit(secret, &r.owner, &r.operation_id).await?;
        let prior = Self::prior(&mut tx, actor.user_id, &r.operation_id, kind, r).await?;
        let now = ToolMarketStore::now(&mut tx).await?;
        let sql = format!(
            "UPDATE core_tool_market.{table} SET revoked_at=CASE WHEN revoked_at=0 THEN $3 ELSE revoked_at END WHERE id=$1 AND actor_user_id=$2 RETURNING revoked_at"
        );
        let revoked_at = sqlx::query_scalar(&sql)
            .bind(&r.id)
            .bind(actor.user_id)
            .bind(now)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
        if prior.is_none() {
            Self::record(
                &mut tx,
                actor.user_id,
                &r.operation_id,
                kind,
                r,
                &r.id,
                now,
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(RevocationReceipt {
            request: r.clone(),
            revoked_at,
        })
    }
    pub async fn set_budget(&self, secret: &str, r: &BudgetRequest) -> Result<BudgetReceipt> {
        if !quota(r.limit_quota)
            || !matches!(r.scope.as_str(), "account" | "client" | "tool")
            || (r.scope == "account" && !r.scope_id.is_empty())
            || (r.scope == "client" && !client(&r.scope_id))
            || (r.scope == "tool" && !name(&r.scope_id))
        {
            return Err(Error::Invalid);
        }
        let (mut tx, actor) = self.admit(secret, &r.owner, &r.operation_id).await?;
        let prior = Self::prior(&mut tx, actor.user_id, &r.operation_id, "budget-set", r).await?;
        let row=sqlx::query("SELECT * FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND scope=$2 AND scope_id=$3 FOR UPDATE").bind(actor.user_id).bind(&r.scope).bind(&r.scope_id).fetch_optional(&mut *tx).await?;
        let (reserved, spent) = if let Some(row) = row {
            (
                row.try_get::<i64, _>("reserved_credits")?,
                row.try_get::<i64, _>("spent_credits")?,
            )
        } else {
            (0, 0)
        };
        if reserved>r.limit_quota-spent || (prior.is_some() && sqlx::query_scalar::<_,i64>("SELECT limit_credits FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND scope=$2 AND scope_id=$3").bind(actor.user_id).bind(&r.scope).bind(&r.scope_id).fetch_optional(&mut *tx).await?!=Some(r.limit_quota)){return Err(Error::Conflict)}
        if prior.is_none() {
            sqlx::query("INSERT INTO core_tool_market.credit_budgets(actor_user_id,scope,scope_id,request,limit_credits) VALUES($1,$2,$3,$4,$5) ON CONFLICT(actor_user_id,scope,scope_id) DO UPDATE SET request=EXCLUDED.request,limit_credits=EXCLUDED.limit_credits").bind(actor.user_id).bind(&r.scope).bind(&r.scope_id).bind(Json(r)).bind(r.limit_quota).execute(&mut *tx).await?;
            let now = ToolMarketStore::now(&mut tx).await?;
            Self::record(
                &mut tx,
                actor.user_id,
                &r.operation_id,
                "budget-set",
                r,
                "",
                now,
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(BudgetReceipt {
            request: r.clone(),
            reserved_quota: reserved,
            spent_quota: spent,
        })
    }
    pub async fn create_token(&self, secret: &str, r: &TokenRequest) -> Result<TokenReceipt> {
        if !client(&r.client_id)
            || matches!(
                r.client_id.as_str(),
                "web-market" | "assistant" | "builtin-assistant"
            )
            || r.client_id.starts_with("oauth:")
        {
            return Err(Error::Invalid);
        }
        let (mut tx, actor) = self.admit(secret, &r.owner, &r.operation_id).await?;
        if let Some(prior) =
            Self::prior(&mut tx, actor.user_id, &r.operation_id, "token-create", r).await?
        {
            let mut out = Self::token(
                &mut tx,
                &prior.try_get::<String, _>("target_id")?,
                actor.user_id,
            )
            .await?;
            out.replayed = true;
            tx.commit().await?;
            return Ok(out);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        if r.expires_at <= now || r.expires_at > now + 90 * 86400 {
            return Err(Error::Invalid);
        }
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *tx)
        .await?;
        let id = hash(&["credit-token-v2", &r.owner.user, &r.operation_id])?;
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let raw = format!("lmm_market_{}", URL_SAFE_NO_PAD.encode(bytes));
        sqlx::query("INSERT INTO core_tool_market.credit_tokens(id,actor_user_id,payer_account_id,request,token_hash,auth_version,created_at) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(&id).bind(actor.user_id).bind(actor.owner_account_id).bind(Json(r)).bind(Sha256::digest(raw.as_bytes()).to_vec()).bind(version).bind(now).execute(&mut *tx).await?;
        Self::record(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "token-create",
            r,
            &id,
            now,
            None,
        )
        .await?;
        let mut out = Self::token(&mut tx, &id, actor.user_id).await?;
        tx.commit().await?;
        out.token = raw;
        Ok(out)
    }
    async fn token(
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        user: i64,
    ) -> Result<TokenReceipt> {
        let row = sqlx::query(
            "SELECT * FROM core_tool_market.credit_tokens WHERE id=$1 AND actor_user_id=$2",
        )
        .bind(id)
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::Missing)?;
        let Json(request) = row.try_get("request")?;
        Ok(TokenReceipt {
            request,
            id: id.into(),
            auth_version: row.try_get("auth_version")?,
            created_at: row.try_get("created_at")?,
            revoked_at: row.try_get("revoked_at")?,
            token: String::new(),
            replayed: false,
        })
    }
    pub async fn validate_token(&self, raw: &str, account: &str) -> Result<TokenReceipt> {
        if raw.len() != 54
            || !raw.starts_with("lmm_market_")
            || URL_SAFE_NO_PAD.decode(&raw[11..]).map(|v| v.len()).ok() != Some(32)
        {
            return Err(Error::Unauthorized);
        }
        let mut tx = self.base.begin().await?;
        let row=sqlx::query("SELECT t.id,t.actor_user_id,t.auth_version,u.auth_version AS current_version FROM core_tool_market.credit_tokens t JOIN core_identity.users u ON u.id=t.actor_user_id JOIN core_identity.accounts a ON a.id=t.payer_account_id WHERE t.token_hash=$1 AND t.revoked_at=0 AND u.active AND a.active FOR SHARE OF t,u,a").bind(Sha256::digest(raw.as_bytes()).to_vec()).fetch_optional(&mut *tx).await?.ok_or(Error::Unauthorized)?;
        let user: i64 = row.try_get("actor_user_id")?;
        if (!account.is_empty() && account != user.to_string())
            || row.try_get::<i64, _>("auth_version")? != row.try_get::<i64, _>("current_version")?
        {
            return Err(Error::Forbidden);
        }
        let mut out = Self::token(&mut tx, &row.try_get::<String, _>("id")?, user).await?;
        if out.request.expires_at <= ToolMarketStore::now(&mut tx).await? {
            return Err(Error::Unauthorized);
        }
        out.request.operation_id.clear();
        tx.commit().await?;
        Ok(out)
    }
    pub async fn disconnect(&self, secret: &str, r: &ClientRequest) -> Result<DisconnectReceipt> {
        if !client(&r.client_id) {
            return Err(Error::Invalid);
        }
        let (mut tx, actor) = self.admit(secret, &r.owner, &r.operation_id).await?;
        if let Some(prior) = Self::prior(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "client-disconnect",
            r,
        )
        .await?
        {
            let Json(value): Json<DisconnectReceipt> = prior.try_get("details")?;
            tx.commit().await?;
            return Ok(value);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        let tokens=sqlx::query("UPDATE core_tool_market.credit_tokens SET revoked_at=$3 WHERE actor_user_id=$1 AND request->>'client_id'=$2 AND revoked_at=0").bind(actor.user_id).bind(&r.client_id).bind(now).execute(&mut *tx).await?.rows_affected();
        let grants=sqlx::query("UPDATE core_tool_market.credit_consents SET revoked_at=$3 WHERE actor_user_id=$1 AND request->>'client_id'=$2 AND revoked_at=0").bind(actor.user_id).bind(&r.client_id).bind(now).execute(&mut *tx).await?.rows_affected();
        let out = DisconnectReceipt {
            request: r.clone(),
            tokens_revoked: i64::try_from(tokens).map_err(|_| Error::Invalid)?,
            grants_revoked: i64::try_from(grants).map_err(|_| Error::Invalid)?,
            revoked_at: now,
        };
        Self::record(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "client-disconnect",
            r,
            "",
            now,
            Some(serde_json::to_value(&out).map_err(|_| Error::Invalid)?),
        )
        .await?;
        tx.commit().await?;
        Ok(out)
    }
    async fn listing(
        &self,
        secret: &str,
        r: &ListRequest,
    ) -> Result<(Transaction<'static, Postgres>, Principal)> {
        if !(1..=100).contains(&r.limit) || !(0..=10000).contains(&r.offset) {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = Self::owned(&mut tx, secret, &r.owner).await?;
        Ok((tx, actor))
    }
    pub async fn list_consents(
        &self,
        secret: &str,
        r: &ListRequest,
    ) -> Result<ListReceipt<ConsentReceipt>> {
        let (mut tx, actor) = self.listing(secret, r).await?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_tool_market.credit_consents WHERE actor_user_id=$1 ORDER BY id LIMIT $2 OFFSET $3").bind(actor.user_id).bind(r.limit).bind(r.offset).fetch_all(&mut *tx).await?;
        let mut rows = Vec::new();
        for id in ids {
            rows.push(Self::consent(&mut tx, &id, actor.user_id).await?)
        }
        tx.commit().await?;
        Ok(ListReceipt {
            owner: r.owner.clone(),
            rows,
        })
    }
    pub async fn list_budgets(
        &self,
        secret: &str,
        r: &ListRequest,
    ) -> Result<ListReceipt<BudgetReceipt>> {
        let (mut tx, actor) = self.listing(secret, r).await?;
        let values=sqlx::query("SELECT * FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 ORDER BY scope,scope_id LIMIT $2 OFFSET $3").bind(actor.user_id).bind(r.limit).bind(r.offset).fetch_all(&mut *tx).await?;
        let mut rows = Vec::new();
        for row in values {
            let Json(request) = row.try_get("request")?;
            rows.push(BudgetReceipt {
                request,
                reserved_quota: row.try_get("reserved_credits")?,
                spent_quota: row.try_get("spent_credits")?,
            })
        }
        tx.commit().await?;
        Ok(ListReceipt {
            owner: r.owner.clone(),
            rows,
        })
    }
    pub async fn list_tokens(
        &self,
        secret: &str,
        r: &ListRequest,
    ) -> Result<ListReceipt<TokenReceipt>> {
        let (mut tx, actor) = self.listing(secret, r).await?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_tool_market.credit_tokens WHERE actor_user_id=$1 ORDER BY id LIMIT $2 OFFSET $3").bind(actor.user_id).bind(r.limit).bind(r.offset).fetch_all(&mut *tx).await?;
        let mut rows = Vec::new();
        for id in ids {
            rows.push(Self::token(&mut tx, &id, actor.user_id).await?)
        }
        tx.commit().await?;
        Ok(ListReceipt {
            owner: r.owner.clone(),
            rows,
        })
    }
}
