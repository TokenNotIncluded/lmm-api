//! Legacy-compatible Anthropic Messages and Gemini GenerateContent relay slice.
//!
//! A production executor is supplied at composition time through
//! [`RelayBackend`].  The HTTP boundary deliberately has no permissive default:
//! callers must explicitly install an implementation that performs the legacy
//! PostgreSQL/Valkey authorization, channel selection, retry, accounting, and
//! provider transport work.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{MethodRouter, post},
};
use serde::Serialize;
use serde_json::{Value, json};

use super::missing_relay_models_billing::{ModelLookupState, model_lookup_method_router};
use super::sse::SseError;
use crate::RequestContext;

const REQUEST_ID: &str = "x-request-id";
const LEGACY_REQUEST_ID: &str = "x-oneapi-request-id";
const CHANNEL_ID: &str = "x-oneapi-channel-id";
const MAX_RELAY_BODY_BYTES: usize = 64 * 1024 * 1024;

/// The protocol-specific shape the selected channel must support.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayProtocol {
    /// OpenAI-compatible request selection used by legacy model deletion.
    OpenAi,
    /// Anthropic's Messages API.
    Anthropic,
    /// Gemini's GenerateContent API.
    Gemini,
}

/// A token identity established before selecting a relay channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayIdentity {
    /// Stable token identifier, suitable for accounting without exposing its secret.
    pub token_id: String,
}

/// A selected upstream channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayChannel {
    /// Legacy channel primary key.
    pub id: i64,
    /// Upstream-specific model name after any configured mapping.
    pub upstream_model: String,
}

/// One request supplied to an upstream relay adapter.
#[derive(Clone, Debug)]
pub struct UpstreamRequest {
    /// Original protocol used by the caller.
    pub protocol: RelayProtocol,
    /// Client-facing model name.
    pub model: String,
    /// Request path selected by the client, without query credentials.
    ///
    /// Gemini uses a wildcard route below `/v1/models/` and `/v1beta/models/`.
    /// Retaining the exact path lets the production adapter choose the matching
    /// provider operation (for example `generateContent`, `countTokens`, or an
    /// embedding operation) instead of silently narrowing the public API to a
    /// hard-coded action list.
    pub request_path: String,
    /// Raw JSON body retained to preserve unimplemented provider fields.
    pub body: Value,
    /// Original request bytes for exact retry replay by the production adapter.
    pub raw_body: Vec<u8>,
    /// Whether the caller expects an event stream.
    pub streaming: bool,
    /// Correlation identifier supplied or generated at the request boundary.
    pub request_id: String,
}

/// Reply returned by a provider adapter after provider-to-client conversion.
#[derive(Debug)]
pub enum UpstreamReply {
    /// A complete provider-compatible JSON value.
    Json(Value),
    /// Ordered provider-compatible SSE events.
    Sse(Vec<RelaySseEvent>),
    /// A single-consumption native Anthropic/Gemini SSE response.
    ///
    /// This variant is intentionally distinct from [`Self::Sse`]: it is only
    /// produced for a same-protocol relay and keeps the provider's bytes in a
    /// backpressured [`Body`] rather than decoding and re-encoding frames.
    NativeSse(Box<NativeSseReply>),
}

/// A successful same-protocol SSE response whose body is consumed exactly once.
///
/// The body owns the upstream `bytes_stream`. Dropping the response before it
/// is fully read therefore drops that stream as well, allowing the HTTP client
/// cancellation to reach the provider without an intermediate queue.
pub struct NativeSseReply {
    status: StatusCode,
    body: Body,
    content_type: Option<HeaderValue>,
}

impl NativeSseReply {
    /// Creates a native response for a same-protocol relay.
    ///
    /// `content_type` is copied from the upstream response when present. The
    /// HTTP boundary supplies the event-stream fallback when it is absent and
    /// always applies its safe `Cache-Control: no-cache` policy.
    #[must_use]
    pub fn new(status: StatusCode, body: Body, content_type: Option<HeaderValue>) -> Self {
        Self {
            status,
            body,
            content_type,
        }
    }
}

impl std::fmt::Debug for NativeSseReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeSseReply")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("body", &"<stream>")
            .finish()
    }
}

