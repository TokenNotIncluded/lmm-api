// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use subtle::ConstantTimeEq;
#[derive(Clone)]
struct Internal {
    store: Store,
    token: Vec<u8>,
}
#[derive(Clone)]
struct Browser {
    store: Store,
    origin: String,
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (code, name) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unknown"),
        };
        secure((code, Json(json!({"error":name}))).into_response())
    }
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
/// Mount only on a separately permissioned internal UDS. Never merge publicly.
pub fn internal_router(store: Store, token: Vec<u8>) -> Result<Router, Error> {
    if token.len() < 32 {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/internal/wallet/v1/ready", post(ready))
        .route("/internal/wallet/v1/{action}", post(internal))
        .layer(DefaultBodyLimit::max(4096))
        .with_state(Internal { store, token }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadyInput {}
// @mark wallet.http_identity_boundary
fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}
async fn ready(
    State(s): State<Internal>,
    h: HeaderMap,
    Json(_): Json<ReadyInput>,
) -> Result<Response, Error> {
    let host = single_header(&h, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(Error::Unauthorized)?;
    if host.len() != s.token.len() || !bool::from(host.as_bytes().ct_eq(&s.token)) {
        return Err(Error::Unauthorized);
    }
    if single_header(&h, "x-lmm-protocol") != Some("1") {
        return Err(Error::Invalid);
    }
    if h.contains_key("x-lmm-user-credential") {
        return Err(Error::Forbidden);
    }
    s.store.check_ready().await?;
    Ok(secure(
        Json(json!({"protocol":1,"service":"wallet","ready":true})).into_response(),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    #[serde(default)]
    amount_credits: i64,
    #[serde(default)]
    request_key: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    id: i64,
    #[serde(default)]
    before_id: i64,
}
async fn internal(
    State(s): State<Internal>,
    Path(action): Path<String>,
    h: HeaderMap,
    Json(i): Json<Input>,
) -> Result<Response, Error> {
    let host = single_header(&h, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(Error::Unauthorized)?;
    if host.len() != s.token.len() || !bool::from(host.as_bytes().ct_eq(&s.token)) {
        return Err(Error::Unauthorized);
    }
    let session = single_header(&h, "x-lmm-user-credential").ok_or(Error::Unauthorized)?;
    let out = match action.as_str() {
        "balance" => serde_json::to_value(s.store.balance(session).await?),
        "prepare" => serde_json::to_value(
            s.store
                .prepare(session, i.amount_credits, &i.request_key, &i.token)
                .await?,
        ),
        "query" => serde_json::to_value(
            s.store
                .query_by_request_key(session, &i.request_key)
                .await?,
        ),
        "list" => serde_json::to_value(s.store.list(session, i.before_id).await?),
        "cancel" => serde_json::to_value(s.store.request_cancel(session, i.id).await?),
        "inspect" => serde_json::to_value(s.store.inspect(session, &i.token).await?),
        "claim" => serde_json::to_value(s.store.claim(session, &i.token).await?),
        _ => return Err(Error::NotFound),
    }
    .map_err(|_| Error::Unavailable)?;
    Ok(secure(Json(out).into_response()))
}
/// Browser routes require HttpOnly refresh-cookie proof, independent of Go's bearer Session.
pub fn browser_router(store: Store, origin: String) -> Result<Router, Error> {
    browser_router_with_local_http(store, origin, false)
}
/// Local HTTP is opt-in and restricted to numeric loopback; production remains HTTPS.
pub fn browser_router_with_local_http(
    store: Store,
    origin: String,
    local_http: bool,
) -> Result<Router, Error> {
    let u = reqwest::Url::parse(&origin).map_err(|_| Error::Invalid)?;
    let local = local_http
        && u.scheme() == "http"
        && u.host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    if (u.scheme() != "https" && !local)
        || u.origin().ascii_serialization() != origin
        || u.username() != ""
        || u.password().is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route(
            "/core/v1/wallet/confirm/{kind}/{id}",
            get(page).post(confirm),
        )
        .layer(DefaultBodyLimit::max(2048))
        .with_state(Browser { store, origin }))
}
fn cookie(h: &HeaderMap) -> Result<&str, Error> {
    let mut out = None;
    for h in h.get_all(header::COOKIE) {
        for item in h.to_str().map_err(|_| Error::Unauthorized)?.split(';') {
            if let Some((name, value)) = item.trim().split_once('=')
                && name == "new_api_refresh"
            {
                if out.is_some() || value.is_empty() {
                    return Err(Error::Unauthorized);
                }
                out = Some(value)
            }
        }
    }
    out.ok_or(Error::Unauthorized)
}
async fn page(
    State(s): State<Browser>,
    Path((kind, id)): Path<(String, i64)>,
    h: HeaderMap,
) -> Result<Response, Error> {
    let (t, csrf) = s.store.confirmation_page(cookie(&h)?, id, &kind).await?;
    // All interpolated fields are integers or cryptographically generated hex.
    let verb = if kind == "create" {
        "Hold funds for transfer"
    } else {
        "Cancel transfer and return funds"
    };
    let html = format!(
        r#"<!doctype html><meta charset="utf-8"><title>Confirm wallet transfer</title><h1>{verb}</h1><p>Transfer #{id}: {} CREDIT (500000 CREDIT = 1 USD).</p><p>This action changes your wallet funds.</p><button id="confirm">Confirm {verb}</button><p id="status"></p><script>document.getElementById('confirm').onclick=async()=>{{const b=document.getElementById('confirm');b.disabled=true;try{{const r=await fetch(location.pathname,{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{csrf:'{csrf}'}})}});document.getElementById('status').textContent=r.ok?'Confirmed. Return to your wallet tool.':'Could not confirm. Reload to check the current state.';}}catch(e){{document.getElementById('status').textContent='Result unknown. Reload and retry the same action.';}}}};</script>"#,
        t.amount_credits
    );
    Ok(secure(Html(html).into_response()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    csrf: String,
}
async fn confirm(
    State(s): State<Browser>,
    Path((kind, id)): Path<(String, i64)>,
    h: HeaderMap,
    Json(i): Json<Confirmation>,
) -> Result<Response, Error> {
    if single_header(&h, "origin") != Some(s.origin.as_str()) {
        return Err(Error::Forbidden);
    }
    let t = s.store.confirm(cookie(&h)?, id, &i.csrf, &kind).await?;
    Ok(secure(Json(t).into_response()))
}
