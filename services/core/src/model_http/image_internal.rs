//! Mount ONLY on the existing protected model UDS listener. This host credential
//! grants transport access, never user identity, quote or spending authority.
use super::image_execution::{DrawingUpload, Error, ImageService, MAX_PREVIEW_BODY, Receipt};
use crate::tool_market::credit_execution::BuiltinContext;
use axum::{
    Json, Router,
    body::{Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_BODY: usize = 64 * 1024;
const MAX_CALLS: usize = 8;

#[derive(Clone)]
struct Internal {
    service: ImageService,
    authority: Authority,
}
#[derive(Clone)]
struct Authority {
    digest: [u8; 32],
    slots: Arc<Semaphore>,
    preview_slots: Arc<Semaphore>,
}
fn valid_host_token(token: &[u8]) -> bool {
    (32..=256).contains(&token.len()) && token.iter().all(u8::is_ascii_graphic)
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, Error> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized);
    }
    value.to_str().map_err(|_| Error::Unauthorized)
}
impl Authority {
    fn new(token: &[u8]) -> Result<Self, Error> {
        if !valid_host_token(token) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            digest: Sha256::digest(token).into(),
            slots: Arc::new(Semaphore::new(MAX_CALLS)),
            preview_slots: Arc::new(Semaphore::new(1)),
        })
    }
    fn host(&self, headers: &HeaderMap) -> Result<(), Error> {
        let token = single(headers, header::AUTHORIZATION.as_str())?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        if !valid_host_token(token.as_bytes()) {
            return Err(Error::Unauthorized);
        }
        let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if !bool::from(self.digest.ct_eq(&supplied)) {
            return Err(Error::Unauthorized);
        }
        if single(headers, "x-lmm-protocol")? != "2" {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    fn session<'a>(&self, headers: &'a HeaderMap) -> Result<&'a str, Error> {
        self.host(headers)?;
        // Same raw Session-secret envelope as existing private Core adapters.
        // ImageService reauthorizes its kind, owner and current permissions.
        let session = single(headers, "x-lmm-user-credential")?;
        if !(32..=256).contains(&session.len())
            || !session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::Unauthorized);
        }
        Ok(session)
    }
    fn acquire(&self) -> Result<OwnedSemaphorePermit, Error> {
        self.slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Pending)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewRequest {
    context: BuiltinContext,
    input: Value,
    #[serde(default)]
    uploads: Vec<DrawingUpload>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationRequest {
    operation_id: String,
    context: BuiltinContext,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    #[serde(default)]
    operation_id: Option<String>,
    context: BuiltinContext,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilitiesRequest {
    context: BuiltinContext,
}
fn ready_body(body: &[u8]) -> Result<(), Error> {
    let value: Value = decode(body)?;
    if value.as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}
fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|_| Error::Invalid)
}
fn settled_only(mut receipt: Receipt) -> Receipt {
    if receipt.state != "completed" {
        receipt.output = None;
        receipt.charged_credits = None;
    }
    receipt
}
fn failure(error: Error) -> Response {
    let (status, code) = match error {
        Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
        Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
        Error::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
        Error::Missing => (StatusCode::NOT_FOUND, "not_found"),
        Error::Conflict => (StatusCode::CONFLICT, "conflict"),
        Error::NoFunds => (StatusCode::PAYMENT_REQUIRED, "insufficient_funds"),
        Error::Pending => (StatusCode::SERVICE_UNAVAILABLE, "pending"),
        Error::Retry | Error::Storage => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    };
    secure((status, Json(json!({"error":code}))).into_response())
}
fn secure(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("static header"),
    );
    response
}
async fn secure_response(response: Response) -> Response {
    secure(response)
}
async fn bounded_body(request: Request) -> Result<Bytes, Box<Response>> {
    bounded_body_limit(request, MAX_BODY).await
}
async fn bounded_body_limit(request: Request, maximum: usize) -> Result<Bytes, Box<Response>> {
    tokio::time::timeout(
        std::time::Duration::from_secs(if maximum > MAX_BODY { 30 } else { 2 }),
        to_bytes(request.into_body(), maximum),
    )
    .await
    .map_err(|_| Box::new(failure(Error::Pending)))?
    .map_err(|_| {
        Box::new(secure(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"error":"body_too_large"})),
            )
                .into_response(),
        ))
    })
}
async fn action(
    State(state): State<Internal>,
    Path(action): Path<String>,
    request: Request,
) -> Response {
    // Admit and authenticate before collecting even the bounded request body.
    let session = match if action == "ready" {
        state.authority.host(request.headers()).map(|()| "")
    } else {
        state.authority.session(request.headers())
    } {
        Ok(session) => session.to_owned(),
        Err(error) => return failure(error),
    };
    let _permit = match state.authority.acquire() {
        Ok(permit) => permit,
        Err(error) => return failure(error),
    };
    if !matches!(
        action.as_str(),
        "preview" | "execute" | "query" | "capabilities" | "ready"
    ) {
        return failure(Error::Missing);
    }
    let _preview_permit = if action == "preview" {
        match state.authority.preview_slots.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => return failure(Error::Pending),
        }
    } else {
        None
    };
    let body = match if action == "preview" {
        bounded_body_limit(request, MAX_PREVIEW_BODY).await
    } else {
        bounded_body(request).await
    } {
        Ok(body) => body,
        Err(response) => return *response,
    };
    let result = async {
        match action.as_str() {
            "ready" => {
                ready_body(&body)?;
                state.service.ready().await?;
                Ok(Json(json!({"protocol":2,"service":"images","ready":true})).into_response())
            }
            "preview" => {
                let input: PreviewRequest = decode(&body)?;
                state
                    .service
                    .preview_with_uploads(&session, &input.context, input.input, input.uploads)
                    .await
                    .map(|preview| Json(preview).into_response())
            }
            "execute" => {
                let input: OperationRequest = decode(&body)?;
                state
                    .service
                    .execute_for_context(&session, &input.operation_id, &input.context)
                    .await
                    .map(|receipt| Json(settled_only(receipt)).into_response())
            }
            "query" => {
                let input: QueryRequest = decode(&body)?;
                let receipt = match input.operation_id {
                    Some(id) => {
                        state
                            .service
                            .query_for_context(&session, &id, &input.context)
                            .await
                    }
                    None => {
                        state
                            .service
                            .query_by_context(&session, &input.context)
                            .await
                    }
                }?;
                Ok(Json(settled_only(receipt)).into_response())
            }
            "capabilities" => {
                let input: CapabilitiesRequest = decode(&body)?;
                state
                    .service
                    .capabilities(&session, &input.context)
                    .await
                    .map(|capabilities| Json(capabilities).into_response())
            }
            _ => Err(Error::Missing),
        }
    }
    .await;
    match result {
        Ok(response) => secure(response),
        Err(error) => failure(error),
    }
}