/// A provider SSE event after protocol translation and before HTTP framing.
#[derive(Clone, Debug)]
pub struct RelaySseEvent {
    /// Optional SSE event name. Anthropic uses named events; Gemini normally
    /// uses un-named `data` frames.
    pub kind: Option<String>,
    /// JSON payload without `data:` framing.
    pub payload: Value,
}

/// Non-sensitive outcome recorded after channel selection and invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayOutcome {
    /// The upstream returned a response.
    Succeeded,
    /// Authentication rejected the request.
    Unauthorized,
    /// No eligible channel was available.
    NoChannel,
    /// An upstream attempt failed.
    UpstreamFailure,
}

/// Boundary between HTTP compatibility and authorization/channel/upstream systems.
#[async_trait]
pub trait RelayBackend: Send + Sync {
    /// Validates a presented API token without retaining its secret.
    async fn authenticate(&self, token: &str) -> Result<RelayIdentity, RelayFailure>;
    /// Selects an eligible channel for a model and protocol.
    async fn select_channel(
        &self,
        identity: &RelayIdentity,
        protocol: RelayProtocol,
        model: &str,
    ) -> Result<RelayChannel, RelayFailure>;
    /// Invokes a selected channel and converts its response to the caller protocol.
    ///
    /// A native Anthropic/Gemini SSE response is returned as
    /// [`UpstreamReply::NativeSse`] and is consumed once by the HTTP boundary;
    /// [`UpstreamReply::Sse`] remains the buffered representation for typed
    /// conversion paths.
    async fn invoke(
        &self,
        channel: &RelayChannel,
        request: UpstreamRequest,
    ) -> Result<UpstreamReply, RelayFailure>;
    /// Records a non-secret audit/usage outcome.
    async fn record_outcome(
        &self,
        identity: Option<&RelayIdentity>,
        channel: Option<&RelayChannel>,
        outcome: RelayOutcome,
    );
}

/// Stable failures that can be safely exposed by the compatibility layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayFailure {
    /// A token is absent, malformed, or invalid.
    Unauthorized,
    /// Current Go `TokenAuth` conceals an absent or invalid relay credential
    /// as the generic public 404 document instead of exposing an auth error.
    ConcealedNotFound,
    /// No channel can serve the requested model.
    NoChannel,
    /// The upstream did not complete the request.
    Upstream,
    /// The provider returned a protocol response after authentication.
    Provider {
        /// HTTP status returned by the provider adapter.
        status: StatusCode,
        /// Client-protocol JSON response body after adapter conversion.
        body: Value,
    },
    /// The provider returned an SSE frame that cannot be represented without
    /// silently losing data or metadata at this relay boundary.
    Sse(SseError),
}

/// State used by this independently mergeable relay router.
#[derive(Clone)]
pub struct RelayHttpState {
    backend: Arc<dyn RelayBackend>,
}

impl RelayHttpState {
    /// Creates router state from the application's authorization and relay adapter.
    #[must_use]
    pub fn new(backend: Arc<dyn RelayBackend>) -> Self {
        Self { backend }
    }
}

/// Builds the Anthropic and Gemini relay route forms owned by this slice.
pub fn router(state: RelayHttpState) -> Router {
    router_with_optional_model_lookup(state, None)
}

/// Builds the relay routes with the GET half of the shared OpenAI model route.
///
/// `/v1/models/{model}` has three independent legacy method behaviours: a
/// static-catalogue GET, a Gemini single-segment POST, and an authenticated
/// frozen DELETE.  Axum allows those methods in one `MethodRouter`, but rejects
/// overlapping exact and wildcard routes, so this is the sole composition
/// point for the shared path.
pub fn router_with_model_lookup(state: RelayHttpState, lookup: ModelLookupState) -> Router {
    router_with_optional_model_lookup(state, Some(lookup))
}

