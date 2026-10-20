//! Core-owned human delegation and client-scoped MCP management. Frozen main
//! 3948a0a model/tool_market_meta_delegation.go; no Go counter is authoritative.
use super::{
    credit_access::{
        BudgetReceipt, BudgetRequest, ConsentReceipt, ConsentRequest, CreditStore, ListReceipt,
        ListRequest,
    },
    credit_actor::{self, ActorSource, MarketAction, MarketActor},
    *,
};
pub const SCHEMA: &str = include_str!("credit_meta_schema.sql");
#[derive(Clone)]
pub struct MetaStore {
    credit: CreditStore,
    profile: Option<crate::market_oauth::MarketProfile>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    pub enabled: bool,
    pub max_total_quota: i64,
    pub expires_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetaView {
    pub owner: Owner,
    pub client_id: String,
    pub delegation: Delegation,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub kind: String,
    pub id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetDelegation {
    pub operation_id: String,
    pub kind: String,
    pub id: String,
    pub enabled: bool,
    pub max_total_quota: i64,
    pub expires_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerView {
    pub operation_id: String,
    pub owner: Owner,
    pub kind: String,
    pub id: String,
    pub delegation: Delegation,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Authorize {
    pub operation_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub max_price_quota: i64,
    pub max_total_quota: i64,
    pub max_calls: i64,
    pub expires_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientBudget {
    pub operation_id: String,
    pub limit_quota: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolBudget {
    pub operation_id: String,
    pub grant_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub limit_quota: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub offset: i64,
    pub limit: i64,
}
#[derive(Serialize)]
pub struct UsageView {
    pub owner: Owner,
    pub client_id: String,
    pub client_budget: Option<BudgetReceipt>,
    pub grants: Vec<ConsentReceipt>,
    pub offset: i64,
    pub limit: i64,
    pub more_grants: bool,
}
fn quota(v: i64) -> bool {
    (0..=credits::MAX_CREDITS).contains(&v)
}
fn page(p: &Page) -> Result<()> {
    if !(0..=10000).contains(&p.offset) || !(1..=100).contains(&p.limit) {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
fn subject(a: &MarketActor) -> Result<(&str, &str, &str)> {
    Ok((
        a.credential_kind(),
        a.credential_id().ok_or(Error::Forbidden)?,
        a.client_id().ok_or(Error::Forbidden)?,
    ))
}
async fn lock(tx: &mut Transaction<'_, Postgres>, a: &MarketActor, op: &str) -> Result<()> {
    if !name(op) {
        return Err(Error::Invalid);
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("credit-access:{}:{op}", a.user_id()))
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("credit-meta:{}", a.user_id()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn policy(tx: &mut Transaction<'_, Postgres>, a: &MarketActor) -> Result<Delegation> {
    let (kind, id, client) = subject(a)?;
    if !a.can_invoke() || !a.can_manage() {
        return Ok(Delegation::default());
    }
    let row=sqlx::query("SELECT * FROM core_tool_market.credit_meta_delegations WHERE actor_user_id=$1 AND credential_kind=$2 AND credential_id=$3 FOR SHARE").bind(a.user_id()).bind(kind).bind(id).fetch_optional(&mut **tx).await?;
    let Some(r) = row else {
        return Ok(Delegation::default());
    };
    let now = ToolMarketStore::now(tx).await?;
    let out = Delegation {
        enabled: r.try_get("enabled")?,
        max_total_quota: r.try_get("max_total_quota")?,
        expires_at: r.try_get("expires_at")?,
        updated_at: r.try_get("updated_at")?,
    };
    if !out.enabled
        || out.expires_at <= now
        || out.expires_at > a.credential_expires_at()
        || r.try_get::<i64, _>("auth_version")? != a.auth_version()
        || r.try_get::<String, _>("client_id")? != client
    {
        return Ok(Delegation::default());
    }
    Ok(out)
}
async fn required_policy(
    tx: &mut Transaction<'_, Postgres>,
    a: &MarketActor,
) -> Result<Delegation> {
    let p = policy(tx, a).await?;
    if !p.enabled {
        Err(Error::Forbidden)
    } else {
        Ok(p)
    }
}
fn summary(views: &[Delegation]) -> Delegation {
    let Some(first) = views.first() else {
        return Delegation::default();
    };
    views.iter().skip(1).fold(first.clone(), |mut a, b| {
        a.enabled &= b.enabled;
        a.max_total_quota = a.max_total_quota.min(b.max_total_quota);
        a.expires_at = a.expires_at.min(b.expires_at);
        a.updated_at = a.updated_at.max(b.updated_at);
        a
    })
}
impl MetaStore {
    pub fn new(credit: CreditStore, profile: Option<crate::market_oauth::MarketProfile>) -> Self {
        Self { credit, profile }
    }
    async fn actor(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        raw: &str,
        action: MarketAction,
    ) -> Result<MarketActor> {
        let a = credit_actor::authenticate(tx, self.profile.as_ref(), raw, action).await?;
        subject(&a)?;
        Ok(a)
    }
    pub async fn read(&self, raw: &str) -> Result<MetaView> {
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Discover).await?;
        let out = MetaView {
            owner: a.owner(),
            client_id: a.client_id().ok_or(Error::Forbidden)?.to_owned(),
            delegation: policy(&mut tx, &a).await?,
        };
        tx.commit().await?;
        Ok(out)
    }
    pub async fn owner_read(&self, secret: &str, target: &Target) -> Result<OwnerView> {
        let mut tx = self.credit.base.begin().await?;
        let owner = credit_actor::authenticate(
            &mut tx,
            self.profile.as_ref(),
            secret,
            MarketAction::Discover,
        )
        .await?;
        let targets = credit_actor::owner_targets(
            &mut tx,
            self.profile.as_ref(),
            &owner,
            &target.kind,
            &target.id,
        )
        .await?;
        let mut views = Vec::new();
        for a in targets {
            views.push(policy(&mut tx, &a).await?)
        }
        let out = OwnerView {
            operation_id: String::new(),
            owner: owner.owner(),
            kind: target.kind.clone(),
            id: target.id.clone(),
            delegation: summary(&views),
        };
        tx.commit().await?;
        Ok(out)
    }
    pub async fn owner_set(&self, secret: &str, r: &SetDelegation) -> Result<OwnerView> {
        if !quota(r.max_total_quota)
            || r.expires_at < 0
            || !r.enabled && (r.max_total_quota != 0 || r.expires_at != 0)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.credit.base.begin().await?;
        let owner = credit_actor::authenticate(
            &mut tx,
            self.profile.as_ref(),
            secret,
            MarketAction::Discover,
        )
        .await?;
        if !matches!(owner.source(), ActorSource::Session { .. }) {
            return Err(Error::Forbidden);
        };
        lock(&mut tx, &owner, &r.operation_id).await?;
        let targets =
            credit_actor::owner_targets(&mut tx, self.profile.as_ref(), &owner, &r.kind, &r.id)
                .await?;
        let prior = CreditStore::prior(
            &mut tx,
            owner.user_id(),
            &r.operation_id,
            "meta-owner-set",
            r,
        )
        .await?;
        if let Some(old) = prior {
            let Json(details): Json<serde_json::Value> = old.try_get("details")?;
            let current = self.target_state(&mut tx, &targets).await?;
            if details.get("state_hash").and_then(|v| v.as_str()) != Some(&hash(&current)?) {
                return Err(Error::Conflict);
            };
            let receipt: OwnerView =
                serde_json::from_value(details.get("receipt").cloned().ok_or(Error::Storage)?)
                    .map_err(|_| Error::Storage)?;
            tx.commit().await?;
            return Ok(receipt);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        let mut views = Vec::new();
        for a in &targets {
            let (kind, id, client) = subject(a)?;
            let mut cap = r.max_total_quota;
            let expiry = if r.enabled && r.expires_at == 0 {
                a.credential_expires_at()
            } else {
                r.expires_at
            };
            if r.enabled && (expiry <= now || expiry > a.credential_expires_at()) {
                return Err(Error::Invalid);
            }
            if r.enabled {
                let budget=sqlx::query("SELECT limit_credits,reserved_credits,spent_credits FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND scope='client' AND scope_id=$2 FOR UPDATE").bind(a.user_id()).bind(client).fetch_optional(&mut *tx).await?;
                let (reserved, spent) = if let Some(b) = budget {
                    cap = cap.min(b.try_get("limit_credits")?);
                    (
                        b.try_get::<i64, _>("reserved_credits")?,
                        b.try_get::<i64, _>("spent_credits")?,
                    )
                } else {
                    let totals=sqlx::query("SELECT COALESCE(SUM(CASE WHEN state IN ('reserved','running','unknown') THEN held_credits ELSE 0 END),0)::bigint AS held,COALESCE(SUM(CASE WHEN state='settled' THEN (receipt->>'charged_quota')::bigint ELSE 0 END),0)::bigint AS spent FROM core_tool_market.credit_calls WHERE actor_user_id=$1 AND snapshot->'request'->>'client_id'=$2").bind(a.user_id()).bind(client).fetch_one(&mut *tx).await?;
                    (
                        totals.try_get::<i64, _>("held")?,
                        totals.try_get::<i64, _>("spent")?,
                    )
                };
                if spent > cap || reserved > cap - spent {
                    return Err(Error::Forbidden);
                }
                let req = BudgetRequest {
                    owner: a.owner(),
                    operation_id: r.operation_id.clone(),
                    scope: "client".into(),
                    scope_id: client.into(),
                    limit_quota: cap,
                };
                sqlx::query("INSERT INTO core_tool_market.credit_budgets(actor_user_id,scope,scope_id,request,limit_credits,reserved_credits,spent_credits) VALUES($1,'client',$2,$3,$4,$5,$6) ON CONFLICT(actor_user_id,scope,scope_id) DO UPDATE SET request=EXCLUDED.request,limit_credits=EXCLUDED.limit_credits").bind(a.user_id()).bind(client).bind(Json(req)).bind(cap).bind(reserved).bind(spent).execute(&mut *tx).await?;
            }
            let view = Delegation {
                enabled: r.enabled,
                max_total_quota: cap,
                expires_at: expiry,
                updated_at: now,
            };
            let (personal, oauth) = match a.source() {
                ActorSource::Personal { token_id } => (Some(token_id.as_str()), None),
                ActorSource::Oauth { family_id, .. } => (None, Some(family_id.as_str())),
                _ => return Err(Error::Forbidden),
            };
            sqlx::query("INSERT INTO core_tool_market.credit_meta_delegations(actor_user_id,credential_kind,credential_id,client_id,auth_version,personal_token_id,oauth_family_id,enabled,max_total_quota,expires_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(actor_user_id,credential_kind,credential_id) DO UPDATE SET enabled=EXCLUDED.enabled,max_total_quota=EXCLUDED.max_total_quota,expires_at=EXCLUDED.expires_at,updated_at=EXCLUDED.updated_at,revision=core_tool_market.credit_meta_delegations.revision+1").bind(a.user_id()).bind(kind).bind(id).bind(client).bind(a.auth_version()).bind(personal).bind(oauth).bind(view.enabled).bind(view.max_total_quota).bind(view.expires_at).bind(now).execute(&mut *tx).await?;
            views.push(view);
        }
        let out = OwnerView {
            operation_id: r.operation_id.clone(),
            owner: owner.owner(),
            kind: r.kind.clone(),
            id: r.id.clone(),
            delegation: summary(&views),
        };
        let state = self.target_state(&mut tx, &targets).await?;
        CreditStore::record(
            &mut tx,
            owner.user_id(),
            &r.operation_id,
            "meta-owner-set",
            r,
            "",
            now,
            Some(serde_json::json!({"receipt":out,"state_hash":hash(&state)?})),
        )
        .await?;
        tx.commit().await?;
        Ok(out)
    }
    async fn target_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        targets: &[MarketActor],
    ) -> Result<Vec<serde_json::Value>> {
        let mut rows = Vec::new();
        for a in targets {
            let (kind, id, _) = subject(a)?;
            let value:Option<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(d) FROM core_tool_market.credit_meta_delegations d WHERE actor_user_id=$1 AND credential_kind=$2 AND credential_id=$3").bind(a.user_id()).bind(kind).bind(id).fetch_optional(&mut **tx).await?;
            rows.push(serde_json::json!({"kind":kind,"id":id,"state":value}));
        }
        Ok(rows)
    }
    pub async fn authorize(&self, raw: &str, r: &Authorize) -> Result<ConsentReceipt> {
        if !name(&r.tool_id)
            || !name(&r.version_id)
            || !quota(r.max_price_quota)
            || !quota(r.max_total_quota)
            || !(1..=1_000_000).contains(&r.max_calls)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Delegate).await?;
        lock(&mut tx, &a, &r.operation_id).await?;
        let p = required_policy(&mut tx, &a).await?;
        let now = ToolMarketStore::now(&mut tx).await?;
        if r.expires_at <= now
            || r.expires_at > p.expires_at
            || r.max_price_quota > r.max_total_quota
            || r.max_total_quota > p.max_total_quota
        {
            return Err(Error::Forbidden);
        }
        let client = a.client_id().ok_or(Error::Forbidden)?;
        let limit:i64=sqlx::query_scalar("SELECT limit_credits FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND scope='client' AND scope_id=$2 FOR SHARE").bind(a.user_id()).bind(client).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        if r.max_total_quota > limit {
            return Err(Error::Forbidden);
        }
        let qrow=sqlx::query("SELECT q.* FROM core_tool_market.credit_quotes q LEFT JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id WHERE q.snapshot->>'tool_id'=$1 AND q.snapshot->>'version_id'=$2 AND q.approved AND NOT q.suspended AND (p.active OR EXISTS(SELECT FROM core_tool_market.credit_builtin_bindings b WHERE b.quote_id=q.id)) FOR SHARE OF q").bind(&r.tool_id).bind(&r.version_id).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        let Json(q): Json<credits::Quote> = qrow.try_get("snapshot")?;
        if let Some(seller) = qrow.try_get::<Option<i64>, _>("seller_user_id")? {
            CreditStore::check_version(&mut tx, seller, &q.service_id, &q.version_id).await?;
        }
        if r.max_price_quota < q.price.ceiling()?
            || r.expires_at > q.expires
            || qrow.try_get::<String, _>("visibility")? == "private"
                && qrow.try_get::<Option<i64>, _>("seller_user_id")? != Some(a.user_id())
        {
            return Err(Error::Forbidden);
        }
        let prior =
            CreditStore::prior(&mut tx, a.user_id(), &r.operation_id, "meta-authorize", r).await?;
        if let Some(old) = prior {
            let out = CreditStore::consent(
                &mut tx,
                &old.try_get::<String, _>("target_id")?,
                a.user_id(),
            )
            .await?;
            if out.request.client_id != client
                || out.revoked_at != 0
                || out.request.max_price_quota != r.max_price_quota
                || out.request.max_total_quota != r.max_total_quota
                || out.request.max_calls != r.max_calls
                || out.request.expires_at != r.expires_at
            {
                return Err(Error::Conflict);
            };
            tx.commit().await?;
            return Ok(out);
        }
        let existing:Option<String>=sqlx::query_scalar("SELECT id FROM core_tool_market.credit_consents WHERE actor_user_id=$1 AND request->>'client_id'=$2 AND request->>'tool_id'=$3 AND request->>'version_id'=$4 AND revoked_at=0 AND (request->>'expires_at')::bigint>$5 ORDER BY created_at DESC,id DESC LIMIT 1 FOR UPDATE").bind(a.user_id()).bind(client).bind(&r.tool_id).bind(&r.version_id).bind(now).fetch_optional(&mut *tx).await?;
        let id = if let Some(id) = existing {
            let old = CreditStore::consent(&mut tx, &id, a.user_id()).await?;
            if old.quote_id != q.id
                || r.max_price_quota > old.request.max_price_quota
                || r.max_total_quota > old.request.max_total_quota
                || r.max_calls > old.request.max_calls
                || r.expires_at > old.request.expires_at
                || old.spent_quota > r.max_total_quota
                || old.reserved_quota > r.max_total_quota - old.spent_quota
                || old.successful_calls > r.max_calls
                || old.reserved_calls > r.max_calls - old.successful_calls
            {
                return Err(Error::Forbidden);
            }
            let mut request = old.request;
            request.max_price_quota = r.max_price_quota;
            request.max_total_quota = r.max_total_quota;
            request.max_calls = r.max_calls;
            request.expires_at = r.expires_at;
            sqlx::query("UPDATE core_tool_market.credit_consents SET request=$2 WHERE id=$1")
                .bind(&id)
                .bind(Json(request))
                .execute(&mut *tx)
                .await?;
            id
        } else {
            let id = hash(&[
                "credit-meta-consent-v2",
                &a.user_id().to_string(),
                &r.operation_id,
            ])?;
            let request = ConsentRequest {
                owner: a.owner(),
                operation_id: r.operation_id.clone(),
                client_id: client.into(),
                tool_id: r.tool_id.clone(),
                version_id: r.version_id.clone(),
                max_price_quota: r.max_price_quota,
                max_total_quota: r.max_total_quota,
                max_calls: r.max_calls,
                expires_at: r.expires_at,
            };
            sqlx::query("INSERT INTO core_tool_market.credit_consents(id,actor_user_id,payer_account_id,quote_id,request,created_at) VALUES($1,$2,$3,$4,$5,$6)").bind(&id).bind(a.user_id()).bind(a.payer_account_id()).bind(&q.id).bind(Json(request)).bind(now).execute(&mut *tx).await?;
            id
        };
        CreditStore::record(
            &mut tx,
            a.user_id(),
            &r.operation_id,
            "meta-authorize",
            r,
            &id,
            now,
            None,
        )
        .await?;
        let out = CreditStore::consent(&mut tx, &id, a.user_id()).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn tighten_client(&self, raw: &str, r: &ClientBudget) -> Result<BudgetReceipt> {
        if !quota(r.limit_quota) {
            return Err(Error::Invalid);
        }
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Delegate).await?;
        lock(&mut tx, &a, &r.operation_id).await?;
        let p = required_policy(&mut tx, &a).await?;
        let client = a.client_id().ok_or(Error::Forbidden)?;
        let prior = CreditStore::prior(
            &mut tx,
            a.user_id(),
            &r.operation_id,
            "meta-client-budget",
            r,
        )
        .await?;
        let b=sqlx::query("SELECT * FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND scope='client' AND scope_id=$2 FOR UPDATE").bind(a.user_id()).bind(client).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        let limit: i64 = b.try_get("limit_credits")?;
        let reserved: i64 = b.try_get("reserved_credits")?;
        let spent: i64 = b.try_get("spent_credits")?;
        if prior.is_some() && r.limit_quota != limit {
            return Err(Error::Conflict);
        }
        if r.limit_quota > p.max_total_quota
            || r.limit_quota > limit
            || spent > r.limit_quota
            || reserved > r.limit_quota - spent
        {
            return Err(Error::Forbidden);
        }

        let request = BudgetRequest {
            owner: a.owner(),
            operation_id: r.operation_id.clone(),
            scope: "client".into(),
            scope_id: client.into(),
            limit_quota: r.limit_quota,
        };
        if prior.is_none() {
            sqlx::query("UPDATE core_tool_market.credit_budgets SET limit_credits=$3,request=$4 WHERE actor_user_id=$1 AND scope='client' AND scope_id=$2").bind(a.user_id()).bind(client).bind(r.limit_quota).bind(Json(&request)).execute(&mut *tx).await?;
            let now = ToolMarketStore::now(&mut tx).await?;
            CreditStore::record(
                &mut tx,
                a.user_id(),
                &r.operation_id,
                "meta-client-budget",
                r,
                client,
                now,
                None,
            )
            .await?;
        }
        let out = BudgetReceipt {
            request,
            reserved_quota: reserved,
            spent_quota: spent,
        };
        tx.commit().await?;
        Ok(out)
    }
    pub async fn tighten_tool(&self, raw: &str, r: &ToolBudget) -> Result<ConsentReceipt> {
        if !quota(r.limit_quota)
            || ![&r.grant_id, &r.tool_id, &r.version_id]
                .into_iter()
                .all(|v| name(v))
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Delegate).await?;
        lock(&mut tx, &a, &r.operation_id).await?;
        let p = required_policy(&mut tx, &a).await?;
        let prior =
            CreditStore::prior(&mut tx, a.user_id(), &r.operation_id, "meta-tool-budget", r)
                .await?;
        sqlx::query("SELECT id FROM core_tool_market.credit_consents WHERE id=$1 FOR UPDATE")
            .bind(&r.grant_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
        let old = CreditStore::consent(&mut tx, &r.grant_id, a.user_id()).await?;
        let now = ToolMarketStore::now(&mut tx).await?;
        if old.request.client_id != a.client_id().ok_or(Error::Forbidden)?
            || old.request.tool_id != r.tool_id
            || old.request.version_id != r.version_id
            || old.revoked_at != 0
            || old.request.expires_at <= now
        {
            return Err(Error::Forbidden);
        }
        if prior.is_some() && old.request.max_total_quota != r.limit_quota {
            return Err(Error::Conflict);
        }
        if r.limit_quota > p.max_total_quota
            || r.limit_quota > old.request.max_total_quota
            || old.spent_quota > r.limit_quota
            || old.reserved_quota > r.limit_quota - old.spent_quota
        {
            return Err(Error::Forbidden);
        }

        if prior.is_none() {
            let mut request = old.request;
            request.max_total_quota = r.limit_quota;
            sqlx::query("UPDATE core_tool_market.credit_consents SET request=$2 WHERE id=$1")
                .bind(&r.grant_id)
                .bind(Json(request))
                .execute(&mut *tx)
                .await?;
            CreditStore::record(
                &mut tx,
                a.user_id(),
                &r.operation_id,
                "meta-tool-budget",
                r,
                &r.grant_id,
                now,
                None,
            )
            .await?;
        }
        let out = CreditStore::consent(&mut tx, &r.grant_id, a.user_id()).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn list_consents(
        &self,
        raw: &str,
        r: &ListRequest,
    ) -> Result<ListReceipt<ConsentReceipt>> {
        page(&Page {
            offset: r.offset,
            limit: r.limit,
        })?;
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Discover).await?;
        a.assert_owner(&r.owner)?;
        let client = a.client_id().ok_or(Error::Forbidden)?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_tool_market.credit_consents WHERE actor_user_id=$1 AND request->>'client_id'=$2 ORDER BY id LIMIT $3 OFFSET $4").bind(a.user_id()).bind(client).bind(r.limit).bind(r.offset).fetch_all(&mut *tx).await?;
        let mut rows = Vec::new();
        for id in ids {
            rows.push(CreditStore::consent(&mut tx, &id, a.user_id()).await?)
        }
        tx.commit().await?;
        Ok(ListReceipt {
            owner: a.owner(),
            rows,
        })
    }
    pub async fn usage(&self, raw: &str, r: &Page) -> Result<UsageView> {
        page(r)?;
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Discover).await?;
        let client = a.client_id().ok_or(Error::Forbidden)?;
        let now = ToolMarketStore::now(&mut tx).await?;
        let b=sqlx::query("SELECT * FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND scope='client' AND scope_id=$2 FOR SHARE").bind(a.user_id()).bind(client).fetch_optional(&mut *tx).await?;
        let client_budget = if let Some(b) = b {
            let Json(request): Json<BudgetRequest> = b.try_get("request")?;
            Some(BudgetReceipt {
                request,
                reserved_quota: b.try_get("reserved_credits")?,
                spent_quota: b.try_get("spent_credits")?,
            })
        } else {
            None
        };
        let mut ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_tool_market.credit_consents WHERE actor_user_id=$1 AND request->>'client_id'=$2 AND revoked_at=0 AND (request->>'expires_at')::bigint>$3 ORDER BY created_at DESC,id DESC LIMIT $4 OFFSET $5").bind(a.user_id()).bind(client).bind(now).bind(r.limit+1).bind(r.offset).fetch_all(&mut *tx).await?;
        let more_grants = ids.len() > r.limit as usize;
        if more_grants {
            ids.truncate(r.limit as usize)
        };
        let mut grants = Vec::new();
        for id in ids {
            grants.push(CreditStore::consent(&mut tx, &id, a.user_id()).await?)
        }
        let out = UsageView {
            owner: a.owner(),
            client_id: client.into(),
            client_budget,
            grants,
            offset: r.offset,
            limit: r.limit,
            more_grants,
        };
        tx.commit().await?;
        Ok(out)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCall {
    pub call_id: String,
    pub service_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub price_quota: i64,
    pub execution_status: String,
    pub settlement_status: String,
    pub created_at: i64,
    pub started_at: i64,
    pub finished_at: i64,
}
#[derive(Serialize)]
pub struct ClientCalls {
    pub owner: Owner,
    pub client_id: String,
    pub calls: Vec<ClientCall>,
    pub offset: i64,
    pub limit: i64,
    pub more: bool,
}
impl MetaStore {
    pub async fn calls(&self, raw: &str, r: &Page) -> Result<ClientCalls> {
        page(r)?;
        let mut tx = self.credit.base.begin().await?;
        let a = self.actor(&mut tx, raw, MarketAction::Discover).await?;
        let client = a.client_id().ok_or(Error::Forbidden)?;
        let mut rows=sqlx::query("SELECT id,snapshot,receipt,created_at,started_at,finished_at,state,dispatch_id FROM core_tool_market.credit_calls WHERE actor_user_id=$1 AND snapshot->'request'->>'client_id'=$2 ORDER BY created_at DESC,id LIMIT $3 OFFSET $4").bind(a.user_id()).bind(client).bind(r.limit+1).bind(r.offset).fetch_all(&mut *tx).await?;
        let more = rows.len() > r.limit as usize;
        if more {
            rows.truncate(r.limit as usize)
        };
        let mut calls = Vec::new();
        for row in rows {
            let Json(receipt): Json<super::credit_execution::Receipt> = row.try_get("receipt")?;
            let Json(snapshot): Json<serde_json::Value> = row.try_get("snapshot")?;
            if receipt.owner != a.owner() || receipt.client_id != client {
                return Err(Error::Forbidden);
            };
            let state: String = row.try_get("state")?;
            let dispatch: Option<String> = row.try_get("dispatch_id")?;
            let execution = match state.as_str() {
                "settled" => "succeeded",
                "released" => {
                    if dispatch.is_some() {
                        "failed"
                    } else {
                        "cancelled"
                    }
                }
                "running" => "running",
                "unknown" => "unknown",
                "reserved" => "reserved",
                _ => return Err(Error::Storage),
            };
            let settlement = match state.as_str() {
                "settled" => "settled",
                "released" => "released",
                _ => "held",
            };
            calls.push(ClientCall {
                call_id: receipt.call_id,
                service_id: snapshot
                    .get("quote")
                    .and_then(|q| q.get("service_id"))
                    .and_then(|v| v.as_str())
                    .ok_or(Error::Storage)?
                    .into(),
                tool_id: receipt.tool_id,
                version_id: receipt.version_id,
                price_quota: if state == "settled" {
                    receipt.charged_quota
                } else {
                    receipt.held_quota
                },
                execution_status: execution.into(),
                settlement_status: settlement.into(),
                created_at: row.try_get("created_at")?,
                started_at: row.try_get("started_at")?,
                finished_at: row.try_get("finished_at")?,
            });
        }
        let out = ClientCalls {
            owner: a.owner(),
            client_id: client.into(),
            calls,
            offset: r.offset,
            limit: r.limit,
            more,
        };
        tx.commit().await?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_summary_keeps_all_families_most_restrictive_policy() {
        let a = Delegation {
            enabled: true,
            max_total_quota: 100,
            expires_at: 200,
            updated_at: 10,
        };
        let b = Delegation {
            enabled: true,
            max_total_quota: 30,
            expires_at: 150,
            updated_at: 11,
        };
        assert_eq!(
            summary(&[a.clone(), b]),
            Delegation {
                enabled: true,
                max_total_quota: 30,
                expires_at: 150,
                updated_at: 11
            }
        );
        assert!(!summary(&[a, Delegation::default()]).enabled);
        assert_eq!(summary(&[]), Delegation::default());
    }
    #[test]
    fn meta_contract_rejects_identity_override_and_missing_finite_limits() {
        for raw in [
            r#"{"operation_id":"op","tool_id":"t","version_id":"v","max_price_quota":0,"max_total_quota":0,"max_calls":1,"expires_at":100,"client_id":"another"}"#,
            r#"{"operation_id":"op","tool_id":"t","version_id":"v","max_price_quota":0,"max_calls":1,"expires_at":100}"#,
        ] {
            assert!(serde_json::from_str::<Authorize>(raw).is_err())
        }
        assert!(!quota(-1));
        assert!(!quota(credits::MAX_CREDITS + 1));
        assert!(quota(0));
        assert!(
            page(&Page {
                offset: 10001,
                limit: 20
            })
            .is_err()
        );
        assert!(
            page(&Page {
                offset: 0,
                limit: 0
            })
            .is_err()
        );
    }
    #[sqlx::test(migrations = false)]
    async fn meta_personal_delegation_cannot_reset_calls_or_expand_client_scope(pool: PgPool) {
        use super::super::{credit_access::TokenRequest, credit_execution::ReserveRequest};
        let f = super::super::credit_test_fixture::builtin(
            &pool,
            "meta-test-tool",
            "meta-test-version",
            "wallet.list_wallets",
            &"a".repeat(64),
            &"b".repeat(64),
        )
        .await;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&pool)
                .await
                .unwrap();
        let owner = Owner {
            user: "7".into(),
            account: "7".into(),
        };
        let token = f
            .store
            .create_token(
                &f.secret,
                &TokenRequest {
                    owner: owner.clone(),
                    operation_id: "meta-token".into(),
                    client_id: "Client".into(),
                    can_invoke: true,
                    can_manage: true,
                    expires_at: now + 600,
                },
            )
            .await
            .unwrap();
        let meta = MetaStore::new(f.store.clone(), None);
        assert_eq!(
            meta.read(&token.token).await.unwrap().delegation,
            Delegation::default()
        );
        let set = SetDelegation {
            operation_id: "owner-enable".into(),
            kind: "personal".into(),
            id: token.id.clone(),
            enabled: true,
            max_total_quota: 100,
            expires_at: now + 500,
        };
        assert!(matches!(
            meta.owner_set(&token.token, &set).await,
            Err(Error::Forbidden)
        ));
        let policy = meta.owner_set(&f.secret, &set).await.unwrap();
        assert!(policy.delegation.enabled);
        assert_eq!(
            meta.owner_set(&f.secret, &set).await.unwrap().delegation,
            policy.delegation
        );
        let mut request = Authorize {
            operation_id: "meta-authorize".into(),
            tool_id: "meta-test-tool".into(),
            version_id: "meta-test-version".into(),
            max_price_quota: 0,
            max_total_quota: 50,
            max_calls: 5,
            expires_at: now + 300,
        };
        let grant = meta.authorize(&token.token, &request).await.unwrap();
        let reserve = ReserveRequest {
            owner: owner.clone(),
            client_id: "Client".into(),
            request_key: "meta-call".into(),
            quote_id: grant.quote_id.clone(),
            grant_id: grant.id.clone(),
            tool_id: request.tool_id.clone(),
            version_id: request.version_id.clone(),
            input_digest: "c".repeat(64),
            resolve_by: now + 100,
        };
        let call = f.store.reserve(&token.token, &reserve).await.unwrap();
        assert_eq!(call.client_id, "Client");
        request.operation_id = "same-grant-new-operation".into();
        let repeated = meta.authorize(&token.token, &request).await.unwrap();
        assert_eq!(repeated.id, grant.id);
        assert_eq!(repeated.reserved_calls, 1);
        request.operation_id = "cannot-expand".into();
        request.max_calls = 6;
        assert!(matches!(
            meta.authorize(&token.token, &request).await,
            Err(Error::Forbidden)
        ));
        let tightened = meta
            .tighten_tool(
                &token.token,
                &ToolBudget {
                    operation_id: "tighten-grant".into(),
                    grant_id: grant.id.clone(),
                    tool_id: request.tool_id.clone(),
                    version_id: request.version_id.clone(),
                    limit_quota: 20,
                },
            )
            .await
            .unwrap();
        assert_eq!(tightened.reserved_calls, 1);
        assert_eq!(tightened.request.max_total_quota, 20);
        request.operation_id = "meta-authorize".into();
        request.max_calls = 5;
        assert!(matches!(
            meta.authorize(&token.token, &request).await,
            Err(Error::Conflict)
        ));
        let mut other = reserve.clone();
        other.client_id = "other".into();
        other.request_key = "other".into();
        assert!(matches!(
            f.store.reserve(&token.token, &other).await,
            Err(Error::Forbidden)
        ));
        let scoped = meta
            .list_consents(
                &token.token,
                &ListRequest {
                    owner: owner.clone(),
                    offset: 0,
                    limit: 100,
                },
            )
            .await
            .unwrap();
        assert_eq!(scoped.rows.len(), 1);
        assert_eq!(scoped.rows[0].request.client_id, "Client");
        let usage = meta
            .usage(
                &token.token,
                &Page {
                    offset: 0,
                    limit: 20,
                },
            )
            .await
            .unwrap();
        assert_eq!(usage.grants[0].reserved_calls, 1);
        assert_eq!(usage.client_budget.unwrap().request.limit_quota, 100);
        let reopened = MetaStore::new(CreditStore::from_pool(pool.clone()), None);
        assert!(
            reopened
                .read(&token.token)
                .await
                .unwrap()
                .delegation
                .enabled
        );
        let disable = SetDelegation {
            operation_id: "owner-disable".into(),
            enabled: false,
            max_total_quota: 0,
            expires_at: 0,
            ..set.clone()
        };
        reopened.owner_set(&f.secret, &disable).await.unwrap();
        assert_eq!(
            reopened.read(&token.token).await.unwrap().delegation,
            Delegation::default()
        );
        assert!(matches!(
            reopened.owner_set(&f.secret, &set).await,
            Err(Error::Conflict)
        ));
        f.store
            .revoke(
                &f.secret,
                &super::super::credit_access::IdRequest {
                    owner,
                    operation_id: "revoke-market-token".into(),
                    id: token.id,
                },
                true,
            )
            .await
            .unwrap();
        assert!(matches!(
            reopened.read(&token.token).await,
            Err(Error::Unauthorized)
        ));
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthClientsRequest {
    pub offset: i64,
    pub limit: i64,
    pub require_manage: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthClient {
    pub client_id: String,
}
#[derive(Serialize)]
pub struct OAuthClients {
    pub owner: Owner,
    pub clients: Vec<OAuthClient>,
    pub offset: i64,
    pub limit: i64,
    pub more: bool,
}
impl MetaStore {
    pub async fn oauth_clients(
        &self,
        secret: &str,
        r: &OAuthClientsRequest,
    ) -> Result<OAuthClients> {
        page(&Page {
            offset: r.offset,
            limit: r.limit,
        })?;
        let mut tx = self.credit.base.begin().await?;
        let owner = credit_actor::authenticate(
            &mut tx,
            self.profile.as_ref(),
            secret,
            MarketAction::Discover,
        )
        .await?;
        if !matches!(owner.source(), ActorSource::Session { .. }) {
            return Err(Error::Forbidden);
        };
        let p = self.profile.as_ref().ok_or(Error::Storage)?;
        let mut scopes = vec!["market:discover", "market:invoke"];
        if r.require_manage {
            scopes.push("market:manage")
        }
        // A still refreshable family remains eligible after access expiry. A used
        // refresh token, narrowed scope, revoked client/family or stale user cannot
        // create a new target. No family identifier is returned to the browser.
        let mut ids:Vec<String>=sqlx::query_scalar("SELECT DISTINCT f.client_id FROM core_market_oauth.families f JOIN core_market_oauth.clients c ON c.id=f.client_id JOIN core_market_oauth.tokens t ON t.family_id=f.id WHERE f.user_id=$1 AND f.account_id=$2 AND f.user_version=$3 AND f.issuer=$4 AND f.resource=$5 AND c.issuer=f.issuer AND c.revoked_at IS NULL AND f.revoked_at IS NULL AND f.expires_at>clock_timestamp() AND f.scopes @> $6::text[] AND t.kind IN ('access','refresh') AND t.consumed_at IS NULL AND t.expires_at>clock_timestamp() AND t.scopes @> $6::text[] ORDER BY f.client_id LIMIT $7 OFFSET $8").bind(owner.user_id()).bind(owner.payer_account_id()).bind(owner.auth_version()).bind(&p.issuer).bind(&p.resource).bind(scopes).bind(r.limit+1).bind(r.offset).fetch_all(&mut *tx).await?;
        let more = ids.len() > r.limit as usize;
        if more {
            ids.truncate(r.limit as usize)
        };
        let out = OAuthClients {
            owner: owner.owner(),
            clients: ids
                .into_iter()
                .map(|id| OAuthClient {
                    client_id: format!("oauth:{id}"),
                })
                .collect(),
            offset: r.offset,
            limit: r.limit,
            more,
        };
        tx.commit().await?;
        Ok(out)
    }
}
