//! Receipt-backed dashboard reads. No ledger commands are submitted here.
mod receipts;
#[cfg(test)]
mod tests;

use super::{HistoryError, HistoryStore, JS_SAFE, error_response, extract_bearer};
use crate::identity::admin_users::AdminUsersError;
use crate::identity::{
    CredentialKind, authenticate_readonly_in_transaction, authorize_admin_read_snapshot,
};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use receipts::{Amounts, Receipt, add, safe};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, HistoryError>;
const MAX_REQUESTS: usize = 100_000;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Params {
    start_timestamp: Option<i64>,
    end_timestamp: Option<i64>,
    default_time: Option<String>,
    username: Option<String>,
}

#[derive(Clone, Copy)]
enum View {
    Flow,
    SelfFlow,
    Users,
}

pub(super) async fn flow(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: std::result::Result<Query<Params>, QueryRejection>,
) -> Response {
    respond(store, headers, query, View::Flow).await
}

pub(super) async fn self_flow(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: std::result::Result<Query<Params>, QueryRejection>,
) -> Response {
    respond(store, headers, query, View::SelfFlow).await
}

pub(super) async fn users(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: std::result::Result<Query<Params>, QueryRejection>,
) -> Response {
    respond(store, headers, query, View::Users).await
}

async fn respond(
    store: HistoryStore,
    headers: HeaderMap,
    query: std::result::Result<Query<Params>, QueryRejection>,
    view: View,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(secret) => secret,
        Err(error) => return error_response(error),
    };
    let Query(params) = match query {
        Ok(params) => params,
        Err(_) => return error_response(HistoryError::Invalid),
    };
    match read(&store, secret, params, view).await {
        Ok(data) => (StatusCode::OK, Json(data)).into_response(),
        Err(error) => error_response(error),
    }
}

// Same immutable inputs and receipt limits as usage.rs; no archive rows or holds.
const SELECT: &str = r#"
 SELECT r.*, a.kind AS payer_kind, p.username,
   coalesce(k.name,r.oauth_source->>'client_id','') AS token_name,
   e.detail AS final_event,
   (SELECT coalesce(jsonb_agg(to_jsonb(c)), '[]'::jsonb) FROM
      (SELECT operation_id,command FROM core_charging.ledger_commands
       WHERE request_id=r.id ORDER BY operation_id LIMIT 2049) c) AS commands,
   (SELECT coalesce(jsonb_agg(to_jsonb(f)), '[]'::jsonb) FROM
      (SELECT refund_id,amount FROM core_charging.refunds
       WHERE request_id=r.id ORDER BY refund_id LIMIT 2049) f) AS refunds
 FROM core_charging.requests r
 JOIN core_identity.accounts a ON a.id=r.payer_account_id
 LEFT JOIN core_identity.user_profiles p ON p.user_id=r.actor_user_id
 LEFT JOIN core_identity.api_key_settings k ON k.credential_id=r.key_id
 LEFT JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='final'
 WHERE r.state='settled' AND r.authorized_at BETWEEN $1 AND $2
   AND ($3::bigint IS NULL OR (r.actor_user_id=$3 AND r.owner_account_id=$4))
   AND ($5='' OR p.username=$5)
 ORDER BY r.authorized_at,r.id LIMIT 100001
"#;