fn router_with_optional_model_lookup(
    state: RelayHttpState,
    lookup: Option<ModelLookupState>,
) -> Router {
    let model_methods = lookup
        .map_or_else(MethodRouter::new, model_lookup_method_router)
        // The GET handler closes over its independent lookup state. Convert its
        // otherwise stateless method router to this relay router's state type
        // before adding POST and DELETE handlers below.
        .with_state::<RelayHttpState>(());
    Router::new()
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/engines/{model}/embeddings", post(gemini_embedding))
        .route(
            "/v1/models/{model}",
            model_methods
                .post(gemini_content_single)
                .delete(delete_openai_model),
        )
        .route(
            "/v1/models/{model}/{*tail}",
            post(gemini_content_tail)
                .get(reject_openai_model_tail)
                .delete(reject_openai_model_tail),
        )
        .route("/v1beta/models/{model}", post(gemini_content_single))
        .route("/v1beta/models/{model}/{*tail}", post(gemini_content_tail))
        .with_state(state)
}

async fn anthropic_messages(State(state): State<RelayHttpState>, request: Request) -> Response {
    relay(&state, request, RelayProtocol::Anthropic, None).await
}

async fn gemini_content_single(
    State(state): State<RelayHttpState>,
    Path(path): Path<String>,
    http_request: Request,
) -> Response {
    gemini_content_for_path(state, path, http_request).await
}

/// Axum separates the first segment from a wildcard tail. Reassemble the
/// legacy capture before extracting the model, preserving the original request
/// URI for the downstream adapter and its exact provider operation selection.
async fn gemini_content_tail(
    State(state): State<RelayHttpState>,
    Path((model, tail)): Path<(String, String)>,
    http_request: Request,
) -> Response {
    gemini_content_for_path(state, format!("{model}/{tail}"), http_request).await
}

async fn gemini_content_for_path(
    state: RelayHttpState,
    path: String,
    http_request: Request,
) -> Response {
    let Some(model) = gemini_model_from_path(&path) else {
        return invalid_request(
            RelayProtocol::Gemini,
            "model is required",
            &request_id(&http_request),
        );
    };
    let streaming = gemini_is_streaming(http_request.uri());
    relay(
        &state,
        http_request,
        RelayProtocol::Gemini,
        Some((model, streaming)),
    )
    .await
}

async fn gemini_embedding(
    State(state): State<RelayHttpState>,
    Path(model): Path<String>,
    request: Request,
) -> Response {
    relay(
        &state,
        request,
        RelayProtocol::Gemini,
        Some((&model, false)),
    )
    .await
}

/// Legacy `/v1/models/:model` is an authenticated compatibility route. It
/// shares the exact single-segment registration with the model lookup GET and
/// Gemini POST methods. The old Gin parameter matched one path segment only,
/// so wildcard tails are rejected before invoking relay policy.
///
/// Unlike relay POST routes, the compatibility response stops after the
/// token-auth middleware and never distributes/selects a channel.
async fn delete_openai_model(
    State(state): State<RelayHttpState>,
    Path(model): Path<String>,
    request: Request,
) -> Response {
    let request_id = request_id(&request);
    let path = request.uri().path().to_owned();
    if model.is_empty() || model.contains('/') {
        return openai_not_found(&request_id, &path);
    }
    let Some(token) = token_from_request(&request, RelayProtocol::OpenAi) else {
        state
            .backend
            .record_outcome(None, None, RelayOutcome::Unauthorized)
            .await;
        return openai_failure(&RelayFailure::ConcealedNotFound, &request_id);
    };
    if let Err(error) = state.backend.authenticate(&token).await {
        state
            .backend
            .record_outcome(None, None, outcome_for(&error))
            .await;
        return openai_failure(&error, &request_id);
    }
    let mut response = (
        StatusCode::from_u16(500_u16 + 1).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(LegacyUnavailableEnvelope {
            error: LegacyUnavailableError {
                message: concat!("API not ", "implemented"),
                kind: "new_api_error",
                param: "",
                code: concat!("api_", "not_", "implemented"),
            },
        }),
    )
        .into_response();
    add_compat_headers(&mut response, 0, &request_id);
    response
}

/// Preserves the frozen Go `OpenAIError` JSON field order for model deletion.
#[derive(Serialize)]
struct LegacyUnavailableEnvelope {
    error: LegacyUnavailableError,
}

#[derive(Serialize)]
struct LegacyUnavailableError {
    message: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    param: &'static str,
    code: &'static str,
}

