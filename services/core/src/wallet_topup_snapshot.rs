// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
//! Reads an explicitly selected, owner-imported legacy order snapshot.
//! No log-to-order inference, money conversion, payment activation or writes.

use serde::Deserialize;
use serde_json::{Number, Value, json};
use sqlx::{PgPool, Row};
use std::str::FromStr;

const SCOPE: &str = "wallet-topup-history.v1";
const JS_SAFE: i64 = 9_007_199_254_740_991;

#[derive(Clone)]
pub struct WalletTopupHistory {
    pool: PgPool,
    migration_id: String,
}

#[derive(Debug)]
pub enum HistoryReadError {
    Invalid,
    Unavailable,
    ScopeUnavailable,
}
impl From<sqlx::Error> for HistoryReadError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    CreateTime,
    Amount,
    Money,
    Status,
    PaymentMethod,
    UserId,
    TradeNo,
}
impl SortBy {
    fn sql(self) -> &'static str {
        match self {
            Self::CreateTime => "(data->>'create_time')::bigint",
            Self::Amount => "(data->>'amount')::bigint",
            Self::Money => "money_text::numeric",
            Self::Status => "data->>'status' COLLATE \"C\"",
            Self::PaymentMethod => "data->>'payment_method' COLLATE \"C\"",
            Self::UserId => "(data->>'user_id')::bigint",
            Self::TradeNo => "data->>'trade_no' COLLATE \"C\"",
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}
impl SortOrder {
    fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryQuery {
    pub p: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_by: SortBy,
    pub sort_order: SortOrder,
}

const OWN_ROWS: &str = "
WITH receipt AS (
 SELECT manifest FROM core_migration.import_runs WHERE migration_id=$1
), orders AS (
 SELECT x->'record' AS data,x->>'money_text' AS money_text
 FROM receipt CROSS JOIN LATERAL jsonb_array_elements(manifest->'rows') x
 WHERE (x->>'target_user_id')::bigint=$2
 AND (x->>'target_personal_account_id')::bigint=$3
 AND (x->'record'->>'user_id')::bigint=$2
)";

impl WalletTopupHistory {
    pub fn new(pool: PgPool, migration_id: String) -> Self {
        Self { pool, migration_id }
    }

    pub async fn read(
        &self,
        user: i64,
        personal_account: i64,
        query: &HistoryQuery,
    ) -> Result<Value, HistoryReadError> {
        // @context-fold begin wallet-history-receipt-and-owner-scope
        let page = query.p.unwrap_or(1);
        let size = query.page_size.unwrap_or(20);
        let keyword = query.keyword.as_deref().unwrap_or("");
        if page == 0 || page > 100_000 || size == 0 || size > 100 || keyword.len() > 256 {
            return Err(HistoryReadError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let receipt = sqlx::query(
            "SELECT manifest-'rows'-'identity_map' AS metadata,\
             jsonb_array_length(manifest->'rows')::bigint AS actual_rows \
             FROM core_migration.import_runs WHERE migration_id=$1 AND manifest->>'scope'=$2",
        )
        .bind(&self.migration_id)
        .bind(SCOPE)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(HistoryReadError::Unavailable)?;
        let metadata: Value = receipt.try_get("metadata")?;
        let actual: i64 = receipt.try_get("actual_rows")?;
        if metadata["complete"] != true
            || metadata["source_row_count"].as_i64() != Some(actual)
            || metadata["source_namespace"].as_str().is_none()
            || metadata["observed_at"].as_i64().is_none()
        {
            return Err(HistoryReadError::Unavailable);
        }
        let source_users: Vec<i64> = sqlx::query_scalar(
            "SELECT x.source_user_id FROM core_migration.import_runs r \
             CROSS JOIN LATERAL jsonb_to_recordset(r.manifest->'identity_map') \
             AS x(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint) \
             WHERE r.migration_id=$1 AND x.target_user_id=$2 AND x.target_personal_account_id=$3 \
             AND EXISTS(SELECT 1 FROM core_identity.users u \
              WHERE u.id=$2 AND u.personal_account_id=$3)",
        )
        .bind(&self.migration_id)
        .bind(user)
        .bind(personal_account)
        .fetch_all(&mut *tx)
        .await?;
        let [source_user] = source_users.as_slice() else {
            return Err(HistoryReadError::ScopeUnavailable);
        };
        let expected = metadata["per_source_user_counts"][source_user.to_string()]
            .as_i64()
            .ok_or(HistoryReadError::Unavailable)?;
        let own_count: i64 = sqlx::query_scalar(&format!("{OWN_ROWS} SELECT count(*) FROM orders"))
            .bind(&self.migration_id)
            .bind(user)
            .bind(personal_account)
            .fetch_one(&mut *tx)
            .await?;
        if own_count != expected {
            return Err(HistoryReadError::Unavailable);
        }
        // @context-fold end wallet-history-receipt-and-owner-scope
        // @context-fold begin wallet-history-exact-decimal-page
        let total: i64 = sqlx::query_scalar(&format!(
            "{OWN_ROWS} SELECT count(*) FROM orders WHERE strpos(data->>'trade_no',$4)>0"
        ))
        .bind(&self.migration_id)
        .bind(user)
        .bind(personal_account)
        .bind(keyword)
        .fetch_one(&mut *tx)
        .await?;
        // Only enum-derived SQL is interpolated. Keywords remain literal parameters.
        let rows = sqlx::query(&format!(
            "{OWN_ROWS} SELECT data,money_text FROM orders \
             WHERE strpos(data->>'trade_no',$4)>0 \
             ORDER BY {} {},(data->>'id')::bigint {} LIMIT $5 OFFSET $6",
            query.sort_by.sql(),
            query.sort_order.sql(),
            query.sort_order.sql(),
        ))
        .bind(&self.migration_id)
        .bind(user)
        .bind(personal_account)
        .bind(keyword)
        .bind(i64::from(size))
        .bind(i64::from(page - 1) * i64::from(size))
        .fetch_all(&mut *tx)
        .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let mut data: Value = row.try_get("data")?;
            // Preserve decimal text as an exact JSON number, never via f64.
            let money: String = row.try_get("money_text")?;
            let money = Number::from_str(&money).map_err(|_| HistoryReadError::Unavailable)?;
            for field in [
                "id",
                "user_id",
                "amount",
                "create_time",
                "complete_time",
                "credited_quota",
                "platform_amount_micros",
            ] {
                if !data[field]
                    .as_i64()
                    .is_some_and(|value| (-JS_SAFE..=JS_SAFE).contains(&value))
                {
                    return Err(HistoryReadError::Unavailable);
                }
            }
            data["money"] = Value::Number(money);
            items.push(data);
        }
        tx.commit().await?;
        Ok(json!({
            "items":items, "total":total,
            "coverage":{
                "scope":"legacy_top_ups_snapshot",
                "source_namespace":metadata["source_namespace"],
                "observed_at":metadata["observed_at"],
                "source_items":expected,"imported_items":own_count,"missing_items":0,
                "native_payment_orders_included":false
            }
        }))
        // @context-fold end wallet-history-exact-decimal-page
    }
}
