//! Server-owned image HTTP boundary. This module observes results, never settles funds.
//! Reference: openai/openai-python 0e5794dc48b8f8d51f6047f8a0b1814eb1e65850
//! (Apache-2.0). Independently implemented; no upstream source copied.
use super::image_price::{ImageEndpoint, ImageQuote, ImageQuoteRequest};
use crate::relay::{
    Limits, RelayError, honeypot_network,
    provider_network::{ProviderConnection, ProviderTransport},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Url,
    header::{self, HeaderValue},
    multipart::{Form, Part},
};
use serde::{
    Deserialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use tokio::time::{Instant, timeout, timeout_at};

const IMAGE_BYTES: usize = 32 * 1024 * 1024;
const WRAPPER_BYTES: usize = 64 * 1024;
const RESPONSE_BYTES: usize = IMAGE_BYTES + WRAPPER_BYTES;

// Deliberately neither Debug nor Serialize/Deserialize: this value is created
// only by the installation owner, never by a model HTTP request or Go payload.
pub(crate) struct ImageProvider {
    upstream_id: u64,
    vendor: String,
    upstream_model: String,
    endpoint: Url,
    authorization: HeaderValue,
    secret: String,
    transport: ProviderTransport,
    limits: Limits,
}

/// No Clone: a prepared call may be dispatched exactly once by ownership.
/// Construct after quote binding, before the host's durable dispatch marker.
pub(crate) struct PreparedImageCall {
    connection: ProviderConnection,
    request: reqwest::Request,
    secret: String,
    requested_n: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    // Tests read the frozen quote; dispatch consumes the request.
    quote: ImageQuote,
    limits: Limits,
    deadline: Instant,
}

/// Core-parsed observation only. The host independently checks it against the
/// immutable quote and its durable dispatch record before recording any charge.
pub(crate) struct ImageObservation {
    pub(crate) output_images: u32,
    pub(crate) provider_receipt_sha256: [u8; 32],
    /// Sanitized complete native response, not an untrusted supplier error body.
    pub(crate) data: Value,
}

/// Owned upload bytes only: no filesystem paths, URLs, caller filenames,
/// arbitrary MIME labels, credentials, or multipart field names.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ImageMediaType {
    Png,
    Jpeg,
    Webp,
}
pub(crate) struct ImageUpload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) media_type: ImageMediaType,
}
pub(crate) struct ImageMultipartInput {
    pub(crate) images: Vec<ImageUpload>,
    pub(crate) mask: Option<ImageUpload>,
}