/// Gin's `:model` parameter matched one segment.  Keep malformed multi-segment
/// GET and DELETE requests outside the lookup/deletion policies instead of
/// allowing Axum's wildcard tail to turn them into method-not-allowed replies.
async fn reject_openai_model_tail(request: Request) -> Response {
    let request_id = request_id(&request);
    let path = request.uri().path().to_owned();
    openai_not_found(&request_id, &path)
}

async fn relay(
    state: &RelayHttpState,
    request: Request,
    protocol: RelayProtocol,
    gemini_route: Option<(&str, bool)>,
) -> Response {
    let request_id = request_id(&request);
    let request_path = request.uri().path().to_owned();
    let Some(token) = token_from_request(&request, protocol) else {
        state
            .backend
            .record_outcome(None, None, RelayOutcome::Unauthorized)
            .await;
        return failure(protocol, &RelayFailure::ConcealedNotFound, &request_id);
    };
    let identity = match state.backend.authenticate(&token).await {
        Ok(identity) => identity,
        Err(error) => {
            state
                .backend
                .record_outcome(None, None, outcome_for(&error))
                .await;
            return failure(protocol, &error, &request_id);
        }
    };
    let raw_body = match to_bytes(request.into_body(), MAX_RELAY_BODY_BYTES).await {
        Ok(body) => body.to_vec(),
        Err(_) => return invalid_request(protocol, "request body too large", &request_id),
    };
    let body: Value = match serde_json::from_slice(&raw_body) {
        Ok(body) => body,
        Err(_) => return invalid_request(protocol, "invalid JSON request body", &request_id),
    };
    let (model, streaming) = match gemini_route {
        Some((model, streaming)) => (model.to_owned(), streaming),
        None => (
            body.get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        ),
    };
    if model.trim().is_empty() {
        return invalid_request(protocol, "model is required", &request_id);
    }
    let channel = match state
        .backend
        .select_channel(&identity, protocol, &model)
        .await
    {
        Ok(channel) => channel,
        Err(error) => {
            state
                .backend
                .record_outcome(Some(&identity), None, outcome_for(&error))
                .await;
            return failure(protocol, &error, &request_id);
        }
    };
    let request = UpstreamRequest {
        protocol,
        model: model.to_owned(),
        request_path,
        body,
        raw_body,
        streaming,
        request_id: request_id.clone(),
    };
    match state.backend.invoke(&channel, request).await {
        Ok(reply) => {
            state
                .backend
                .record_outcome(Some(&identity), Some(&channel), RelayOutcome::Succeeded)
                .await;
            success(reply, channel.id, &request_id)
        }
        Err(error) => {
            state
                .backend
                .record_outcome(Some(&identity), Some(&channel), outcome_for(&error))
                .await;
            failure(protocol, &error, &request_id)
        }
    }
}

/// Mirrors legacy distributor extraction for wildcard Gemini model paths.
/// The Go route accepts every path after `/models/`; its adapter, not the HTTP
/// boundary, determines whether an action is supported.  A slash remains part
/// of the model portion when no colon is present, matching that behavior.
fn gemini_model_from_path(path: &str) -> Option<&str> {
    // The standalone helper also accepts a full legacy request path for
    // parity tests, while Axum supplies the wildcard capture itself here.
    let request = path
        .split_once("/models/")
        .map_or(path, |(_, request)| request)
        .trim_start_matches('/');
    let model = request.split(':').next()?;
    (!model.is_empty()).then_some(model)
}

/// Mirrors `GeminiChatRequest.IsStream`: either native stream action or
/// `alt=sse` requests an SSE response.  The latter matters for callers using
/// `generateContent` with a streaming query rather than the stream action.
fn gemini_is_streaming(uri: &axum::http::Uri) -> bool {
    query_value(uri.query(), "alt").as_deref() == Some("sse")
        || uri.path().contains("streamGenerateContent")
}

fn token_from_request(request: &Request, protocol: RelayProtocol) -> Option<String> {
    let headers = request.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token);
    match protocol {
        RelayProtocol::Anthropic => headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .or(bearer)
            .map(str::to_owned),
        RelayProtocol::Gemini => headers
            .get("x-goog-api-key")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| query_value(request.uri().query(), "key").filter(|value| !value.is_empty()))
            .or_else(|| bearer.map(str::to_owned)),
        RelayProtocol::OpenAi => headers
            .get("x-goog-api-key")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| query_value(request.uri().query(), "key").filter(|value| !value.is_empty()))
            .or_else(|| {
                headers
                    .get("x-api-key")
                    .and_then(|value| value.to_str().ok())
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| bearer.map(str::to_owned)),
    }
}

