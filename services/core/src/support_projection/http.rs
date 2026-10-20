//! Mount only on Core's protected private UDS listener, with a support-specific
//! host token. This route must never be mounted on the public HTTP listener.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;
#[derive(Clone)]
struct Service {
    projection: SupportProjection,
    token: [u8; 32],
    slots: Arc<Semaphore>,
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, Error> {
    let mut values = headers.get_all(name).iter();
    let v = values.next().ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized);
    }
    v.to_str().map_err(|_| Error::Unauthorized)
}
impl Service {
    fn check(&self, headers: &HeaderMap) -> Result<(), Error> {
        let bearer = single(headers, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        let digest: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        if !(32..=256).contains(&bearer.len()) || !bool::from(digest.ct_eq(&self.token)) {
            return Err(Error::Unauthorized);
        }
        if single(headers, "x-lmm-protocol")? != "1" {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
async fn capabilities(State(s): State<Service>, headers: HeaderMap) -> Response {
    if s.check(&headers).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(
        serde_json::json!({"protocol_major":1,"features":["support.customers.integer_credits.v1"]}),
    )
    .into_response()
}
async fn customers(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Response {
    let result = async {
        s.check(&headers)?;
        let _slot = s.slots.try_acquire().map_err(|_| Error::Unavailable)?;
        s.projection
            .customers(single(&headers, "x-lmm-user-credential")?, &request)
            .await
    }
    .await;
    match result {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => {
            let (status, code) = match e {
                Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
                Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
                Error::Forbidden => (StatusCode::FORBIDDEN, "denied"),
                Error::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            };
            (status, Json(serde_json::json!({"error":code}))).into_response()
        }
    }
}
pub fn router(projection: SupportProjection, token: &[u8]) -> Result<Router, Error> {
    if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/internal/support/v1/customers", post(customers))
        .route("/internal/support/v1/capabilities", post(capabilities))
        .layer(DefaultBodyLimit::max(1_048_576))
        .with_state(Service {
            projection,
            token: Sha256::digest(token).into(),
            slots: Arc::new(Semaphore::new(4)),
        }))
}