impl ImageProvider {
    #[cfg_attr(not(test), allow(dead_code))] // Tests construct a provider; runtime uses the local-http entry.
    pub(crate) fn new(
        upstream_id: u64,
        vendor: &str,
        upstream_model: &str,
        base_url: &str,
        secret: &str,
        limits: &Limits,
    ) -> Result<Self, RelayError> {
        Self::construct(
            upstream_id,
            vendor,
            upstream_model,
            base_url,
            secret,
            limits,
            false,
        )
    }
    /// Explicit installation-owned isolated mode: numeric loopback only,
    /// never DNS localhost or other private network addresses.
    pub(crate) fn new_with_local_http(
        upstream_id: u64,
        vendor: &str,
        upstream_model: &str,
        base_url: &str,
        secret: &str,
        limits: &Limits,
        local_http: bool,
    ) -> Result<Self, RelayError> {
        Self::construct(
            upstream_id,
            vendor,
            upstream_model,
            base_url,
            secret,
            limits,
            local_http,
        )
    }
    fn construct(
        upstream_id: u64,
        vendor: &str,
        upstream_model: &str,
        base_url: &str,
        secret: &str,
        limits: &Limits,
        local: bool,
    ) -> Result<Self, RelayError> {
        limits.validate()?;
        if upstream_id == 0
            || vendor.is_empty()
            || vendor.len() > 128
            || upstream_model.is_empty()
            || upstream_model.len() > 256
            || secret.is_empty()
            || secret.len() > 4096
            || secret.chars().any(char::is_control)
        {
            return Err(RelayError::Invalid("invalid image provider configuration"));
        }
        let mut endpoint = honeypot_network::validate_base_url(base_url, local)
            .map_err(|_| RelayError::Invalid("invalid image provider endpoint"))?;
        if local && !honeypot_network::literal_ip(&endpoint).is_some_and(|ip| ip.is_loopback()) {
            return Err(RelayError::Invalid(
                "isolated image provider requires numeric loopback",
            ));
        }
        let path = format!(
            "{}/images/generations",
            endpoint.path().trim_end_matches('/')
        );
        endpoint.set_path(&path);
        let mut authorization = HeaderValue::from_str(&format!("Bearer {secret}"))
            .map_err(|_| RelayError::Invalid("invalid image provider credential"))?;
        authorization.set_sensitive(true);
        Ok(Self {
            upstream_id,
            vendor: vendor.into(),
            upstream_model: upstream_model.into(),
            endpoint,
            authorization,
            secret: secret.into(),
            transport: ProviderTransport::new(limits.max_in_flight),
            limits: limits.clone(),
        })
    }
    fn validate_binding(
        &self,
        quote: &ImageQuote,
        prompt: &str,
        endpoint: ImageEndpoint,
    ) -> Result<(), RelayError> {
        let b = quote.binding();
        if b.upstream_id != self.upstream_id
            || b.vendor != self.vendor
            || b.upstream_model != self.upstream_model
            || b.endpoint != endpoint
        {
            return Err(RelayError::RouteUnavailable);
        }
        if prompt.is_empty()
            || prompt.len() > self.limits.request_bytes
            || prompt.contains(&self.secret)
        {
            return Err(RelayError::Invalid("invalid image prompt"));
        }
        if b.n == 0 || b.n > 128 {
            return Err(RelayError::Invalid("invalid image count"));
        }
        Ok(())
    }
    pub(crate) async fn prepare(
        &self,
        quote: &ImageQuote,
        prompt: &str,
    ) -> Result<PreparedImageCall, RelayError> {
        self.prepare_with_response_format(quote, prompt, "b64_json")
            .await
    }
    pub(crate) async fn prepare_with_response_format(
        &self,
        quote: &ImageQuote,
        prompt: &str,
        response_format: &str,
    ) -> Result<PreparedImageCall, RelayError> {
        validate_quote_response_format(quote, response_format)?;
        let deadline = Instant::now() + self.limits.total_timeout;
        self.validate_binding(quote, prompt, ImageEndpoint::Generations)?;
        let b = quote.binding();
        let mut payload = json!({"prompt":prompt,"model":b.upstream_model,"n":b.n});
        if !response_format.is_empty() {
            payload["response_format"] = json!(response_format);
        }
        if !b.size.is_empty() {
            payload["size"] = json!(b.size);
        }
        if !b.quality.is_empty() {
            payload["quality"] = json!(b.quality);
        }
        let body = serde_json::to_vec(&payload)
            .map_err(|_| RelayError::Invalid("invalid image request"))?;
        if body.len() > self.limits.request_bytes {
            return Err(RelayError::Limit("image request body"));
        }
        self.prepare_call(
            quote,
            self.endpoint.clone(),
            PreparedBody::Json(body),
            deadline,
        )
        .await
    }
    pub(crate) async fn prepare_edits(
        &self,
        quote: &ImageQuote,
        prompt: &str,
        input: ImageMultipartInput,
    ) -> Result<PreparedImageCall, RelayError> {
        self.prepare_edits_with_response_format(quote, prompt, input, "b64_json")
            .await
    }
    pub(crate) async fn prepare_edits_with_response_format(
        &self,
        quote: &ImageQuote,
        prompt: &str,
        input: ImageMultipartInput,
        response_format: &str,
    ) -> Result<PreparedImageCall, RelayError> {
        validate_quote_response_format(quote, response_format)?;
        let deadline = Instant::now() + self.limits.total_timeout;
        self.validate_binding(quote, prompt, ImageEndpoint::Edits)?;
        let b = quote.binding();
        let form = edits_form_with_response_format(
            quote,
            prompt,
            input,
            &self.secret,
            self.limits.request_bytes,
            response_format,
        )?;
        let mut endpoint = self.endpoint.clone();
        let prefix = endpoint
            .path()
            .strip_suffix("/images/generations")
            .ok_or(RelayError::RouteUnavailable)?;
        let path = format!("{prefix}/images/edits");
        endpoint.set_path(&path);
        if b.endpoint != ImageEndpoint::Edits {
            return Err(RelayError::RouteUnavailable);
        }
        self.prepare_call(quote, endpoint, PreparedBody::Multipart(form), deadline)
            .await
    }
    async fn prepare_call(
        &self,
        quote: &ImageQuote,
        endpoint: Url,
        body: PreparedBody,
        deadline: Instant,
    ) -> Result<PreparedImageCall, RelayError> {
        if Instant::now() >= deadline {
            return Err(RelayError::RouteUnavailable);
        }
        let connection = timeout_at(
            deadline,
            timeout(
                self.limits.header_timeout,
                self.transport.prepare(&endpoint, &self.limits),
            ),
        )
        .await
        .map_err(|_| RelayError::RouteUnavailable)?
        .map_err(|_| RelayError::RouteUnavailable)?
        .map_err(|_| RelayError::RouteUnavailable)?;
        let builder = connection
            .client()
            .post(endpoint)
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::ACCEPT, "application/json")
            .timeout(self.limits.total_timeout);
        let builder = match body {
            PreparedBody::Json(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(body),
            PreparedBody::Multipart(form) => builder.multipart(form),
        };
        let request = builder
            .build()
            .map_err(|_| RelayError::Invalid("invalid image request"))?;
        if quote.binding().endpoint == ImageEndpoint::Edits {
            let length = request
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(RelayError::Invalid("unknown image multipart length"))?;
            if length > RESPONSE_BYTES as u64 {
                return Err(RelayError::Limit("image multipart request"));
            }
        }
        Ok(PreparedImageCall {
            connection,
            request,
            secret: self.secret.clone(),
            requested_n: quote.binding().n,
            quote: quote.clone(),
            limits: self.limits.clone(),
            deadline,
        })
    }
}
impl PreparedImageCall {
    /// Full frozen identity including billing group, semantic model,
    /// operation and dimensions for the host's durable dispatch marker.
    #[cfg_attr(not(test), allow(dead_code))] // Tests compare the frozen binding.
    pub(crate) fn binding(&self) -> &ImageQuoteRequest {
        self.quote.binding()
    }
    #[cfg_attr(not(test), allow(dead_code))] // Tests compare the frozen quote.
    pub(crate) fn quote(&self) -> &ImageQuote {
        &self.quote
    }
    /// The host must durably mark dispatch before calling this consume-once
    /// method. Once called, all ambiguity retains the hold, including network
    /// failures, HTTP rejection, redirects, timeouts and invalid payloads.
    pub(crate) async fn dispatch(self) -> Result<ImageObservation, RelayError> {
        let deadline = self.deadline;
        timeout_at(deadline, self.dispatch_inner())
            .await
            .map_err(|_| RelayError::SettlementPending)?
    }
    async fn dispatch_inner(self) -> Result<ImageObservation, RelayError> {
        let mut response = timeout(
            self.limits.header_timeout.min(self.limits.write_timeout),
            self.connection.client().execute(self.request),
        )
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)?;
        if !self.connection.matches_peer(response.remote_addr()) || !response.status().is_success()
        {
            return Err(RelayError::SettlementPending);
        }
        if response.headers().get(header::CONTENT_ENCODING).is_some() {
            return Err(RelayError::SettlementPending);
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type
            .split(';')
            .next()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))
        {
            return Err(RelayError::SettlementPending);
        }
        let cap = RESPONSE_BYTES.min(self.limits.upstream_bytes);
        if response.content_length().is_some_and(|n| n > cap as u64) {
            return Err(RelayError::SettlementPending);
        }
        let mut raw = Vec::new();
        while let Some(chunk) = timeout(self.limits.read_timeout, response.chunk())
            .await
            .map_err(|_| RelayError::SettlementPending)?
            .map_err(|_| RelayError::SettlementPending)?
        {
            if chunk.len() > self.limits.transport_chunk_bytes
                || raw.len().checked_add(chunk.len()).is_none_or(|n| n > cap)
            {
                return Err(RelayError::SettlementPending);
            }
            raw.extend_from_slice(&chunk);
        }
        observe(&raw, self.requested_n, &self.secret)
    }
}