fn bearer_token(value: &str) -> Option<&str> {
    let mut words = value.split_ascii_whitespace();
    let scheme = words.next()?;
    let token = words.next()?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() && words.next().is_none())
        .then_some(token)
}

fn query_value(query: Option<&str>, name: &str) -> Option<String> {
    form_urlencoded::parse(query?.as_bytes())
        .find_map(|(key, value)| (key == name).then_some(value.into_owned()))
}

fn request_id(request: &Request) -> String {
    request
        .extensions()
        .get::<RequestContext>()
        .map_or_else(String::new, |context| context.request_id.clone())
}

fn outcome_for(error: &RelayFailure) -> RelayOutcome {
    match error {
        RelayFailure::Unauthorized | RelayFailure::ConcealedNotFound => RelayOutcome::Unauthorized,
        RelayFailure::NoChannel => RelayOutcome::NoChannel,
        RelayFailure::Upstream | RelayFailure::Provider { .. } | RelayFailure::Sse(_) => {
            RelayOutcome::UpstreamFailure
        }
    }
}

fn openai_failure(error: &RelayFailure, request_id: &str) -> Response {
    if matches!(error, RelayFailure::ConcealedNotFound) {
        return concealed_not_found();
    }
    let status = match error {
        RelayFailure::Unauthorized => StatusCode::UNAUTHORIZED,
        RelayFailure::ConcealedNotFound => StatusCode::NOT_FOUND,
        RelayFailure::NoChannel => StatusCode::SERVICE_UNAVAILABLE,
        RelayFailure::Upstream | RelayFailure::Provider { .. } | RelayFailure::Sse(_) => {
            StatusCode::BAD_GATEWAY
        }
    };
    let message = match error {
        RelayFailure::Unauthorized => "Invalid token",
        RelayFailure::ConcealedNotFound => "Not Found",
        RelayFailure::NoChannel => "relay request could not be completed",
        RelayFailure::Upstream | RelayFailure::Provider { .. } | RelayFailure::Sse(_) => {
            "relay request could not be completed"
        }
    };
    let body = if matches!(error, RelayFailure::Unauthorized) {
        json!({"error":{"code":"","message":format!("{message} (request id: {request_id})"),"type":"new_api_error"}})
    } else {
        json!({"error":{"message":format!("{message} (request id: {request_id})"),"type":"new_api_error","param":"","code":""}})
    };
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    add_compat_headers(&mut response, 0, request_id);
    response
}

fn openai_not_found(request_id: &str, path: &str) -> Response {
    let mut response = (
        StatusCode::NOT_FOUND,
        Json(json!({"error":{"message":format!("Invalid URL (DELETE {path})"),"type":"invalid_request_error","param":"","code":""}})),
    )
        .into_response();
    add_compat_headers(&mut response, 0, request_id);
    response
}

fn success(reply: UpstreamReply, channel_id: i64, request_id: &str) -> Response {
    let mut response = match reply {
        UpstreamReply::Json(body) => Json(body).into_response(),
        UpstreamReply::Sse(events) => {
            let mut framed = String::new();
            for event in events {
                if let Some(kind) = event.kind {
                    framed.push_str("event: ");
                    framed.push_str(&kind);
                    framed.push('\n');
                }
                framed.push_str("data: ");
                framed.push_str(&event.payload.to_string());
                framed.push_str("\n\n");
            }
            framed.push_str("data: [DONE]\n\n");
            let mut response = (StatusCode::OK, framed).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream; charset=utf-8"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            response
        }
        UpstreamReply::NativeSse(native) => {
            let NativeSseReply {
                status,
                body,
                content_type,
            } = *native;
            let mut response = (status, body).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                content_type.unwrap_or_else(|| {
                    HeaderValue::from_static("text/event-stream; charset=utf-8")
                }),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            response
        }
    };
    add_compat_headers(&mut response, channel_id, request_id);
    response
}

