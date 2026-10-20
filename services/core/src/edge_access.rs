//! Optional production edge boundary. Standalone Core does not enable it.
//!
//! The bounded rule subset and response semantics preserve go-v0.2.118
//! setting/ip_access_routing.go and controller/{ip_access_routing,access_policy_error}.go
//! (AGPL-3.0-or-later, LIghtJUNction). Unsupported legacy predicates reject setup.
//! Header extraction follows imbolc/axum-client-ip's single trusted-header design;
//! transport authentication is additionally required here, not supplied by that crate.
use axum::{
    Json, Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use ipnet::IpNet;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

pub const PROOF_HEADER: &str = "x-lmm-edge-proof";
pub const CLIENT_HEADER: &str = "x-lmm-edge-client-ip";
const RULE_KEY: &str = "IPAccessRoutingRules";
const ELIGIBILITY_KEY: &str = "IPAccessRoutingBypassEligibilityV1";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedConfig {
    pub proof_file: PathBuf,
}
#[derive(Debug)]
pub struct SetupError;
impl std::fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid or unavailable edge access configuration")
    }
}
impl std::error::Error for SetupError {}
/// Never replace the physical peer used by this security boundary with a header.
#[derive(Clone, Copy)]
pub struct OriginalSocketPeer(pub SocketAddr);
#[derive(Clone, Copy)]
struct EdgeClient(IpAddr);
#[derive(Clone)]
pub struct EdgeAccess {
    pool: PgPool,
    proof: Arc<Vec<u8>>,
    limiter: Arc<Mutex<Limiter>>,
}
impl EdgeAccess {
    pub async fn new(pool: PgPool, config: TypedConfig) -> Result<Self, SetupError> {
        if !config.proof_file.is_absolute() {
            return Err(SetupError);
        }
        #[cfg(unix)]
        let proof =
            crate::internal_rpc::read_service_token(&config.proof_file).map_err(|_| SetupError)?;
        #[cfg(not(unix))]
        let proof: Vec<u8> = return Err(SetupError);
        let state = Self {
            pool,
            proof: Arc::new(proof),
            limiter: Arc::new(Mutex::new(Limiter::default())),
        };
        let source: Option<String> =
            sqlx::query_scalar("SELECT value FROM core_console.options WHERE key=$1")
                .bind(RULE_KEY)
                .fetch_optional(&state.pool)
                .await
                .map_err(|_| SetupError)?;
        Policy::parse(&source.ok_or(SetupError)?)?;
        Ok(state)
    }
    /// Call once after all public routers are assembled. No private UDS router is wrapped.
    pub fn protect(self, routes: Router) -> Router {
        let internal = Router::new()
            .route("/internal/access-ip-policy", get(check))
            .route("/internal/errors/access-policy", get(error_page))
            .with_state(self.clone());
        routes
            .merge(internal)
            .layer(middleware::from_fn_with_state(self, trusted_edge))
    }
    async fn bypass(&self, headers: &HeaderMap, client: IpAddr) -> bool {
        let Some(secret) = bearer(headers) else {
            return false;
        };
        let Some(reservation) = Reservation::take(self.limiter.clone(), client) else {
            return false;
        };
        let verified = tokio::time::timeout(LOOKUP_TIMEOUT, async {
            let mut tx = self.pool.begin().await.ok()?;
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await
                .ok()?;
            let actor = crate::identity::authenticate_readonly_in_transaction(&mut tx, secret)
                .await
                .ok()?;
            if actor.credential_kind != crate::identity::CredentialKind::ApiKey {
                return None;
            }
            // A gate bypass must not widen the native Key's source-IP scope.
            let ip_allowed: Option<bool> = sqlx::query_scalar("SELECT enabled AND (cardinality(allow_ips)=0 OR $2::inet <<= ANY(allow_ips)) FROM core_identity.api_key_settings WHERE credential_id=$1")
                .bind(actor.credential_id).bind(client.to_string()).fetch_optional(&mut *tx).await.ok()?;
            if ip_allowed != Some(true) { return None; }
            let setting: String = sqlx::query_scalar(
                "SELECT setting FROM core_identity.user_profiles WHERE user_id=$1",
            )
            .bind(actor.user_id)
            .fetch_one(&mut *tx)
            .await
            .ok()?;
            if setting.len() > 32768 {
                return None;
            }
            // The original explicit opt-in is independent of every trust/access fact.
            if !serde_json::from_str::<OptIn>(&setting)
                .ok()?
                .allow_key_bypass_ip_policy
            {
                return None;
            }
            let document: Option<String> =
                sqlx::query_scalar("SELECT value FROM core_console.options WHERE key=$1")
                    .bind(ELIGIBILITY_KEY)
                    .fetch_optional(&mut *tx)
                    .await
                    .ok()?;
            let document = document?;
            if document.len() > 1048576 {
                return None;
            }
            let facts: Eligibility = serde_json::from_str(&document).ok()?;
            if !facts.allows(actor.user_id) {
                return None;
            }
            tx.commit().await.ok()?;
            Some(())
        })
        .await
        .ok()
        .flatten()
        .is_some();
        if verified {
            reservation.success();
        }
        verified
    }
}

fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() || value.is_empty() || value != value.trim() {
        return None;
    }
    Some(value)
}
fn canonical_ip(raw: &str) -> Option<IpAddr> {
    // No comma chains, ports, zone IDs, whitespace or fallback header guessing.
    let ip: IpAddr = raw.parse().ok()?;
    Some(match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        _ => ip,
    })
}
fn edge_peer(state: &EdgeAccess, headers: &HeaderMap, physical: SocketAddr) -> Option<IpAddr> {
    if !physical.ip().is_loopback() {
        return None;
    }
    let proof = single(headers, PROOF_HEADER)?.as_bytes();
    if proof.len() != state.proof.len() || !bool::from(proof.ct_eq(state.proof.as_slice())) {
        return None;
    }
    canonical_ip(single(headers, CLIENT_HEADER)?)
}
async fn trusted_edge(
    State(state): State<EdgeAccess>,
    mut request: Request,
    next: Next,
) -> Response {
    let physical = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0);
    let Some((physical, client)) =
        physical.and_then(|p| edge_peer(&state, request.headers(), p).map(|c| (p, c)))
    else {
        return denied("INTERNAL_ONLY");
    };
    request
        .extensions_mut()
        .insert(OriginalSocketPeer(physical));
    request.extensions_mut().insert(EdgeClient(client));
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(client, 0)));
    // A provider/Go request must never learn the proof used by Nginx to Core.
    request.headers_mut().remove(PROOF_HEADER);
    next.run(request).await
}
fn denied(code: &'static str) -> Response {
    (
        StatusCode::FORBIDDEN,
        [("x-lmm-access-policy", "denied")],
        Json(json!({"success":false,"code":code,"message":"access policy unavailable"})),
    )
        .into_response()
}
async fn check(State(state): State<EdgeAccess>, request: Request) -> Response {
    let Some(client) = request.extensions().get::<EdgeClient>().map(|p| p.0) else {
        return denied("INTERNAL_ONLY");
    };
    let country = match country(request.headers()) {
        Ok(c) => c,
        Err(_) => return denied("POLICY_UNAVAILABLE"),
    };
    let policy = tokio::time::timeout(LOOKUP_TIMEOUT, async {
        let source: Option<String> =
            sqlx::query_scalar("SELECT value FROM core_console.options WHERE key=$1")
                .bind(RULE_KEY)
                .fetch_optional(&state.pool)
                .await
                .map_err(|_| SetupError)?;
        Policy::parse(&source.ok_or(SetupError)?)
    })
    .await;
    let Ok(Ok(policy)) = policy else {
        return denied("POLICY_UNAVAILABLE");
    };
    let action = policy.evaluate(client, country.as_deref()).ok();
    if action == Some(Action::Direct) || state.bypass(request.headers(), client).await {
        return StatusCode::NO_CONTENT.into_response();
    }
    denied(if action == Some(Action::Reject) {
        "IP_ACCESS_ROUTE_REJECTED"
    } else {
        "POLICY_UNAVAILABLE"
    })
}
fn country(headers: &HeaderMap) -> Result<Option<String>, SetupError> {
    let mut values = headers.get_all("x-lmm-edge-country").iter();
    let value = values
        .next()
        .map(|v| v.to_str().map_err(|_| SetupError))
        .transpose()?
        .unwrap_or("");
    if values.next().is_some() || value != value.trim() {
        return Err(SetupError);
    }
    if value.is_empty() {
        return match single(headers, "x-lmm-cn-source") {
            Some("1") => Ok(Some("CN".into())),
            Some("0") | None => Ok(None),
            _ => Err(SetupError),
        };
    }
    if value.len() != 2 || !value.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(SetupError);
    }
    Ok(Some(value.to_ascii_uppercase()))
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let raw = single(headers, "authorization")?;
    let mut parts = raw.split_ascii_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if !scheme.eq_ignore_ascii_case("Bearer") || parts.next().is_some() || token.len() > 256 {
        return None;
    }
    // Native Core keys are opaque. Never strip sk- or a group suffix.
    Some(token)
}
#[derive(Deserialize)]
struct OptIn {
    #[serde(default)]
    allow_key_bypass_ip_policy: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Eligibility {
    version: u8,
    users: Vec<EligibilityUser>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EligibilityUser {
    user_id: i64,
    minimum_trust_level: u8,
    basis: EligibilityBasis,
    source_facts_sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum EligibilityBasis {
    SourceConsoleActivation,
    SourceExplicitOverride,
    SourceAdminRole,
    SourcePaidProjection,
}
impl Eligibility {
    fn allows(&self, user: i64) -> bool {
        if self.version != 1 || self.users.len() > 4096 {
            return false;
        }
        let mut seen = std::collections::HashSet::new();
        for fact in &self.users {
            if fact.user_id <= 0
                || !seen.insert(fact.user_id)
                || !(1..=6).contains(&fact.minimum_trust_level)
                || fact.source_facts_sha256.len() != 64
                || !fact
                    .source_facts_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return false;
            }
            let _ = &fact.basis;
        }
        self.users.iter().any(|fact| fact.user_id == user)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Direct,
    Reject,
}
enum Matcher {
    Prefix(IpNet),
    Country(String),
}
struct Policy {
    rules: Vec<(Matcher, Action)>,
    fallback: Action,
}
impl Policy {
    fn parse(source: &str) -> Result<Self, SetupError> {
        if source.len() > 16384 {
            return Err(SetupError);
        }
        let mut rules = Vec::new();
        let mut fallback = None;
        for raw in source.lines() {
            if raw.len() > 1024 {
                return Err(SetupError);
            }
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line == "routing {" || line == "}" {
                continue;
            }
            if let Some(value) = line.strip_prefix("fallback:") {
                if fallback.is_some() {
                    return Err(SetupError);
                }
                fallback = Some(action(value)?);
                continue;
            }
            if rules.len() >= 256 {
                return Err(SetupError);
            }
            let (predicate, result) = line.split_once("->").ok_or(SetupError)?;
            let argument = predicate
                .trim()
                .strip_prefix("dip(")
                .and_then(|s| s.strip_suffix(')'))
                .ok_or(SetupError)?
                .trim();
            let matcher = if let Some(code) = argument.strip_prefix("geoip:") {
                if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
                    return Err(SetupError);
                }
                Matcher::Country(code.to_ascii_uppercase())
            } else {
                let net = argument
                    .parse::<IpNet>()
                    .or_else(|_| argument.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| SetupError)?;
                Matcher::Prefix(net)
            };
            rules.push((matcher, action(result)?));
        }
        if rules.is_empty() && fallback.is_none() {
            return Err(SetupError);
        }
        Ok(Self {
            rules,
            fallback: fallback.unwrap_or(Action::Direct),
        })
    }
    fn evaluate(&self, ip: IpAddr, country: Option<&str>) -> Result<Action, SetupError> {
        for (matcher, result) in &self.rules {
            let matched = match matcher {
                Matcher::Prefix(prefix) => prefix.contains(&ip),
                Matcher::Country(expected) => country.ok_or(SetupError)? == expected,
            };
            if matched {
                return Ok(*result);
            }
        }
        Ok(self.fallback)
    }
}
fn action(raw: &str) -> Result<Action, SetupError> {
    match raw.trim() {
        "direct" => Ok(Action::Direct),
        "reject" => Ok(Action::Reject),
        _ => Err(SetupError),
    }
}

#[derive(Default)]
struct Limiter {
    global: Counter,
    sources: HashMap<IpAddr, Counter>,
}
struct Counter {
    start: Instant,
    failures: u16,
    active: u16,
}
impl Default for Counter {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            failures: 0,
            active: 0,
        }
    }
}
impl Counter {
    fn refresh(&mut self) {
        if self.active == 0 && self.start.elapsed() >= Duration::from_secs(60) {
            *self = Self::default();
        }
    }
}
struct Reservation {
    limiter: Arc<Mutex<Limiter>>,
    ip: IpAddr,
    failed: bool,
}
impl Reservation {
    fn take(limiter: Arc<Mutex<Limiter>>, ip: IpAddr) -> Option<Self> {
        {
            let mut state = limiter.lock().ok()?;
            state.global.refresh();
            if state.global.failures + state.global.active >= 256 {
                return None;
            }
            state
                .sources
                .retain(|_, v| v.active > 0 || v.start.elapsed() < Duration::from_secs(60));
            if state.sources.len() >= 256 && !state.sources.contains_key(&ip) {
                return None;
            }
            let source = state.sources.entry(ip).or_default();
            source.refresh();
            if source.failures + source.active >= 10 {
                return None;
            }
            source.active += 1;
            state.global.active += 1;
        }
        Some(Self {
            limiter,
            ip,
            failed: true,
        })
    }
    fn success(mut self) {
        self.failed = false;
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.limiter.lock() {
            state.global.active = state.global.active.saturating_sub(1);
            state.global.failures += u16::from(self.failed);
            if let Some(source) = state.sources.get_mut(&self.ip) {
                source.active = source.active.saturating_sub(1);
                source.failures += u16::from(self.failed);
            }
        }
    }
}