fn validate_response_format(format: &str) -> Result<(), RelayError> {
    if !matches!(format, "" | "url" | "b64_json") {
        return Err(RelayError::Unsupported("unsupported image response format"));
    }
    Ok(())
}
fn validate_quote_response_format(quote: &ImageQuote, format: &str) -> Result<(), RelayError> {
    validate_response_format(format)?;
    if format == "url"
        && (quote.binding().semantic_model.starts_with("gpt-image-")
            || quote.binding().semantic_model == "chatgpt-image-latest")
    {
        return Err(RelayError::Unsupported(
            "native GPT image models do not support URL response format",
        ));
    }
    Ok(())
}
enum PreparedBody {
    Json(Vec<u8>),
    Multipart(Form),
}

pub(crate) fn dimensions(upload: &ImageUpload) -> Result<(u32, u32, bool), RelayError> {
    let b = &upload.bytes;
    if b.is_empty() || b.len() > IMAGE_BYTES {
        return Err(RelayError::Limit("image upload size"));
    }
    let invalid = RelayError::Invalid("invalid image MIME or dimensions");
    let dims = match upload.media_type {
        ImageMediaType::Png => {
            if b.len() < 45
                || &b[..8] != b"\x89PNG\r\n\x1a\n"
                || &b[8..12] != 13u32.to_be_bytes().as_slice()
                || &b[12..16] != b"IHDR"
                || b[26] != 0
                || b[27] != 0
                || b[28] > 1
            {
                return Err(invalid);
            }
            let depth = b[24];
            let color = b[25];
            if !match color {
                0 => [1, 2, 4, 8, 16].contains(&depth),
                2 | 4 | 6 => [8, 16].contains(&depth),
                3 => [1, 2, 4, 8].contains(&depth),
                _ => false,
            } {
                return Err(invalid);
            }
            let mut offset = 8usize;
            let mut idat = false;
            let mut ended = false;
            let mut chunks = 0usize;
            while offset.checked_add(12).is_some_and(|end| end <= b.len()) {
                chunks += 1;
                if chunks > 65536 {
                    return Err(invalid);
                }
                let size = u32::from_be_bytes(
                    b[offset..offset + 4]
                        .try_into()
                        .map_err(|_| invalid.clone())?,
                ) as usize;
                let end = offset
                    .checked_add(12)
                    .and_then(|n| n.checked_add(size))
                    .filter(|n| *n <= b.len())
                    .ok_or_else(|| invalid.clone())?;
                let kind = &b[offset + 4..offset + 8];
                if kind == b"IHDR" && offset != 8 {
                    return Err(invalid);
                }
                if kind == b"IDAT" && size > 0 {
                    idat = true;
                }
                if kind == b"IEND" {
                    if size != 0 || end != b.len() {
                        return Err(invalid);
                    }
                    ended = true;
                    break;
                }
                offset = end;
            }
            if !idat || !ended {
                return Err(invalid);
            }
            (
                u32::from_be_bytes(b[16..20].try_into().map_err(|_| invalid.clone())?),
                u32::from_be_bytes(b[20..24].try_into().map_err(|_| invalid.clone())?),
                matches!(color, 4 | 6),
            )
        }
        ImageMediaType::Jpeg => {
            if b.len() < 12 || !b.starts_with(&[0xff, 0xd8]) || !b.ends_with(&[0xff, 0xd9]) {
                return Err(invalid);
            }
            let mut pos = 2usize;
            let mut found = None;
            while pos + 4 <= b.len() {
                if b[pos] != 0xff {
                    return Err(invalid);
                }
                while pos < b.len() && b[pos] == 0xff {
                    pos += 1;
                }
                let marker = *b.get(pos).ok_or_else(|| invalid.clone())?;
                pos += 1;
                if marker == 0xda || marker == 0xd9 {
                    break;
                }
                if matches!(marker, 0x01 | 0xd0..=0xd8) {
                    continue;
                }
                if pos + 2 > b.len() {
                    return Err(invalid);
                }
                let size = u16::from_be_bytes([b[pos], b[pos + 1]]) as usize;
                if size < 2 || pos.checked_add(size).is_none_or(|n| n > b.len()) {
                    return Err(invalid);
                }
                if matches!(marker,0xc0..=0xc3|0xc5..=0xc7|0xc9..=0xcb|0xcd..=0xcf) {
                    if size < 8 {
                        return Err(invalid);
                    }
                    found = Some((
                        u16::from_be_bytes([b[pos + 5], b[pos + 6]]) as u32,
                        u16::from_be_bytes([b[pos + 3], b[pos + 4]]) as u32,
                        false,
                    ));
                    break;
                }
                pos += size;
            }
            found.ok_or_else(|| invalid.clone())?
        }
        ImageMediaType::Webp => {
            if b.len() < 26
                || &b[..4] != b"RIFF"
                || &b[8..12] != b"WEBP"
                || u32::from_le_bytes(b[4..8].try_into().map_err(|_| invalid.clone())?) as usize
                    != b.len() - 8
            {
                return Err(invalid);
            }
            let chunk =
                u32::from_le_bytes(b[16..20].try_into().map_err(|_| invalid.clone())?) as usize;
            if 20usize.checked_add(chunk).is_none_or(|n| n > b.len()) {
                return Err(invalid);
            }
            let mut offset = 12usize;
            let mut frame = false;
            let mut chunks = 0usize;
            while offset.checked_add(8).is_some_and(|n| n <= b.len()) {
                chunks += 1;
                if chunks > 65536 {
                    return Err(invalid);
                }
                let length = u32::from_le_bytes(
                    b[offset + 4..offset + 8]
                        .try_into()
                        .map_err(|_| invalid.clone())?,
                ) as usize;
                let end = offset
                    .checked_add(8)
                    .and_then(|n| n.checked_add(length))
                    .filter(|n| *n <= b.len())
                    .ok_or_else(|| invalid.clone())?;
                if matches!(&b[offset..offset + 4], b"VP8 " | b"VP8L") && length > 0 {
                    frame = true;
                }
                offset = end
                    .checked_add(length % 2)
                    .filter(|n| *n <= b.len())
                    .ok_or_else(|| invalid.clone())?;
            }
            if !frame || offset != b.len() {
                return Err(invalid);
            }
            match &b[12..16] {
                b"VP8 " if chunk >= 10 && b.len() >= 30 && b[23..26] == [0x9d, 0x01, 0x2a] => (
                    (u16::from_le_bytes([b[26], b[27]]) & 0x3fff) as u32,
                    (u16::from_le_bytes([b[28], b[29]]) & 0x3fff) as u32,
                    false,
                ),
                b"VP8L" if chunk >= 5 && b[20] == 0x2f => {
                    let bits =
                        u32::from_le_bytes(b[21..25].try_into().map_err(|_| invalid.clone())?);
                    (
                        (bits & 0x3fff) + 1,
                        ((bits >> 14) & 0x3fff) + 1,
                        bits & (1 << 28) != 0,
                    )
                }
                b"VP8X" if chunk == 10 && b.len() >= 30 => {
                    let w = u32::from_le_bytes([b[24], b[25], b[26], 0]) + 1;
                    let h = u32::from_le_bytes([b[27], b[28], b[29], 0]) + 1;
                    (w, h, b[20] & 0x10 != 0)
                }
                _ => return Err(invalid),
            }
        }
    };
    if dims.0 == 0
        || dims.1 == 0
        || dims.0 > 8192
        || dims.1 > 8192
        || u64::from(dims.0) * u64::from(dims.1) > 64 * 1024 * 1024
    {
        return Err(invalid);
    }
    Ok(dims)
}
fn image_part(upload: ImageUpload, index: usize) -> Result<Part, RelayError> {
    let (mime, extension) = match upload.media_type {
        ImageMediaType::Png => ("image/png", "png"),
        ImageMediaType::Jpeg => ("image/jpeg", "jpg"),
        ImageMediaType::Webp => ("image/webp", "webp"),
    };
    Part::bytes(upload.bytes)
        .file_name(format!("image-{index}.{extension}"))
        .mime_str(mime)
        .map_err(|_| RelayError::Invalid("invalid image MIME"))
}
#[cfg_attr(not(test), allow(dead_code))] // Tests use the default b64 response format.
fn edits_form(
    quote: &ImageQuote,
    prompt: &str,
    input: ImageMultipartInput,
    secret: &str,
    request_cap: usize,
) -> Result<Form, RelayError> {
    edits_form_with_response_format(quote, prompt, input, secret, request_cap, "b64_json")
}
fn edits_form_with_response_format(
    quote: &ImageQuote,
    prompt: &str,
    input: ImageMultipartInput,
    secret: &str,
    request_cap: usize,
    response_format: &str,
) -> Result<Form, RelayError> {
    validate_response_format(response_format)?;
    if secret.is_empty() {
        return Err(RelayError::Invalid("missing image provider credential"));
    }
    if quote.binding().endpoint != ImageEndpoint::Edits
        || input.images.is_empty()
        || input.images.len() > 16
    {
        return Err(RelayError::Invalid("invalid image edit count"));
    }
    let b = quote.binding();
    let metadata = prompt
        .len()
        .checked_add(b.upstream_model.len())
        .and_then(|n| n.checked_add(b.size.len()))
        .and_then(|n| n.checked_add(b.quality.len()))
        .and_then(|n| n.checked_add(128))
        .ok_or(RelayError::Limit("image edit metadata"))?;
    // Controlled field names and filenames plus reqwest's generated boundary
    // use the other half of the bounded 64KiB wrapper allowance.
    if metadata > request_cap.min(WRAPPER_BYTES / 2) {
        return Err(RelayError::Limit("image edit metadata"));
    }
    let mut total = 0usize;
    let mut first = None;
    for upload in &input.images {
        total = total
            .checked_add(upload.bytes.len())
            .ok_or(RelayError::Limit("image edit uploads"))?;
        if total > IMAGE_BYTES
            || upload
                .bytes
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        {
            return Err(RelayError::Limit("image edit uploads"));
        }
        let dims = dimensions(upload)?;
        if first.is_none() {
            first = Some(dims);
        }
        if b.semantic_model == "dall-e-2"
            && (input.images.len() != 1
                || upload.media_type != ImageMediaType::Png
                || upload.bytes.len() >= 4 * 1024 * 1024
                || dims.0 != dims.1)
        {
            return Err(RelayError::Unsupported(
                "dall-e-2 edits require one square PNG below 4MiB",
            ));
        }
    }
    if let Some(mask) = &input.mask {
        total = total
            .checked_add(mask.bytes.len())
            .ok_or(RelayError::Limit("image edit uploads"))?;
        if total > IMAGE_BYTES
            || mask.media_type != ImageMediaType::Png
            || mask.bytes.len() >= 4 * 1024 * 1024
            || mask
                .bytes
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        {
            return Err(RelayError::Invalid("invalid image mask"));
        }
        let dims = dimensions(mask)?;
        let image = first.ok_or(RelayError::Invalid("missing image"))?;
        if !dims.2 || (dims.0, dims.1) != (image.0, image.1) {
            return Err(RelayError::Invalid(
                "mask needs matching dimensions and alpha channel",
            ));
        }
    }
    let mut form = Form::new()
        .text("model", b.upstream_model.clone())
        .text("prompt", prompt.to_owned())
        .text("n", b.n.to_string());
    if !response_format.is_empty() {
        form = form.text("response_format", response_format.to_owned());
    }
    if !b.size.is_empty() {
        form = form.text("size", b.size.clone());
    }
    if !b.quality.is_empty() {
        form = form.text("quality", b.quality.clone());
    }
    let field = if input.images.len() == 1 {
        "image"
    } else {
        "image[]"
    };
    for (index, upload) in input.images.into_iter().enumerate() {
        form = form.part(field, image_part(upload, index)?);
    }
    if let Some(mask) = input.mask {
        form = form.part("mask", image_part(mask, 0)?);
    }
    Ok(form)
}

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unambiguous bounded image JSON")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut out = Map::new();
                let mut seen = BTreeSet::new();
                while let Some(k) = a.next_key::<String>()? {
                    if !seen.insert(k.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate image key"));
                    }
                    out.insert(k, a.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(out)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element::<Unique>()? {
                    out.push(v.0);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
fn object<'a>(v: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, RelayError> {
    let o = v.as_object().ok_or(RelayError::SettlementPending)?;
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(RelayError::SettlementPending);
    }
    Ok(o)
}
fn text(v: &Value, max: usize) -> Result<&str, RelayError> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= max)
        .ok_or(RelayError::SettlementPending)
}
fn percent_decoded_contains(s: &str, secret: &str) -> bool {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let raw = s.as_bytes();
    let mut decoded = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let (Some(a), Some(b)) = (hex(raw[i + 1]), hex(raw[i + 2]))
        {
            decoded.push(a * 16 + b);
            i += 3;
            continue;
        }
        decoded.push(raw[i]);
        i += 1;
    }
    decoded
        .windows(secret.len())
        .any(|w| w == secret.as_bytes())
}
fn observe(raw: &[u8], requested_n: u32, secret: &str) -> Result<ImageObservation, RelayError> {
    if raw.len() > RESPONSE_BYTES
        || raw.is_empty()
        || requested_n == 0
        || requested_n > 128
        || secret.is_empty()
    {
        return Err(RelayError::SettlementPending);
    }
    let raw_text = std::str::from_utf8(raw).map_err(|_| RelayError::SettlementPending)?;
    if raw_text.contains(secret) {
        return Err(RelayError::SettlementPending);
    }
    let v = serde_json::from_slice::<Unique>(raw)
        .map_err(|_| RelayError::SettlementPending)?
        .0;
    object(&v, &["created", "data"])?;
    if v.get("created").is_some_and(|n| n.as_u64().is_none()) {
        return Err(RelayError::SettlementPending);
    }
    let data = v["data"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= requested_n as usize)
        .ok_or(RelayError::SettlementPending)?;
    let mut image_bytes = 0usize;
    let mut wrapper_bytes = 0usize;
    let mut sanitized = Vec::new();
    for image in data {
        object(image, &["b64_json", "url", "revised_prompt"])?;
        if image.get("b64_json").is_some() == image.get("url").is_some() {
            return Err(RelayError::SettlementPending);
        }
        let mut out = Map::new();
        if let Some(b64) = image.get("b64_json") {
            let s = text(b64, IMAGE_BYTES)?;
            if s.contains(secret) {
                return Err(RelayError::SettlementPending);
            }
            image_bytes = image_bytes
                .checked_add(s.len())
                .ok_or(RelayError::SettlementPending)?;
            let decoded = STANDARD
                .decode(s)
                .map_err(|_| RelayError::SettlementPending)?;
            if decoded.is_empty()
                || decoded
                    .windows(secret.len())
                    .any(|w| w == secret.as_bytes())
            {
                return Err(RelayError::SettlementPending);
            }
            out.insert("b64_json".into(), json!(s));
        } else {
            let s = text(&image["url"], 8192)?;
            let u = Url::parse(s).map_err(|_| RelayError::SettlementPending)?;
            if s.contains(secret)
                || percent_decoded_contains(s, secret)
                || u.scheme() != "https"
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
                || u.fragment().is_some()
                || u.query_pairs()
                    .any(|(k, v)| k.contains(secret) || v.contains(secret))
            {
                return Err(RelayError::SettlementPending);
            }
            image_bytes = image_bytes
                .checked_add(s.len())
                .ok_or(RelayError::SettlementPending)?;
            out.insert("url".into(), json!(s)); // Deliberately never fetched by Core.
        }
        if let Some(p) = image.get("revised_prompt") {
            let p = p
                .as_str()
                .filter(|s| s.len() <= WRAPPER_BYTES && !s.contains(secret))
                .ok_or(RelayError::SettlementPending)?;
            wrapper_bytes = wrapper_bytes
                .checked_add(p.len())
                .ok_or(RelayError::SettlementPending)?;
            out.insert("revised_prompt".into(), json!(p));
        }
        if image_bytes > IMAGE_BYTES || wrapper_bytes > WRAPPER_BYTES {
            return Err(RelayError::SettlementPending);
        }
        sanitized.push(Value::Object(out));
    }
    // Include JSON escaping/keys/punctuation in the non-image wrapper budget.
    // The total raw cap remains authoritative even for heavily escaped strings.
    if raw.len().saturating_sub(image_bytes) > WRAPPER_BYTES {
        return Err(RelayError::SettlementPending);
    }
    let mut response = json!({"data":sanitized});
    if let Some(created) = v.get("created") {
        response["created"] = created.clone();
    }
    Ok(ImageObservation {
        output_images: data.len() as u32,
        provider_receipt_sha256: Sha256::digest(raw).into(),
        data: response,
    })
}

