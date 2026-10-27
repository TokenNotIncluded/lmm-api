//! Host-selected input admission evidence, never usage or dispatch authority.
//! The counter and generator share one frozen body and one credential binding.
use super::{Gate, Limits, Phase, RelayError, Route, provider_network::ProviderTransport};
use axum::{body::Bytes, http::header};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{num::NonZeroU64, time::Duration};

pub(crate) const PROJECTION_REVISION: &str = "responses-stateless-text-v1";
const GENERATION_URL: &str = "https://api.openai.com/v1/responses";
const COUNT_URL: &str = "https://api.openai.com/v1/responses/input_tokens";

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) enum CountPlan {
    #[default]
    ContextMax,
    OpenAiResponsesExactV1(ResponsesCountProfile),
}

/// Only reviewed host configuration can construct this profile.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResponsesCountProfile {
    model: String,
    input_max: u32,
    output_max: u32,
    count_timeout: Duration,
}

impl ResponsesCountProfile {
    pub(crate) fn new(
        base: &str,
        model: String,
        input_max: u32,
        output_max: u32,
        count_timeout_ms: NonZeroU64,
    ) -> Result<Self, RelayError> {
        // @context-fold begin official-responses-count-profile
        if !matches!(
            base,
            "https://api.openai.com/v1" | "https://api.openai.com/v1/"
        ) || !fixed_snapshot(&model)
            || input_max == 0
            || output_max == 0
        {
            return Err(RelayError::Invalid(
                "exact count requires official Responses snapshot contract",
            ));
        }
        Ok(Self {
            model,
            input_max,
            output_max,
            count_timeout: Duration::from_millis(count_timeout_ms.get()),
        })
        // @context-fold end official-responses-count-profile
    }
}

fn fixed_snapshot(model: &str) -> bool {
    // @context-fold begin fixed-model-snapshot-syntax
    // This rejects aliases; it is not evidence of supplier model acceptance.
    if !model.is_ascii() || model.len() < 12 {
        return false;
    }
    let (name, date) = model.split_at(model.len() - 10);
    name.ends_with('-')
        && name.len() > 1
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && date[5..7]
            .parse::<u8>()
            .is_ok_and(|m| (1..=12).contains(&m))
        && date[8..].parse::<u8>().is_ok_and(|d| (1..=31).contains(&d))
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_'))
    // @context-fold end fixed-model-snapshot-syntax
}

/// Digests are local evidence, not a supplier signature. No raw body or key.
pub struct CountObservation {
    pub(crate) api: &'static str,
    pub(crate) input_tokens: u32,
    pub(crate) requested_output: u32,
    pub(crate) wire_sha256: [u8; 32],
    pub(crate) request_sha256: [u8; 32],
    pub(crate) response_sha256: [u8; 32],
    pub(crate) projection_revision: &'static str,
}

