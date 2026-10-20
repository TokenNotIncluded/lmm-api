//! Public native-Key image adapter. Core's ordinary model authorization and the
//! image execution service retain identity, group, IP policy and funds authority.
use super::{
    ModelGateway, ModelIdentity,
    image_execution::{DrawingInput, Error, Receipt},
    image_provider::{ImageMediaType, ImageMultipartInput, ImageUpload},
    one_header, request_id,
};
use crate::{
    identity::{CredentialKind, IdentityError},
    relay::RelayError,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        ConnectInfo, DefaultBodyLimit, Multipart, Path, RawQuery, State,
        multipart::{Field, MultipartRejection},
        rejection::{BytesRejection, ExtensionRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

const METADATA_BYTES: usize = 64 * 1024;
const UPLOAD_BYTES: usize = 32 * 1024 * 1024;
const EDIT_BODY_BYTES: usize = UPLOAD_BYTES + METADATA_BYTES;
const OPERATION_HEADER: &str = "x-lmm-image-operation-id";

pub(crate) fn router(gateway: ModelGateway) -> Router {
    Router::new()
        .route(
            "/v1/images/generations",
            post(generations).layer(DefaultBodyLimit::max(METADATA_BYTES)),
        )
        .route(
            "/v1/images/edits",
            post(edits).layer(DefaultBodyLimit::max(EDIT_BODY_BYTES)),
        )
        .route("/core/v1/images/operations/{id}", get(query))
        .with_state(gateway)
}
struct Admission {
    actor: ModelIdentity,
    secret: String,
    groups: Vec<String>,
    request_key: String,
}
fn native_headers(headers: &HeaderMap) -> Result<(), RelayError> {
    if ["x-api-key", "x-goog-api-key", "x-auth-session"]
        .iter()
        .any(|name| headers.contains_key(*name))
    {
        return Err(RelayError::Invalid("ambiguous image credentials"));
    }
    let auth =
        one_header(headers, header::AUTHORIZATION.as_str())?.ok_or(RelayError::Unauthorized)?;
    let (scheme, secret) = auth.split_once(' ').ok_or(RelayError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(RelayError::Unauthorized);
    }
    if secret.starts_with("lmm_at_") {
        return Err(RelayError::Forbidden);
    }
    Ok(())
}
async fn native_identity(
    gateway: &ModelGateway,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<(ModelIdentity, String, Vec<String>), RelayError> {
    native_headers(headers)?;
    if let Some(honeypot) = &gateway.honeypot {
        match honeypot.models(headers, peer).await? {
            Some(_) => {
                return Err(RelayError::Unsupported(
                    "image requests are unavailable for this credential policy",
                ));
            }
            None => (),
        }
    }
    let (actor, secret, groups) = gateway.authenticate(headers).await?;
    if !matches!(&actor,ModelIdentity::Native(p) if p.credential_kind==CredentialKind::ApiKey) {
        return Err(RelayError::Forbidden);
    }
    Ok((actor, secret, groups))
}
async fn admission(
    gateway: &ModelGateway,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Admission, RelayError> {
    let (actor, secret, groups) = native_identity(gateway, headers, peer).await?;
    let key_id = actor.key_id().ok_or(RelayError::Forbidden)?;
    let request_key = request_id(key_id, headers)?;
    Ok(Admission {
        actor,
        secret,
        groups,
        request_key,
    })
}
async fn allowed_groups(
    gateway: &ModelGateway,
    admission: &Admission,
    model: &str,
    peer: Option<IpAddr>,
) -> Result<Vec<String>, RelayError> {
    let mut groups = Vec::new();
    for group in &admission.groups {
        match gateway
            .authorize_request(&admission.actor, &admission.secret, model, group, peer)
            .await
        {
            Ok(()) => groups.push(group.clone()),
            Err(IdentityError::Forbidden) => (),
            Err(e) => return Err(super::identity_error(e)),
        }
    }
    if groups.is_empty() {
        return Err(RelayError::Forbidden);
    }
    Ok(groups)
}
fn operation_id(key: &str) -> String {
    format!("image:{key}")
}
fn valid_operation(id: &str) -> bool {
    id.strip_prefix("image:").is_some_and(|key| {
        key.len() == 64
            && key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn finish(mut response: Response, id: Option<&str>) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Some(id) = id.filter(|id| valid_operation(id)) {
        if let Ok(value) = HeaderValue::from_str(id) {
            response.headers_mut().insert(OPERATION_HEADER, value);
        }
    }
    response
}
fn error_response(error: RelayError, id: Option<&str>) -> Response {
    let status = if error == RelayError::SettlementPending {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::from_u16(error.status()).unwrap_or(StatusCode::SERVICE_UNAVAILABLE)
    };
    let mut value = error.json();
    if let Some(id) = id.filter(|id| valid_operation(id)) {
        value["operation_id"] = json!(id);
    }
    finish((status, Json(value)).into_response(), id)
}
fn execution_error(error: Error) -> RelayError {
    match error {
        Error::Invalid => RelayError::Invalid("invalid image request"),
        Error::Unauthorized => RelayError::Unauthorized,
        Error::Forbidden => RelayError::Forbidden,
        Error::Missing => RelayError::Invalid("image operation not found"),
        Error::Conflict => RelayError::Conflict,
        Error::NoFunds => RelayError::NoFunds,
        Error::Pending => RelayError::SettlementPending,
        Error::Retry | Error::Storage => RelayError::BillingUnavailable,
    }
}
fn service_error(error: Error, id: &str) -> Response {
    if error == Error::Missing {
        return finish((StatusCode::NOT_FOUND,Json(json!({"error":{"type":"not_found","code":"not_found","message":"image operation not found"},"operation_id":id}))).into_response(),Some(id));
    }
    error_response(execution_error(error), Some(id))
}
fn completed(receipt: Receipt, query: bool) -> Response {
    let id = receipt.operation_id.clone();
    if receipt.state != "completed" || receipt.output.is_none() {
        // A nonterminal receipt never releases supplier output before settlement.
        return error_response(RelayError::SettlementPending, Some(&id));
    }
    if query {
        finish(Json(receipt).into_response(), Some(&id))
    } else {
        finish(
            Json(receipt.output.expect("checked completed output")).into_response(),
            Some(&id),
        )
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationInput {
    prompt: String,
    model: String,
    #[serde(default = "one")]
    n: u32,
    #[serde(default)]
    size: String,
    #[serde(default)]
    quality: String,
    #[serde(default = "format_default")]
    response_format: String,
}
fn one() -> u32 {
    1
}
// Historical main 3948a0a ImageRequest uses response_format,omitempty and
// GetAndValidOpenAIImageRequest does not assign a default. Preserve omission;
// the supplier chooses its documented default. Drawing separately pins b64.
fn format_default() -> String {
    String::new()
}
fn drawing(input: GenerationInput) -> Result<(DrawingInput, String), RelayError> {
    if input.prompt.is_empty()
        || input.prompt.len() > METADATA_BYTES
        || input.model.is_empty()
        || input.model.len() > 256
        || input.model.chars().any(char::is_control)
        || input.size.len() > 128
        || input.quality.len() > 128
        || input.n > 128
    {
        return Err(RelayError::Invalid("invalid image request fields"));
    }
    if !matches!(input.response_format.as_str(), "" | "url" | "b64_json") {
        return Err(RelayError::Unsupported("unsupported image response format"));
    }
    Ok((
        DrawingInput {
            prompt: input.prompt,
            model: input.model,
            group: String::new(),
            size: input.size,
            quality: input.quality,
            n: input.n.max(1),
        },
        input.response_format,
    ))
}
fn generation_input(bytes: &[u8]) -> Result<(DrawingInput, String), RelayError> {
    if bytes.len() > METADATA_BYTES {
        return Err(RelayError::Limit("image metadata"));
    }
    drawing(
        serde_json::from_slice(bytes)
            .map_err(|_| RelayError::Invalid("invalid or ambiguous image JSON"))?,
    )
}
async fn execute(
    gateway: ModelGateway,
    admission: Admission,
    input: DrawingInput,
    files: Option<ImageMultipartInput>,
    peer: Option<IpAddr>,
    response_format: String,
) -> Response {
    let id = operation_id(&admission.request_key);
    let groups = match allowed_groups(&gateway, &admission, &input.model, peer).await {
        Ok(v) => v,
        Err(e) => return error_response(e, Some(&id)),
    };
    let Some(service) = gateway.image_service() else {
        return error_response(RelayError::RouteUnavailable, Some(&id));
    };
    match service
        .execute_api_with_response_format(
            &admission.secret,
            &admission.request_key,
            &groups,
            input,
            files,
            peer,
            &response_format,
        )
        .await
    {
        Ok(receipt) => completed(receipt, false),
        Err(e) => service_error(e, &id),
    }
}
async fn generations(
    State(gateway): State<ModelGateway>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let peer = peer.ok().map(|p| p.0.ip());
    let admission = match admission(&gateway, &headers, peer).await {
        Ok(v) => v,
        Err(e) => return error_response(e, None),
    };
    let id = operation_id(&admission.request_key);
    if query.is_some() {
        return error_response(
            RelayError::Invalid("image query parameters are unsupported"),
            Some(&id),
        );
    }
    let content = match one_header(&headers, header::CONTENT_TYPE.as_str()) {
        Ok(Some(v)) => v,
        _ => {
            return error_response(
                RelayError::Invalid("image JSON content type required"),
                Some(&id),
            );
        }
    };
    if !content
        .split(';')
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    {
        return error_response(
            RelayError::Invalid("image JSON content type required"),
            Some(&id),
        );
    }
    let body = match body {
        Ok(v) => v,
        Err(_) => return error_response(RelayError::Limit("image request body"), Some(&id)),
    };
    let (input, response_format) = match generation_input(&body) {
        Ok(v) => v,
        Err(e) => return error_response(e, Some(&id)),
    };
    execute(gateway, admission, input, None, peer, response_format).await
}
async fn field_bytes(field: &mut Field<'_>, cap: usize) -> Result<Vec<u8>, RelayError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|_| RelayError::Invalid("invalid image multipart body"))?
    {
        if bytes.len().checked_add(chunk.len()).is_none_or(|n| n > cap) {
            return Err(RelayError::Limit("image multipart field"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn media_type(value: Option<&str>) -> Result<ImageMediaType, RelayError> {
    match value {
        Some("image/png") => Ok(ImageMediaType::Png),
        Some("image/jpeg") => Ok(ImageMediaType::Jpeg),
        Some("image/webp") => Ok(ImageMediaType::Webp),
        _ => Err(RelayError::Unsupported("unsupported image upload MIME")),
    }
}
async fn multipart_input(
    mut multipart: Multipart,
) -> Result<(DrawingInput, ImageMultipartInput, String), RelayError> {
    let mut metadata = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut metadata_bytes = 0usize;
    let mut upload_bytes = 0usize;
    let mut images = Vec::new();
    let mut mask = None;
    let mut image_name = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| RelayError::Invalid("invalid image multipart body"))?
    {
        let name = field
            .name()
            .ok_or(RelayError::Invalid("unnamed image multipart field"))?
            .to_owned();
        if name.len() > 32 || field.file_name().is_some_and(|name| name.len() > 255) {
            return Err(RelayError::Limit("image multipart headers"));
        }
        let mut header_bytes = 0usize;
        for (key, value) in field.headers() {
            if key.as_str() != header::CONTENT_DISPOSITION.as_str()
                && key.as_str() != header::CONTENT_TYPE.as_str()
            {
                return Err(RelayError::Unsupported(
                    "unsupported image multipart header",
                ));
            }
            header_bytes = header_bytes
                .checked_add(key.as_str().len())
                .and_then(|n| n.checked_add(value.as_bytes().len()))
                .ok_or(RelayError::Limit("image multipart headers"))?;
        }
        // Reserve 8KiB for delimiters/CRLF across the at most 23 allowed parts.
        metadata_bytes = metadata_bytes
            .checked_add(header_bytes)
            .and_then(|n| n.checked_add(name.len()))
            .filter(|n| *n <= METADATA_BYTES - 8192)
            .ok_or(RelayError::Limit("image multipart metadata"))?;
        match name.as_str() {
            "image" | "image[]" | "mask" => {
                if name == "mask" && mask.is_some() {
                    return Err(RelayError::Invalid("duplicate image mask"));
                }
                if name != "mask" {
                    if images.len() >= 16
                        || image_name.as_ref().is_some_and(|old: &String| old != &name)
                        || (name == "image" && !images.is_empty())
                    {
                        return Err(RelayError::Invalid("ambiguous image upload list"));
                    }
                    image_name = Some(name.clone());
                }
                let media_type = media_type(field.content_type())?;
                let bytes =
                    field_bytes(&mut field, UPLOAD_BYTES.saturating_sub(upload_bytes)).await?;
                if bytes.is_empty() {
                    return Err(RelayError::Invalid("empty image upload"));
                }
                upload_bytes = upload_bytes
                    .checked_add(bytes.len())
                    .filter(|n| *n <= UPLOAD_BYTES)
                    .ok_or(RelayError::Limit("image uploads"))?;
                let upload = ImageUpload { bytes, media_type };
                if name == "mask" {
                    mask = Some(upload)
                } else {
                    images.push(upload)
                }
            }
            "prompt" | "model" | "n" | "size" | "quality" | "response_format" => {
                if !seen.insert(name.clone()) || field.file_name().is_some() {
                    return Err(RelayError::Invalid("duplicate or invalid image metadata"));
                }
                let bytes = field_bytes(
                    &mut field,
                    (METADATA_BYTES - 8192).saturating_sub(metadata_bytes),
                )
                .await?;
                metadata_bytes = metadata_bytes
                    .checked_add(bytes.len())
                    .filter(|n| *n <= METADATA_BYTES - 8192)
                    .ok_or(RelayError::Limit("image metadata"))?;
                metadata.insert(
                    name,
                    String::from_utf8(bytes)
                        .map_err(|_| RelayError::Invalid("invalid image metadata encoding"))?,
                );
            }
            _ => return Err(RelayError::Unsupported("unsupported image multipart field")),
        }
    }
    if images.is_empty() {
        return Err(RelayError::Invalid("image edit upload required"));
    }
    let (input, response_format) = multipart_metadata(metadata)?;
    Ok((input, ImageMultipartInput { images, mask }, response_format))
}
fn multipart_metadata(
    mut fields: BTreeMap<String, String>,
) -> Result<(DrawingInput, String), RelayError> {
    let prompt = fields
        .remove("prompt")
        .ok_or(RelayError::Invalid("image prompt required"))?;
    let model = fields
        .remove("model")
        .ok_or(RelayError::Invalid("image model required"))?;
    let n = fields
        .remove("n")
        .map(|n| {
            n.parse::<u32>()
                .map_err(|_| RelayError::Invalid("invalid image count"))
        })
        .transpose()?
        .unwrap_or(1);
    let input = GenerationInput {
        prompt,
        model,
        n,
        size: fields.remove("size").unwrap_or_default(),
        quality: fields.remove("quality").unwrap_or_default(),
        response_format: fields
            .remove("response_format")
            .unwrap_or_else(format_default),
    };
    if !fields.is_empty() {
        return Err(RelayError::Unsupported("unsupported image metadata"));
    }
    drawing(input)
}
async fn edits(
    State(gateway): State<ModelGateway>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let peer = peer.ok().map(|p| p.0.ip());
    let admission = match admission(&gateway, &headers, peer).await {
        Ok(v) => v,
        Err(e) => return error_response(e, None),
    };
    let id = operation_id(&admission.request_key);
    if query.is_some() {
        return error_response(
            RelayError::Invalid("image query parameters are unsupported"),
            Some(&id),
        );
    }
    if one_header(&headers, header::CONTENT_TYPE.as_str())
        .ok()
        .flatten()
        .is_none()
    {
        return error_response(
            RelayError::Invalid("image multipart content type required"),
            Some(&id),
        );
    }
    let multipart = match multipart {
        Ok(v) => v,
        Err(_) => {
            return error_response(
                RelayError::Invalid("invalid image multipart body"),
                Some(&id),
            );
        }
    };
    let (input, files, response_format) = match multipart_input(multipart).await {
        Ok(v) => v,
        Err(e) => return error_response(e, Some(&id)),
    };
    execute(
        gateway,
        admission,
        input,
        Some(files),
        peer,
        response_format,
    )
    .await
}
async fn query(
    State(gateway): State<ModelGateway>,
    Path(id): Path<String>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Response {
    let peer = peer.ok().map(|p| p.0.ip());
    let (_, secret, _) = match native_identity(&gateway, &headers, peer).await {
        Ok(identity) => identity,
        Err(error) => return error_response(error, None),
    };
    if !valid_operation(&id) {
        return error_response(RelayError::Invalid("invalid image operation ID"), None);
    }
    if query.is_some() {
        return error_response(
            RelayError::Invalid("image query parameters are unsupported"),
            Some(&id),
        );
    }
    let Some(service) = gateway.image_service() else {
        return error_response(RelayError::RouteUnavailable, Some(&id));
    };
    match service.query_api_with_peer(&secret, &id, peer).await {
        Ok(receipt) => completed(receipt, true),
        Err(e) => service_error(e, &id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Build an unsent reqwest multipart body to exercise the actual axum
    // extractor. No socket, supplier credential or paid request is involved.
    async fn parse_form(
        form: reqwest::multipart::Form,
    ) -> Result<(DrawingInput, ImageMultipartInput, String), RelayError> {
        use axum::extract::FromRequest;
        let mut request = reqwest::Client::new()
            .post("http://127.0.0.1:1/unsent-fixture")
            .multipart(form)
            .build()
            .unwrap();
        let headers = request.headers().clone();
        let body = request.body_mut().take().unwrap();
        let mut request = axum::http::Request::new(axum::body::Body::new(body));
        *request.headers_mut() = headers;
        let multipart = Multipart::from_request(request, &())
            .await
            .map_err(|_| RelayError::Invalid("fixture multipart"))?;
        multipart_input(multipart).await
    }
    fn form() -> reqwest::multipart::Form {
        reqwest::multipart::Form::new()
            .text("prompt", "draw")
            .text("model", "m")
    }
    fn file() -> reqwest::multipart::Part {
        reqwest::multipart::Part::bytes(vec![1, 2, 3])
            .file_name("ignored.png")
            .mime_str("image/png")
            .unwrap()
    }
    #[tokio::test]
    async fn image_api_multipart_extractor_preserves_images_and_rejects_ambiguous_parts() {
        let (input, files, format) = parse_form(
            form()
                .text("n", "2")
                .part("image[]", file())
                .part("image[]", file())
                .part("mask", file()),
        )
        .await
        .unwrap();
        assert_eq!(input.n, 2);
        assert!(format.is_empty());
        assert!(input.group.is_empty());
        assert_eq!(files.images.len(), 2);
        assert!(files.mask.is_some());
        for form in [
            form().text("model", "second").part("image", file()),
            form().part("image", file()).part("image[]", file()),
            form()
                .part("image", file())
                .part("mask", file())
                .part("mask", file()),
            form().text("payer", "other").part("image", file()),
        ] {
            assert!(parse_form(form).await.is_err());
        }
    }
    #[tokio::test]
    async fn image_api_multipart_extractor_caps_metadata_and_file_count() {
        assert!(
            parse_form(
                form()
                    .text("quality", "x".repeat(METADATA_BYTES))
                    .part("image", file())
            )
            .await
            .is_err()
        );
        let mut over = form();
        for _ in 0..17 {
            over = over.part("image[]", file());
        }
        assert!(parse_form(over).await.is_err());
        let bad = reqwest::multipart::Part::bytes(vec![1])
            .file_name("x.bin")
            .mime_str("application/octet-stream")
            .unwrap();
        assert!(parse_form(form().part("image", bad)).await.is_err());
    }
    #[test]
    fn image_api_json_rejects_ambiguous_unknown_authority_and_non_b64() {
        for body in [
            r#"{"prompt":"a","model":"m","model":"other"}"#,
            r#"{"prompt":"a","model":"m","group":"free"}"#,
            r#"{"prompt":"a","model":"m","payer":1}"#,
            r#"{"prompt":"a","model":"m","url":"https://evil.invalid"}"#,
            r#"{"prompt":"a","model":"m","response_format":"unsupported"}"#,
            r#"{"prompt":"a","model":"m","quality":null}"#,
        ] {
            assert!(generation_input(body.as_bytes()).is_err());
        }
        let (parsed, format) = generation_input(br#"{"prompt":"a","model":"m","n":0}"#).unwrap();
        assert!(format.is_empty());
        assert_eq!(parsed.n, 1);
        assert!(parsed.group.is_empty());
    }
    #[test]
    fn image_api_response_format_preserves_main_omission_and_explicit_choices() {
        let (input, format) = generation_input(br#"{"model":"m","prompt":"draw"}"#).unwrap();
        assert!(format.is_empty());
        assert_eq!(input.n, 1);
        for format in ["url", "b64_json", ""] {
            let body = json!({"model":"m","prompt":"draw","response_format":format});
            let (_, actual) = generation_input(&serde_json::to_vec(&body).unwrap()).unwrap();
            assert_eq!(actual, format);
            let (_, actual) = multipart_metadata(BTreeMap::from([
                ("model".into(), "m".into()),
                ("prompt".into(), "draw".into()),
                ("response_format".into(), format.into()),
            ]))
            .unwrap();
            assert_eq!(actual, format);
        }
    }
    #[test]
    fn image_api_header_policy_rejects_oauth_aliases_and_duplicates() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            "Bearer lmm_at_not-an-image-key".parse().unwrap(),
        );
        assert_eq!(native_headers(&h), Err(RelayError::Forbidden));
        h.insert(header::AUTHORIZATION, "Bearer private-key".parse().unwrap());
        assert!(native_headers(&h).is_ok());
        h.insert("x-auth-session", "other".parse().unwrap());
        assert!(native_headers(&h).is_err());
        h.remove("x-auth-session");
        h.append(header::AUTHORIZATION, "Bearer second".parse().unwrap());
        assert!(native_headers(&h).is_err());
    }
    #[test]
    fn image_api_operation_id_is_native_key_scoped_and_preserved_on_pending() {
        let mut h = HeaderMap::new();
        h.insert("idempotency-key", "retry-same-image".parse().unwrap());
        let first = request_id(1, &h).unwrap();
        assert_eq!(first, request_id(1, &h).unwrap());
        assert_ne!(first, request_id(2, &h).unwrap());
        let id = operation_id(&first);
        let response = error_response(RelayError::SettlementPending, Some(&id));
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()[OPERATION_HEADER], id);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    #[test]
    fn image_api_multipart_metadata_and_types_are_strict() {
        let make = || {
            BTreeMap::from([
                ("prompt".into(), "draw".into()),
                ("model".into(), "m".into()),
            ])
        };
        assert!(multipart_metadata(make()).unwrap().0.group.is_empty());
        let mut input = make();
        input.insert("confirmed".into(), "true".into());
        assert!(multipart_metadata(input).is_err());
        let mut input = make();
        input.insert("n".into(), "-1".into());
        assert!(multipart_metadata(input).is_err());
        assert!(media_type(Some("application/octet-stream")).is_err());
        assert!(media_type(None).is_err());
        assert!(media_type(Some("image/png")).is_ok());
    }
    #[tokio::test]
    async fn image_api_output_is_not_published_from_nonterminal_receipt() {
        let id = operation_id(&"a".repeat(64));
        let receipt = Receipt {
            expires_at: None,
            operation_id: id.clone(),
            state: "billing_pending".into(),
            preview_digest: "digest".into(),
            reservation_credits: 100,
            charged_credits: None,
            output: Some(json!({"data":["must-not-publish"]})),
        };
        let response = completed(receipt, false);
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()[OPERATION_HEADER], id);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["operation_id"], id);
        assert!(
            !std::str::from_utf8(&body)
                .unwrap()
                .contains("must-not-publish")
        );
        assert!(!valid_operation("image:untrusted\r\nheader"));
    }
}