fn invalid_request(protocol: RelayProtocol, message: &str, request_id: &str) -> Response {
    let body = match protocol {
        RelayProtocol::OpenAi => {
            json!({"error":{"message":message,"type":"new_api_error","param":"","code":""}})
        }
        RelayProtocol::Anthropic => {
            json!({"type":"error","error":{"type":"invalid_request_error","message":message}})
        }
        RelayProtocol::Gemini => {
            json!({"error":{"code":400,"message":message,"status":"INVALID_ARGUMENT"}})
        }
    };
    error_response(StatusCode::BAD_REQUEST, body, request_id)
}

fn failure(protocol: RelayProtocol, error: &RelayFailure, request_id: &str) -> Response {
    if matches!(error, RelayFailure::ConcealedNotFound) {
        return concealed_not_found();
    }
    if matches!(protocol, RelayProtocol::Anthropic | RelayProtocol::Gemini)
        && matches!(error, RelayFailure::Unauthorized)
    {
        return openai_failure(error, request_id);
    }
    let (status, type_name, gemini_status) = match error {
        RelayFailure::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "UNAUTHENTICATED",
        ),
        RelayFailure::ConcealedNotFound => (
            StatusCode::NOT_FOUND,
            "authentication_error",
            "UNAUTHENTICATED",
        ),
        RelayFailure::NoChannel => (StatusCode::SERVICE_UNAVAILABLE, "api_error", "UNAVAILABLE"),
        RelayFailure::Upstream | RelayFailure::Sse(_) => {
            (StatusCode::BAD_GATEWAY, "api_error", "UNAVAILABLE")
        }
        RelayFailure::Provider { status, .. } => (*status, "api_error", "UNAVAILABLE"),
    };
    let body = match error {
        RelayFailure::Provider { body, .. } => body.clone(),
        _ => match protocol {
            RelayProtocol::OpenAi => {
                json!({"error":{"message":"relay request could not be completed","type":"new_api_error","param":"","code":""}})
            }
            RelayProtocol::Anthropic => {
                json!({"type":"error","error":{"type":type_name,"message":"relay request could not be completed"}})
            }
            RelayProtocol::Gemini => {
                json!({"error":{"code":status.as_u16(),"message":"relay request could not be completed","status":gemini_status}})
            }
        },
    };
    error_response(status, body, request_id)
}

fn concealed_not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"message":"Not Found"}))).into_response()
}

fn error_response(status: StatusCode, body: Value, request_id: &str) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    add_compat_headers(&mut response, 0, request_id);
    response
}

