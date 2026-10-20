//! Tool-market consent and fixed-price escrow. Money is posted exclusively to
//! the existing Core ledger; the extension never chooses a payer or recipient.
pub use crate::commerce::Error;
use crate::{
    billing::{
        Credits, charging,
        ledger::{Action, Outcome, Request, Wallet},
    },
    identity::{CredentialKind, Principal},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, types::Json};
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) mod assistant_consent;
pub mod builtin_install;
pub mod credit_access;
pub mod credit_actor;
pub mod credit_config;
pub mod credit_execution;
pub mod credit_http;
pub mod credit_lifecycle;
pub mod credit_meta;
pub mod credit_reports;
#[cfg(test)]
pub(crate) mod credit_test_fixture;
pub mod credits;
pub mod fixed_witness_http;
pub mod http;
pub mod receipt_verifier;
mod result_contract;
pub mod server;
#[cfg(test)]
mod tests;
pub const SCHEMA: &str = include_str!("../../schema/tool_market.sql");
pub const POLICY: &str = "exact-usd-micros-500k-v1";
pub const AUDIENCE: &str = "lmm-toolmarket/grant/v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub user: String,
    pub account: String,
}
// Field order intentionally matches Go ConfirmationIntent's canonical digest.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub audience: String,
    pub grant: String,
    pub owner: Owner,
    pub installation: String,
    pub release: String,
    pub tool: String,
    pub handle: String,
    pub quote: String,
    pub arguments_hash: String,
    pub currency: String,
    pub generation: i64,
    pub amount: i64,
    pub max_calls: i64,
    pub max_amount: i64,
    pub expires: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: String,
    pub audience: String,
    pub intent_hash: String,
    pub owner: Owner,
    pub actor_kind: String,
    pub valid_until: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Quote {
    pub id: String,
    pub release: String,
    pub tool: String,
    pub version: String,
    pub price_hash: String,
    pub amount: i64,
    pub currency: String,
    pub expires: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub id: String,
    pub installation: String,
    pub tool: String,
    pub quote: String,
    pub currency: String,
    pub amount: i64,
    pub release: String,
    pub grant: String,
    pub handle: String,
    pub arguments_hash: String,
    pub approval: String,
    pub generation: i64,
    pub idempotency_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub operation_id: String,
    pub reservation: String,
    pub state: String,
    pub credits: i64,
    pub journal_id: i64,
}
#[derive(Clone)]
pub struct ToolMarketStore {
    pool: PgPool,
}
fn hash<T: Serialize>(v: &T) -> Result<String> {
    Ok(
        Sha256::digest(serde_json::to_vec(v).map_err(|_| Error::Invalid)?)
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect(),
    )
}
fn name(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 80
        && v.bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"_.-".contains(&v))
}
fn digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
/// Fixed prices must be exactly representable. No per-call rounding or hidden
/// cross-call debt: one micro USD is precisely half one integer ledger credit.
pub fn credits(amount: i64) -> Result<i64> {
    if amount <= 0 || amount % 2 != 0 {
        Err(Error::Invalid)
    } else {
        Ok(amount / 2)
    }
}
impl ToolMarketStore {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn begin(&self) -> Result<Transaction<'static, Postgres>> {
        let mut tx = self.pool.begin().await?;
        for q in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL synchronous_commit=on",
            "SET LOCAL lock_timeout='500ms'",
            "SET LOCAL statement_timeout='2000ms'",
        ] {
            sqlx::query(q).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    async fn actor(tx: &mut Transaction<'_, Postgres>, secret: &str) -> Result<Principal> {
        let actor = crate::identity::authorize_in_transaction(tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != crate::accounts::AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(Error::Forbidden);
        }
        Ok(actor)
    }
    async fn now(tx: &mut Transaction<'_, Postgres>) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut **tx)
                .await?,
        )
    }
    /// Publisher admission is installer-owned. Runtime may only publish immutable
    /// releases to its own reviewed seller identity, never a submitted account.
    pub async fn register_quote(&self, secret: &str, quote: &Quote) -> Result<Quote> {
        if ![&quote.id, &quote.release, &quote.tool, &quote.version]
            .into_iter()
            .all(|s| name(s))
            || !digest(&quote.price_hash)
            || quote.id != quote.price_hash
            || quote.currency != "USD"
        {
            return Err(Error::Invalid);
        }
        let amount = if quote.amount == 0 {
            0
        } else {
            credits(quote.amount)?
        };
        let mut tx = self.begin().await?;
        let actor = Self::actor(&mut tx, secret).await?;
        let now = Self::now(&mut tx).await?;
        if quote.expires <= now {
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
        sqlx::query("INSERT INTO core_tool_market.quotes(id,seller_user_id,seller_account_id,quote,credits,policy) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING").bind(&quote.id).bind(actor.user_id).bind(actor.owner_account_id).bind(Json(quote)).bind(amount).bind(POLICY).execute(&mut *tx).await?;
        let row =
            sqlx::query("SELECT seller_user_id,quote FROM core_tool_market.quotes WHERE id=$1")
                .bind(&quote.id)
                .fetch_one(&mut *tx)
                .await?;
        let Json(saved): Json<Quote> = row.try_get("quote")?;
        if row.try_get::<i64, _>("seller_user_id")? != actor.user_id || &saved != quote {
            return Err(Error::Conflict);
        }
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn confirm(&self, secret: &str, intent: &Intent) -> Result<Evidence> {
        if intent.audience != AUDIENCE
            || intent.currency != "USD"
            || intent.generation <= 0
            || ![
                &intent.grant,
                &intent.installation,
                &intent.release,
                &intent.tool,
                &intent.handle,
                &intent.quote,
            ]
            .into_iter()
            .all(|v| name(v))
            || !digest(&intent.arguments_hash)
            || !digest(&intent.handle)
            || !digest(&intent.quote)
            || intent.max_calls <= 0
            || intent.max_calls > 1024
            || intent.max_amount < intent.amount
        {
            return Err(Error::Invalid);
        }
        if intent.amount != 0 {
            credits(intent.amount)?;
        }
        let mut tx = self.begin().await?;
        let actor = Self::actor(&mut tx, secret).await?;
        let owner = Owner {
            user: actor.user_id.to_string(),
            account: actor.user_id.to_string(),
        };
        if owner != intent.owner {
            return Err(Error::Forbidden);
        }
        let now = Self::now(&mut tx).await?;
        if intent.expires <= now || intent.expires > now + 86400 {
            return Err(Error::Invalid);
        }
        let row=sqlx::query("SELECT q.quote,q.seller_account_id FROM core_tool_market.quotes q JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id JOIN core_identity.users u ON u.id=p.user_id JOIN core_identity.accounts a ON a.id=q.seller_account_id WHERE q.id=$1 AND p.active AND u.active AND a.active ").bind(&intent.quote).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        let Json(q): Json<Quote> = row.try_get("quote")?;
        if q.release != intent.release
            || q.tool != intent.tool
            || q.amount != intent.amount
            || q.expires < intent.expires
            || (intent.amount > 0
                && row.try_get::<i64, _>("seller_account_id")? == actor.owner_account_id)
        {
            return Err(Error::Conflict);
        }
        let intent_hash = hash(intent)?;
        let id = hash(&["tool-market-approval-v1", &intent_hash])?;
        let evidence = Evidence {
            id: id.clone(),
            audience: AUDIENCE.into(),
            intent_hash,
            owner,
            actor_kind: "human_session".into(),
            valid_until: intent.expires.min(now + 300),
        };
        sqlx::query("INSERT INTO core_tool_market.approvals(id,grant_id,actor_user_id,payer_account_id,intent,evidence) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING").bind(&id).bind(&intent.grant).bind(actor.user_id).bind(actor.owner_account_id).bind(Json(intent)).bind(Json(&evidence)).execute(&mut *tx).await?;
        let Json(saved): Json<Evidence> =
            sqlx::query_scalar("SELECT evidence FROM core_tool_market.approvals WHERE id=$1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn reserve(&self, secret: &str, call: &Call) -> Result<Receipt> {
        if !name(&call.id) || !digest(&call.approval) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let actor = Self::actor(&mut tx, secret).await?;
        if !name(&call.idempotency_key)
            || call.id
                != hash(&[
                    "toolmarket/v1",
                    &format!("{}:{}", actor.user_id, actor.user_id),
                    &call.idempotency_key,
                ])?
        {
            return Err(Error::Invalid);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("tool-market:{}", call.id))
            .execute(&mut *tx)
            .await?;
        if let Some(row) =
            sqlx::query("SELECT actor_user_id,call,receipt FROM core_tool_market.calls WHERE id=$1")
                .bind(&call.id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let Json(saved): Json<Call> = row.try_get("call")?;
            if saved != *call || row.try_get::<i64, _>("actor_user_id")? != actor.user_id {
                return Err(Error::Conflict);
            }
            let Json(receipt) = row.try_get("receipt")?;
            tx.commit().await?;
            return Ok(receipt);
        }
        let row = sqlx::query("SELECT * FROM core_tool_market.approvals WHERE id=$1 FOR UPDATE")
            .bind(&call.approval)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
        let Json(i): Json<Intent> = row.try_get("intent")?;

        let now = Self::now(&mut tx).await?;
        if row.try_get::<i64, _>("actor_user_id")? != actor.user_id
            || row.try_get::<i64, _>("payer_account_id")? != actor.owner_account_id
            || row.try_get::<bool, _>("revoked")?
            || i.expires <= now
            || i.grant != call.grant
            || i.installation != call.installation
            || i.release != call.release
            || i.tool != call.tool
            || i.handle != call.handle
            || i.quote != call.quote
            || i.currency != call.currency
            || i.amount != call.amount
            || i.arguments_hash != call.arguments_hash
            || i.generation != call.generation
        {
            return Err(Error::Forbidden);
        }
        let used: i64 = row.try_get("used_calls")?;
        let spent: i64 = row.try_get("used_amount")?;
        let next = spent.checked_add(i.amount).ok_or(Error::Invalid)?;
        if used >= i.max_calls || next > i.max_amount {
            return Err(Error::Forbidden);
        }
        let q=sqlx::query("SELECT q.quote,q.seller_account_id,q.credits,q.policy FROM core_tool_market.quotes q JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id JOIN core_identity.users u ON u.id=p.user_id JOIN core_identity.accounts a ON a.id=q.seller_account_id WHERE q.id=$1 AND p.active AND u.active AND a.active ").bind(&call.quote).fetch_one(&mut *tx).await?;
        let Json(quote): Json<Quote> = q.try_get("quote")?;
        let amount: i64 = q.try_get("credits")?;
        let seller: i64 = q.try_get("seller_account_id")?;
        if quote.expires <= now
            || q.try_get::<String, _>("policy")? != POLICY
            || amount != if i.amount == 0 { 0 } else { credits(i.amount)? }
        {
            return Err(Error::Conflict);
        }
        // New spending obeys the same mark and shared budget gates as Commerce.
        crate::identity::reject_marked_new_action(
            &mut tx,
            actor.user_id,
            crate::identity::NewResourceAction::CommercePayment,
        )
        .await?;
        let consumption = format!("tool-market:{}", call.id);
        if amount > 0 {
            sqlx::query("INSERT INTO core_charging.external_consumption(id,domain,actor_user_id,key_id,payer_account_id,authorized_at,amount_credits) VALUES($1,'toolmarket',$2,$3,$4,$5,$6)").bind(&consumption).bind(actor.user_id).bind(actor.credential_id).bind(actor.owner_account_id).bind(now).bind(amount).execute(&mut *tx).await?;
            charging::external_budgets(&mut tx, &consumption)
                .await
                .map_err(|e| match e {
                    charging::Error::BudgetExceeded => Error::Forbidden,
                    charging::Error::Retry | charging::Error::Conflict => Error::Retry,
                    _ => Error::Storage,
                })?;
            for account in [actor.owner_account_id, seller] {
                let _: Json<Wallet> =
                    sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
                        .bind(account)
                        .fetch_one(&mut *tx)
                        .await?;
            }
        }
        let journal = if amount == 0 {
            0
        } else {
            post(
                &mut tx,
                &call.id,
                "reserve",
                Some(actor.user_id),
                Action::Reserve {
                    account_id: actor.owner_account_id,
                    amount_units: Credits::try_from(amount).map_err(|_| Error::Invalid)?,
                },
            )
            .await?
        };
        let receipt = Receipt {
            operation_id: call.id.clone(),
            reservation: if journal == 0 {
                format!("tool-market:free:{}", call.id)
            } else {
                format!("tool-market:journal:{journal}")
            },
            state: "reserved".into(),
            credits: amount,
            journal_id: journal,
        };
        sqlx::query("INSERT INTO core_tool_market.calls(id,approval_id,actor_user_id,payer_account_id,seller_account_id,call,reserve_journal_id,receipt,consumption_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(&call.id).bind(&call.approval).bind(actor.user_id).bind(actor.owner_account_id).bind(seller).bind(Json(call)).bind(if journal==0 {None} else {Some(journal)}).bind(Json(&receipt)).bind(if amount>0 {Some(consumption)} else {None}).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_tool_market.approvals SET used_calls=used_calls+1,used_amount=$2 WHERE id=$1").bind(&call.approval).bind(next).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(receipt)
    }
    /// Trusted toolmarket host port ONLY. It consumes the original durable
    /// reservation, never current caller-selected amounts/accounts. A dispatch
    /// lease is persisted before the external request; UNKNOWN stays reserved.
    pub async fn mark_dispatched(
        &self,
        secret: &str,
        id: &str,
        reservation: &str,
    ) -> Result<Receipt> {
        self.transition(id, reservation, "dispatched", secret).await
    }
    pub async fn commit(&self, id: &str, reservation: &str, result_hash: &str) -> Result<Receipt> {
        if !digest(result_hash) {
            return Err(Error::Invalid);
        }
        self.transition(id, reservation, "committed", result_hash)
            .await
    }
    pub async fn release_undispatched(&self, id: &str, reservation: &str) -> Result<Receipt> {
        self.transition(id, reservation, "released", "").await
    }
    async fn transition(
        &self,
        id: &str,
        reservation: &str,
        target: &str,
        result_hash: &str,
    ) -> Result<Receipt> {
        if !name(id) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let row = sqlx::query("SELECT * FROM core_tool_market.calls WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
        let Json(mut r): Json<Receipt> = row.try_get("receipt")?;
        if target == "dispatched" {
            let actor = Self::actor(&mut tx, result_hash).await?;
            if row.try_get::<i64, _>("actor_user_id")? != actor.user_id {
                return Err(Error::Forbidden);
            }
            crate::identity::reject_marked_new_action(
                &mut tx,
                actor.user_id,
                crate::identity::NewResourceAction::CommercePayment,
            )
            .await?;
            let approval = sqlx::query(
                "SELECT revoked,intent FROM core_tool_market.approvals WHERE id=$1 FOR SHARE",
            )
            .bind(row.try_get::<String, _>("approval_id")?)
            .fetch_one(&mut *tx)
            .await?;
            let Json(intent): Json<Intent> = approval.try_get("intent")?;
            if approval.try_get::<bool, _>("revoked")?
                || intent.expires <= Self::now(&mut tx).await?
            {
                return Err(Error::Forbidden);
            }
        }
        if r.reservation != reservation {
            return Err(Error::Conflict);
        }
        if r.state == target {
            if target == "committed"
                && row.try_get::<Option<String>, _>("result_hash")?.as_deref() != Some(result_hash)
            {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(r);
        }
        match (r.state.as_str(), target) {
            ("reserved", "dispatched" | "released") | ("dispatched", "committed") => {}
            _ => return Err(Error::Conflict),
        }
        if target != "dispatched" && r.credits > 0 {
            let original = row.try_get("reserve_journal_id")?;
            let action = if target == "released" {
                Action::Release {
                    reservation_journal_id: original,
                }
            } else {
                Action::CommerceCapture {
                    reservation_journal_id: original,
                    destination_account_id: row.try_get("seller_account_id")?,
                    amount_units: Credits::try_from(r.credits).map_err(|_| Error::Invalid)?,
                }
            };
            r.journal_id = post(&mut tx, id, target, None, action).await?;
            if target == "released" {
                sqlx::query("UPDATE core_charging.external_consumption SET refunded=amount_credits WHERE id=$1 AND refunded=0").bind(row.try_get::<String,_>("consumption_id")?).execute(&mut *tx).await?;
            }
        }
        r.state = target.into();
        sqlx::query("UPDATE core_tool_market.calls SET receipt=$2,result_hash=CASE WHEN $3='committed' THEN $4 ELSE result_hash END WHERE id=$1").bind(id).bind(Json(&r)).bind(target).bind(result_hash).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(r)
    }
    pub async fn query(&self, secret: &str, id: &str) -> Result<Option<Receipt>> {
        let mut tx = self.begin().await?;
        let actor = Self::actor(&mut tx, secret).await?;
        let value: Option<Json<Receipt>> = sqlx::query_scalar(
            "SELECT receipt FROM core_tool_market.calls WHERE id=$1 AND actor_user_id=$2",
        )
        .bind(id)
        .bind(actor.user_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(value.map(|Json(r)| r))
    }
    /// No user credential is retained for recovery. This trusted port can only
    /// read a previously frozen exact call and cannot reserve new money.
    pub async fn reconcile(&self, call: &Call) -> Result<Option<Receipt>> {
        let row = sqlx::query("SELECT call,receipt FROM core_tool_market.calls WHERE id=$1")
            .bind(&call.id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let Json(saved): Json<Call> = row.try_get("call")?;
        if saved != *call {
            return Err(Error::Conflict);
        }
        let Json(r) = row.try_get("receipt")?;
        Ok(Some(r))
    }
    pub async fn authorize(&self, secret: &str, account: &str, action: &str) -> Result<Owner> {
        if !matches!(
            action,
            "toolmarket.read"
                | "toolmarket.manage"
                | "toolmarket.authorize"
                | "toolmarket.execute"
                | "toolmarket.review"
                | "toolmarket.config"
        ) {
            return Err(Error::Forbidden);
        }
        if matches!(action, "toolmarket.review" | "toolmarket.config") {
            let mut tx = self.pool.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await?;
            let actor = crate::identity::authorize_admin_read_snapshot(&mut tx, secret)
                .await
                .map_err(|e| match e {
                    crate::identity::admin_users::AdminUsersError::Unauthorized => {
                        Error::Unauthorized
                    }
                    crate::identity::admin_users::AdminUsersError::Forbidden => Error::Forbidden,
                    _ => Error::Storage,
                })?;
            let role: String =
                sqlx::query_scalar("SELECT platform_role FROM core_identity.users WHERE id=$1")
                    .bind(actor.user_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if account != actor.user_id.to_string()
                || (action == "toolmarket.config" && role != "superadmin")
            {
                return Err(Error::Forbidden);
            }
            tx.commit().await?;
            return Ok(Owner {
                user: actor.user_id.to_string(),
                account: actor.user_id.to_string(),
            });
        }
        let mut tx = self.begin().await?;
        let actor = Self::actor(&mut tx, secret).await?;
        if account != actor.user_id.to_string() {
            return Err(Error::Forbidden);
        }
        let owner = Owner {
            user: actor.user_id.to_string(),
            account: actor.user_id.to_string(),
        };
        tx.commit().await?;
        Ok(owner)
    }
    pub async fn revoke(&self, secret: &str, approval: &str) -> Result<()> {
        let mut tx = self.begin().await?;
        let actor = Self::actor(&mut tx, secret).await?;
        if sqlx::query(
            "UPDATE core_tool_market.approvals SET revoked=TRUE WHERE id=$1 AND actor_user_id=$2",
        )
        .bind(approval)
        .bind(actor.user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            != 1
        {
            return Err(Error::Missing);
        }
        tx.commit().await?;
        Ok(())
    }
}
async fn post(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    phase: &str,
    actor: Option<i64>,
    action: Action,
) -> Result<i64> {
    let req = Request {
        scope: "core-tool-market".into(),
        operation_key: hash(&["tool-market-v1", id, phase])?,
        actor_user_id: actor,
        reason: format!("Tool market {phase} {id}"),
        action,
    };
    let Json(result): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(req))
        .fetch_one(&mut **tx)
        .await?;
    match result {
        Outcome::Posted { journal_id, .. } => Ok(journal_id),
        Outcome::Rejected { .. } => Err(Error::Conflict),
    }
}
