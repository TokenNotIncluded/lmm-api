//! Persistent legacy fixed/metered calls backed by the SAME Core ledger and
//! aggregate budgets. External execution never runs inside a DB transaction.
use super::{
    credit_access::{ConsentRequest, CreditStore},
    credits::{self, Price, Usage},
    *,
};
use sqlx::{PgConnection, postgres::PgRow};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReserveRequest {
    pub owner: Owner,
    pub client_id: String,
    pub request_key: String,
    pub quote_id: String,
    pub grant_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub input_digest: String,
    pub resolve_by: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CallRequest {
    pub call_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DispatchRequest {
    pub call_id: String,
    pub dispatch_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SettleRequest {
    pub call_id: String,
    pub dispatch_id: String,
    pub result_digest: String,
    pub usage: Option<Usage>,
    #[serde(default)]
    pub raw_result: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRequest {
    pub call_id: String,
    pub dispatch_id: String,
    pub reason: String,
    pub evidence_digest: String,
    #[serde(default)]
    pub raw_result: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub call_id: String,
    pub owner: Owner,
    pub client_id: String,
    pub request_key: String,
    pub quote_id: String,
    pub grant_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub input_digest: String,
    pub pricing_digest: String,
    pub created_at: i64,
    pub resolve_by: i64,
    pub state: String,
    pub reservation: String,
    pub held_quota: i64,
    pub charged_quota: i64,
    pub fee_quota: i64,
    pub seller_quota: i64,
    pub refunded_quota: i64,
    pub dispatch_id: String,
    pub dispatch_won: bool,
    pub result_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Snapshot {
    request: ReserveRequest,
    quote: credits::Quote,
    seller_account_id: Option<i64>,
    platform_account_id: Option<i64>,
    fee_bps: i64,
    policy_version: i64,
    seller_user_id: Option<i64>,
    platform_user_id: Option<i64>,
}
fn call_id(user: i64, client: &str, key: &str) -> Result<String> {
    hash(&serde_json::json!([user, client, key]))
}
fn usage_matches(a: &Usage, b: &Usage) -> bool {
    a.call_id == b.call_id
        && a.version_id == b.version_id
        && a.tool_id == b.tool_id
        && a.input_digest == b.input_digest
        && a.pricing_digest == b.pricing_digest
        && a.result_digest == b.result_digest
        && a.quantities == b.quantities
}
fn source_columns(
    source: &super::credit_actor::ActorSource,
) -> (Option<i64>, Option<&str>, Option<&str>) {
    match source {
        super::credit_actor::ActorSource::Session { credential_id } => {
            (Some(*credential_id), None, None)
        }
        super::credit_actor::ActorSource::Personal { token_id } => (None, Some(token_id), None),
        super::credit_actor::ActorSource::Oauth { family_id, .. } => (None, None, Some(family_id)),
    }
}
async fn row(tx: &mut Transaction<'_, Postgres>, id: &str) -> Result<PgRow> {
    sqlx::query("SELECT * FROM core_tool_market.credit_calls WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::Missing)
}
async fn save(
    tx: &mut Transaction<'_, Postgres>,
    r: &Receipt,
    usage: Option<&Usage>,
    journal: Option<i64>,
) -> Result<()> {
    sqlx::query("UPDATE core_tool_market.credit_calls SET state=$2,dispatch_id=NULLIF($3,''),result_digest=NULLIF($4,''),usage=$5,receipt=$6,settlement_journal_id=$7,started_at=CASE WHEN $2='running' AND started_at=0 THEN floor(extract(epoch FROM clock_timestamp()))::BIGINT ELSE started_at END,finished_at=CASE WHEN $2 IN ('settled','released') AND finished_at=0 THEN floor(extract(epoch FROM clock_timestamp()))::BIGINT ELSE finished_at END WHERE id=$1").bind(&r.call_id).bind(&r.state).bind(&r.dispatch_id).bind(&r.result_digest).bind(usage.map(Json)).bind(Json(r)).bind(journal).execute(&mut **tx).await?;
    Ok(())
}
async fn post(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    phase: &str,
    actor: i64,
    action: Action,
) -> Result<i64> {
    let request = Request {
        scope: "core-tool-market-credit".into(),
        operation_key: hash(&["credit-call-v2", id, phase])?,
        actor_user_id: Some(actor),
        reason: format!("Tool market CREDIT {phase} {id}"),
        action,
    };
    let Json(out): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(request))
        .fetch_one(&mut **tx)
        .await?;
    match out {
        Outcome::Posted { journal_id, .. } => Ok(journal_id),
        Outcome::Rejected { .. } => Err(Error::Conflict),
    }
}
async fn budgets(
    tx: &mut Transaction<'_, Postgres>,
    actor: i64,
    client: &str,
    tool: &str,
) -> Result<Vec<PgRow>> {
    Ok(sqlx::query("SELECT * FROM core_tool_market.credit_budgets WHERE actor_user_id=$1 AND ((scope='account' AND scope_id='') OR (scope='client' AND scope_id=$2) OR (scope='tool' AND scope_id=$3)) ORDER BY scope,scope_id FOR UPDATE").bind(actor).bind(client).bind(tool).fetch_all(&mut **tx).await?)
}
impl CreditStore {
    async fn financial_admission(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        quote: &credits::Quote,
        held: i64,
    ) -> Result<()> {
        if held > 0 {
            if quote.price.ceiling()? != held {
                return Err(Error::Conflict);
            }
            let (mode, _, _) = Self::result_contract(tx, &quote.id).await?;
            match &quote.price {
                Price::Metered { rules } => {
                    if mode != "supplier_signed" {
                        return Err(Error::Forbidden);
                    }
                    let limits = rules
                        .iter()
                        .map(|v| (v.metric.clone(), v.max_quantity))
                        .collect();
                    if !self.verifier.as_ref().is_some_and(|v| {
                        v.supports(
                            &quote.service_id,
                            &quote.version_id,
                            &quote.tool_id,
                            &quote.remote_digest,
                            &limits,
                        )
                    }) {
                        return Err(Error::Forbidden);
                    }
                }
                Price::Fixed { .. } if mode == "core_host_fixed_success" => {}
                _ => return Err(Error::Forbidden),
            }
        }
        Ok(())
    }
    pub async fn reserve(&self, secret: &str, r: &ReserveRequest) -> Result<Receipt> {
        if matches!(r.client_id.as_str(), "assistant" | "builtin-assistant")
            || r.client_id.is_empty()
            || r.client_id.len() > 128
            || r.client_id.trim() != r.client_id
            || !name(&r.request_key)
            || ![&r.quote_id, &r.input_digest]
                .into_iter()
                .all(|v| digest(v))
            || ![&r.grant_id, &r.tool_id, &r.version_id]
                .into_iter()
                .all(|v| name(v))
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = super::credit_actor::authenticate(
            &mut tx,
            self.market_profile.as_deref(),
            secret,
            super::credit_actor::MarketAction::Invoke,
        )
        .await?;
        actor.assert_owner(&r.owner)?;
        actor.assert_client(&r.client_id)?;
        let (session_id, market_token_id, market_oauth_family_id) = source_columns(actor.source());
        let id = call_id(actor.user_id(), &r.client_id, &r.request_key)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("credit-call:{id}"))
            .execute(&mut *tx)
            .await?;
        if let Some(old) =
            sqlx::query("SELECT snapshot,receipt FROM core_tool_market.credit_calls WHERE id=$1")
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let Json(s): Json<Snapshot> = old.try_get("snapshot")?;
            if s.request != *r {
                return Err(Error::Conflict);
            }
            let Json(receipt) = old.try_get("receipt")?;
            tx.commit().await?;
            return Ok(receipt);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        if r.resolve_by <= now || r.resolve_by > now + 86400 {
            return Err(Error::Invalid);
        }
        crate::identity::reject_marked_new_action(
            &mut tx,
            actor.user_id(),
            crate::identity::NewResourceAction::CommercePayment,
        )
        .await?;
        let q=sqlx::query("SELECT q.* FROM core_tool_market.credit_quotes q LEFT JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id LEFT JOIN core_identity.users u ON u.id=q.seller_user_id LEFT JOIN core_identity.accounts a ON a.id=q.seller_account_id WHERE q.id=$1 AND q.approved AND ((p.active AND u.active AND a.active) OR EXISTS(SELECT FROM core_tool_market.credit_builtin_bindings b WHERE b.quote_id=q.id)) FOR SHARE OF q").bind(&r.quote_id).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let Json(quote): Json<credits::Quote> = q.try_get("snapshot")?;
        if q.try_get::<Option<i64>, _>("seller_user_id")?.is_some() {
            Self::check_version(
                &mut tx,
                q.try_get::<i64, _>("seller_user_id")?,
                &quote.service_id,
                &quote.version_id,
            )
            .await?;
        }
        if quote.tool_id != r.tool_id
            || quote.version_id != r.version_id
            || quote.expires <= now
            || (q.try_get::<String, _>("visibility")? == "private"
                && q.try_get::<Option<i64>, _>("seller_user_id")? != Some(actor.user_id()))
        {
            return Err(Error::Forbidden);
        }
        let grant=sqlx::query("SELECT * FROM core_tool_market.credit_consents WHERE id=$1 AND actor_user_id=$2 FOR UPDATE").bind(&r.grant_id).bind(actor.user_id()).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let Json(consent): Json<ConsentRequest> = grant.try_get("request")?;
        let held = quote.price.ceiling()?;
        self.financial_admission(&mut tx, &quote, held).await?;
        if grant.try_get::<i64, _>("revoked_at")? != 0
            || grant.try_get::<String, _>("quote_id")? != quote.id
            || consent.expires_at <= now
            || r.resolve_by > consent.expires_at
            || consent.client_id != r.client_id
            || consent.tool_id != r.tool_id
            || consent.version_id != r.version_id
            || held > consent.max_price_quota
            || grant.try_get::<i64, _>("reserved_calls")?
                + grant.try_get::<i64, _>("successful_calls")?
                >= consent.max_calls
            || held
                > consent.max_total_quota
                    - grant.try_get::<i64, _>("reserved_credits")?
                    - grant.try_get::<i64, _>("spent_credits")?
        {
            return Err(Error::Forbidden);
        }
        let policy =
            sqlx::query("SELECT * FROM core_tool_market.credit_policy WHERE singleton FOR SHARE")
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::Forbidden)?;
        let builtin: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_tool_market.credit_builtin_bindings WHERE quote_id=$1)",
        )
        .bind(&r.quote_id)
        .fetch_one(&mut *tx)
        .await?;
        if builtin {
            if held != 0 {
                return Err(Error::Forbidden);
            }
        } else {
            if !policy.try_get::<bool, _>("enabled")? {
                return Err(Error::Forbidden);
            }
            let recipient: Option<i64> = policy.try_get("platform_user_id")?;
            let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND u.active AND a.active AND u.platform_role='superadmin')").bind(recipient).fetch_one(&mut *tx).await?;
            if !valid {
                return Err(Error::Forbidden);
            }
        }
        let platform: Option<i64> = policy.try_get("platform_account_id")?;
        let seller: Option<i64> = q.try_get("seller_account_id")?;
        if held > 0 && (platform.is_none() || seller.is_none()) {
            return Err(Error::Forbidden);
        }
        for (scope, scope_id) in [
            ("account", ""),
            ("client", r.client_id.as_str()),
            ("tool", r.tool_id.as_str()),
        ] {
            let default = super::credit_access::BudgetRequest {
                owner: r.owner.clone(),
                operation_id: String::new(),
                scope: scope.into(),
                scope_id: scope_id.into(),
                limit_quota: credits::MAX_CREDITS,
            };
            sqlx::query("INSERT INTO core_tool_market.credit_budgets(actor_user_id,scope,scope_id,request,limit_credits) VALUES($1,$2,$3,$4,$5) ON CONFLICT(actor_user_id,scope,scope_id) DO NOTHING").bind(actor.user_id()).bind(scope).bind(scope_id).bind(Json(default)).bind(credits::MAX_CREDITS).execute(&mut *tx).await?;
        }
        let caps = budgets(&mut tx, actor.user_id(), &r.client_id, &r.tool_id).await?;
        for b in &caps {
            if held
                > b.try_get::<i64, _>("limit_credits")?
                    - b.try_get::<i64, _>("reserved_credits")?
                    - b.try_get::<i64, _>("spent_credits")?
            {
                return Err(Error::Forbidden);
            }
        }
        if let Some(family) = market_oauth_family_id {
            // The OAuth ceremony is a separate lifetime spending ceiling.
            // All refresh tokens retain this same family; pending/UNKNOWN holds
            // remain counted until Core records actual capture or release.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(format!("credit-oauth-budget:{family}"))
                .execute(&mut *tx)
                .await?;
            let family_budget:i64=sqlx::query_scalar("SELECT budget_credits FROM core_market_oauth.families WHERE id=$1 AND user_id=$2 FOR SHARE").bind(family).bind(actor.user_id()).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
            let fits:bool=sqlx::query_scalar("SELECT $2::NUMERIC+COALESCE(sum(amount_credits-refunded),0)<=$3::NUMERIC FROM core_charging.external_consumption WHERE market_oauth_family_id=$1").bind(family).bind(held).bind(family_budget).fetch_one(&mut *tx).await?;
            if !fits {
                return Err(Error::Forbidden);
            }
        }
        let consume = format!("credit-tool:{id}");
        let reservation = if held > 0 {
            sqlx::query("INSERT INTO core_charging.external_consumption(id,domain,actor_user_id,key_id,payer_account_id,authorized_at,amount_credits,market_token_id,market_oauth_family_id) VALUES($1,'toolmarket',$2,$3,$4,$5,$6,$7,$8)").bind(&consume).bind(actor.user_id()).bind(session_id).bind(actor.payer_account_id()).bind(now).bind(held).bind(market_token_id).bind(market_oauth_family_id).execute(&mut *tx).await?;
            charging::external_budgets(&mut tx, &consume)
                .await
                .map_err(|e| match e {
                    charging::Error::BudgetExceeded => Error::Forbidden,
                    charging::Error::Retry | charging::Error::Conflict => Error::Retry,
                    _ => Error::Storage,
                })?;
            for account in [Some(actor.payer_account_id()), seller, platform]
                .into_iter()
                .flatten()
            {
                let _: Json<Wallet> =
                    sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
                        .bind(account)
                        .fetch_one(&mut *tx)
                        .await?;
            }
            Some(
                post(
                    &mut tx,
                    &id,
                    "reserve",
                    actor.user_id(),
                    Action::Reserve {
                        account_id: actor.payer_account_id(),
                        amount_units: Credits::try_from(held).map_err(|_| Error::Invalid)?,
                    },
                )
                .await?,
            )
        } else {
            None
        };
        let snap = Snapshot {
            request: r.clone(),
            quote: quote.clone(),
            seller_account_id: seller,
            platform_account_id: platform,
            fee_bps: policy.try_get("fee_bps")?,
            policy_version: policy.try_get("version")?,
            seller_user_id: q.try_get("seller_user_id")?,
            platform_user_id: policy.try_get("platform_user_id")?,
        };
        let out = Receipt {
            call_id: id.clone(),
            owner: r.owner.clone(),
            client_id: r.client_id.clone(),
            request_key: r.request_key.clone(),
            quote_id: r.quote_id.clone(),
            grant_id: r.grant_id.clone(),
            tool_id: r.tool_id.clone(),
            version_id: r.version_id.clone(),
            input_digest: r.input_digest.clone(),
            pricing_digest: quote.pricing_digest.clone(),
            created_at: now,
            resolve_by: r.resolve_by,
            state: "reserved".into(),
            reservation: reservation
                .map(|v| format!("credit:journal:{v}"))
                .unwrap_or_default(),
            held_quota: held,
            charged_quota: 0,
            fee_quota: 0,
            seller_quota: 0,
            refunded_quota: 0,
            dispatch_id: String::new(),
            dispatch_won: false,
            result_digest: String::new(),
        };
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id())
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO core_tool_market.credit_calls(id,actor_user_id,session_id,auth_version,payer_account_id,consent_id,quote_id,snapshot,held_credits,reserve_journal_id,state,receipt,consumption_id,created_at,resolve_by,market_token_id,market_oauth_family_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'reserved',$11,$12,$13,$14,$15,$16)").bind(&id).bind(actor.user_id()).bind(session_id).bind(version).bind(actor.payer_account_id()).bind(&r.grant_id).bind(&r.quote_id).bind(Json(&snap)).bind(held).bind(reservation).bind(Json(&out)).bind(if held>0{Some(consume)}else{None}).bind(now).bind(r.resolve_by).bind(market_token_id).bind(market_oauth_family_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_tool_market.credit_consents SET reserved_calls=reserved_calls+1,reserved_credits=reserved_credits+$2 WHERE id=$1").bind(&r.grant_id).bind(held).execute(&mut *tx).await?;
        for b in caps {
            sqlx::query("UPDATE core_tool_market.credit_budgets SET reserved_credits=reserved_credits+$4 WHERE actor_user_id=$1 AND scope=$2 AND scope_id=$3").bind(actor.user_id()).bind(b.try_get::<String,_>("scope")?).bind(b.try_get::<String,_>("scope_id")?).bind(held).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(out)
    }
    pub async fn dispatch(&self, secret: &str, id: &str) -> Result<Receipt> {
        if !digest(id) {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = super::credit_actor::authenticate(
            &mut tx,
            self.market_profile.as_deref(),
            secret,
            super::credit_actor::MarketAction::Invoke,
        )
        .await?;
        let row = row(&mut tx, id).await?;
        let Json(mut out): Json<Receipt> = row.try_get("receipt")?;
        actor.assert_client(&out.client_id)?;
        let (session, token, family) = source_columns(actor.source());
        if row.try_get::<Option<i64>, _>("session_id")? != session
            || row
                .try_get::<Option<String>, _>("market_token_id")?
                .as_deref()
                != token
            || row
                .try_get::<Option<String>, _>("market_oauth_family_id")?
                .as_deref()
                != family
        {
            return Err(Error::Forbidden);
        }

        if row.try_get::<i64, _>("actor_user_id")? != actor.user_id() {
            return Err(Error::Forbidden);
        }
        // A replay cannot regain single-winner dispatch authority, even on a new host.
        if out.state != "reserved" {
            tx.commit().await?;
            return Ok(out);
        }
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id())
        .fetch_one(&mut *tx)
        .await?;
        let now = ToolMarketStore::now(&mut tx).await?;
        let grant = Self::consent(&mut tx, &out.grant_id, actor.user_id()).await?;
        let Json(snapshot): Json<Snapshot> = row.try_get("snapshot")?;
        self.financial_admission(&mut tx, &snapshot.quote, out.held_quota)
            .await?;
        if snapshot.seller_user_id.is_some() {
            Self::check_version(
                &mut tx,
                snapshot.seller_user_id.ok_or(Error::Forbidden)?,
                &snapshot.quote.service_id,
                &snapshot.quote.version_id,
            )
            .await?;
        }
        let approved: bool = sqlx::query_scalar(
            "SELECT approved FROM core_tool_market.credit_quotes WHERE id=$1 FOR SHARE",
        )
        .bind(&out.quote_id)
        .fetch_one(&mut *tx)
        .await?;
        if let Some(seller_user) = snapshot.seller_user_id {
            let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_tool_market.publishers p JOIN core_identity.users u ON u.id=p.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_tool_market.credit_policy policy ON policy.singleton WHERE p.user_id=$1 AND p.active AND u.active AND a.active AND policy.enabled)").bind(seller_user).fetch_one(&mut *tx).await?;
            if !live {
                return Err(Error::Forbidden);
            }
        }
        if version != row.try_get::<i64, _>("auth_version")?
            || grant.revoked_at != 0
            || grant.request.expires_at <= now
            || out.resolve_by <= now
            || !approved
        {
            return Err(Error::Forbidden);
        }
        crate::identity::reject_marked_new_action(
            &mut tx,
            actor.user_id(),
            crate::identity::NewResourceAction::CommercePayment,
        )
        .await?;
        out.state = "running".into();
        out.dispatch_id = hash(&[
            "credit-dispatch-v2",
            id,
            &out.input_digest,
            &version.to_string(),
        ])?;
        save(&mut tx, &out, None, None).await?;
        tx.commit().await?;
        out.dispatch_won = true;
        Ok(out)
    }
    pub async fn unknown(&self, r: &DispatchRequest) -> Result<Receipt> {
        let mut tx = self.base.begin().await?;
        let row = row(&mut tx, &r.call_id).await?;
        let Json(mut out): Json<Receipt> = row.try_get("receipt")?;
        if r.dispatch_id.is_empty() || out.dispatch_id != r.dispatch_id {
            return Err(Error::Conflict);
        }
        if out.state == "unknown" || matches!(out.state.as_str(), "settled" | "released") {
            tx.commit().await?;
            return Ok(out);
        }
        if out.state != "running" {
            return Err(Error::Conflict);
        }
        out.state = "unknown".into();
        save(&mut tx, &out, None, None).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn settle(&self, r: &SettleRequest) -> Result<Receipt> {
        self.settle_authorized(r, None).await
    }
    pub(super) async fn settle_fixed_witness(
        &self,
        r: &SettleRequest,
        role: &str,
    ) -> Result<Receipt> {
        self.settle_authorized(r, Some(role)).await
    }
    async fn settle_authorized(
        &self,
        r: &SettleRequest,
        witness_role: Option<&str>,
    ) -> Result<Receipt> {
        if !digest(&r.result_digest) {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let row = row(&mut tx, &r.call_id).await?;
        let Json(mut out): Json<Receipt> = row.try_get("receipt")?;
        if r.dispatch_id.is_empty() || out.dispatch_id != r.dispatch_id {
            return Err(Error::Conflict);
        }
        if out.state == "settled" {
            let saved: Option<Json<Usage>> = row.try_get("usage")?;
            if out.result_digest != r.result_digest
                || r.usage
                    .as_ref()
                    .is_some_and(|u| saved.as_ref().is_none_or(|Json(v)| !usage_matches(u, v)))
            {
                return Err(Error::Conflict);
            }
            if out.held_quota > 0 {
                use base64::Engine as _;
                let encoded = r.raw_result.as_ref().ok_or(Error::Conflict)?;
                if encoded.len() > 2_800_000 {
                    return Err(Error::Invalid);
                }
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| Error::Invalid)?;
                let supplier:Option<Vec<u8>>=sqlx::query_scalar("SELECT raw_result FROM core_tool_market.credit_supplier_receipts WHERE call_id=$1").bind(&out.call_id).fetch_optional(&mut *tx).await?;
                let host:Option<Vec<u8>>=sqlx::query_scalar("SELECT raw_result FROM core_tool_market.credit_fixed_witness_receipts WHERE call_id=$1").bind(&out.call_id).fetch_optional(&mut *tx).await?;
                let original = match (supplier, host) {
                    (Some(v), None) | (None, Some(v)) => v,
                    _ => return Err(Error::Storage),
                };
                if Sha256::digest(&raw) != Sha256::digest(&original) {
                    return Err(Error::Conflict);
                }
            }
            tx.commit().await?;
            return Ok(out);
        }
        if !matches!(out.state.as_str(), "running" | "unknown") {
            return Err(Error::Conflict);
        }
        let Json(snap): Json<Snapshot> = row.try_get("snapshot")?;
        let contract = if out.held_quota > 0 {
            Some(Self::result_contract(&mut tx, &out.quote_id).await?)
        } else {
            None
        };
        let verified;
        let mut authoritative_usage = None;
        let quantities = match &snap.quote.price {
            Price::Metered { rules } if out.held_quota > 0 => {
                let (mode, _, schema) = contract.as_ref().ok_or(Error::Forbidden)?;
                if mode != "supplier_signed" || witness_role.is_some() {
                    return Err(Error::Forbidden);
                }
                use base64::Engine as _;
                let raw = r.raw_result.as_ref().ok_or(Error::Forbidden)?;
                if raw.len() > 2_800_000 {
                    return Err(Error::Invalid);
                }
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(raw)
                    .map_err(|_| Error::Invalid)?;
                let limits = rules
                    .iter()
                    .map(|v| (v.metric.clone(), v.max_quantity))
                    .collect();
                let supplier_digest = hash(rules)?;
                let expected = super::receipt_verifier::ExpectedReceipt {
                    call_id: &out.call_id,
                    service_id: &snap.quote.service_id,
                    tool_id: &out.tool_id,
                    version_id: &out.version_id,
                    input_digest: &out.input_digest,
                    pricing_digest: &supplier_digest,
                    remote_digest: &snap.quote.remote_digest,
                    metrics: &limits,
                };
                verified = self
                    .verifier
                    .as_ref()
                    .ok_or(Error::Forbidden)?
                    .verify(&expected, &raw)
                    .map_err(|_| Error::Forbidden)?;
                if verified.result_digest() != r.result_digest
                    || !verified.successful()
                    || r.usage
                        .as_ref()
                        .is_some_and(|u| &u.quantities != verified.quantities())
                {
                    return Err(Error::Forbidden);
                }
                super::result_contract::validate_mcp(verified.result())?;
                super::result_contract::validate_schema(
                    schema,
                    verified
                        .result()
                        .get("structuredContent")
                        .unwrap_or(&serde_json::Value::Null),
                )?;
                sqlx::query("INSERT INTO core_tool_market.credit_supplier_receipts(receipt_hash,call_id,raw_receipt_hash,result_digest,raw_result,outcome) VALUES($1,$2,$3,$4,$5,'success')")
                    .bind(verified.receipt_hash()).bind(&out.call_id).bind(verified.raw_receipt_hash()).bind(verified.result_digest()).bind(&raw).execute(&mut *tx).await?;
                authoritative_usage = Some(Usage {
                    source: credits::UsageSource::ToolReported,
                    call_id: out.call_id.clone(),
                    version_id: out.version_id.clone(),
                    tool_id: out.tool_id.clone(),
                    input_digest: out.input_digest.clone(),
                    pricing_digest: out.pricing_digest.clone(),
                    result_digest: r.result_digest.clone(),
                    quantities: verified.quantities().clone(),
                });
                Some(verified.quantities())
            }
            Price::Fixed { .. } if out.held_quota > 0 => {
                use base64::Engine as _;
                let (mode, role, schema) = contract.as_ref().ok_or(Error::Forbidden)?;
                if mode != "core_host_fixed_success"
                    || witness_role != Some(role.as_str())
                    || r.usage.is_some()
                {
                    return Err(Error::Forbidden);
                }
                let encoded = r.raw_result.as_ref().ok_or(Error::Forbidden)?;
                if encoded.len() > 2_800_000 {
                    return Err(Error::Invalid);
                }
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| Error::Invalid)?;
                let result = super::receipt_verifier::decode_original_result(&raw)
                    .map_err(|_| Error::Invalid)?;
                super::result_contract::validate_mcp(&result)?;
                if !super::receipt_verifier::successful_result(&result)
                    || super::receipt_verifier::result_digest(&result)
                        .map_err(|_| Error::Invalid)?
                        != r.result_digest
                {
                    return Err(Error::Forbidden);
                }
                super::result_contract::validate_schema(
                    schema,
                    result
                        .get("structuredContent")
                        .unwrap_or(&serde_json::Value::Null),
                )?;
                sqlx::query("INSERT INTO core_tool_market.credit_fixed_witness_receipts(call_id,witness_role,raw_result,raw_digest,result_digest,outcome) VALUES($1,$2,$3,$4,$5,'success')").bind(&out.call_id).bind(role).bind(&raw).bind(format!("{:x}",Sha256::digest(&raw))).bind(&r.result_digest).execute(&mut *tx).await?;
                None
            }
            _ if out.held_quota > 0 => return Err(Error::Forbidden),
            _ => r.usage.as_ref().map(|u| &u.quantities),
        };
        if let Some(u) = &r.usage
            && (u.call_id != out.call_id
                || u.tool_id != out.tool_id
                || u.version_id != out.version_id
                || u.input_digest != out.input_digest
                || u.pricing_digest != out.pricing_digest
                || u.result_digest != r.result_digest)
        {
            return Err(Error::Invalid);
        }
        let economic = credits::economics(&snap.quote.price, quantities, snap.fee_bps)?;
        if economic.held != out.held_quota {
            return Err(Error::Conflict);
        }
        let journal = if out.held_quota > 0 {
            Some(
                post(
                    &mut tx,
                    &out.call_id,
                    "settle",
                    row.try_get("actor_user_id")?,
                    Action::SplitCommerceCapture {
                        reservation_journal_id: row.try_get("reserve_journal_id")?,
                        seller_account_id: snap.seller_account_id.ok_or(Error::Invalid)?,
                        platform_account_id: snap.platform_account_id.ok_or(Error::Invalid)?,
                        amount_units: Credits::try_from(economic.charged)
                            .map_err(|_| Error::Invalid)?,
                        fee_basis_points: snap.fee_bps,
                    },
                )
                .await?,
            )
        } else {
            None
        };
        Self::finish_counters(&mut tx, &row, &out, economic.charged, true).await?;
        out.state = "settled".into();
        out.charged_quota = economic.charged;
        out.fee_quota = economic.fee;
        out.seller_quota = economic.seller;
        out.refunded_quota = economic.refunded;
        out.result_digest = r.result_digest.clone();
        save(&mut tx, &out, authoritative_usage.as_ref(), journal).await?;
        tx.commit().await?;
        Ok(out)
    }
    async fn finish_counters(
        tx: &mut Transaction<'_, Postgres>,
        row: &PgRow,
        out: &Receipt,
        charged: i64,
        success: bool,
    ) -> Result<()> {
        // Lock the grant and ALL matching caps before reducing held/add spent.
        let _: i64 = sqlx::query_scalar(
            "SELECT reserved_calls FROM core_tool_market.credit_consents WHERE id=$1 FOR UPDATE",
        )
        .bind(&out.grant_id)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query("UPDATE core_tool_market.credit_consents SET reserved_calls=reserved_calls-1,reserved_credits=reserved_credits-$2,spent_credits=spent_credits+$3,successful_calls=successful_calls+$4 WHERE id=$1").bind(&out.grant_id).bind(out.held_quota).bind(charged).bind(if success{1_i64}else{0}).execute(&mut **tx).await?;
        let actor: i64 = row.try_get("actor_user_id")?;
        for b in budgets(tx, actor, &out.client_id, &out.tool_id).await? {
            sqlx::query("UPDATE core_tool_market.credit_budgets SET reserved_credits=reserved_credits-$4,spent_credits=spent_credits+$5 WHERE actor_user_id=$1 AND scope=$2 AND scope_id=$3").bind(actor).bind(b.try_get::<String,_>("scope")?).bind(b.try_get::<String,_>("scope_id")?).bind(out.held_quota).bind(charged).execute(&mut **tx).await?;
        }
        if out.held_quota > 0 {
            let result=sqlx::query("UPDATE core_charging.external_consumption SET refunded=amount_credits-$2 WHERE id=$1 AND refunded=0").bind(row.try_get::<String,_>("consumption_id")?).bind(charged).execute(&mut **tx).await?;
            if result.rows_affected() != 1 {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }
    pub async fn release(&self, r: &ReleaseRequest) -> Result<Receipt> {
        self.release_authorized(r, None).await
    }
    pub(super) async fn release_fixed_witness(
        &self,
        r: &ReleaseRequest,
        role: &str,
    ) -> Result<Receipt> {
        self.release_authorized(r, Some(role)).await
    }
    async fn release_authorized(
        &self,
        r: &ReleaseRequest,
        witness_role: Option<&str>,
    ) -> Result<Receipt> {
        if !matches!(r.reason.as_str(), "failed" | "cancelled" | "expired") {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let row = row(&mut tx, &r.call_id).await?;
        let Json(mut out): Json<Receipt> = row.try_get("receipt")?;
        let now = ToolMarketStore::now(&mut tx).await?;
        // Deadline expiry does not prove an external effect did not happen.
        // Only the original, never-claimed reservation may be abandoned.
        if out.dispatch_id != r.dispatch_id {
            return Err(Error::Conflict);
        }
        if out.state == "released" {
            if !out.result_digest.is_empty() && out.result_digest != r.evidence_digest {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(out);
        }
        if out.state == "settled" {
            return Err(Error::Conflict);
        }
        if out.state == "reserved" && out.dispatch_id.is_empty() {
            match r.reason.as_str() {
                "expired" if now >= out.resolve_by => {}
                "cancelled" => {}
                _ => return Err(Error::Conflict),
            }
        } else {
            if !matches!(out.state.as_str(), "running" | "unknown")
                || r.reason != "failed"
                || !digest(&r.evidence_digest)
            {
                return Err(Error::Conflict);
            }
            use base64::Engine as _;
            let encoded = r.raw_result.as_ref().ok_or(Error::Forbidden)?;
            if encoded.len() > 2_800_000 {
                return Err(Error::Invalid);
            }
            let raw = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| Error::Invalid)?;
            let Json(snap): Json<Snapshot> = row.try_get("snapshot")?;
            let (mode, role, _) = Self::result_contract(&mut tx, &out.quote_id).await?;
            match &snap.quote.price {
                Price::Fixed { .. }
                    if mode == "core_host_fixed_success" && witness_role == Some(role.as_str()) =>
                {
                    let result = super::receipt_verifier::decode_original_result(&raw)
                        .map_err(|_| Error::Invalid)?;
                    if !super::result_contract::terminal_failure(&result)?
                        || super::receipt_verifier::result_digest(&result)
                            .map_err(|_| Error::Invalid)?
                            != r.evidence_digest
                    {
                        return Err(Error::Forbidden);
                    }
                    sqlx::query("INSERT INTO core_tool_market.credit_fixed_witness_receipts(call_id,witness_role,raw_result,raw_digest,result_digest,outcome) VALUES($1,$2,$3,$4,$5,'failure')").bind(&out.call_id).bind(role).bind(&raw).bind(format!("{:x}",Sha256::digest(&raw))).bind(&r.evidence_digest).execute(&mut *tx).await?;
                }
                Price::Metered { rules } if mode == "supplier_signed" && witness_role.is_none() => {
                    let limits = rules
                        .iter()
                        .map(|v| (v.metric.clone(), v.max_quantity))
                        .collect();
                    let supplier_digest = hash(rules)?;
                    let expected = super::receipt_verifier::ExpectedReceipt {
                        call_id: &out.call_id,
                        service_id: &snap.quote.service_id,
                        tool_id: &out.tool_id,
                        version_id: &out.version_id,
                        input_digest: &out.input_digest,
                        pricing_digest: &supplier_digest,
                        remote_digest: &snap.quote.remote_digest,
                        metrics: &limits,
                    };
                    let verified = self
                        .verifier
                        .as_ref()
                        .ok_or(Error::Forbidden)?
                        .verify(&expected, &raw)
                        .map_err(|_| Error::Forbidden)?;
                    if verified.result_digest() != r.evidence_digest
                        || !super::result_contract::terminal_failure(verified.result())?
                    {
                        return Err(Error::Forbidden);
                    }
                    sqlx::query("INSERT INTO core_tool_market.credit_supplier_receipts(receipt_hash,call_id,raw_receipt_hash,result_digest,raw_result,outcome) VALUES($1,$2,$3,$4,$5,'failure')").bind(verified.receipt_hash()).bind(&out.call_id).bind(verified.raw_receipt_hash()).bind(verified.result_digest()).bind(&raw).execute(&mut *tx).await?;
                }
                _ => return Err(Error::Forbidden),
            }
            out.result_digest = r.evidence_digest.clone();
        }
        let journal = if out.held_quota > 0 {
            Some(
                post(
                    &mut tx,
                    &out.call_id,
                    "release",
                    row.try_get("actor_user_id")?,
                    Action::Release {
                        reservation_journal_id: row.try_get("reserve_journal_id")?,
                    },
                )
                .await?,
            )
        } else {
            None
        };
        Self::finish_counters(&mut tx, &row, &out, 0, false).await?;
        out.state = "released".into();
        out.refunded_quota = out.held_quota;
        save(&mut tx, &out, None, journal).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn query(&self, secret: &str, id: &str) -> Result<Option<Receipt>> {
        let mut tx = self.base.begin().await?;
        let actor = super::credit_actor::authenticate(
            &mut tx,
            self.market_profile.as_deref(),
            secret,
            super::credit_actor::MarketAction::Discover,
        )
        .await?;
        let out: Option<Json<Receipt>> = sqlx::query_scalar(
            "SELECT receipt FROM core_tool_market.credit_calls WHERE id=$1 AND actor_user_id=$2",
        )
        .bind(id)
        .bind(actor.user_id())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(Json(v)) = &out {
            actor.assert_client(&v.client_id)?;
        }
        tx.commit().await?;
        Ok(out.map(|Json(v)| v))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuiltinContext {
    pub call_id: String,
    pub dispatch_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub grant_id: String,
    pub input_digest: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinKind {
    service: &'static str,
    tool: &'static str,
}
impl BuiltinKind {
    pub fn code_owned(service: &'static str, tool: &'static str) -> Result<Self> {
        if !matches!(service, "wallet" | "drawing" | "open_source_bounties")
            || tool.is_empty()
            || tool.len() > 128
            || !tool
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err(Error::Invalid);
        }
        Ok(Self { service, tool })
    }
    pub fn name(self) -> String {
        format!("{}.{}", self.service, self.tool)
    }
}
pub struct BuiltinActor {
    pub user_id: i64,
    pub owner_account_id: i64,
    pub call_id: String,
    pub dispatch_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub input_schema_digest: String,
    _verified: (),
}
impl CreditStore {
    pub async fn validate_builtin_context(
        c: &mut PgConnection,
        secret: &str,
        context: &BuiltinContext,
        kind: BuiltinKind,
    ) -> Result<BuiltinActor> {
        let serializable: bool =
            sqlx::query_scalar("SELECT current_setting('transaction_isolation')='serializable'")
                .fetch_one(&mut *c)
                .await?;
        if !serializable {
            return Err(Error::Invalid);
        }
        let actor = crate::identity::authorize_in_transaction(&mut *c, secret).await?;
        if actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != crate::accounts::AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(Error::Forbidden);
        }
        sqlx::query("SELECT core_tool_market.lock_builtin_context($1)")
            .bind(&context.call_id)
            .execute(&mut *c)
            .await?;
        let row=sqlx::query("SELECT c.*,g.request,g.revoked_at,q.approved,b.kind,b.input_schema_digest FROM core_tool_market.credit_calls c JOIN core_tool_market.credit_consents g ON g.id=c.consent_id JOIN core_tool_market.credit_quotes q ON q.id=c.quote_id JOIN core_tool_market.credit_builtin_bindings b ON b.quote_id=q.id WHERE c.id=$1 AND c.actor_user_id=$2").bind(&context.call_id).bind(actor.user_id).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
        let Json(receipt): Json<Receipt> = row.try_get("receipt")?;
        let Json(grant): Json<ConsentRequest> = row.try_get("request")?;
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *c)
        .await?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *c)
                .await?;
        if !matches!(receipt.state.as_str(), "running" | "unknown")
            || receipt.dispatch_id != context.dispatch_id
            || receipt.tool_id != context.tool_id
            || receipt.version_id != context.version_id
            || receipt.grant_id != context.grant_id
            || receipt.input_digest != context.input_digest
            || receipt.resolve_by <= now
            || row.try_get::<i64, _>("auth_version")? != version
            || row.try_get::<i64, _>("revoked_at")? != 0
            || grant.expires_at <= now
            || !row.try_get::<bool, _>("approved")?
            || row.try_get::<String, _>("kind")? != kind.name()
        {
            return Err(Error::Forbidden);
        }
        Ok(BuiltinActor {
            user_id: actor.user_id,
            owner_account_id: actor.owner_account_id,
            call_id: receipt.call_id,
            dispatch_id: receipt.dispatch_id,
            tool_id: receipt.tool_id,
            version_id: receipt.version_id,
            input_schema_digest: row.try_get("input_schema_digest")?,
            _verified: (),
        })
    }
}