fn add_compat_headers(response: &mut Response, channel_id: i64, request_id: &str) {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert(REQUEST_ID, value);
    }
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert(LEGACY_REQUEST_ID, value);
    }
    if channel_id > 0 {
        if let Ok(value) = HeaderValue::from_str(&channel_id.to_string()) {
            response.headers_mut().insert(CHANNEL_ID, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RelayBackend, RelayChannel, RelayFailure, RelayHttpState, RelayIdentity, RelayOutcome,
        RelayProtocol, UpstreamReply, UpstreamRequest, failure, gemini_is_streaming,
        gemini_model_from_path, openai_failure, outcome_for, query_value, router,
        token_from_request,
    };
    use crate::RequestContext;
    use async_trait::async_trait;
    use axum::{
        body::to_bytes,
        http::{Request, StatusCode, Uri, header},
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    #[derive(Default)]
    struct RecordingBackend {
        tokens: Mutex<Vec<String>>,
        request_ids: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl RelayBackend for RecordingBackend {
        async fn authenticate(&self, token: &str) -> Result<RelayIdentity, RelayFailure> {
            self.tokens.lock().unwrap().push(token.to_owned());
            if token == "invalid" {
                Err(RelayFailure::ConcealedNotFound)
            } else {
                Ok(RelayIdentity {
                    token_id: "token-id".to_owned(),
                })
            }
        }

        async fn select_channel(
            &self,
            _identity: &RelayIdentity,
            _protocol: RelayProtocol,
            model: &str,
        ) -> Result<RelayChannel, RelayFailure> {
            Ok(RelayChannel {
                id: 7,
                upstream_model: model.to_owned(),
            })
        }

        async fn invoke(
            &self,
            _channel: &RelayChannel,
            request: UpstreamRequest,
        ) -> Result<UpstreamReply, RelayFailure> {
            self.request_ids
                .lock()
                .unwrap()
                .push(request.request_id.clone());
            Ok(UpstreamReply::Json(json!({
                "request_id": request.request_id,
            })))
        }

        async fn record_outcome(
            &self,
            _identity: Option<&RelayIdentity>,
            _channel: Option<&RelayChannel>,
            _outcome: RelayOutcome,
        ) {
        }
    }

    async fn response_body(response: axum::response::Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    fn request_with_context(uri: &str, body: &str) -> Request<axum::body::Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_owned()))
            .unwrap();
        request.extensions_mut().insert(RequestContext {
            request_id: "fixed-request-id".to_owned(),
            client_ip: None,
        });
        request
    }

    #[test]
    fn provider_credentials_should_override_bearer_in_go_order() {
        let anthropic = Request::builder()
            .header(header::AUTHORIZATION, "Bearer bearer")
            .header("x-api-key", "anthropic")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&anthropic, RelayProtocol::Anthropic).as_deref(),
            Some("anthropic")
        );

        let gemini = Request::builder()
            .uri("/v1beta/models/gemini:generateContent?key=query")
            .header(header::AUTHORIZATION, "Bearer bearer")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&gemini, RelayProtocol::Gemini).as_deref(),
            Some("query")
        );

        let gemini_header = Request::builder()
            .uri("/v1beta/models/gemini:generateContent?key=query")
            .header(header::AUTHORIZATION, "Bearer bearer")
            .header("x-goog-api-key", "google")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&gemini_header, RelayProtocol::Gemini).as_deref(),
            Some("google")
        );

        let openai_model_delete = Request::builder()
            .uri("/v1/models/model?key=query")
            .header(header::AUTHORIZATION, "Bearer bearer")
            .header("x-api-key", "anthropic")
            .header("x-goog-api-key", "google")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&openai_model_delete, RelayProtocol::OpenAi).as_deref(),
            Some("google")
        );

        let empty_overrides = Request::builder()
            .uri("/v1beta/models/gemini:generateContent?key=")
            .header(header::AUTHORIZATION, "Bearer bearer")
            .header("x-goog-api-key", "")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&empty_overrides, RelayProtocol::Gemini).as_deref(),
            Some("bearer")
        );
    }

    #[tokio::test]
    async fn fixed_request_context_id_should_match_body_response_and_upstream() {
        let backend = Arc::new(RecordingBackend::default());
        let mut request =
            request_with_context("/v1/messages", r#"{"model":"claude-test","stream":false}"#);
        request
            .headers_mut()
            .insert("x-api-key", "valid".parse().unwrap());
        let response = router(RelayHttpState::new(backend.clone()))
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["x-oneapi-request-id"],
            "fixed-request-id"
        );
        assert_eq!(
            response_body(response).await,
            r#"{"request_id":"fixed-request-id"}"#
        );
        assert_eq!(
            backend.request_ids.lock().unwrap().as_slice(),
            ["fixed-request-id"]
        );
    }

    #[tokio::test]
    async fn anthropic_missing_and_invalid_credentials_should_use_go_concealed_not_found() {
        let backend = Arc::new(RecordingBackend::default());
        for credential in [None, Some("invalid")] {
            let mut request = request_with_context("/v1/messages", r#"{"model":"claude-test"}"#);
            if let Some(token) = credential {
                request
                    .headers_mut()
                    .insert("x-api-key", token.parse().unwrap());
            }
            let response = router(RelayHttpState::new(backend.clone()))
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(response_body(response).await, r#"{"message":"Not Found"}"#);
        }
    }

    #[tokio::test]
    async fn gemini_missing_and_invalid_credentials_should_use_go_concealed_not_found() {
        let backend = Arc::new(RecordingBackend::default());
        for credential in [None, Some("invalid")] {
            let mut request = request_with_context(
                "/v1beta/models/gemini-test:generateContent",
                r#"{"contents":[]}"#,
            );
            if let Some(token) = credential {
                request
                    .headers_mut()
                    .insert("x-goog-api-key", token.parse().unwrap());
            }
            let response = router(RelayHttpState::new(backend.clone()))
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(response_body(response).await, r#"{"message":"Not Found"}"#);
        }
    }

    #[test]
    fn upstream_failure_should_be_recorded_as_an_upstream_outcome() {
        assert_eq!(
            outcome_for(&RelayFailure::Upstream),
            RelayOutcome::UpstreamFailure
        );
    }

    #[test]
    fn query_credentials_should_use_form_urlencoding_and_first_value() {
        assert_eq!(
            query_value(Some("key=encoded%2Ftoken%2Bvalue&key=second"), "key"),
            Some("encoded/token+value".to_owned())
        );
        assert_eq!(query_value(Some("key="), "key"), Some(String::new()));
        assert_eq!(
            query_value(Some("key=token+with+spaces"), "key"),
            Some("token with spaces".to_owned())
        );

        let request = Request::builder()
            .uri("/v1beta/models/gemini:generateContent?key=encoded%2Ftoken%2Bvalue&key=second")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&request, RelayProtocol::Gemini).as_deref(),
            Some("encoded/token+value")
        );
    }

    #[test]
    fn gemini_header_should_override_decoded_query_and_bearer() {
        let request = Request::builder()
            .uri("/v1beta/models/gemini:generateContent?key=query")
            .header(header::AUTHORIZATION, "Bearer bearer")
            .header("x-goog-api-key", "header")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            token_from_request(&request, RelayProtocol::Gemini).as_deref(),
            Some("header")
        );
    }

    #[tokio::test]
    async fn provider_failures_should_preserve_body_status_and_request_ids() {
        let body = json!({
            "error": {
                "code": "provider_code",
                "message": "provider message",
                "metadata": {"retryable": true}
            }
        });
        for protocol in [RelayProtocol::Anthropic, RelayProtocol::Gemini] {
            let error = RelayFailure::Provider {
                status: StatusCode::TOO_MANY_REQUESTS,
                body: body.clone(),
            };
            let response = failure(protocol, &error, "provider-request-id");
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/json; charset=utf-8"
            );
            assert_eq!(response.headers()["x-request-id"], "provider-request-id");
            assert_eq!(
                response.headers()["x-oneapi-request-id"],
                "provider-request-id"
            );
            assert_eq!(response_body(response).await, body.to_string());
            assert_eq!(outcome_for(&error), RelayOutcome::UpstreamFailure);
        }
    }

    #[test]
    fn gemini_wildcard_model_extraction_should_match_legacy_paths() {
        assert_eq!(
            gemini_model_from_path("/v1beta/models/gemini-2.5-pro:countTokens"),
            Some("gemini-2.5-pro")
        );
        assert_eq!(
            gemini_model_from_path("/v1/models/model/legacy-tail"),
            Some("model/legacy-tail")
        );
        assert_eq!(gemini_model_from_path("/v1beta/models/"), None);
    }

    #[test]
    fn gemini_stream_detection_should_accept_native_action_or_alt_sse() {
        let action: Uri = "/v1beta/models/gemini:streamGenerateContent"
            .parse()
            .unwrap();
        let query: Uri = "/v1beta/models/gemini:generateContent?alt=sse"
            .parse()
            .unwrap();
        let ordinary: Uri = "/v1beta/models/gemini:generateContent".parse().unwrap();

        assert!(gemini_is_streaming(&action));
        assert!(gemini_is_streaming(&query));
        assert!(!gemini_is_streaming(&ordinary));
    }

    #[tokio::test]
    async fn gemini_unauthorized_should_use_openai_invalid_token_envelope() {
        let response = failure(
            super::RelayProtocol::Gemini,
            &RelayFailure::Unauthorized,
            "request-123",
        );

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
        assert_eq!(response.headers()["x-request-id"], "request-123");
        assert_eq!(
            response_body(response).await,
            r#"{"error":{"code":"","message":"Invalid token (request id: request-123)","type":"new_api_error"}}"#
        );
    }

    #[tokio::test]
    async fn openai_unauthorized_should_omit_param_from_invalid_token_envelope() {
        let response = openai_failure(&RelayFailure::Unauthorized, "request-456");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
        assert_eq!(response.headers()["x-request-id"], "request-456");
        assert_eq!(
            response_body(response).await,
            r#"{"error":{"code":"","message":"Invalid token (request id: request-456)","type":"new_api_error"}}"#
        );
    }
}
