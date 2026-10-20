//! Immutable historical logs. Native receipt-backed usage remains in `usage`.
//! Nothing here creates wallets, imports facts, replays requests or values raw
//! opening counters. Source installation and HTTP mounting are explicit gates.
pub mod projection;
use crate::identity::{self, CredentialKind, IdentityError};
use projection::{MAX_BASES, MAX_BASIS, MAX_RECORDS, MoneyProjection};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, Postgres, Row, postgres::PgArguments, query::Query};
use std::collections::BTreeMap;

pub const CONTRACT: &str = "core.history-read.v1";
const JS_SAFE: i64 = 9_007_199_254_740_991;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryError {
    Unauthorized,
    Forbidden,
    Invalid,
    Unavailable,
    TooDense,
}
impl From<sqlx::Error> for HistoryError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<IdentityError> for HistoryError {
    fn from(value: IdentityError) -> Self {
        match value {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            IdentityError::Invalid => Self::Invalid,
            _ => Self::Unavailable,
        }
    }
}
impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HistoryError {}
type Result<T> = std::result::Result<T, HistoryError>;

/// Chosen by the server's route registration, never deserialized from a client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadRoute {
    SelfSession,
    ReadOnlyKey,
    Administrator,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryQuery {
    pub p: Option<u32>,
    pub page_size: Option<u32>,
    pub r#type: Option<i16>,
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub username: Option<String>,
    pub token_name: Option<String>,
    pub model_name: Option<String>,
    pub channel: Option<i64>,
    pub group: Option<String>,
    pub request_id: Option<String>,
    pub upstream_request_id: Option<String>,
}
impl HistoryQuery {
    fn validate(&self, route: ReadRoute) -> Result<(u32, u32, i64)> {
        let page = self.p.unwrap_or(1);
        let size = self.page_size.unwrap_or(20);
        let start = self.start_timestamp.unwrap_or(0);
        let end = self.end_timestamp.unwrap_or(253402300799);
        if page == 0
            || size == 0
            || size > 100
            || page > 100_000
            || start < 0
            || end < start
            || end > 253402300799
            || self.r#type.is_some_and(|v| !(0..=7).contains(&v))
            || self.channel.is_some_and(|v| v < 0)
            || [
                &self.username,
                &self.token_name,
                &self.model_name,
                &self.group,
                &self.request_id,
                &self.upstream_request_id,
            ]
            .iter()
            .any(|v| {
                v.as_ref()
                    .is_some_and(|s| s.len() > 256 || s.chars().any(char::is_control))
            })
        {
            return Err(HistoryError::Invalid);
        }
        // Match the fixed main's explicit log LIKE rules for these two fields.
        for value in [&self.username, &self.model_name].into_iter().flatten() {
            let pattern = text_pattern(Some(value));
            let wildcards = pattern.bytes().filter(|b| *b == b'%').count();
            if pattern.contains("%%")
                || wildcards > 2
                || (wildcards > 0 && pattern.replace('%', "").len() < 2)
            {
                return Err(HistoryError::Invalid);
            }
        }
        if route != ReadRoute::Administrator
            && (self.username.as_ref().is_some_and(|s| !s.is_empty())
                || self.channel.is_some_and(|v| v != 0)
                || self
                    .upstream_request_id
                    .as_ref()
                    .is_some_and(|s| !s.is_empty()))
        {
            return Err(HistoryError::Forbidden);
        }
        Ok((page, size, i64::from(page - 1) * i64::from(size)))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryRow {
    pub source_log_id: i64,
    pub user_id: i64,
    pub token_id: Option<i64>,
    pub created_at: i64,
    pub r#type: i16,
    pub username: String,
    pub token_name: String,
    pub model_name: String,
    pub quota: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub group: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_request_id: Option<String>,
    #[serde(flatten)]
    pub money: MoneyProjection,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistorySummary {
    /// Exact raw quota sum, including NULL/negative validity separately below.
    /// String avoids browser rounding and is never a normalized usage counter.
    pub raw_consumption_quota: String,
    pub invalid_quota_records: i64,
    pub unpriced_quota_records: i64,
    pub quota_projection_truncated: bool,
    #[serde(flatten)]
    pub money: MoneyProjection,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistorySnapshot {
    pub contract: &'static str,
    pub snapshot_at: i64,
    pub items: Vec<HistoryRow>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
    pub summary: HistorySummary,
    /// This repository covers historical logs only. Do not add its raw sum to
    /// normalized usage or native settled credits without A03's lineage proof.
    pub source: &'static str,
}

struct Selection<'a> {
    user: i64,
    owner: i64,
    key: i64,
    q: &'a HistoryQuery,
}
fn text_pattern(value: Option<&str>) -> String {
    let value = value.unwrap_or("");
    if value.contains('%') {
        value.replace('!', "!!").replace('_', "!_")
    } else {
        value.to_owned()
    }
}
impl Selection<'_> {
    fn bind<'a>(&'a self, sql: &'a str) -> Query<'a, Postgres, PgArguments> {
        sqlx::query(sql)
            .bind(self.user)
            .bind(self.owner)
            .bind(self.key)
            .bind(self.q.r#type.unwrap_or(0))
            .bind(self.q.start_timestamp.unwrap_or(0))
            .bind(self.q.end_timestamp.unwrap_or(253402300799))
            .bind(text_pattern(self.q.username.as_deref()))
            .bind(self.q.token_name.as_deref().unwrap_or(""))
            .bind(text_pattern(self.q.model_name.as_deref()))
            .bind(self.q.channel.unwrap_or(0))
            .bind(self.q.group.as_deref().unwrap_or(""))
            .bind(self.q.request_id.as_deref().unwrap_or(""))
            .bind(self.q.upstream_request_id.as_deref().unwrap_or(""))
    }
}
// Every query derives from this exact scope/filter expression. The source FK is
// a reviewed immutable import receipt. Missing/incomplete installation is an
// error in check_source, not an empty successful history.
const FILTER: &str = r#"
 FROM core_history.log_records l JOIN core_history.sources s ON s.source_id=l.source_id
 WHERE s.complete AND s.contract_version='core.history-source.v1'
 AND ($1::bigint=0 OR l.user_id=$1) AND ($2::bigint=0 OR l.owner_account_id=$2)
 AND ($3::bigint=0 OR l.token_id=$3) AND ($4::smallint=0 OR l.log_type=$4)
 AND l.created_at BETWEEN $5 AND $6
 AND ($7='' OR CASE WHEN strpos($7,'%')>0 THEN l.username LIKE $7 ESCAPE '!' ELSE l.username=$7 END)
 AND ($8='' OR l.token_name=$8)
 AND ($9='' OR CASE WHEN strpos($9,'%')>0 THEN l.model_name LIKE $9 ESCAPE '!' ELSE l.model_name=$9 END)
 AND ($10::bigint=0 OR l.channel_id=$10) AND ($11='' OR l.group_name=$11)
 AND ($12='' OR l.request_id=$12) AND ($13='' OR l.upstream_request_id=$13)
"#;

#[derive(Clone)]
pub struct HistoryStore {
    pool: PgPool,
}
impl HistoryStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Admit an unavailable category without reading unrelated historical logs.
    /// This establishes read identity only; it grants no data or write authority.
    pub(crate) async fn authenticate_category(&self, secret: &str, route: ReadRoute) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        match route {
            ReadRoute::Administrator => {
                authorize(&mut tx, secret, route).await?;
            }
            ReadRoute::SelfSession => {
                let actor = identity::authenticate_readonly_in_transaction(&mut tx, secret).await?;
                if actor.credential_kind != CredentialKind::Session
                    || actor.owner.kind != crate::accounts::AccountKind::Personal
                    || actor.owner.id != actor.user_id
                {
                    return Err(HistoryError::Forbidden);
                }
                let current_owner: i64 = sqlx::query_scalar(
                    "SELECT personal_account_id FROM core_identity.users WHERE id=$1",
                )
                .bind(actor.user_id)
                .fetch_one(&mut *tx)
                .await?;
                if actor.owner_account_id != current_owner {
                    return Err(HistoryError::Forbidden);
                }
            }
            ReadRoute::ReadOnlyKey => return Err(HistoryError::Forbidden),
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn read(
        &self,
        secret: &str,
        route: ReadRoute,
        query: &HistoryQuery,
    ) -> Result<HistorySnapshot> {
        self.read_bounded(secret, route, query, false).await
    }
    pub(crate) async fn export(
        &self,
        secret: &str,
        query: &HistoryQuery,
    ) -> Result<HistorySnapshot> {
        self.read_bounded(secret, ReadRoute::Administrator, query, true)
            .await
    }
    async fn read_bounded(
        &self,
        secret: &str,
        route: ReadRoute,
        q: &HistoryQuery,
        export: bool,
    ) -> Result<HistorySnapshot> {
        let (page, size, offset) = q.validate(route)?;
        if export && (page != 1 || q.page_size.is_some()) {
            return Err(HistoryError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        let (user, owner, key) = authorize(&mut tx, secret, route).await?;
        let snapshot_at: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        // Complete coverage must be explicitly installed even for empty data.
        let complete: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_history.sources) AND NOT EXISTS(SELECT 1 FROM core_history.sources WHERE NOT complete OR contract_version<>'core.history-source.v1')")
            .fetch_one(&mut *tx).await?;
        if !complete {
            return Err(HistoryError::Unavailable);
        }
        if route != ReadRoute::Administrator {
            // An archive-wide receipt proves neither this identity mapping nor
            // that an unmapped credential has an empty history. Bind coverage
            // to every source in this same read-only repeatable-read snapshot.
            let covered: bool = sqlx::query_scalar(
                r#"SELECT NOT EXISTS (
                    SELECT 1 FROM core_history.sources s
                    WHERE NOT EXISTS (
                        SELECT 1 FROM core_history.scope_receipts p
                        WHERE p.source_id=s.source_id AND p.user_id=$1
                          AND p.owner_account_id=$2 AND p.token_id IS NULL
                          AND p.complete AND p.source_archive_sha256=s.source_archive_sha256
                          AND p.mapping_sha256=s.mapping_sha256
                    ) OR ($3::bigint<>0 AND NOT EXISTS (
                        SELECT 1 FROM core_history.scope_receipts p
                        WHERE p.source_id=s.source_id AND p.user_id=$1
                          AND p.owner_account_id=$2 AND p.token_id=$3
                          AND p.complete AND p.source_archive_sha256=s.source_archive_sha256
                          AND p.mapping_sha256=s.mapping_sha256
                    ))
                )"#,
            )
            .bind(user)
            .bind(owner)
            .bind(key)
            .fetch_one(&mut *tx)
            .await?;
            if !covered {
                return Err(HistoryError::Unavailable);
            }
        }
        let selection = Selection {
            user,
            owner,
            key,
            q,
        };
        let counts = selection.bind(&format!("SELECT count(*)::bigint AS total, COALESCE(sum(l.quota::numeric) FILTER(WHERE l.log_type=2),0)::text AS raw, count(*) FILTER(WHERE l.log_type=2 AND l.quota IS NULL)::bigint AS invalid {FILTER}"))
            .fetch_one(&mut *tx).await?;
        let total: i64 = counts.try_get("total")?;
        if total > JS_SAFE {
            return Err(HistoryError::Unavailable);
        }
        if export && total as usize > MAX_RECORDS {
            return Err(HistoryError::TooDense);
        }
        let mut summary = project_summary(&mut tx, &selection).await?;
        summary.raw_consumption_quota = counts.try_get("raw")?;
        summary.invalid_quota_records = counts.try_get("invalid")?;
        let limit = if export {
            MAX_RECORDS as i64
        } else {
            i64::from(size)
        };
        let offset = if export { 0 } else { offset };
        let sql = format!(
            "SELECT l.* {FILTER} ORDER BY l.created_at DESC,l.source_id DESC,l.source_log_id DESC LIMIT $14 OFFSET $15"
        );
        let rows = selection
            .bind(&sql)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for r in rows {
            let quota: Option<i64> = r.try_get("quota")?;
            let input: Option<i64> = r.try_get("prompt_tokens")?;
            let output: Option<i64> = r.try_get("completion_tokens")?;
            let user_id: i64 = r.try_get("user_id")?;
            let token_id: Option<i64> = r.try_get("token_id")?;
            let channel: i64 = r.try_get("channel_id")?;
            // Refuse lossy legacy JSON numbers instead of rewriting raw source.
            if [quota, input, output]
                .into_iter()
                .flatten()
                .any(|v| !(-JS_SAFE..=JS_SAFE).contains(&v))
                || [Some(user_id), token_id, Some(channel)]
                    .into_iter()
                    .flatten()
                    .any(|v| !(0..=JS_SAFE).contains(&v))
            {
                return Err(HistoryError::Unavailable);
            }
            let log_type = r.try_get("log_type")?;
            items.push(HistoryRow {
                source_log_id: r.try_get("source_log_id")?,
                user_id,
                token_id,
                created_at: r.try_get("created_at")?,
                r#type: log_type,
                username: r.try_get("username")?,
                token_name: r.try_get("token_name")?,
                model_name: r.try_get("model_name")?,
                quota,
                prompt_tokens: input,
                completion_tokens: output,
                group: r.try_get("group_name")?,
                request_id: r.try_get("request_id")?,
                channel: (route == ReadRoute::Administrator).then_some(channel),
                upstream_request_id: (route == ReadRoute::Administrator)
                    .then(|| r.try_get("upstream_request_id"))
                    .transpose()?,
                money: projection::log_money(
                    quota,
                    log_type,
                    r.try_get("pricing_credits_per_usd")?,
                ),
            });
        }
        tx.commit().await?;
        Ok(HistorySnapshot {
            contract: CONTRACT,
            snapshot_at,
            items,
            total,
            page: if export { 1 } else { page },
            page_size: if export { MAX_RECORDS as u32 } else { size },
            summary,
            source: "legacy_log_only",
        })
    }
}

async fn authorize(
    c: &mut PgConnection,
    secret: &str,
    route: ReadRoute,
) -> Result<(i64, i64, i64)> {
    if route == ReadRoute::Administrator {
        identity::authorize_admin_read_snapshot(c, secret)
            .await
            .map_err(|e| match e {
                identity::admin_users::AdminUsersError::Unauthorized => HistoryError::Unauthorized,
                identity::admin_users::AdminUsersError::Forbidden => HistoryError::Forbidden,
                _ => HistoryError::Unavailable,
            })?;
        return Ok((0, 0, 0));
    }
    let actor = identity::authenticate_readonly_in_transaction(c, secret).await?;
    match route {
        ReadRoute::SelfSession if actor.credential_kind == CredentialKind::Session => {
            let personal: i64 = sqlx::query_scalar(
                "SELECT personal_account_id FROM core_identity.users WHERE id=$1",
            )
            .bind(actor.user_id)
            .fetch_one(&mut *c)
            .await?;
            Ok((actor.user_id, personal, 0))
        }
        ReadRoute::ReadOnlyKey if actor.credential_kind == CredentialKind::ApiKey => {
            Ok((actor.user_id, actor.owner_account_id, actor.credential_id))
        }
        _ => Err(HistoryError::Forbidden),
    }
}

async fn project_summary(
    c: &mut PgConnection,
    selection: &Selection<'_>,
) -> Result<HistorySummary> {
    let sql = format!(
        "SELECT l.quota,l.pricing_credits_per_usd {FILTER} AND l.log_type=2 ORDER BY l.source_id,l.source_log_id LIMIT 10001"
    );
    let rows = selection.bind(&sql).fetch_all(&mut *c).await?;
    let truncated = rows.len() > MAX_RECORDS;
    let mut unpriced = 0;
    let mut bases: BTreeMap<i64, i128> = BTreeMap::new();
    for row in rows.iter().take(MAX_RECORDS) {
        let quota: Option<i64> = row.try_get("quota")?;
        let basis: Option<i64> = row.try_get("pricing_credits_per_usd")?;
        match (quota, basis) {
            (Some(q), Some(k)) if q >= 0 && (1..=MAX_BASIS).contains(&k) => {
                *bases.entry(k).or_default() += i128::from(q);
            }
            _ => unpriced += 1,
        }
    }
    let truncated = truncated || bases.len() > MAX_BASES;
    let money = if truncated {
        MoneyProjection::unknown("projection_limit")
    } else if unpriced != 0 {
        MoneyProjection::unknown("aggregate_missing_credit_basis")
    } else if bases.is_empty() {
        MoneyProjection::scaled("0", "empty_result").unwrap()
    } else {
        // PostgreSQL numeric multiplication/addition are exact. Maintain a
        // rational numerator/denominator (<=64 JS-safe bases), then div/mod
        // round once at 18 decimals; ordinary numeric division is not used.
        let values: Vec<_> = bases
            .into_iter()
            .map(|(k, q)| serde_json::json!({"k":k.to_string(),"q":q.to_string()}))
            .collect();
        let rounded: String = sqlx::query_scalar(r#"
WITH RECURSIVE bases AS (
 SELECT row_number() OVER(ORDER BY (v->>'k')::numeric) AS n,(v->>'k')::numeric AS k,(v->>'q')::numeric AS q
 FROM jsonb_array_elements($1::jsonb) v
), fractions(n,num,den) AS (
 SELECT 0::bigint,0::numeric,1::numeric
 UNION ALL SELECT b.n,f.num*b.k+b.q*f.den,f.den*b.k FROM fractions f JOIN bases b ON b.n=f.n+1
), final AS (SELECT num*1000000000000000000::numeric AS num,den FROM fractions ORDER BY n DESC LIMIT 1)
SELECT (div(num,den)+CASE WHEN mod(num,den)*2>=den THEN 1 ELSE 0 END)::text FROM final
"#).bind(serde_json::Value::Array(values)).fetch_one(&mut *c).await?;
        MoneyProjection::scaled(&rounded, "captured_credits_per_usd")
            .ok_or(HistoryError::Unavailable)?
    };
    Ok(HistorySummary {
        raw_consumption_quota: String::new(),
        invalid_quota_records: 0,
        unpriced_quota_records: unpriced,
        quota_projection_truncated: truncated,
        money,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_like_validation_follows_the_pinned_oracles_escape_order() {
        for value in ["%model%", "%_", "%!", "exact_name"] {
            let query = HistoryQuery {
                model_name: Some(value.to_owned()),
                ..Default::default()
            };
            assert!(query.validate(ReadRoute::SelfSession).is_ok(), "{value}");
        }
        for value in ["%a", "%%model", "%a%b%c", "%"] {
            let query = HistoryQuery {
                model_name: Some(value.to_owned()),
                ..Default::default()
            };
            assert_eq!(
                query.validate(ReadRoute::SelfSession),
                Err(HistoryError::Invalid),
                "{value}"
            );
        }
        assert_eq!(text_pattern(Some("%_!")), "%!_!!");
        assert_eq!(text_pattern(Some("_!")), "_!");
    }
}