async fn error_page(request: Request) -> Response {
    if single(request.headers(), "x-lmm-internal-error") != Some("access-policy")
        || single(request.headers(), "x-lmm-access-policy") != Some("denied")
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let uri = single(request.headers(), "x-lmm-original-uri").unwrap_or("");
    let path = uri.split('?').next().unwrap_or("");
    if concealed(path) {
        return (StatusCode::NOT_FOUND, Json(json!({"message":"Not Found"}))).into_response();
    }
    let id = ["x-oneapi-request-id", "x-request-id"]
        .iter()
        .find_map(|k| single(request.headers(), k))
        .filter(|s| {
            s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()));
    let json_requested = single(request.headers(), "x-lmm-original-accept").is_some_and(wants_json);
    let mut response = if json_requested {
        (StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,Json(json!({"error":{"code":"IP_ACCESS_ROUTE_REJECTED","message":"Request rejected by IP access policy.","request_id":id,"type":"access_policy_error"}}))).into_response()
    } else {
        let english = single(request.headers(), "accept-language")
            .is_some_and(|s| s.to_ascii_lowercase().starts_with("en"));
        (
            StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
            Html(render_error(english, &id)),
        )
            .into_response()
    };
    for (name, value) in [
        ("cache-control", "private, no-store, max-age=0"),
        ("pragma", "no-cache"),
        ("vary", "Accept, Origin"),
        ("x-content-type-options", "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-oneapi-request-id", value);
    }
    if json_requested && request.headers().contains_key(header::ORIGIN) {
        response
            .headers_mut()
            .insert("access-control-allow-origin", HeaderValue::from_static("*"));
        response.headers_mut().insert(
            "access-control-expose-headers",
            HeaderValue::from_static("X-Oneapi-Request-Id"),
        );
    }
    response
}
fn wants_json(accept: &str) -> bool {
    if accept.len() > 8192 {
        return false;
    }
    accept.split(',').take(128).any(|range| {
        let mut parts = range.trim().split(';');
        let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let json = media == "application/json"
            || media == "text/json"
            || media
                .split_once('/')
                .is_some_and(|(top, sub)| !top.is_empty() && sub.ends_with("+json"));
        json && parts.all(|parameter| {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                return false;
            };
            if !name.trim().eq_ignore_ascii_case("q") {
                return true;
            }
            value
                .trim()
                .parse::<f32>()
                .is_ok_and(|q| q > 0.0 && q <= 1.0)
        })
    })
}
fn concealed(path: &str) -> bool {
    [
        "/api",
        "/mcp",
        "/v1",
        "/v1beta",
        "/pg",
        "/mj",
        "/suno",
        "/kling/v1",
        "/jimeng",
        "/typesafe",
        "/core",
    ]
    .iter()
    .any(|p| path == *p || path.strip_prefix(p).is_some_and(|s| s.starts_with('/')))
        || matches!(
            path,
            "/dashboard/billing/subscription" | "/dashboard/billing/usage"
        )
        || {
            let mut parts = path.trim_start_matches('/').split('/');
            matches!((parts.next(),parts.next()),(Some(a),Some("mj")) if !a.is_empty() && a!="oauth" && a!="static")
        }
}
fn render_error(english: bool, id: &str) -> String {
    let text = if english {
        [
            "en",
            "Regional Access Restriction",
            "Access is restricted in your region.",
            "This is an access restriction, not a temporary network outage or server failure.",
            "If you believe this decision is incorrect, provide the request ID to the website administrator.",
            "Go back",
            "Request ID",
        ]
    } else {
        [
            "zh",
            "地区访问限制",
            "当前访问受地区限制。",
            "这是访问限制，并非暂时断网或服务器故障。",
            "若认为判定有误，请向网站管理员提供请求编号。",
            "返回上一页",
            "请求编号",
        ]
    };
    let mut page = include_str!("edge_access/error.html").to_owned();
    for (key, value) in [
        "Language",
        "Title",
        "Message",
        "Reason",
        "Help",
        "Back",
        "RequestIDLabel",
    ]
    .into_iter()
    .zip(text)
    {
        page = page.replace(&format!("{{{{.{key}}}}}"), value);
    }
    page.replace("{{.RequestID}}", id)
}

#[cfg(test)]
#[path = "edge_access/tests.rs"]
mod tests;
