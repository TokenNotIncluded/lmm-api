//! Native client envelopes. All money, key authorization and networking stay in
//! the ordinary Core gateway; these headers never select upstream credentials.
use super::{ModelGateway, one_header};
use crate::relay::{ClientProtocol, RelayError};
use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, Path, RawQuery, State, rejection::ExtensionRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;

pub(super) struct GeminiOperation {
    pub(super) model: String,
    pub(super) stream: bool,
}

fn key_headers(mut headers: HeaderMap, client: ClientProtocol) -> Result<HeaderMap, RelayError> {
    let name = match client {
        ClientProtocol::Anthropic => "x-api-key",
        ClientProtocol::Gemini => "x-goog-api-key",
        _ => return Err(RelayError::Invalid("invalid native client")),
    };
    let auth = one_header(&headers, header::AUTHORIZATION.as_str())?;
    let key = one_header(&headers, name)?;
    let wrong_name = if name == "x-api-key" {
        "x-goog-api-key"
    } else {
        "x-api-key"
    };
    if headers.contains_key(wrong_name)
        || headers.contains_key("x-auth-session")
        || (auth.is_some() && key.is_some())
    {
        return Err(RelayError::Invalid("ambiguous native model credentials"));
    }
    if auth.is_some_and(|value| {
        value
            .split_once(' ')
            .is_some_and(|(_, token)| token.starts_with("lmm_at_"))
    }) || key.is_some_and(|key| key.starts_with("lmm_at_"))
    {
        return Err(RelayError::Forbidden);
    }
    if let Some(key) = key {
        if key.is_empty() || key.len() > 4096 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(RelayError::Unauthorized);
        }
        let mut value = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| RelayError::Unauthorized)?;
        value.set_sensitive(true);
        headers.remove(name);
        headers.insert(header::AUTHORIZATION, value);
    }
    Ok(headers)
}

pub(super) fn error_response(client: ClientProtocol, error: RelayError) -> Response {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
    let value = match client {
        ClientProtocol::Anthropic => {
            json!({"type":"error","error":{"type":match status.as_u16() { 400|413 => "invalid_request_error",401 => "authentication_error",403=>"permission_error",429=>"rate_limit_error",503=>"overloaded_error",_=>"api_error"},"message":error.to_string()}})
        }
        ClientProtocol::Gemini => {
            json!({"error":{"code":status.as_u16(),"message":error.to_string(),"status":match status.as_u16() {400=>"INVALID_ARGUMENT",401=>"UNAUTHENTICATED",403=>"PERMISSION_DENIED",413|429=>"RESOURCE_EXHAUSTED",503=>"UNAVAILABLE",504=>"DEADLINE_EXCEEDED",_=>"FAILED_PRECONDITION"}}})
        }
        _ => return super::public_error(error),
    };
    let mut response = (status, Json(value)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn messages(
    State(gateway): State<ModelGateway>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    body: Bytes,
) -> Response {
    let client = ClientProtocol::Anthropic;
    let headers = match (|| {
        if query.is_some()
            || one_header(&headers, "anthropic-version")?.is_some_and(|v| v != "2023-06-01")
            || headers.contains_key("anthropic-beta")
        {
            return Err(RelayError::Unsupported(
                "unsupported native Anthropic version or query",
            ));
        }
        key_headers(headers, client)
    })() {
        Ok(headers) => headers,
        Err(error) => return error_response(client, error),
    };
    super::dispatch(
        gateway,
        headers,
        body,
        client,
        peer.ok().map(|p| p.0.ip()),
        None,
    )
    .await
}

fn gemini_operation(action: &str, query: Option<&str>) -> Result<GeminiOperation, RelayError> {
    let (model, action) = action
        .rsplit_once(':')
        .ok_or(RelayError::Invalid("invalid Gemini model operation"))?;
    let stream = match action {
        "generateContent" => false,
        "streamGenerateContent" => true,
        _ => {
            return Err(RelayError::Unsupported(
                "unsupported Gemini model operation",
            ));
        }
    };
    if model.is_empty()
        || model.len() > 256
        || model.chars().any(char::is_control)
        || query.is_some_and(|q| !stream || q != "alt=sse")
    {
        return Err(RelayError::Invalid(
            "invalid Gemini model operation or query",
        ));
    }
    Ok(GeminiOperation {
        model: model.to_owned(),
        stream,
    })
}
pub(super) async fn gemini(
    State(gateway): State<ModelGateway>,
    Path(action): Path<String>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    body: Bytes,
) -> Response {
    let client = ClientProtocol::Gemini;
    let operation = match gemini_operation(&action, query.as_deref()) {
        Ok(v) => v,
        Err(error) => return error_response(client, error),
    };
    let headers = match key_headers(headers, client) {
        Ok(v) => v,
        Err(error) => return error_response(client, error),
    };
    super::dispatch(
        gateway,
        headers,
        body,
        client,
        peer.ok().map(|p| p.0.ip()),
        Some(operation),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_core_key_headers_are_unique_and_never_accept_oauth_or_session_aliases() {
        for client in [ClientProtocol::Anthropic, ClientProtocol::Gemini] {
            let name = if client == ClientProtocol::Anthropic {
                "x-api-key"
            } else {
                "x-goog-api-key"
            };
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static("synthetic-key"));
            let mapped = key_headers(headers.clone(), client).unwrap();
            assert_eq!(mapped[header::AUTHORIZATION], "Bearer synthetic-key");
            assert!(!mapped.contains_key(name));
            headers.append(name, HeaderValue::from_static("second"));
            assert!(key_headers(headers, client).is_err());
            let mut headers = mapped;
            headers.insert(name, HeaderValue::from_static("synthetic-key"));
            assert!(key_headers(headers, client).is_err());
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static("lmm_at_synthetic"));
            assert!(key_headers(headers, client).is_err());
        }
    }
    #[test]
    fn gemini_path_binds_model_stream_and_rejects_url_credentials_and_unknown_operations() {
        let operation =
            gemini_operation("model-version:streamGenerateContent", Some("alt=sse")).unwrap();
        assert_eq!(operation.model, "model-version");
        assert!(operation.stream);
        for (path, query) in [
            ("model:generateContent", Some("key=synthetic")),
            ("model:streamGenerateContent", Some("alt=sse&key=synthetic")),
            ("model:generateContent", Some("alt=sse")),
            ("model:countTokens", None),
            (":generateContent", None),
        ] {
            assert!(gemini_operation(path, query).is_err());
        }
    }
}
