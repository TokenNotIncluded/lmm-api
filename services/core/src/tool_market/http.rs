//! Internal-only fixed action port. Mount ONLY on the protected UDS listener,
//! with an independent toolmarket host credential. Never mount on public HTTP.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;
#[derive(Clone)]
struct Service {
    store: ToolMarketStore,
    token: [u8; 32],
    slots: Arc<Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorize {
    account: String,
    action: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    operation_id: String,
    reservation: String,
    #[serde(default)]
    result_hash: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    operation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    approval: String,
}
fn single<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str> {
    let mut all = headers.get_all(key).iter();
    let value = all.next().ok_or(Error::Unauthorized)?;
    if all.next().is_some() {
        return Err(Error::Unauthorized);
    }
    value.to_str().map_err(|_| Error::Unauthorized)
}
impl Service {
    fn check(&self, h: &HeaderMap) -> Result<()> {
        let bearer = single(h, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        let hash: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        if !(32..=256).contains(&bearer.len()) || !bool::from(hash.ct_eq(&self.token)) {
            return Err(Error::Unauthorized);
        }
        if single(h, "x-lmm-protocol")? != "1" {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn error(e: Error) -> Response {
    let (status, code) = match e {
        Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
        Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
        Error::Forbidden => (StatusCode::FORBIDDEN, "denied"),
        Error::Missing => (StatusCode::NOT_FOUND, "missing"),
        Error::Conflict => (StatusCode::CONFLICT, "conflict"),
        Error::Pending | Error::Retry | Error::Storage => {
            (StatusCode::SERVICE_UNAVAILABLE, "unknown")
        }
    };
    (status, Json(serde_json::json!({"error":code}))).into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
async fn capabilities(State(s): State<Service>, h: HeaderMap, Json(_): Json<Empty>) -> Response {
    if let Err(e) = s.check(&h) {
        return error(e);
    }
    Json(serde_json::json!({"protocol_major":1,"policy":POLICY,"features":["authorize","quotes","confirm","reserve","dispatch","commit","release","query","reconcile","revoke"]})).into_response()
}
// Explicit functions avoid future routes accidentally omitting either credential.
async fn authorize(State(s): State<Service>, h: HeaderMap, Json(v): Json<Authorize>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .authorize(single(&h, "x-lmm-user-credential")?, &v.account, &v.action)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn quote(State(s): State<Service>, h: HeaderMap, Json(v): Json<Quote>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .register_quote(single(&h, "x-lmm-user-credential")?, &v)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn confirm(State(s): State<Service>, h: HeaderMap, Json(v): Json<Intent>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .confirm(single(&h, "x-lmm-user-credential")?, &v)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn reserve(State(s): State<Service>, h: HeaderMap, Json(v): Json<Call>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .reserve(single(&h, "x-lmm-user-credential")?, &v)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn dispatch(State(s): State<Service>, h: HeaderMap, Json(v): Json<Terminal>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .mark_dispatched(
                single(&h, "x-lmm-user-credential")?,
                &v.operation_id,
                &v.reservation,
            )
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn commit(State(s): State<Service>, h: HeaderMap, Json(v): Json<Terminal>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .commit(&v.operation_id, &v.reservation, &v.result_hash)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn release(State(s): State<Service>, h: HeaderMap, Json(v): Json<Terminal>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .release_undispatched(&v.operation_id, &v.reservation)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn query(State(s): State<Service>, h: HeaderMap, Json(v): Json<Query>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .query(single(&h, "x-lmm-user-credential")?, &v.operation_id)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn reconcile(State(s): State<Service>, h: HeaderMap, Json(v): Json<Call>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store.reconcile(&v).await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn revoke(State(s): State<Service>, h: HeaderMap, Json(v): Json<Revoke>) -> Response {
    let result = async {
        s.check(&h)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        s.store
            .revoke(single(&h, "x-lmm-user-credential")?, &v.approval)
            .await
            .map(|()| serde_json::json!({"revoked":true}))
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}
pub fn router(store: ToolMarketStore, token: &[u8]) -> Result<Router> {
    if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
        return Err(Error::Invalid);
    }
    let state = Service {
        store,
        token: Sha256::digest(token).into(),
        slots: Arc::new(Semaphore::new(4)),
    };
    Ok(Router::new()
        .route("/internal/tool-market/v1/capabilities", post(capabilities))
        .route("/internal/tool-market/v1/authorize", post(authorize))
        .route("/internal/tool-market/v1/quotes", post(quote))
        .route("/internal/tool-market/v1/confirm", post(confirm))
        .route("/internal/tool-market/v1/reserve", post(reserve))
        .route("/internal/tool-market/v1/dispatch", post(dispatch))
        .route("/internal/tool-market/v1/commit", post(commit))
        .route("/internal/tool-market/v1/release", post(release))
        .route("/internal/tool-market/v1/query", post(query))
        .route("/internal/tool-market/v1/reconcile", post(reconcile))
        .route("/internal/tool-market/v1/revoke", post(revoke))
        .layer(DefaultBodyLimit::max(16384))
        .with_state(state))
}
