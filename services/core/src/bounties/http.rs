//! Mount internal_router only on protected UDS with independent bounty token.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use rand::{RngCore, rngs::OsRng};
use subtle::ConstantTimeEq;
#[derive(Clone)]
struct Internal {
    store: BountyStore,
    token: [u8; 32],
}
#[derive(Clone)]
struct Browser {
    store: BountyStore,
    origin: String,
}
fn single<'a>(h: &'a HeaderMap, k: &str) -> Result<&'a str> {
    let mut v = h.get_all(k).iter();
    let x = v.next().ok_or(Error::Forbidden)?;
    if v.next().is_some() {
        return Err(Error::Forbidden);
    }
    x.to_str().map_err(|_| Error::Forbidden)
}
fn secure(mut r: Response) -> Response {
    for (k, v) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    ] {
        r.headers_mut().insert(k, v.parse().unwrap());
    }
    r
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (s, e) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Missing => (StatusCode::NOT_FOUND, "missing"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::Storage => (StatusCode::SERVICE_UNAVAILABLE, "unknown"),
        };
        secure((s, Json(serde_json::json!({"error":e}))).into_response())
    }
}
pub fn internal_router(store: BountyStore, token: &str) -> Result<Router> {
    if !(32..=256).contains(&token.len()) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/internal/bounties/v1/{action}", post(internal))
        .layer(DefaultBodyLimit::max(24576))
        .with_state(Internal {
            store,
            token: Sha256::digest(token.as_bytes()).into(),
        }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryInput {
    operation_id: String,
    #[serde(default)]
    expected_command: Option<Command>,
}
async fn internal(
    State(s): State<Internal>,
    Path(action): Path<String>,
    h: HeaderMap,
    Json(input): Json<serde_json::Value>,
) -> Result<Response> {
    let bearer = single(&h, "authorization")?
        .strip_prefix("Bearer ")
        .ok_or(Error::Forbidden)?;
    let digest: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
    if !(32..=256).contains(&bearer.len())
        || !bool::from(digest.ct_eq(&s.token))
        || single(&h, "x-lmm-protocol")? != "1"
    {
        return Err(Error::Forbidden);
    }
    if action == "ready" {
        if input != serde_json::json!({}) {
            return Err(Error::Invalid);
        }
        s.store.check_ready().await?;
        let policy = s.store.fee_config().await?;
        if !policy.enabled {
            return Ok(secure((StatusCode::SERVICE_UNAVAILABLE,Json(serde_json::json!({"protocol":1,"service":"bounties","ready":false,"policy_configured":false,"error":"policy_not_configured"}))).into_response()));
        }
        return Ok(secure(Json(serde_json::json!({"protocol":1,"service":"bounties","ready":true,"policy_configured":true})).into_response()));
    }
    if action == "config" {
        if input != serde_json::json!({}) {
            return Err(Error::Invalid);
        }
        return Ok(secure(Json(s.store.fee_config().await?).into_response()));
    }
    let secret = single(&h, "x-lmm-user-credential")?;
    if action == "query" {
        let q: QueryInput = serde_json::from_value(input).map_err(|_| Error::Invalid)?;
        let receipt = match q.expected_command {
            Some(expected) => {
                if expected.operation_id != q.operation_id {
                    return Err(Error::Invalid);
                }
                s.store.query_for_command(secret, &expected).await?
            }
            None => s.store.query(secret, &q.operation_id).await?,
        };
        return Ok(secure(Json(receipt).into_response()));
    }
    let cmd: Command = serde_json::from_value(input).map_err(|_| Error::Invalid)?;

    let out = match action.as_str() {
        "authorize" => serde_json::to_value(s.store.authorize(secret).await?),
        "session-authorize" => serde_json::to_value(s.store.session_authorize(secret).await?),
        "preview" => serde_json::to_value(s.store.preview(secret, &cmd).await?),
        "execute" => serde_json::to_value(s.store.execute(secret, &cmd).await?),
        _ => return Err(Error::Missing),
    }
    .map_err(|_| Error::Storage)?;
    Ok(secure(Json(out).into_response()))
}
pub fn confirm_router(store: BountyStore, origin: String) -> Result<Router> {
    confirm_router_with_local_http(store, origin, false)
}
pub fn confirm_router_with_local_http(
    store: BountyStore,
    origin: String,
    allow_local_http: bool,
) -> Result<Router> {
    let u = reqwest::Url::parse(&origin).map_err(|_| Error::Invalid)?;
    let local = allow_local_http
        && u.scheme() == "http"
        && u.port().is_some()
        && matches!(u.host_str(), Some("127.0.0.1" | "[::1]"));
    if u.origin().ascii_serialization() != origin || !(u.scheme() == "https" || local) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/core/v1/bounties/confirm/{id}", get(page).post(confirm))
        .layer(DefaultBodyLimit::max(1024))
        .with_state(Browser { store, origin }))
}
fn cookie(h: &HeaderMap) -> Result<&str> {
    let mut found = None;
    for v in h.get_all(header::COOKIE) {
        for item in v.to_str().map_err(|_| Error::Forbidden)?.split(';') {
            if let Some((k, v)) = item.trim().split_once('=') {
                if k == "new_api_refresh" {
                    if found.is_some() || v.is_empty() {
                        return Err(Error::Forbidden);
                    }
                    found = Some(v)
                }
            }
        }
    }
    found.ok_or(Error::Forbidden)
}
fn escaped(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
async fn page(State(s): State<Browser>, Path(id): Path<String>, h: HeaderMap) -> Result<Response> {
    let mut tx = s.store.pool.begin().await?;
    let a = identity::BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie(&h)?)
        .await
        .map_err(|_| Error::Forbidden)?;
    let row=sqlx::query("SELECT request,policy FROM core_bounties.confirmations WHERE id=$1 AND actor=$2 AND expires>=extract(epoch FROM clock_timestamp()) FOR UPDATE").bind(&id).bind(a.user).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
    let sqlx::types::Json(cmd): sqlx::types::Json<Command> = row.try_get("request")?;
    let sqlx::types::Json(policy): sqlx::types::Json<Option<(i64, i64)>> = row.try_get("policy")?;
    let details = escaped(&serde_json::to_string_pretty(&cmd).map_err(|_| Error::Storage)?);
    let economics = if let Some((bps, account)) = policy {
        let (total, fee, net) = publication(cmd.reward_credit, cmd.reward_slots, bps)?;
        format!(
            "Total {total} CREDIT; fee {fee} CREDIT to account {account}; reward per slot {net} CREDIT."
        )
    } else {
        format!("Additional transfer: {} CREDIT.", cmd.credit)
    };
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| Error::Storage)?;
    let csrf = format!("{:x}", Sha256::digest(bytes));
    sqlx::query("UPDATE core_bounties.confirmations SET csrf=$2,browser_sid=$3 WHERE id=$1")
        .bind(&id)
        .bind(Sha256::digest(csrf.as_bytes()).to_vec())
        .bind(a.sid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(secure(Html(format!(r#"<!doctype html><meta charset="utf-8"><title>Confirm bounty action</title><h1>Confirm bounty action</h1><p>{economics} 500000 CREDIT = 1 USD.</p><pre>{details}</pre><button id="confirm">Confirm this exact action</button><p id="status"></p><script>document.getElementById('confirm').onclick=async()=>{{const b=document.getElementById('confirm');b.disabled=true;try{{const r=await fetch(location.pathname,{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{csrf:'{csrf}'}})}});document.getElementById('status').textContent=r.ok?'Confirmed. Return to the bounty tool.':'Could not confirm. Reload to review.';}}catch(e){{document.getElementById('status').textContent='Unknown result. Reload this action.';}}}};</script>"#)).into_response()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    csrf: String,
}
async fn confirm(
    State(s): State<Browser>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Confirmation>,
) -> Result<Response> {
    if single(&h, "origin")? != s.origin {
        return Err(Error::Forbidden);
    }
    let mut tx = s.store.pool.begin().await?;
    let a = identity::BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie(&h)?)
        .await
        .map_err(|_| Error::Forbidden)?;
    let r=sqlx::query("SELECT csrf,browser_sid FROM core_bounties.confirmations WHERE id=$1 AND actor=$2 AND expires>=extract(epoch FROM clock_timestamp()) FOR UPDATE").bind(&id).bind(a.user).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
    if r.try_get::<Option<String>, _>("browser_sid")? != Some(a.sid)
        || r.try_get::<Option<Vec<u8>>, _>("csrf")?
            != Some(Sha256::digest(input.csrf.as_bytes()).to_vec())
    {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE core_bounties.confirmations SET approved=TRUE WHERE id=$1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(secure(
        Json(serde_json::json!({"confirmed":true,"confirmation":id})).into_response(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    const TOKEN: &str = "synthetic_bounty_ready_host_token_0123456789";
    async fn closed_router() -> Router {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1/synthetic")
            .unwrap();
        pool.close().await;
        internal_router(BountyStore::from_pool(pool), TOKEN).unwrap()
    }
    fn ready(token: &str, protocol: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/internal/bounties/v1/ready")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("x-lmm-protocol", protocol)
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    #[tokio::test]
    async fn ready_internal_port_requires_exact_purpose_host_and_protocol() {
        let app = closed_router().await;
        for req in [
            ready("synthetic_other_purpose_token_0123456789", "1", "{}"),
            ready(TOKEN, "2", "{}"),
            Request::builder()
                .method("POST")
                .uri("/internal/bounties/v1/ready")
                .header("content-type", "application/json")
                .header("x-lmm-user-credential", "synthetic_user_not_host")
                .body(Body::from("{}"))
                .unwrap(),
        ] {
            assert_eq!(
                app.clone().oneshot(req).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        let mut duplicate = ready(TOKEN, "1", "{}");
        duplicate
            .headers_mut()
            .append("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        assert_eq!(
            app.oneshot(duplicate).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn ready_internal_port_has_closed_body_and_never_requires_user_session() {
        let app = closed_router().await;
        for body in [
            "[]",
            "null",
            r#"{"session_token":"synthetic"}"#,
            r#"{"action":"publish"}"#,
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(ready(TOKEN, "1", body))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        // A closed database yields unavailable, not an absent-user-credential denial.
        // This is router admission evidence, not an actual UDS or PG startup sign-off.
        let response = app.oneshot(ready(TOKEN, "1", "{}")).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"error":"unknown"})
        );
    }
}