#[cfg(test)]
mod tests {
    use super::super::image_price::{
        ExactRatio, ImageCountBasis, ImagePriceConfig, ImagePriceRule, ImageQuoteRequest,
    };
    use super::*;
    use crate::billing::Credits;
    use axum::{Router, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    fn quote(n: u32) -> ImageQuote {
        quote_endpoint(n, ImageEndpoint::Generations)
    }
    fn quote_endpoint(n: u32, endpoint: ImageEndpoint) -> ImageQuote {
        quote_model(n, endpoint, "dall-e-2")
    }
    fn quote_model(n: u32, endpoint: ImageEndpoint, semantic_model: &str) -> ImageQuote {
        let rule = ImagePriceRule {
            upstream_id: 42,
            vendor: "compatible".into(),
            billing_group: "default".into(),
            upstream_model: "supplier-image".into(),
            semantic_model: semantic_model.into(),
            endpoint,
            size: "1024x1024".into(),
            quality: "standard".into(),
            usd_micros_per_image: ExactRatio {
                numerator: 1000,
                denominator: 1,
            },
            group_multiplier: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            max_n: 2,
            count_basis: ImageCountBasis::ReturnedPayloads,
            max_budget: Credits::try_from(1000000).unwrap(),
            source_revision: "synthetic-v1".into(),
        };
        let request = ImageQuoteRequest {
            upstream_id: rule.upstream_id,
            vendor: rule.vendor.clone(),
            billing_group: rule.billing_group.clone(),
            upstream_model: rule.upstream_model.clone(),
            semantic_model: rule.semantic_model.clone(),
            endpoint: rule.endpoint,
            size: rule.size.clone(),
            quality: rule.quality.clone(),
            n,
        };
        ImagePriceConfig {
            revision: 1,
            rules: vec![rule],
        }
        .quote(&request)
        .unwrap()
    }
    fn png() -> ImageUpload {
        ImageUpload {bytes:STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==").unwrap(),media_type:ImageMediaType::Png}
    }
    #[test]
    fn image_provider_edit_uploads_reject_wrong_magic_masks_counts_and_total() {
        let q = quote_endpoint(1, ImageEndpoint::Edits);
        let invalid = ImageUpload {
            bytes: png().bytes,
            media_type: ImageMediaType::Jpeg,
        };
        assert!(
            edits_form(
                &q,
                "draw",
                ImageMultipartInput {
                    images: vec![invalid],
                    mask: None
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
        assert!(
            edits_form(
                &q,
                "draw",
                ImageMultipartInput {
                    images: vec![],
                    mask: None
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
        assert!(
            edits_form(
                &q,
                "draw",
                ImageMultipartInput {
                    images: (0..17).map(|_| png()).collect(),
                    mask: None
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
        let mut mismatch = png();
        mismatch.bytes[19] = 2;
        assert!(
            edits_form(
                &q,
                "draw",
                ImageMultipartInput {
                    images: vec![png()],
                    mask: Some(mismatch)
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
        let oversize = ImageUpload {
            bytes: vec![0; IMAGE_BYTES + 1],
            media_type: ImageMediaType::Png,
        };
        assert!(
            edits_form(
                &q,
                "draw",
                ImageMultipartInput {
                    images: vec![oversize],
                    mask: None
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
        assert!(
            edits_form(
                &quote(1),
                "draw",
                ImageMultipartInput {
                    images: vec![png()],
                    mask: None
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
    }
    #[test]
    fn image_provider_edit_form_preserves_alpha_mask_and_metadata_cap() {
        let q = quote_endpoint(1, ImageEndpoint::Edits);
        assert!(
            edits_form(
                &q,
                "draw",
                ImageMultipartInput {
                    images: vec![png()],
                    mask: Some(png())
                },
                "synthetic-secret",
                32768
            )
            .is_ok()
        );
        assert!(
            edits_form(
                &q,
                &"x".repeat(32768),
                ImageMultipartInput {
                    images: vec![png()],
                    mask: None
                },
                "synthetic-secret",
                32768
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn image_provider_edits_tcp_uses_reqwest_multipart_and_frozen_quote() {
        let app = Router::new().route(
            "/v1/images/edits",
            post(
                |headers: axum::http::HeaderMap, body: axum::body::Bytes| async move {
                    assert!(
                        headers[header::CONTENT_TYPE]
                            .to_str()
                            .unwrap()
                            .starts_with("multipart/form-data; boundary=")
                    );
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer synthetic-secret");
                    let body = String::from_utf8_lossy(&body);
                    assert!(body.contains("name=\"image\"; filename=\"image-0.png\""));
                    assert!(body.contains("name=\"mask\"; filename=\"image-0.png\""));
                    assert!(
                        body.contains("Content-Type: image/png")
                            || body.contains("content-type: image/png")
                    );
                    assert!(body.contains("draw a tree"));
                    assert!(body.contains("supplier-image"));
                    axum::Json(json!({"created":1,"data":[{"b64_json":"aW1hZ2U="}]}))
                },
            ),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = ImageProvider::new_with_local_http(
            42,
            "compatible",
            "supplier-image",
            &format!("http://{address}/v1"),
            "synthetic-secret",
            &Limits::default(),
            true,
        )
        .unwrap();
        let quote = quote_endpoint(1, ImageEndpoint::Edits);
        let prepared = provider
            .prepare_edits(
                &quote,
                "draw a tree",
                ImageMultipartInput {
                    images: vec![png()],
                    mask: Some(png()),
                },
            )
            .await
            .unwrap();
        assert_eq!(prepared.binding(), quote.binding());
        assert_eq!(prepared.quote(), &quote);
        assert_eq!(prepared.dispatch().await.unwrap().output_images, 1);
        server.abort();
        let _ = server.await;
    }
    #[test]
    fn image_provider_native_gpt_url_is_rejected_without_guessing_token_usage() {
        let quote = quote_model(1, ImageEndpoint::Generations, "gpt-image-1");
        assert!(validate_quote_response_format(&quote, "url").is_err());
        assert!(validate_quote_response_format(&quote, "").is_ok());
        assert!(validate_quote_response_format(&quote, "b64_json").is_ok());
        assert!(observe(br#"{"created":1,"data":[{"b64_json":"aW1hZ2U="}],"usage":{"input_tokens":3,"output_tokens":5}}"#,1,"synthetic-secret").is_err());
    }
    #[tokio::test]
    async fn image_provider_response_format_omission_and_url_are_bound_before_dispatch() {
        use axum::extract::{FromRequest, Multipart};
        let provider = ImageProvider::new_with_local_http(
            42,
            "compatible",
            "supplier-image",
            "http://127.0.0.1:1/v1",
            "synthetic-secret",
            &Limits::default(),
            true,
        )
        .unwrap();
        for format in ["", "url", "b64_json"] {
            let prepared = provider
                .prepare_with_response_format(&quote(1), "draw", format)
                .await
                .unwrap();
            let body: Value =
                serde_json::from_slice(prepared.request.body().unwrap().as_bytes().unwrap())
                    .unwrap();
            assert_eq!(body["model"], "supplier-image");
            assert_eq!(body["n"], 1);
            assert_eq!(body["size"], "1024x1024");
            assert_eq!(body["quality"], "standard");
            assert_eq!(body["prompt"], "draw");
            if format.is_empty() {
                assert!(body.get("response_format").is_none());
            } else {
                assert_eq!(body["response_format"], format);
            }
            let q = quote_endpoint(1, ImageEndpoint::Edits);
            let mut prepared = provider
                .prepare_edits_with_response_format(
                    &q,
                    "draw",
                    ImageMultipartInput {
                        images: vec![png()],
                        mask: None,
                    },
                    format,
                )
                .await
                .unwrap();
            assert_eq!(prepared.quote(), &q);
            let headers = prepared.request.headers().clone();
            let body = prepared.request.body_mut().take().unwrap();
            let mut request = axum::http::Request::new(axum::body::Body::new(body));
            *request.headers_mut() = headers;
            let mut multipart = Multipart::from_request(request, &()).await.unwrap();
            let mut fields = std::collections::BTreeMap::new();
            let mut files = 0;
            while let Some(field) = multipart.next_field().await.unwrap() {
                let name = field.name().unwrap().to_owned();
                if name == "image" {
                    files += 1;
                    assert_eq!(field.content_type(), Some("image/png"));
                } else {
                    fields.insert(name, field.text().await.unwrap());
                }
            }
            assert_eq!(files, 1);
            assert_eq!(fields.get("model").unwrap(), "supplier-image");
            assert_eq!(fields.get("n").unwrap(), "1");
            assert_eq!(fields.get("size").unwrap(), "1024x1024");
            assert_eq!(fields.get("quality").unwrap(), "standard");
            if format.is_empty() {
                assert!(!fields.contains_key("response_format"));
            } else {
                assert_eq!(fields.get("response_format").unwrap(), format);
            }
        }
        assert!(
            provider
                .prepare_with_response_format(&quote(1), "draw", "invalid")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn image_provider_url_generation_forwarding_returns_only_safe_https_reference() {
        let app=Router::new().route("/v1/images/generations",post(|axum::Json(body):axum::Json<Value>| async move {
            assert_eq!(body,json!({"prompt":"draw","model":"supplier-image","n":1,"size":"1024x1024","quality":"standard","response_format":"url"}));
            axum::Json(json!({"created":1,"data":[{"url":"https://example.invalid/signed-output?token=result-only"}]}))
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = ImageProvider::new_with_local_http(
            42,
            "compatible",
            "supplier-image",
            &format!("http://{address}/v1"),
            "synthetic-secret",
            &Limits::default(),
            true,
        )
        .unwrap();
        let observation = provider
            .prepare_with_response_format(&quote(1), "draw", "url")
            .await
            .unwrap()
            .dispatch()
            .await
            .unwrap();
        assert_eq!(observation.output_images, 1);
        assert_eq!(
            observation.data["data"][0]["url"],
            "https://example.invalid/signed-output?token=result-only"
        );
        assert!(!observation.data.to_string().contains("synthetic-secret"));
        server.abort();
        let _ = server.await;
    }
    #[test]
    fn image_provider_observation_hashes_raw_and_preserves_count() {
        let raw = br#"{"created":123,"data":[{"b64_json":"aW1hZ2U="}]}"#;
        let o = observe(raw, 2, "synthetic-secret").unwrap();
        assert_eq!(o.output_images, 1);
        assert_eq!(
            o.provider_receipt_sha256,
            <[u8; 32]>::from(Sha256::digest(raw))
        );
        assert_eq!(o.data["created"], 123);
    }
    #[test]
    fn image_provider_rejects_ambiguous_malformed_secret_and_unknown_payloads() {
        for raw in [
            r#"{"data":[]}"#,
            r#"{"data":[{"b64_json":"!"}]}"#,
            r#"{"data":[{"b64_json":"aQ==","b64_json":"bQ=="}]}"#,
            r#"{"data":[{"b64_json":"aQ==","url":"https://example.com/a"}]}"#,
            r#"{"data":[{"url":"http://example.com/a"}]}"#,
            r#"{"data":[{"url":"https://example.com/?key=synthetic%2Dsecret"}]}"#,
            r#"{"data":[{"b64_json":"c3ludGhldGljLXNlY3JldA=="}]}"#,
            r#"{"data":[{"b64_json":"aQ==","revised_prompt":"synthetic-secret"}]}"#,
            r#"{"data":[{"b64_json":"aQ=="}],"api_key":"redacted"}"#,
        ] {
            assert!(matches!(
                observe(raw.as_bytes(), 1, "synthetic-secret"),
                Err(RelayError::SettlementPending)
            ));
        }
    }
    #[test]
    fn image_provider_accepts_https_reference_without_fetch_and_caps_counts() {
        let raw =
            br#"{"data":[{"url":"https://example.invalid/image?signature=public-result-token"}]}"#;
        assert_eq!(
            observe(raw, 1, "synthetic-secret").unwrap().output_images,
            1
        );
        assert!(
            observe(
                br#"{"data":[{"b64_json":"aQ=="},{"b64_json":"aQ=="}]}"#,
                1,
                "synthetic-secret"
            )
            .is_err()
        );
        assert!(
            ImageProvider::new(
                42,
                "compatible",
                "supplier-image",
                "http://127.0.0.1:1/v1",
                "synthetic-secret",
                &Limits::default()
            )
            .is_err()
        );
    }
    #[test]
    fn image_provider_caps_wrapper_and_detects_escaped_url_credential() {
        let oversized =
            json!({"data":[{"b64_json":"aQ==","revised_prompt":"x".repeat(WRAPPER_BYTES+1)}]});
        assert!(
            observe(
                &serde_json::to_vec(&oversized).unwrap(),
                1,
                "synthetic-secret"
            )
            .is_err()
        );
        assert!(
            observe(
                br#"{"data":[{"url":"https://example.invalid/synthetic%2Dsecret/a"}]}"#,
                1,
                "synthetic-secret"
            )
            .is_err()
        );
        assert!(
            observe(
                br#"{"data":[{"url":"https://example.invalid/synthetic\u002dsecret/a"}]}"#,
                1,
                "synthetic-secret"
            )
            .is_err()
        );
    }
    #[test]
    fn image_provider_isolated_mode_requires_explicit_numeric_loopback() {
        for base in [
            "http://localhost/v1",
            "https://localhost/v1",
            "http://10.0.0.1/v1",
            "https://169.254.169.254/v1",
        ] {
            assert!(
                ImageProvider::new_with_local_http(
                    42,
                    "compatible",
                    "supplier-image",
                    base,
                    "synthetic-secret",
                    &Limits::default(),
                    true
                )
                .is_err()
            );
        }
        assert!(
            ImageProvider::new_with_local_http(
                42,
                "compatible",
                "supplier-image",
                "http://127.0.0.1:1/v1",
                "synthetic-secret",
                &Limits::default(),
                true
            )
            .is_ok()
        );
    }
    #[tokio::test]
    async fn image_provider_tcp_dispatch_matches_quote_and_yields_raw_receipt() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let app=Router::new().route("/v1/images/generations",post(move |headers:axum::http::HeaderMap,axum::Json(body):axum::Json<Value>| {let c=c.clone();async move {c.fetch_add(1,Ordering::SeqCst);assert_eq!(headers[header::AUTHORIZATION],"Bearer synthetic-secret");assert_eq!(body,json!({"prompt":"draw a tree","model":"supplier-image","n":2,"size":"1024x1024","quality":"standard","response_format":"b64_json"}));axum::Json(json!({"created":123,"data":[{"b64_json":"aW1hZ2U="}]}))}}));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = ImageProvider::construct(
            42,
            "compatible",
            "supplier-image",
            &format!("http://{address}/v1"),
            "synthetic-secret",
            &Limits::default(),
            true,
        )
        .unwrap();
        let quote = quote(2);
        let prepared = provider.prepare(&quote, "draw a tree").await.unwrap();
        assert_eq!(prepared.binding(), quote.binding());
        let result = prepared.dispatch().await.unwrap();
        assert_eq!(result.output_images, 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn image_provider_http_redirect_and_malformed_retain_hold() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let app = Router::new().route(
            "/v1/images/generations",
            post(move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("location", "http://127.0.0.1:1/credential-leak")],
                        "supplier-private-error",
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = ImageProvider::construct(
            42,
            "compatible",
            "supplier-image",
            &format!("http://{address}/v1"),
            "synthetic-secret",
            &Limits::default(),
            true,
        )
        .unwrap();
        assert!(matches!(
            provider
                .prepare(&quote(1), "draw")
                .await
                .unwrap()
                .dispatch()
                .await,
            Err(RelayError::SettlementPending)
        ));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn image_provider_binding_mismatch_fails_before_dispatch() {
        let provider = ImageProvider::construct(
            41,
            "compatible",
            "supplier-image",
            "http://127.0.0.1:1/v1",
            "synthetic-secret",
            &Limits::default(),
            true,
        )
        .unwrap();
        assert!(matches!(
            provider.prepare(&quote(1), "draw").await,
            Err(RelayError::RouteUnavailable)
        ));
    }
}
