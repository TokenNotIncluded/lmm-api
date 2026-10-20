use super::*;
use axum::{
    Form, Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};
use subtle::ConstantTimeEq;
pub fn router(issuer: MarketIssuer) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server/api/oauth2/market",
            get(metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp/market",
            get(resource),
        )
        .route("/api/oauth2/market/register", post(register))
        .route("/api/oauth2/market/authorize", get(begin))
        .route("/api/oauth2/market/continue/{binding}", post(continue_flow))
        .route("/api/oauth2/market/consent/{binding}", post(consent))
        .route("/api/oauth2/market/token", post(token))
        .route("/api/oauth2/market/revoke", post(revoke))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(issuer.clone(), security))
        .with_state(issuer)
}
async fn security(State(i): State<MarketIssuer>, request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|p| p.0.ip().to_string())
        .unwrap_or_else(|| "unknown-peer".into());
    let mut r = match i.rate_limit(&peer).await {
        Ok(true) => next.run(request).await,
        Ok(false) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"temporarily_unavailable"})),
        )
            .into_response(),
        Err(e) => error(e),
    };
    for (k, v) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::PRAGMA, "no-cache"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        r.headers_mut()
            .insert(k, axum::http::HeaderValue::from_static(v));
    }
    r
}
fn error(e: Error) -> Response {
    let (status, code) = match e {
        Error::Invalid => (StatusCode::BAD_REQUEST, "invalid_request"),
        Error::Unauthorized => (StatusCode::BAD_REQUEST, "invalid_grant"),
        Error::Forbidden => (StatusCode::FORBIDDEN, "invalid_scope"),
        Error::Conflict => (StatusCode::CONFLICT, "temporarily_unavailable"),
        Error::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    (status, Json(json!({"error":code}))).into_response()
}
async fn metadata(State(i): State<MarketIssuer>) -> Json<serde_json::Value> {
    let p = i.profile();
    Json(
        json!({"issuer":p.issuer(),"authorization_endpoint":format!("{}/authorize",p.issuer()),"token_endpoint":format!("{}/token",p.issuer()),"registration_endpoint":format!("{}/register",p.issuer()),"revocation_endpoint":format!("{}/revoke",p.issuer()),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"scopes_supported":SCOPES,"authorization_response_iss_parameter_supported":true,"lmm_isolated_loopback_test":p.isolated}),
    )
}
async fn resource(State(i): State<MarketIssuer>) -> Json<serde_json::Value> {
    Json(
        json!({"resource":i.profile().resource(),"authorization_servers":[i.profile().issuer()],"scopes_supported":SCOPES,"bearer_methods_supported":["header"]}),
    )
}
fn no_credentials(h: &HeaderMap) -> Result<()> {
    if [
        "authorization",
        "x-api-key",
        "x-auth-session",
        "x-goog-api-key",
    ]
    .iter()
    .any(|k| h.contains_key(*k))
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
async fn register(
    State(i): State<MarketIssuer>,
    h: HeaderMap,
    Json(r): Json<Registration>,
) -> Response {
    if let Err(e) = no_credentials(&h) {
        return error(e);
    }
    match i.register(r).await {
        Ok(r) => (StatusCode::CREATED, Json(r)).into_response(),
        Err(e) => error(e),
    }
}
fn cookie<'a>(p: &MarketProfile, h: &'a HeaderMap) -> Result<&'a str> {
    no_credentials(h)?;
    let mut origin = h.get_all(header::ORIGIN).iter();
    let o = origin
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    if origin.next().is_some() || o != p.origin() {
        return Err(Error::Unauthorized);
    }
    let mut found = None;
    for raw in h.get_all(header::COOKIE) {
        for part in raw.to_str().map_err(|_| Error::Unauthorized)?.split(';') {
            if let Some((k, v)) = part.trim().split_once('=')
                && k == "new_api_refresh"
            {
                if found.is_some() || v.is_empty() || v.len() > 256 {
                    return Err(Error::Unauthorized);
                }
                found = Some(v);
            }
        }
    }
    found.ok_or(Error::Unauthorized)
}
fn escape(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn page(p: ConsentPage) -> Response {
    let binding = escape(&p.binding);
    let csrf = escape(&p.csrf);
    let name = escape(&p.client_name);
    let resource = escape(&p.resource);
    let html = if let Some(account) = p.account {
        let budget = if p.scopes.iter().any(|s| s == "market:invoke") {
            format!(
                "<label>市场最高消费额度（整数 Credit）<input type=number name=budget_credits min=1 max={} required></label>",
                p.maximum_budget_credits
            )
        } else {
            "<input type=hidden name=budget_credits value=0>".into()
        };
        format!(
            "<!doctype html><html lang=zh-CN><meta charset=utf-8><title>授权工具市场</title><h1>授权工具市场</h1><p>应用：{name}</p><p>服务地址：{resource}</p><p>账号：{}</p><p>权限：{}</p><form method=post action=/api/oauth2/market/consent/{binding}><input type=hidden name=csrf value={csrf}>{budget}<button name=decision value=allow>确认授权</button><button name=decision value=deny formnovalidate>拒绝</button></form></html>",
            escape(&account),
            escape(&p.scopes.join(" "))
        )
    } else {
        format!(
            "<!doctype html><html lang=zh-CN><meta charset=utf-8><title>登录后授权</title><h1>登录后授权工具市场</h1><p>应用：{name}</p><p>服务地址：{resource}</p><p><a href=/login target=_blank rel=noopener>打开登录页面</a>，完成后返回此页。</p><form method=post action=/api/oauth2/market/continue/{binding}><input type=hidden name=csrf value={csrf}><button>继续</button></form></html>"
        )
    };
    Html(html).into_response()
}
async fn begin(
    State(i): State<MarketIssuer>,
    h: HeaderMap,
    Query(r): Query<AuthorizationRequest>,
) -> Response {
    if let Err(e) = no_credentials(&h) {
        return error(e);
    }
    match i.begin(r).await {
        Ok(p) => page(p),
        Err(e) => error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Continue {
    csrf: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Consent {
    csrf: String,
    decision: String,
    budget_credits: Option<i64>,
}
async fn continue_flow(
    State(i): State<MarketIssuer>,
    h: HeaderMap,
    Path(b): Path<String>,
    Form(f): Form<Continue>,
) -> Response {
    let c = match cookie(i.profile(), &h) {
        Ok(c) => c,
        Err(e) => return error(e),
    };
    match i.continue_authorization(&b, &f.csrf, c).await {
        Ok(p) => page(p),
        Err(e) => error(e),
    }
}
async fn consent(
    State(i): State<MarketIssuer>,
    h: HeaderMap,
    Path(b): Path<String>,
    Form(f): Form<Consent>,
) -> Response {
    let c = match cookie(i.profile(), &h) {
        Ok(c) => c,
        Err(e) => return error(e),
    };
    if !matches!(f.decision.as_str(), "allow" | "deny") {
        return error(Error::Invalid);
    }
    match i
        .consent(
            &b,
            &f.csrf,
            c,
            f.decision == "allow",
            f.budget_credits.unwrap_or(0),
        )
        .await
    {
        Ok(u) => Redirect::to(&u).into_response(),
        Err(e) => error(e),
    }
}
async fn token(
    State(i): State<MarketIssuer>,
    h: HeaderMap,
    uri: axum::http::Uri,
    Form(f): Form<TokenForm>,
) -> Response {
    if uri.query().is_some() {
        return error(Error::Invalid);
    }
    if let Err(e) = no_credentials(&h) {
        return error(e);
    }
    match i.token(f).await {
        Ok(t) => Json(t).into_response(),
        Err(e) => error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    client_id: String,
    token: String,
    token_type_hint: Option<String>,
}
async fn revoke(State(i): State<MarketIssuer>, h: HeaderMap, Form(f): Form<Revoke>) -> Response {
    if let Err(e) = no_credentials(&h) {
        return error(e);
    }
    if f.token_type_hint
        .as_deref()
        .is_some_and(|v| !matches!(v, "access_token" | "refresh_token"))
    {
        return error(Error::Invalid);
    }
    match i.revoke(&f.client_id, &f.token).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => error(e),
    }
}
#[derive(Clone)]
struct Private {
    issuer: MarketIssuer,
    key: Arc<[u8]>,
    slots: Arc<tokio::sync::Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorize {
    token: String,
    required_scopes: Vec<String>,
}
/// Merge into the existing private UDS server only, never the public router.
pub fn private_router(issuer: MarketIssuer, host_token: &[u8]) -> Result<Router> {
    if !(32..=4096).contains(&host_token.len()) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/internal/tool-market/v1/oauth-authorize", post(authorize))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
        .with_state(Private {
            issuer,
            key: Sha256::digest(host_token).to_vec().into(),
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }))
}
async fn authorize(State(s): State<Private>, h: HeaderMap, Json(r): Json<Authorize>) -> Response {
    let Ok(_slot) = s.slots.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if h.get_all("x-lmm-protocol").iter().count() != 1
        || h.get("x-lmm-protocol").and_then(|v| v.to_str().ok()) != Some("1")
        || h.contains_key("x-lmm-user-credential")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut values = h.get_all(header::AUTHORIZATION).iter();
    let token = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if values.next().is_some()
        || token.is_none_or(|t| {
            Sha256::digest(t.as_bytes())
                .as_slice()
                .ct_eq(&s.key)
                .unwrap_u8()
                != 1
        })
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let refs = r
        .required_scopes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    match s.issuer.validate(&r.token,&refs).await{Ok(a)=>Json(json!({"user_id":a.user_id.to_string(),"account_id":a.user_id.to_string(),"client_id":a.client_id,"credential_kind":"oauth","credential_id":a.credential_id,"auth_version":a.auth_version,"expires_at_unix":a.expires_at_unix,"can_invoke":a.can_invoke,"can_manage":a.can_manage,"issuer":a.issuer,"resource":a.resource})).into_response(),Err(Error::Unauthorized)=>StatusCode::UNAUTHORIZED.into_response(),Err(Error::Forbidden)=>StatusCode::FORBIDDEN.into_response(),Err(e)=>error(e)}
}
