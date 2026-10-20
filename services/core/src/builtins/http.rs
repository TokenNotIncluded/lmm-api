//! Mount only on the existing protected marketplace UDS. The marketplace host
//! credential cannot authenticate a user or bypass a code-owned call binding.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use std::{sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;
#[derive(Clone)]
struct Service {
    store: Store,
    token: [u8; 32],
    slots: Arc<Semaphore>,
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    let mut all = headers.get_all(name).iter();
    let value = all.next().ok_or(Error::Unauthorized)?;
    if all.next().is_some() {
        return Err(Error::Unauthorized);
    }
    value.to_str().map_err(|_| Error::Unauthorized)
}
fn failure(e: Error) -> Response {
    let (status, code) = match e {
        Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
        Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
        Error::Forbidden => (StatusCode::FORBIDDEN, "denied"),
        Error::Conflict => (StatusCode::CONFLICT, "conflict"),
        Error::Missing => (StatusCode::NOT_FOUND, "missing"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "unknown"),
    };
    (status, Json(serde_json::json!({"error":code}))).into_response()
}
async fn ready(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(input): Json<serde_json::Value>,
) -> Response {
    let result = async {
        let supplied = single(&headers, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        if !(32..=256).contains(&supplied.len()) || !supplied.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Error::Unauthorized);
        }
        let digest: [u8; 32] = Sha256::digest(supplied.as_bytes()).into();
        if !bool::from(digest.ct_eq(&s.token)) {
            return Err(Error::Unauthorized);
        }
        if single(&headers, "x-lmm-protocol")? != "1"
            || input != serde_json::json!({})
            || headers.contains_key("x-lmm-user-credential")
        {
            return Err(Error::Invalid);
        }
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        tokio::time::timeout(Duration::from_secs(2), s.store.check_ready())
            .await
            .map_err(|_| Error::Pending)?
    }
    .await;
    match result{Ok(v)=>Json(serde_json::json!({"protocol":1,"service":"builtins","ready":true,"tools":v.tools,"catalog_digest":v.catalog_digest})).into_response(),Err(e)=>failure(e)}
}
async fn authorize(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(input): Json<Invocation>,
) -> Response {
    let result = async {
        let supplied = single(&headers, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        if !(32..=256).contains(&supplied.len()) || !supplied.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Error::Unauthorized);
        }
        let digest: [u8; 32] = Sha256::digest(supplied.as_bytes()).into();
        if !bool::from(digest.ct_eq(&s.token)) {
            return Err(Error::Unauthorized);
        }
        if single(&headers, "x-lmm-protocol")? != "1" {
            return Err(Error::Invalid);
        }
        let _slot = s.slots.try_acquire().map_err(|_| Error::Pending)?;
        let session = single(&headers, "x-lmm-user-credential")?;
        tokio::time::timeout(Duration::from_secs(2), s.store.authorize(session, &input))
            .await
            .map_err(|_| Error::Pending)?
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(e),
    }
}
pub fn router(store: Store, token: &[u8]) -> Result<Router> {
    if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
        return Err(Error::Invalid);
    }
    let service = Service {
        store,
        token: Sha256::digest(token).into(),
        slots: Arc::new(Semaphore::new(8)),
    };
    Ok(Router::new()
        .route("/internal/builtins/v1/authorize", post(authorize))
        .route("/internal/builtins/v1/ready", post(ready))
        .layer(DefaultBodyLimit::max(80 << 10))
        .with_state(service))
}
