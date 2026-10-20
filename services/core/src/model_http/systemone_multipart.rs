//! Native System One multipart framing. Contract/asset/image semantics belong
//! to `contract`; uploaded bytes are never interpreted as paths or fetched URLs.
use super::contract::{DangerZone, Request, Upload};
use crate::relay::{Phase, RelayError};
use axum::{
    body::{Body, to_bytes},
    extract::{DefaultBodyLimit, FromRequest, Multipart, multipart::Field},
    http::{HeaderMap, Request as HttpRequest, StatusCode, header},
};
use reqwest::multipart::{Form, Part};
use std::{collections::BTreeSet, time::Duration};

const BODY_BYTES: usize = 32 * 1024 * 1024;
const IMAGE_BYTES: usize = 16 * 1024 * 1024;
const READ_DEADLINE: Duration = Duration::from_secs(5);

fn unique_headers(headers: &HeaderMap) -> Result<(), RelayError> {
    for name in headers.keys() {
        if headers.get_all(name).iter().count() > 1 {
            return Err(RelayError::Invalid("duplicate System One multipart header"));
        }
    }
    Ok(())
}
fn field_type(field: &Field<'_>) -> Result<Option<String>, RelayError> {
    field
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|value| {
            value
                .to_str()
                .map(|value| value.trim().to_ascii_lowercase())
                .map_err(|_| RelayError::Invalid("invalid System One part content type"))
        })
        .transpose()
}
fn image_type(value: Option<&str>) -> Result<(), RelayError> {
    match value {
        None | Some("image/png" | "image/jpeg" | "image/jpg" | "application/octet-stream") => {
            Ok(())
        }
        _ => Err(RelayError::Invalid(
            "unsupported System One image part content type",
        )),
    }
}
// multer converts raw headers with HeaderMap::insert, which overwrites duplicate
// names. Inspect header names before that conversion; axum remains the actual
// multipart parser and owns all field/body syntax decisions.
fn boundary(headers: &HeaderMap) -> Result<Vec<u8>, RelayError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or(RelayError::Invalid(
            "missing System One multipart content type",
        ))?;
    let mut parameters = content_type.split(';');
    if !parameters
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("multipart/form-data"))
    {
        return Err(RelayError::Invalid(
            "expected System One multipart/form-data",
        ));
    }
    let mut boundary = None;
    for parameter in parameters {
        let (name, value) = parameter
            .trim()
            .split_once('=')
            .ok_or(RelayError::Invalid("invalid multipart parameter"))?;
        if !name.trim().eq_ignore_ascii_case("boundary") || boundary.is_some() {
            return Err(RelayError::Invalid(
                "unknown or duplicate multipart parameter",
            ));
        }
        let value = value.trim();
        let value = if value.starts_with('"') {
            value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .ok_or(RelayError::Invalid("invalid multipart boundary"))?
        } else {
            value
        };
        if value.is_empty()
            || value.len() > 70
            || value.ends_with(' ')
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(&b))
        {
            return Err(RelayError::Invalid("invalid multipart boundary"));
        }
        boundary = Some(value.as_bytes().to_vec());
    }
    boundary.ok_or(RelayError::Invalid("missing multipart boundary"))
}
fn unique_raw_part_headers(raw: &[u8], boundary: &[u8]) -> Result<(), RelayError> {
    let mut delimiter = b"--".to_vec();
    delimiter.extend_from_slice(boundary);
    let mut closing = delimiter.clone();
    closing.extend_from_slice(b"--");
    let mut in_headers = false;
    let mut names = BTreeSet::new();
    for line in raw.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line == delimiter.as_slice() || line == closing.as_slice() {
            if in_headers {
                return Err(RelayError::Invalid("incomplete multipart headers"));
            }
            in_headers = line != closing.as_slice();
            names.clear();
        } else if in_headers {
            if line.is_empty() {
                in_headers = false;
                continue;
            }
            let colon = line
                .iter()
                .position(|b| *b == b':')
                .ok_or(RelayError::Invalid("invalid multipart header"))?;
            let name = axum::http::HeaderName::from_bytes(&line[..colon])
                .map_err(|_| RelayError::Invalid("invalid multipart header name"))?;
            if !names.insert(name.as_str().to_owned()) {
                return Err(RelayError::Invalid("duplicate System One multipart header"));
            }
        }
    }
    Ok(())
}
async fn field_bytes(field: &mut Field<'_>, cap: usize) -> Result<Vec<u8>, RelayError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            RelayError::Limit("System One multipart body")
        } else {
            RelayError::Invalid("invalid System One multipart body")
        }
    })? {
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|size| size > cap)
        {
            return Err(RelayError::Limit("System One multipart field"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// The host authenticates and acquires its global admission permit first. The
/// complete wire body, including all multipart framing, has one 32 MiB limit.
pub(super) async fn parse(
    request: HttpRequest<Body>,
    flags: DangerZone,
) -> Result<Request, RelayError> {
    tokio::time::timeout(READ_DEADLINE, parse_inner(request, flags))
        .await
        .map_err(|_| RelayError::Timeout(Phase::Read))?
}
async fn parse_inner(request: HttpRequest<Body>, flags: DangerZone) -> Result<Request, RelayError> {
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_ENCODING,
        header::TRANSFER_ENCODING,
    ] {
        if request.headers().get_all(name).iter().count() > 1 {
            return Err(RelayError::Invalid("duplicate System One body header"));
        }
    }
    if let Some(encoding) = request.headers().get(header::CONTENT_ENCODING) {
        if encoding
            .to_str()
            .ok()
            .is_none_or(|encoding| !encoding.eq_ignore_ascii_case("identity"))
        {
            return Err(RelayError::Unsupported(
                "compressed System One multipart body",
            ));
        }
    }
    if let Some(length) = request.headers().get(header::CONTENT_LENGTH) {
        let length = length
            .to_str()
            .ok()
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or(RelayError::Invalid("invalid System One content length"))?;
        if length > BODY_BYTES as u64 {
            return Err(RelayError::Limit("System One multipart body"));
        }
    }
    let boundary = boundary(request.headers())?;
    let (parts, body) = request.into_parts();
    let raw = to_bytes(body, BODY_BYTES).await.map_err(|error| {
        if error.into_inner().is::<http_body_util::LengthLimitError>() {
            RelayError::Limit("System One multipart body")
        } else {
            RelayError::Invalid("incomplete System One multipart body")
        }
    })?;
    unique_raw_part_headers(&raw, &boundary)?;
    let mut request = HttpRequest::from_parts(parts, Body::from(raw));
    DefaultBodyLimit::max(BODY_BYTES).apply(&mut request);
    let mut multipart = Multipart::from_request(request, &())
        .await
        .map_err(|_| RelayError::Invalid("invalid System One multipart content type"))?;
    let mut names = BTreeSet::new();
    let mut json = None;
    let mut uploads = Vec::new();
    while let Some(mut field) = multipart.next_field().await.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            RelayError::Limit("System One multipart body")
        } else {
            RelayError::Invalid("invalid System One multipart body")
        }
    })? {
        unique_headers(field.headers())?;
        if field.headers().contains_key(header::CONTENT_ENCODING) {
            return Err(RelayError::Unsupported("encoded System One multipart part"));
        }
        let name = field
            .name()
            .filter(|name| !name.is_empty())
            .ok_or(RelayError::Invalid("missing System One part name"))?
            .to_owned();
        if !names.insert(name.clone()) {
            return Err(RelayError::Invalid("duplicate System One part name"));
        }
        let content_type = field_type(&field)?;
        if json.is_none() {
            if name != "request" || content_type.as_deref() != Some("application/json") {
                return Err(RelayError::Invalid(
                    "first System One part must be request application/json",
                ));
            }
            json = Some(field_bytes(&mut field, BODY_BYTES).await?);
        } else {
            if name == "request" {
                return Err(RelayError::Invalid("reserved System One part name"));
            }
            image_type(content_type.as_deref())?;
            let bytes = field_bytes(&mut field, IMAGE_BYTES).await?;
            if bytes.is_empty() {
                return Err(RelayError::Invalid("empty System One image part"));
            }
            uploads.push(Upload {
                part: name,
                bytes,
                content_type,
            });
        }
    }
    let json = json.ok_or(RelayError::Invalid("missing System One request part"))?;
    tokio::task::spawn_blocking(move || Request::parse_multipart_json(&json, flags, uploads))
        .await
        .map_err(|_| RelayError::Invalid("System One multipart parsing failed"))?
}