async fn read(store: &HistoryStore, secret: &str, params: Params, view: View) -> Result<Value> {
    // @context-fold begin dashboard-authorized-snapshot
    let mut tx = store.read_snapshot().await?;
    let own = matches!(view, View::SelfFlow);
    let actor = if own {
        let actor = authenticate_readonly_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != crate::accounts::AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(HistoryError::Forbidden);
        }
        Some(actor)
    } else {
        authorize_admin_read_snapshot(&mut tx, secret)
            .await
            .map_err(|error| match error {
                AdminUsersError::Unauthorized => HistoryError::Unauthorized,
                AdminUsersError::Forbidden => HistoryError::Forbidden,
                _ => HistoryError::Unavailable,
            })?;
        None
    };
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM transaction_timestamp())::bigint")
        .fetch_one(&mut *tx)
        .await?;
    let start = params.start_timestamp.unwrap_or(now - 86400);
    let end = params.end_timestamp.unwrap_or(now);
    if start < 0
        || end < start
        || end > JS_SAFE
        || end - start > 366 * 86400
        || !matches!(
            params.default_time.as_deref(),
            None | Some("hour" | "day" | "week" | "month")
        )
        || params.username.as_deref().is_some_and(|name| {
            name.len() > 128 || name.chars().any(char::is_control) || (own && !name.is_empty())
        })
    {
        return Err(HistoryError::Invalid);
    }
    let rows = sqlx::query(SELECT)
        .bind(start)
        .bind(end)
        .bind(actor.as_ref().map(|actor| actor.user_id))
        .bind(actor.as_ref().map(|actor| actor.owner_account_id))
        .bind(params.username.as_deref().unwrap_or(""))
        .fetch_all(&mut *tx)
        .await?;
    if rows.len() > MAX_REQUESTS {
        return Err(HistoryError::TooDense);
    }
    let mut buckets: BTreeMap<BucketKey, Bucket> = BTreeMap::new();
    for row in rows {
        let username: Option<String> = row.try_get("username")?;
        let receipt = Receipt::from_row(row)?;
        let key = BucketKey {
            user: receipt.user,
            hour: if matches!(view, View::Users) {
                receipt.at / 3600 * 3600
            } else {
                0
            },
            model: receipt.price.model.clone(),
            group: (!matches!(view, View::Users)).then(|| receipt.price.group.clone()),
            credential: (!matches!(view, View::Users)).then_some((
                receipt.key,
                receipt
                    .oauth_source
                    .as_ref()
                    .map(|source| source.client_id.clone()),
                receipt.credential_source,
            )),
            payer_kind: receipt.payer_kind.clone(),
            subscription: receipt.subscription(),
            sponsored: receipt.sponsored,
        };
        buckets
            .entry(key)
            .or_default()
            .include(&receipt, username)?;
    }
    let data: Vec<Value> = buckets
        .into_iter()
        .map(|(key, bucket)| bucket.project(key, view))
        .collect();
    tx.commit().await?;
    let scope = if own {
        "actor_personal_key"
    } else {
        "administrator_all_accounts"
    };
    Ok(json!({
        "success": true,
        "data": data,
        "availability": "partial",
        "coverage": {
            "scope": scope,
            "native_requests": true,
            "historical_imports": false,
            "complete": false,
            "missing_sources": ["historical_usage_receipts"],
            "settlement_scope": "settled_requests",
            "excluded_states": ["reserved", "streaming", "reconcile", "released"],
            "time_basis": "authorized_at",
            "start_timestamp": start,
            "end_timestamp": end,
            "bucket_seconds": if matches!(view, View::Users) { Some(3600) } else { None },
            "identity_labels": "current_core_identity_metadata",
            "unavailable_dimensions": if matches!(view, View::Users) { vec![] } else { vec!["node", "channel"] }
        }
    }))
    // @context-fold end dashboard-authorized-snapshot
}

#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct BucketKey {
    user: i64,
    hour: i64,
    model: String,
    group: Option<String>,
    credential: Option<(Option<i64>, Option<String>, &'static str)>,
    payer_kind: String,
    subscription: bool,
    sponsored: bool,
}

#[derive(Default)]
struct Bucket {
    username: Option<String>,
    token_name: Option<String>,
    amounts: Amounts,
    count: i64,
    tokens: i64,
    images: i64,
    has_text: bool,
}

impl Bucket {
    fn include(&mut self, receipt: &Receipt, username: Option<String>) -> Result<()> {
        self.username = username;
        self.token_name = (!receipt.token_name.is_empty()).then(|| receipt.token_name.clone());
        self.amounts.accumulate(receipt.amounts)?;
        add(&mut self.count, 1)?;
        if let Some(image) = receipt.usage.image {
            add(&mut self.images, i64::from(image.output_images))?;
        } else {
            self.has_text = true;
            add(
                &mut self.tokens,
                safe(
                    receipt
                        .usage
                        .input
                        .checked_add(receipt.usage.output)
                        .ok_or(HistoryError::Unavailable)?,
                )?,
            )?;
        }
        Ok(())
    }

    fn project(self, key: BucketKey, view: View) -> Value {
        let mut value = json!({
            "user_id": key.user,
            "model_name": key.model,
            "count": self.count,
            "token_used": if self.has_text { Some(self.tokens) } else { None },
            "output_image_count": self.images,
            "token_count_scope": "settled_text_requests",
            "payer_kind": key.payer_kind,
            "team_funded": key.payer_kind == "team",
            "subscription_funded": key.subscription,
            "platform_sponsored": key.sponsored,
            "usage_scope": if matches!(view, View::SelfFlow) { "actor_personal_key" } else { "administrator_all_accounts" },
            "settlement_scope": "settled_requests"
        });
        if let Some(username) = self.username {
            value["username"] = json!(username);
        }
        if matches!(view, View::Users) {
            value["created_at"] = json!(key.hour);
        } else {
            value["use_group"] = json!(key.group);
            if let Some(id) = key.credential.as_ref().and_then(|source| source.0) {
                value["token_id"] = json!(id);
            }
            if let Some(name) = self.token_name {
                value["token_name"] = json!(name);
            }
            value["credential_source"] = json!(key.credential.as_ref().map(|source| source.2));
        }
        self.amounts.project(&mut value);
        value
    }
}
