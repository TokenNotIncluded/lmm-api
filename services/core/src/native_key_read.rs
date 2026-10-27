//! Native API-key compatibility reads. No Session/OAuth authority or financial writes.
//! Wire contracts: go-v0.2.118 controller/account_balance.go and
//! controller/quota_query.go (AGPL-3.0-or-later).
use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Reads {
    pool: PgPool,
    queries: Arc<Mutex<BTreeMap<i64, VecDeque<Instant>>>>,
}
/// Mount on Core's existing public server, inside its existing trusted-edge layer.
/// This does not install any grant or initialize a wallet.
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/balance", get(balance))
        .route("/v1/usage", get(usage))
        .with_state(Reads {
            pool,
            queries: Arc::default(),
        })
}

enum Failure {
    Unauthorized,
    Ip,
    Consent,
    Limit,
    Unavailable,
    QuotaUnavailable,
}
impl From<sqlx::Error> for Failure {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "invalid_api_key"),
            Self::Ip => (StatusCode::FORBIDDEN, "ip_not_allowed"),
            Self::Consent => (StatusCode::FORBIDDEN, "account_balance_access_required"),
            Self::Limit => (StatusCode::TOO_MANY_REQUESTS, "quota_query_rate_limited"),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "account_balance_unavailable",
            ),
            Self::QuotaUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "quota_query_unavailable"),
        };
        no_store((status, Json(json!({"valid":false,"error":error}))).into_response())
    }
}
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}
fn bearer(headers: &HeaderMap) -> Result<&str, Failure> {
    if ["x-api-key", "x-goog-api-key", "x-auth-session"]
        .iter()
        .any(|h| headers.contains_key(*h))
    {
        return Err(Failure::Unauthorized);
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(Failure::Unauthorized)?;
    if values.next().is_some() {
        return Err(Failure::Unauthorized);
    }
    let mut parts = value.split_whitespace();
    let scheme = parts.next().ok_or(Failure::Unauthorized)?;
    let secret = parts.next().ok_or(Failure::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || parts.next().is_some()
        || secret.starts_with("lmm_at_")
    {
        return Err(Failure::Unauthorized);
    }
    Ok(secret)
}
impl Reads {
    fn admit(&self, user: i64) -> Result<(), Failure> {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut users = self.queries.lock().map_err(|_| Failure::Unavailable)?;
        users.retain(|_, values| {
            while values
                .front()
                .is_some_and(|t| now.duration_since(*t) >= window)
            {
                values.pop_front();
            }
            !values.is_empty()
        });
        if !users.contains_key(&user) && users.len() >= 4096 {
            return Err(Failure::Limit);
        }
        let values = users.entry(user).or_default();
        if values.len() >= 30 {
            return Err(Failure::Limit);
        }
        values.push_back(now);
        Ok(())
    }
    async fn key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        secret: &str,
        peer: SocketAddr,
    ) -> Result<(crate::identity::Principal, bool), Failure> {
        let actor = crate::identity::authenticate_readonly_in_transaction(&mut **tx, secret)
            .await
            .map_err(|e| match e {
                crate::identity::IdentityError::Unauthorized
                | crate::identity::IdentityError::Invalid
                | crate::identity::IdentityError::Forbidden => Failure::Unauthorized,
                _ => Failure::Unavailable,
            })?;
        if actor.credential_kind != crate::identity::CredentialKind::ApiKey {
            return Err(Failure::Unauthorized);
        }
        let settings = sqlx::query("SELECT account_balance_read,cardinality(allow_ips)=0 OR $2::inet <<= ANY(allow_ips) AS ip_allowed FROM core_identity.api_key_settings WHERE credential_id=$1 AND enabled")
            .bind(actor.credential_id).bind(peer.ip().to_string()).fetch_optional(&mut **tx).await?
            .ok_or(Failure::Unauthorized)?;
        if !settings.try_get::<bool, _>("ip_allowed")? {
            return Err(Failure::Ip);
        }
        self.admit(actor.user_id)?;
        Ok((actor, settings.try_get("account_balance_read")?))
    }
    async fn balance(&self, secret: &str, peer: SocketAddr) -> Result<Value, Failure> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (actor, account_balance_read) = self.key(&mut tx, secret, peer).await?;
        if actor.owner.kind != crate::accounts::AccountKind::Personal {
            return Err(Failure::Unauthorized);
        }
        if !account_balance_read {
            return Err(Failure::Consent);
        }
        // Reuse the existing exact-unit/native-or-opening provenance proof.
        // The serialized proof owns the USD number: no float or new money formula.
        let verified =
            crate::oauth2_balance::read_balance(&mut tx, actor.user_id, actor.owner_account_id)
                .await
                .map_err(|_| Failure::Unavailable)?;
        let verified = serde_json::to_value(verified).map_err(|_| Failure::Unavailable)?;
        let remaining = verified
            .get("balance")
            .filter(|v| v.is_number())
            .ok_or(Failure::Unavailable)?
            .clone();
        let updated_at = verified
            .get("updated_at")
            .and_then(Value::as_u64)
            .ok_or(Failure::Unavailable)?;
        tx.commit().await?;
        Ok(
            json!({"valid":true,"scope":"account","currency":"USD","remaining":remaining,
            "updated_at":updated_at,"consistency":"persisted_snapshot"}),
        )
    }

    async fn usage(&self, secret: &str, peer: SocketAddr) -> Result<Value, Failure> {
        // Read authority, migrated history and current charges in one snapshot.
        // @context-fold begin key-usage-snapshot
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (actor, _) = self
            .key(&mut tx, secret, peer)
            .await
            .map_err(Failure::for_quota)?;
        let (used, remaining, unlimited) =
            crate::billing::charging::key_quota_snapshot(&mut *tx, actor.credential_id)
                .await
                .map_err(|_| Failure::QuotaUnavailable)?;
        let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| Failure::QuotaUnavailable)?;
        let covered: bool = sqlx::query_scalar(
            r#"SELECT ((SELECT extract(epoch FROM created_at)::bigint FROM core_identity.credentials WHERE id=$1)
                     > coalesce((SELECT max(observed_at) FROM core_history.sources),0)
                     AND NOT EXISTS (SELECT 1 FROM core_charging.budget_opening_consumption
                                     WHERE scope='key' AND key_id=$1))
                OR NOT EXISTS (
                    SELECT 1 FROM core_history.sources s
                    WHERE NOT s.complete OR s.contract_version<>'core.history-source.v1'
                       OR NOT EXISTS (SELECT 1 FROM core_history.scope_receipts p
                           WHERE p.source_id=s.source_id AND p.user_id=$2
                             AND p.owner_account_id=$3 AND p.token_id IS NULL
                             AND p.complete AND p.source_archive_sha256=s.source_archive_sha256
                             AND p.mapping_sha256=s.mapping_sha256)
                       OR NOT EXISTS (SELECT 1 FROM core_history.scope_receipts p
                           WHERE p.source_id=s.source_id AND p.user_id=$2
                             AND p.owner_account_id=$3 AND p.token_id=$1
                             AND p.complete AND p.source_archive_sha256=s.source_archive_sha256
                             AND p.mapping_sha256=s.mapping_sha256))"#,
        )
        .bind(actor.credential_id)
        .bind(actor.user_id)
        .bind(actor.owner_account_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Failure::QuotaUnavailable)?;
        if !covered {
            return Err(Failure::QuotaUnavailable);
        }
        let day_start = now - now.rem_euclid(86_400);
        let history = sqlx::query(
            "SELECT count(*) FILTER (WHERE quota IS NULL) AS missing,coalesce(sum(quota),0)::bigint AS credits \
             FROM core_history.log_records WHERE token_id=$1 AND user_id=$2 AND owner_account_id=$3 \
             AND log_type=2 AND created_at BETWEEN $4 AND $5",
        )
        .bind(actor.credential_id)
        .bind(actor.user_id)
        .bind(actor.owner_account_id)
        .bind(day_start)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Failure::QuotaUnavailable)?;
        if history
            .try_get::<i64, _>("missing")
            .map_err(|_| Failure::QuotaUnavailable)?
            != 0
        {
            return Err(Failure::QuotaUnavailable);
        }
        let historical: i64 = history
            .try_get("credits")
            .map_err(|_| Failure::QuotaUnavailable)?;
        let native: i64 = sqlx::query_scalar(
            "SELECT ((SELECT coalesce(sum(core_charging.committed_credits(state,reserved,settled,refunded)),0) \
             FROM core_charging.requests WHERE key_id=$1 AND authorized_at BETWEEN $2 AND $3) \
             + (SELECT coalesce(sum(amount_credits-refunded),0) FROM core_charging.external_consumption \
             WHERE key_id=$1 AND authorized_at BETWEEN $2 AND $3))::bigint",
        )
        .bind(actor.credential_id)
        .bind(day_start)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Failure::QuotaUnavailable)?;
        let used_today = historical
            .checked_add(native)
            .ok_or(Failure::QuotaUnavailable)?;
        let total = if unlimited {
            None
        } else {
            Some(
                remaining
                    .checked_add(used)
                    .ok_or(Failure::QuotaUnavailable)?,
            )
        };
        tx.commit().await.map_err(|_| Failure::QuotaUnavailable)?;
        let usd = |credits: i64| credits as f64 / crate::billing::CREDITS_PER_USD as f64;
        Ok(json!({"valid":true,"scope":"token","currency":"USD",
            "remaining":(!unlimited).then(|| usd(remaining)),
            "used_today":usd(used_today),"used_total":usd(used),
            "used_quota":used,"normalized_used_quota":used,
            "usage_projection_available":true,"total_quota":total.map(usd),
            "unlimited":unlimited,"updated_at":now,"day_timezone":"UTC",
            "used_today_source":"retained_consumption_logs",
            "consistency":"persisted_snapshot"}))
        // @context-fold end key-usage-snapshot
    }
}
impl Failure {
    fn for_quota(self) -> Self {
        match self {
            Self::Unavailable => Self::QuotaUnavailable,
            other => other,
        }
    }
}
async fn balance(
    State(state): State<Reads>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
) -> Result<Response, Failure> {
    let secret = bearer(&headers)?;
    let peer = peer.map_err(|_| Failure::Ip)?.0;
    let result = state.balance(secret, peer).await?;
    Ok(no_store(Json(result).into_response()))
}
async fn usage(
    State(state): State<Reads>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
) -> Result<Response, Failure> {
    let secret = bearer(&headers)?;
    let peer = peer.map_err(|_| Failure::Ip)?.0;
    let result = state
        .usage(secret, peer)
        .await
        .map_err(Failure::for_quota)?;
    Ok(no_store(Json(result).into_response()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_balance_bearer_is_single_opaque_and_never_oauth_or_header_alias() {
        let mut h = HeaderMap::new();
        let key = "sk-opaque_native_balance_read_test000000";
        h.insert(
            header::AUTHORIZATION,
            format!("bEaReR {key}").parse().unwrap(),
        );
        assert_eq!(bearer(&h).ok(), Some(key));
        h.append(
            header::AUTHORIZATION,
            format!("Bearer {key}").parse().unwrap(),
        );
        assert!(bearer(&h).is_err());
        h.remove(header::AUTHORIZATION);
        h.insert(
            header::AUTHORIZATION,
            "Bearer lmm_at_oauth_is_not_a_native_key".parse().unwrap(),
        );
        assert!(bearer(&h).is_err());
        h.insert(
            header::AUTHORIZATION,
            format!("Bearer {key}").parse().unwrap(),
        );
        h.insert("x-api-key", key.parse().unwrap());
        assert!(bearer(&h).is_err());
    }
}