/// This returned router must be merged into the private UDS router only.
/// Only the typed preview accepts the bounded image sidecar. All other actions
/// retain 64 KiB bodies; neither limit caps image output responses.
pub fn internal_router(service: ImageService, token: Vec<u8>) -> Result<Router, Error> {
    let authority = Authority::new(&token)?;
    Ok(Router::new()
        .route("/internal/models/images/v1/{action}", post(action))
        .layer(middleware::map_response(secure_response))
        .with_state(Internal { service, authority }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    fn authority() -> Authority {
        Authority::new(&[b'a'; 32]).unwrap()
    }
    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", "a".repeat(32)).parse().unwrap(),
        );
        headers.insert("x-lmm-protocol", "2".parse().unwrap());
        headers.insert("x-lmm-user-credential", "s".repeat(32).parse().unwrap());
        headers
    }
    fn context() -> Value {
        json!({"call_id":"call","dispatch_id":"dispatch","tool_id":"drawing.generate","version_id":"1","grant_id":"grant","input_digest":"digest"})
    }
    #[test]
    fn image_internal_readiness_is_host_only_and_closed() {
        let authority = authority();
        let mut headers = headers();
        headers.remove("x-lmm-user-credential");
        assert!(authority.host(&headers).is_ok());
        assert_eq!(authority.session(&headers), Err(Error::Unauthorized));
        assert!(ready_body(br#"{}"#).is_ok());
        for body in [
            b"[]".as_slice(),
            b"null",
            br#"{"ready":true}"#,
            br#"{"context":{}}"#,
        ] {
            assert_eq!(ready_body(body), Err(Error::Invalid));
        }
        headers.append("x-lmm-protocol", "2".parse().unwrap());
        assert_eq!(authority.host(&headers), Err(Error::Unauthorized));
    }
    #[test]
    fn image_internal_unique_credentials_and_exact_protocol() {
        let a = authority();
        let h = headers();
        assert!(a.session(&h).is_ok());
        for name in ["authorization", "x-lmm-protocol", "x-lmm-user-credential"] {
            let mut h = h.clone();
            let value = h.get(name).unwrap().clone();
            h.append(name, value);
            assert_eq!(a.session(&h), Err(Error::Unauthorized));
        }
        let mut h = h.clone();
        h.insert("x-lmm-protocol", "1".parse().unwrap());
        assert_eq!(a.session(&h), Err(Error::Invalid));
        h.insert("x-lmm-protocol", "2, 2".parse().unwrap());
        assert_eq!(a.session(&h), Err(Error::Invalid));
    }
    #[test]
    fn image_internal_wrong_host_token_and_user_envelope_fail() {
        let a = authority();
        let mut h = headers();
        h.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", "b".repeat(32)).parse().unwrap(),
        );
        assert_eq!(a.session(&h), Err(Error::Unauthorized));
        h = headers();
        h.insert("x-lmm-user-credential", "Session abc".parse().unwrap());
        assert_eq!(a.session(&h), Err(Error::Unauthorized));
        assert!(Authority::new(&[b'a'; 31]).is_err());
        assert!(Authority::new(&[b'a'; 257]).is_err());
        assert!(Authority::new(&[b' '; 32]).is_err());
        assert!(Authority::new(&[b'a'; 256]).is_ok());
    }
    #[test]
    fn image_internal_action_dtos_reject_money_url_and_success_claims() {
        let mut preview = json!({"context":context(),"input":{"prompt":"draw"}});
        assert!(decode::<PreviewRequest>(&serde_json::to_vec(&preview).unwrap()).is_ok());
        for field in ["payer", "price", "url", "usage", "success", "confirmed"] {
            preview[field] = json!(true);
            assert!(decode::<PreviewRequest>(&serde_json::to_vec(&preview).unwrap()).is_err());
            preview.as_object_mut().unwrap().remove(field);
            let operation = json!({"operation_id":"image:test","context":context(),field:true});
            assert!(decode::<OperationRequest>(&serde_json::to_vec(&operation).unwrap()).is_err());
        }
        assert!(
            decode::<CapabilitiesRequest>(
                &serde_json::to_vec(&json!({"context":context()})).unwrap()
            )
            .is_ok()
        );
        assert!(decode::<CapabilitiesRequest>(br#"{"context":{},"payer":1}"#).is_err());
        assert!(
            decode::<QueryRequest>(&serde_json::to_vec(&json!({"context":context()})).unwrap())
                .is_ok()
        );
        assert!(
            decode::<OperationRequest>(&serde_json::to_vec(&json!({"context":context()})).unwrap())
                .is_err()
        );
        assert!(
            decode::<OperationRequest>(br#"{"operation_id":"one","operation_id":"two"}"#).is_err()
        );
        assert!(decode::<OperationRequest>(br#"{"operation_id":"one"} {}"#).is_err());
    }
    #[test]
    fn image_internal_global_slots_are_bounded_and_released() {
        let a = authority();
        let mut held = Vec::new();
        for _ in 0..MAX_CALLS {
            held.push(a.acquire().unwrap());
        }
        assert!(matches!(a.acquire(), Err(Error::Pending)));
        held.pop();
        assert!(a.acquire().is_ok());
    }
    #[tokio::test]
    async fn image_internal_body_limit_is_exact_and_no_store() {
        let body = bounded_body(Request::new(Body::from(vec![b' '; MAX_BODY])))
            .await
            .unwrap();
        assert_eq!(body.len(), MAX_BODY);
        let failure = bounded_body(Request::new(Body::from(vec![b' '; MAX_BODY + 1])))
            .await
            .unwrap_err();
        assert_eq!(failure.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(failure.headers()[header::CACHE_CONTROL], "no-store");
    }
    #[test]
    fn image_internal_pending_output_is_never_exposed() {
        let mut receipt = Receipt {
            expires_at: None,
            operation_id: "id".into(),
            state: "billing_pending".into(),
            preview_digest: "d".into(),
            reservation_credits: 5,
            charged_credits: Some(5),
            output: Some(json!({"data":["private"]})),
        };
        let hidden = settled_only(receipt.clone());
        assert!(hidden.output.is_none());
        assert!(hidden.charged_credits.is_none());
        receipt.state = "completed".into();
        assert!(settled_only(receipt).output.is_some());
    }
    #[test]
    fn image_internal_errors_have_stable_service_status_and_no_store() {
        for (error, status) in [
            (Error::Invalid, 400),
            (Error::Unauthorized, 401),
            (Error::Forbidden, 403),
            (Error::Missing, 404),
            (Error::Conflict, 409),
            (Error::NoFunds, 402),
            (Error::Pending, 503),
            (Error::Storage, 503),
            (Error::Retry, 503),
        ] {
            let response = failure(error);
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    #[tokio::test]
    async fn image_internal_preview_sidecar_limit_does_not_expand_other_actions() {
        let bytes = vec![b' '; MAX_BODY + 1];
        assert!(
            bounded_body(Request::new(Body::from(bytes.clone())))
                .await
                .is_err()
        );
        assert!(
            bounded_body_limit(Request::new(Body::from(bytes)), MAX_PREVIEW_BODY)
                .await
                .is_ok()
        );
        let auth = authority();
        let permit = auth.preview_slots.clone().try_acquire_owned().unwrap();
        assert!(auth.preview_slots.clone().try_acquire_owned().is_err());
        drop(permit);
        assert!(auth.preview_slots.clone().try_acquire_owned().is_ok());
    }
}