// `Form::compute_length` is private. Reserve a conservative upper bound for
// reqwest-generated boundary/header overhead and percent-escaped part names.
// The actual host request may additionally check its computed Content-Length.
fn outbound_budget(json: usize, uploads: &[Upload], boundary: usize) -> Result<(), RelayError> {
    let mut total = json
        .checked_add(boundary)
        .and_then(|n| n.checked_add(512))
        .ok_or(RelayError::Limit("System One outbound multipart body"))?;
    for upload in uploads {
        let header = upload
            .part
            .len()
            .checked_mul(3)
            .and_then(|n| n.checked_add(upload.content_type.as_ref().map_or(0, String::len)))
            .and_then(|n| n.checked_add(boundary))
            .and_then(|n| n.checked_add(256))
            .ok_or(RelayError::Limit("System One outbound multipart body"))?;
        total = total
            .checked_add(header)
            .and_then(|n| n.checked_add(upload.bytes.len()))
            .ok_or(RelayError::Limit("System One outbound multipart body"))?;
    }
    if total > BODY_BYTES {
        return Err(RelayError::Limit("System One outbound multipart body"));
    }
    Ok(())
}
/// Preserve the validated JSON and exact owned image bytes. The JSON request is
/// always first. Client filenames and filesystem paths never enter the builder.
pub(super) fn outbound(request: &Request) -> Result<Form, RelayError> {
    let json = serde_json::to_vec(request.wire())
        .map_err(|_| RelayError::Invalid("System One JSON encoding"))?;
    let form = Form::new();
    outbound_budget(json.len(), request.uploads(), form.boundary().len())?;
    let mut form = form.part(
        "request",
        Part::bytes(json)
            .mime_str("application/json")
            .map_err(|_| RelayError::Protocol("System One multipart encoding"))?,
    );
    for upload in request.uploads() {
        let mut part = Part::bytes(upload.bytes.clone()).file_name("image");
        if let Some(content_type) = &upload.content_type {
            image_type(Some(content_type))?;
            part = part
                .mime_str(content_type)
                .map_err(|_| RelayError::Protocol("System One multipart encoding"))?;
        }
        form = form.part(upload.part.clone(), part);
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn wire() -> String {
        json!({"model":"jev-latest","state":"hello","questions":{"q":{"type":"noul","instructions":"Does this say hello?"}}}).to_string()
    }
    fn json_part() -> Part {
        Part::text(wire()).mime_str("application/json").unwrap()
    }
    // Serialize an unsent request into the actual axum body extractor. This
    // exercises multipart bytes and headers without opening any socket.
    fn body(form: Form) -> HttpRequest<Body> {
        let mut request = reqwest::Client::new()
            .post("http://127.0.0.1:1/unsent-systemone-fixture")
            .multipart(form)
            .build()
            .unwrap();
        let headers = request.headers().clone();
        let body = request.body_mut().take().unwrap();
        let mut request = HttpRequest::new(Body::new(body));
        *request.headers_mut() = headers;
        request
    }
    #[tokio::test]
    async fn systemone_multipart_json_first_roundtrip_is_native() {
        let parsed = parse(
            body(Form::new().part("request", json_part())),
            DangerZone::default(),
        )
        .await
        .unwrap();
        let reparsed = parse(body(outbound(&parsed).unwrap()), DangerZone::default())
            .await
            .unwrap();
        assert_eq!(parsed.wire(), reparsed.wire());
        assert!(reparsed.uploads().is_empty());
    }
    #[tokio::test]
    async fn systemone_multipart_rejects_wrong_first_part_and_json_type() {
        for form in [
            Form::new().text("not-request", wire()),
            Form::new().text("request", wire()),
            Form::new().part(
                "request",
                Part::text(wire()).mime_str("text/plain").unwrap(),
            ),
        ] {
            assert!(matches!(
                parse(body(form), DangerZone::default()).await,
                Err(RelayError::Invalid(_))
            ));
        }
    }
    #[tokio::test]
    async fn systemone_multipart_rejects_duplicate_and_empty_names() {
        for form in [
            Form::new()
                .part("request", json_part())
                .part("request", json_part()),
            Form::new()
                .part("request", json_part())
                .text("pic", "bytes")
                .text("pic", "bytes"),
            Form::new().part("request", json_part()).text("", "bytes"),
        ] {
            assert!(matches!(
                parse(
                    body(form),
                    DangerZone {
                        images: true,
                        long_context: false
                    }
                )
                .await,
                Err(RelayError::Invalid(_))
            ));
        }
    }
    #[tokio::test]
    async fn systemone_multipart_rejects_invalid_mime_and_unused_files() {
        let bad_mime = Form::new()
            .part("request", json_part())
            .part("pic", Part::bytes(vec![1]).mime_str("text/html").unwrap());
        assert!(matches!(
            parse(
                body(bad_mime),
                DangerZone {
                    images: true,
                    long_context: false
                }
            )
            .await,
            Err(RelayError::Invalid(_))
        ));
        let unused = Form::new().part("request", json_part()).part(
            "pic",
            Part::bytes(vec![1])
                .mime_str("application/octet-stream")
                .unwrap(),
        );
        assert!(
            parse(
                body(unused),
                DangerZone {
                    images: true,
                    long_context: false
                }
            )
            .await
            .is_err()
        );
        for content_type in [
            None,
            Some("image/png"),
            Some("image/jpeg"),
            Some("image/jpg"),
            Some("application/octet-stream"),
        ] {
            image_type(content_type).unwrap();
        }
    }
    #[tokio::test]
    async fn systemone_multipart_wire_limit_includes_framing() {
        let request = body(
            Form::new().part(
                "request",
                Part::bytes(vec![b' '; BODY_BYTES])
                    .mime_str("application/json")
                    .unwrap(),
            ),
        );
        assert!(matches!(
            parse(request, DangerZone::default()).await,
            Err(RelayError::Limit(_))
        ));
        let mut raw_request = body(
            Form::new().part(
                "request",
                Part::bytes(vec![b' '; BODY_BYTES])
                    .mime_str("application/json")
                    .unwrap(),
            ),
        );
        raw_request.headers_mut().remove(header::CONTENT_LENGTH);
        assert!(matches!(
            parse(raw_request, DangerZone::default()).await,
            Err(RelayError::Limit(_))
        ));
        assert!(outbound_budget(BODY_BYTES, &[], 64).is_err());
    }
    #[tokio::test]
    async fn systemone_multipart_rejects_encoding_and_duplicate_headers() {
        let mut request = body(Form::new().part("request", json_part()));
        request
            .headers_mut()
            .insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        assert!(matches!(
            parse(request, DangerZone::default()).await,
            Err(RelayError::Unsupported(_))
        ));
        let mut request = body(Form::new().part("request", json_part()));
        request.headers_mut().append(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=other".parse().unwrap(),
        );
        assert!(matches!(
            parse(request, DangerZone::default()).await,
            Err(RelayError::Invalid(_))
        ));
        let raw = format!(
            "--fixture\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\nContent-Type: text/plain\r\n\r\n{}\r\n--fixture--\r\n",
            wire()
        );
        let request = HttpRequest::builder()
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=fixture",
            )
            .body(Body::from(raw))
            .unwrap();
        assert!(matches!(
            parse(request, DangerZone::default()).await,
            Err(RelayError::Invalid(_))
        ));
    }
}