// @context-fold begin stateless-text-projection-types
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    model: String,
    input: TextInput,
    #[serde(default, deserialize_with = "optional_text")]
    instructions: Option<String>,
    stream: bool,
    store: bool,
    max_output_tokens: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum TextInput {
    Text(String),
    Messages(Vec<TextMessage>),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TextMessage {
    role: TextRole,
    content: TextContent,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<MessageType>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum TextRole {
    System,
    Developer,
    User,
    Assistant,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MessageType {
    Message,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum TextContent {
    Text(String),
    Blocks(Vec<TextBlock>),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TextBlock {
    #[serde(rename = "type")]
    kind: TextBlockType,
    text: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum TextBlockType {
    InputText,
    OutputText,
}

#[derive(Serialize)]
struct CountRequest<'a> {
    model: &'a str,
    input: &'a TextInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
}

fn optional_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountResponse {
    object: CountObject,
    input_tokens: u64,
}

#[derive(Deserialize)]
enum CountObject {
    #[serde(rename = "response.input_tokens")]
    InputTokens,
}
// @context-fold end stateless-text-projection-types

struct FrozenCountRequest {
    bytes: Bytes,
    wire_sha256: [u8; 32],
    requested_output: u32,
}

fn project(
    profile: &ResponsesCountProfile,
    generation_url: &Url,
    body: &[u8],
    limits: &Limits,
) -> Result<FrozenCountRequest, RelayError> {
    // @context-fold begin freeze-responses-count-projection
    if generation_url.as_str() != GENERATION_URL {
        return Err(RelayError::RouteUnavailable);
    }
    if body.len() > limits.request_bytes {
        return Err(RelayError::Limit("count generation body"));
    }
    let generation: Generation = serde_json::from_slice(body).map_err(|_| {
        RelayError::Unsupported("exact count requires stateless pure text Responses")
    })?;
    if generation.model != profile.model
        || generation.stream
        || generation.store
        || generation.max_output_tokens == 0
        || generation.max_output_tokens > profile.output_max
    {
        return Err(RelayError::Invalid(
            "exact count generation binding mismatch",
        ));
    }
    if let TextInput::Messages(messages) = &generation.input {
        for message in messages {
            if let TextContent::Blocks(blocks) = &message.content {
                for block in blocks {
                    if matches!(block.kind, TextBlockType::OutputText)
                        && !matches!(message.role, TextRole::Assistant)
                    {
                        return Err(RelayError::Invalid("invalid exact count text role"));
                    }
                }
            }
        }
    }
    let projection = CountRequest {
        model: &generation.model,
        input: &generation.input,
        instructions: generation.instructions.as_deref(),
    };
    let bytes = serde_json::to_vec(&projection)
        .map_err(|_| RelayError::Invalid("invalid count projection"))?;
    if bytes.len() > limits.request_bytes {
        return Err(RelayError::Limit("count projection body"));
    }
    Ok(FrozenCountRequest {
        bytes: bytes.into(),
        wire_sha256: Sha256::digest(body).into(),
        requested_output: generation.max_output_tokens,
    })
    // @context-fold end freeze-responses-count-projection
}

/// Caller retains and later sends the exact `generation_body`. This operation
/// is outside the billing transaction and creates neither usage nor a start.
pub(super) async fn execute(
    route: &Route,
    generation_body: &[u8],
    limits: &Limits,
    transport: &ProviderTransport,
    gate: &Gate,
) -> Result<Option<CountObservation>, RelayError> {
    // @context-fold begin count-before-funds-reservation
    let CountPlan::OpenAiResponsesExactV1(profile) = &route.count_plan else {
        return Ok(None);
    };
    let frozen = project(profile, &route.endpoint, generation_body, limits)?;
    let url = Url::parse(COUNT_URL).map_err(|_| RelayError::RouteUnavailable)?;
    count_once(profile, &url, route, frozen, limits, transport, gate)
        .await
        .map(Some)
    // @context-fold end count-before-funds-reservation
}

async fn count_once(
    profile: &ResponsesCountProfile,
    url: &Url,
    route: &Route,
    frozen: FrozenCountRequest,
    limits: &Limits,
    transport: &ProviderTransport,
    gate: &Gate,
) -> Result<CountObservation, RelayError> {
    // @context-fold begin bounded-pinned-counter-exchange
    if !route.is_available() {
        return Err(RelayError::RouteUnavailable);
    }
    gate.run(
        profile.count_timeout.min(limits.total_timeout),
        Phase::Headers,
        async {
            let prepared = gate
                .run(
                    limits.connect_timeout,
                    Phase::Connect,
                    transport.prepare(url, limits),
                )
                .await?;
            // The count sends the prompt and the same credential as generation.
            // Honor revocation after the asynchronous connection preparation too.
            if !route.is_available() {
                return Err(RelayError::RouteUnavailable);
            }
            let request_sha256 = Sha256::digest(&frozen.bytes).into();
            let mut response = gate
                .run(limits.header_timeout, Phase::Headers, async {
                    prepared
                        .client()
                        .post(url.clone())
                        .header(header::AUTHORIZATION, route.credential.clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::ACCEPT_ENCODING, "identity")
                        .body(frozen.bytes)
                        .send()
                        .await
                        .map_err(|_| RelayError::UpstreamConnection)
                })
                .await?;
            if !prepared.matches_peer(response.remote_addr()) {
                return Err(RelayError::RouteUnavailable);
            }
            if response.status() != reqwest::StatusCode::OK {
                return Err(RelayError::UpstreamStatus(response.status().as_u16()));
            }
            if response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_none_or(|v| v.split(';').next().unwrap_or("").trim() != "application/json")
                || response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .is_some_and(|v| v.as_bytes() != b"identity")
            {
                return Err(RelayError::Protocol("invalid input count content type"));
            }
            let cap = limits.event_bytes.min(limits.upstream_bytes);
            if response
                .content_length()
                .is_some_and(|length| length > cap as u64)
            {
                return Err(RelayError::Limit("input count response"));
            }
            let mut bytes = Vec::new();
            loop {
                let chunk = gate
                    .run(limits.read_timeout, Phase::Read, async {
                        response
                            .chunk()
                            .await
                            .map_err(|_| RelayError::UpstreamConnection)
                    })
                    .await?;
                let Some(chunk) = chunk else { break };
                if chunk.len() > limits.transport_chunk_bytes
                    || bytes
                        .len()
                        .checked_add(chunk.len())
                        .is_none_or(|len| len > cap)
                {
                    return Err(RelayError::Limit("input count response"));
                }
                bytes.extend_from_slice(&chunk);
            }
            observation(
                profile,
                frozen.wire_sha256,
                frozen.requested_output,
                request_sha256,
                &bytes,
            )
        },
    )
    .await
    // @context-fold end bounded-pinned-counter-exchange
}

fn observation(
    profile: &ResponsesCountProfile,
    wire_sha256: [u8; 32],
    requested_output: u32,
    request_sha256: [u8; 32],
    bytes: &[u8],
) -> Result<CountObservation, RelayError> {
    // @context-fold begin validate-counter-admission-observation
    let response: CountResponse = serde_json::from_slice(bytes)
        .map_err(|_| RelayError::Protocol("invalid input count response"))?;
    let CountObject::InputTokens = response.object;
    if response.input_tokens > u64::from(profile.input_max) {
        return Err(RelayError::Invalid("input count exceeds model contract"));
    }
    let input_tokens = u32::try_from(response.input_tokens)
        .map_err(|_| RelayError::Invalid("input count exceeds model contract"))?;
    Ok(CountObservation {
        api: "responses.input_tokens",
        input_tokens,
        requested_output,
        wire_sha256,
        request_sha256,
        response_sha256: Sha256::digest(bytes).into(),
        projection_revision: PROJECTION_REVISION,
    })
    // @context-fold end validate-counter-admission-observation
}

#[cfg(test)]
#[path = "input_count_tests.rs"]
mod tests;
