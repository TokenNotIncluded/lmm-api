//! Typed conversions for OpenAI Chat Completions and OpenAI Responses.
//!
//! Legacy provenance:
//! - `relaykit/relayconvert/internal/oai_chat/to_oai_responses_{req,resp,stream_resp}.go`
//! - `relaykit/relayconvert/internal/oai_responses/to_oai_chat_{req,resp,stream_resp}.go`
//! - `relaykit/relayconvert/{request_registry,response_registry}.go`

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
};

use serde::{Deserialize, Serialize};

use super::{
    CanonicalContent, CanonicalMessage, CanonicalRequest, CanonicalResponse, CanonicalStreamEvent,
    CanonicalTool, CanonicalToolChoice, ClaudeContentBlock, ClaudeMessage, ClaudeRequest,
    ClaudeResponse, ClaudeStreamEvent, ClaudeStreamSnapshot, ClaudeTool, ClaudeToolChoice,
    Converted, FinishReason, FixtureKind, GEMINI_SYNTHETIC_THOUGHT_SIGNATURE, GeminiCandidate,
    GeminiContent, GeminiFunctionCall, GeminiFunctionCallingConfig, GeminiFunctionDeclaration,
    GeminiFunctionResponse, GeminiGenerationConfig, GeminiPart, GeminiRequest, GeminiResponse,
    GeminiStreamSnapshot, GeminiTool, GeminiToolConfig, GeminiUsage, JsonData, LossReport,
    OpaqueProviderState, OpaqueStateProvenance, OpenAiChatContentPart, OpenAiChatMessage,
    OpenAiAnthropicBlock, OpenAiAnthropicExtraContent, OpenAiChatRequest, OpenAiChatResponse,
    OpenAiChatTool, OpenAiChoice, OpenAiExtraContent, OpenAiFunction, OpenAiGoogleExtraContent,
    OpenAiResponsesRequest, OpenAiStreamChunk,
    OpenAiStreamDelta, OpenAiStreamSnapshot, OpenAiToolCall, Protocol, ReasoningConfig,
    RequestOptions, ResponsesContentPart, ResponsesInput, ResponsesInputItem,
    ResponsesOutputContent, ResponsesOutputItem, ResponsesResponse, ResponsesStreamEvent,
    ResponsesStreamSnapshot, ResponsesTool, Role, StreamContentKind, StringOrParts, TokenDetails,
    TokenUsage, WireError, WireUsage,
};

#[derive(Debug)]
pub enum RelayConvertError {
    Json(serde_json::Error),
    Missing(&'static str),
    Unsupported(String),
    /// A source field is known at the protocol boundary but cannot be
    /// represented by the requested target protocol.  Unlike the legacy
    /// string variant this carries stable machine-readable routing data.
    UnsupportedFeature(ConversionUnsupportedFeature),
}

/// Structured cross-protocol rejection returned before an upstream request is
/// sent.  `path` always points into the source envelope (for example,
/// `tools[2]` or `input[1].content[0]`).  No request body or provider value is
/// included in this type or its display implementation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConversionUnsupportedFeature {
    /// Stable machine-readable error code.
    pub code: String,
    /// Source wire format identifier.
    pub source_format: String,
    /// Target wire format identifier.
    pub target_format: String,
    /// Stable semantic feature name.
    pub feature: String,
    /// Source JSON path at which the feature was found.
    pub path: String,
    /// Optional PLAN loss code associated with the rejected feature.
    pub loss_code: Option<String>,
    /// These errors are not fixed by retrying the same request.
    pub retryable: bool,
}

impl ConversionUnsupportedFeature {
    /// The public error code used by relay HTTP adapters.
    pub const CODE: &'static str = "conversion_unsupported_feature";
}

impl fmt::Display for RelayConvertError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid relay JSON: {error}"),
            Self::Missing(field) => write!(formatter, "missing required relay field: {field}"),
            Self::Unsupported(message) => formatter.write_str(message),
            Self::UnsupportedFeature(error) => write!(
                formatter,
                "{} at {} cannot be converted from {} to {}",
                error.feature, error.path, error.source_format, error.target_format
            ),
        }
    }
}

impl Error for RelayConvertError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Missing(_) | Self::Unsupported(_) | Self::UnsupportedFeature(_) => None,
        }
    }
}

impl From<serde_json::Error> for RelayConvertError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

const OPENAI_RESPONSES_FORMAT: &str = "openai_responses";
const OPENAI_CHAT_FORMAT: &str = "openai_chat";

fn unsupported_responses_feature(
    feature: impl Into<String>,
    path: impl Into<String>,
    loss_code: Option<&str>,
) -> RelayConvertError {
    RelayConvertError::UnsupportedFeature(ConversionUnsupportedFeature {
        code: ConversionUnsupportedFeature::CODE.to_owned(),
        source_format: OPENAI_RESPONSES_FORMAT.to_owned(),
        target_format: OPENAI_CHAT_FORMAT.to_owned(),
        feature: feature.into(),
        path: path.into(),
        loss_code: loss_code.map(str::to_owned),
        retryable: false,
    })
}

fn unsupported_responses_extra(path: impl Into<String>) -> RelayConvertError {
    unsupported_responses_feature("unknown_field", path, None)
}

fn first_extra_path(
    base: &str,
    extra: &BTreeMap<String, JsonData>,
) -> Option<RelayConvertError> {
    extra.keys().next().map(|key| {
        let path = if base.is_empty() {
            key.clone()
        } else {
            format!("{base}.{key}")
        };
        unsupported_responses_extra(path)
    })
}

/// Pre-scans an OpenAI Responses request before any cross-protocol request is
/// emitted.  Every feature which the OpenAI Chat representation cannot carry
/// is rejected with a stable feature/path pair; no lossy `LossReport` entry is
/// allowed to reach the upstream boundary.
pub fn preflight_openai_responses_request_to_openai_chat(
    request: &OpenAiResponsesRequest,
) -> Result<(), RelayConvertError> {
    if let Some(error) = first_extra_path("", &request.extra) {
        return Err(error);
    }
    for (field, value) in [
        ("conversation", request.conversation.as_ref()),
        ("previous_response_id", request.previous_response_id.as_ref()),
        ("prompt", request.prompt.as_ref()),
        ("context_management", request.context_management.as_ref()),
    ] {
        if value.is_some() {
            let feature = match field {
                "conversation" => "stateful_conversation",
                "previous_response_id" => "previous_response_id",
                "prompt" => "prompt_template",
                _ => "context_management",
            };
            let loss_code = match field {
                "conversation" | "previous_response_id" | "context_management" => {
                    Some("LOSS_STATEFUL_CONTEXT")
                }
                _ => None,
            };
            return Err(unsupported_responses_feature(feature, field, loss_code));
        }
    }
    for (field, value) in [
        ("include", request.include.as_ref()),
        ("moderation", request.moderation.as_ref()),
        ("max_tool_calls", request.max_tool_calls.as_ref()),
        ("client_metadata", request.client_metadata.as_ref()),
    ] {
        if value.is_some() {
            return Err(unsupported_responses_feature(
                "responses_request_option",
                field,
                None,
            ));
        }
    }
    if let Some(reasoning) = request.reasoning.as_ref() {
        if let Some(error) = first_extra_path("reasoning", &reasoning.extra) {
            return Err(error);
        }
        for (field, value) in [
            ("summary", reasoning.summary.as_ref()),
            ("mode", reasoning.mode.as_ref()),
            ("context", reasoning.context.as_ref()),
        ] {
            if value.is_some() {
                return Err(unsupported_responses_feature(
                    "reasoning_summary",
                    format!("reasoning.{field}"),
                    Some("LOSS_OPAQUE_REASONING"),
                ));
            }
        }
    }
    if let Some(JsonData::String(_)) = request.prompt_cache_key.as_ref() {
        // The string form is carried by Chat's prompt-cache extension.
    } else if request.prompt_cache_key.is_some() {
        return Err(unsupported_responses_feature(
            "prompt_cache_key",
            "prompt_cache_key",
            Some("LOSS_CACHE_CONTROL"),
        ));
    }
    for (index, tool) in request.tools.iter().enumerate() {
        preflight_responses_tool(tool, index)?;
    }
    let mut seen_call_ids = BTreeSet::new();
    let mut outstanding_call_ids = BTreeSet::new();
    match request.input.as_ref() {
        None | Some(ResponsesInput::String(_)) => {}
        Some(ResponsesInput::Json(_)) => {
            return Err(unsupported_responses_feature(
                "input_shape",
                "input",
                None,
            ));
        }
        Some(ResponsesInput::Items(items)) => {
            for (index, item) in items.iter().enumerate() {
                preflight_responses_input_item(
                    item,
                    index,
                    &mut seen_call_ids,
                    &mut outstanding_call_ids,
                )?;
            }
        }
    }
    Ok(())
}

fn format_for_protocol(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAi => OPENAI_CHAT_FORMAT,
        Protocol::OpenAiResponses => OPENAI_RESPONSES_FORMAT,
        Protocol::Claude => "anthropic_messages",
        Protocol::Gemini => "google_gemini_generate_content",
    }
}

fn retarget_unsupported_error(
    error: RelayConvertError,
    target: Protocol,
) -> RelayConvertError {
    retarget_unsupported_error_format(error, format_for_protocol(target))
}

fn retarget_unsupported_error_format(
    error: RelayConvertError,
    target_format: &str,
) -> RelayConvertError {
    match error {
        RelayConvertError::UnsupportedFeature(mut detail) => {
            detail.target_format = target_format.to_owned();
            RelayConvertError::UnsupportedFeature(detail)
        }
        other => other,
    }
}

/// Runs the Responses cross-protocol preflight with an explicit target.  A
/// native Responses route is already raw passthrough and therefore has no
/// conversion feature to reject.  The current canonical request subset uses
/// the same strict representability scan for non-native targets, while the
/// returned target format remains the actual selected protocol.
pub fn preflight_openai_responses_request_for_target(
    request: &OpenAiResponsesRequest,
    target: Protocol,
) -> Result<(), RelayConvertError> {
    if target == Protocol::OpenAiResponses {
        return Ok(());
    }
    preflight_openai_responses_request_to_openai_chat(request)
        .map_err(|error| retarget_unsupported_error(error, target))
}

/// Runs the same scanner for a provider-neutral canonical IR target.  The IR
/// conversion boundary is stricter than native passthrough, but its errors do
/// not claim that Chat was the selected destination.
pub fn preflight_openai_responses_request_for_canonical(
    request: &OpenAiResponsesRequest,
) -> Result<(), RelayConvertError> {
    preflight_openai_responses_request_to_openai_chat(request)
        .map_err(|error| retarget_unsupported_error_format(error, "provider_neutral_ir"))
}

fn preflight_responses_tool(
    tool: &ResponsesTool,
    index: usize,
) -> Result<(), RelayConvertError> {
    let path = format!("tools[{index}]");
    match tool.kind.as_str() {
        "function" => {
            if tool.name.as_deref().is_none_or(|name| name.trim().is_empty()) {
                return Err(unsupported_responses_feature(
                    "function_tool_name",
                    format!("{path}.name"),
                    None,
                ));
            }
            if let Some(error) = first_extra_path(&path, &tool.extra) {
                return Err(error);
            }
        }
        "custom" | "custom_tool" => {
            return Err(unsupported_responses_feature(
                "custom_tool",
                path,
                Some("LOSS_CUSTOM_TOOL"),
            ));
        }
        "web_search" | "web_search_preview" => {
            return Err(unsupported_responses_feature(
                "builtin_web_search",
                path,
                Some("LOSS_BUILTIN_TOOL"),
            ));
        }
        "file_search" => {
            return Err(unsupported_responses_feature(
                "builtin_file_search",
                path,
                Some("LOSS_BUILTIN_TOOL"),
            ));
        }
        "code_interpreter" => {
            return Err(unsupported_responses_feature(
                "builtin_code_execution",
                path,
                Some("LOSS_BUILTIN_TOOL"),
            ));
        }
        "mcp" => {
            return Err(unsupported_responses_feature(
                "mcp",
                path,
                Some("LOSS_BUILTIN_TOOL"),
            ));
        }
        "computer_use" | "computer_use_preview" => {
            return Err(unsupported_responses_feature(
                "computer_use",
                path,
                Some("LOSS_BUILTIN_TOOL"),
            ));
        }
        "image_generation"
        | "hosted_shell"
        | "apply_patch"
        | "skills"
        | "tool_search"
        | "programmatic_tool_calling" => {
            return Err(unsupported_responses_feature(
                format!("builtin_{}", tool.kind),
                path,
                Some("LOSS_BUILTIN_TOOL"),
            ));
        }
        _ => {
            return Err(unsupported_responses_feature(
                "unknown_tool_type",
                path,
                None,
            ));
        }
    }
    Ok(())
}

fn preflight_responses_input_item(
    item: &ResponsesInputItem,
    index: usize,
    seen_call_ids: &mut BTreeSet<String>,
    outstanding_call_ids: &mut BTreeSet<String>,
) -> Result<(), RelayConvertError> {
    let path = format!("input[{index}]");
    match item.kind.as_deref() {
        None | Some("message") => {
            if let Some(role) = item.role.as_deref() {
                if !matches!(role, "system" | "developer" | "user" | "assistant" | "tool") {
                    return Err(unsupported_responses_feature(
                        "unknown_input_role",
                        format!("{path}.role"),
                        None,
                    ));
                }
            }
            if let Some(error) = first_extra_path(&path, &item.extra) {
                return Err(error);
            }
            preflight_responses_content(item.content.as_ref(), &path)?;
        }
        Some("function_call") => {
            let Some(call_id) = item.call_id.as_deref().filter(|id| !id.trim().is_empty()) else {
                return Err(unsupported_responses_feature(
                    "function_call_id",
                    format!("{path}.call_id"),
                    Some("LOSS_TOOL_CALL_ID"),
                ));
            };
            if item
                .name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
            {
                return Err(unsupported_responses_feature(
                    "function_call_name",
                    format!("{path}.name"),
                    None,
                ));
            }
            if !seen_call_ids.insert(call_id.to_owned()) {
                return Err(unsupported_responses_feature(
                    "duplicate_function_call_id",
                    format!("{path}.call_id"),
                    Some("LOSS_TOOL_CALL_ID"),
                ));
            }
            outstanding_call_ids.insert(call_id.to_owned());
            if let Some(error) = first_extra_path(&path, &item.extra) {
                return Err(error);
            }
        }
        Some("function_call_output") => {
            let Some(call_id) = item.call_id.as_deref().filter(|id| !id.trim().is_empty()) else {
                return Err(unsupported_responses_feature(
                    "function_call_output_id",
                    format!("{path}.call_id"),
                    Some("LOSS_TOOL_CALL_ID"),
                ));
            };
            if !outstanding_call_ids.remove(call_id) {
                return Err(unsupported_responses_feature(
                    "function_call_output_id",
                    format!("{path}.call_id"),
                    Some("LOSS_TOOL_CALL_ID"),
                ));
            }
            if let Some(error) = first_extra_path(&path, &item.extra) {
                return Err(error);
            }
        }
        Some("custom_tool_call") | Some("custom_tool_call_output") => {
            return Err(unsupported_responses_feature(
                "custom_tool",
                path,
                Some("LOSS_CUSTOM_TOOL"),
            ));
        }
        Some(kind) => {
            return Err(unsupported_responses_feature(
                format!("responses_input_{kind}"),
                path,
                None,
            ));
        }
    }
    Ok(())
}

fn preflight_responses_content(
    content: Option<&StringOrParts<ResponsesContentPart>>,
    path: &str,
) -> Result<(), RelayConvertError> {
    let Some(content) = content else {
        return Ok(());
    };
    let StringOrParts::Parts(parts) = content else {
        return Ok(());
    };
    for (index, part) in parts.iter().enumerate() {
        let part_path = format!("{path}.content[{index}]");
        match part.kind.as_str() {
            "input_text" | "output_text" | "text" => {
                if part.text.is_none() {
                    return Err(unsupported_responses_feature(
                        "text_content",
                        format!("{part_path}.text"),
                        None,
                    ));
                }
                if let Some(error) = first_extra_path(&part_path, &part.extra) {
                    return Err(error);
                }
            }
            "input_image" => {
                if part.image_url.as_deref().is_none_or(str::is_empty) {
                    return Err(unsupported_responses_feature(
                        "image_content",
                        format!("{part_path}.image_url"),
                        None,
                    ));
                }
                if let Some(error) = first_extra_path(&part_path, &part.extra) {
                    return Err(error);
                }
            }
            "input_file" | "input_audio" | "input_video" => {
                return Err(unsupported_responses_feature(
                    format!("{kind}_content", kind = part.kind),
                    part_path,
                    None,
                ));
            }
            _ => {
                return Err(unsupported_responses_feature(
                    "unknown_content_part",
                    part_path,
                    None,
                ));
            }
        }
    }
    Ok(())
}

pub fn openai_chat_request_to_canonical(
    request: OpenAiChatRequest,
) -> Result<Converted<CanonicalRequest>, RelayConvertError> {
    if request.model.trim().is_empty() {
        return Err(RelayConvertError::Missing("model"));
    }

    if request.n.unwrap_or(1) > 1 {
        return Err(RelayConvertError::Unsupported(
            "Chat n > 1 cannot be represented by Responses".to_owned(),
        ));
    }

    let mut instructions = Vec::new();
    let mut messages = Vec::new();
    let mut loss = LossReport::default();
    let mut observed_call_ids = Vec::new();
    let mut authoritative_tool_result_outputs = BTreeMap::new();
    for (message_index, message) in request.messages.into_iter().enumerate() {
        if message.name.is_some() {
            record_dropped(&mut loss, "messages[].name");
        }
        let role = role_from_wire(&message.role)?;
        let anthropic_parts = anthropic_content_from_extra(&message.extra_content)?;
        let has_anthropic_parts = anthropic_parts.is_some();
        if matches!(role, Role::System | Role::Developer) {
            for part in anthropic_parts
                .clone()
                .unwrap_or(chat_content_to_canonical(message.content)?)
            {
                match part {
                    CanonicalContent::Text { text } => instructions.push(text),
                    _ => record_dropped(&mut loss, "system/developer message non-text content"),
                }
            }
            if anthropic_parts.is_none() && message.reasoning_content.is_some() {
                record_dropped(&mut loss, "system/developer reasoning_content");
            }
            if anthropic_parts.is_none() && !message.tool_calls.is_empty() {
                record_dropped(&mut loss, "system/developer tool_calls");
            }
            continue;
        }

        if role == Role::Tool {
            let authoritative_duplicate = message
                .tool_call_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .and_then(|id| {
                    authoritative_tool_result_outputs
                        .get(id)
                        .map(|expected| (id, expected))
                });
            if let Some((id, expected)) = authoritative_duplicate {
                let actual = chat_tool_output_to_json(message.content.clone())?;
                let equivalent = &actual == expected
                    || expected
                        .compact_string()
                        .is_ok_and(|serialized| actual == JsonData::String(serialized));
                if !equivalent {
                    return Err(RelayConvertError::Unsupported(format!(
                        "OpenAI tool result {id:?} conflicts with authoritative Anthropic extension"
                    )));
                }
                loss.normalized_fields
                    .push("duplicate native tool result omitted after Anthropic extension");
                continue;
            }
            let id = match message.tool_call_id {
                Some(id) if !id.is_empty() => {
                    observed_call_ids.retain(|candidate| candidate != &id);
                    id
                }
                _ => {
                    if observed_call_ids.len() == 1 {
                        observed_call_ids.remove(0)
                    } else {
                        record_synthetic(&mut loss, "SYNTHETIC_TOOL_CALL_ID");
                        synthetic_tool_result_id(message_index)
                    }
                }
            };
            if let Some(mut parts) = anthropic_parts.clone() {
                let first_extension_result_id = parts.iter().find_map(|part| {
                    if let CanonicalContent::ToolResult { id, .. } = part {
                        Some(id.as_str())
                    } else {
                        None
                    }
                });
                if let Some(extension_id) = first_extension_result_id {
                    if message.tool_call_id.as_deref() != Some(extension_id) {
                        return Err(RelayConvertError::Unsupported(
                            "native tool_call_id disagrees with authoritative Anthropic tool result"
                                .to_owned(),
                        ));
                    }
                }
                for part in &parts {
                    if let CanonicalContent::ToolResult { id, output, .. } = part {
                        if authoritative_tool_result_outputs
                            .insert(id.clone(), output.clone())
                            .is_some()
                        {
                            return Err(RelayConvertError::Unsupported(format!(
                                "Anthropic extension repeats tool result id {id:?}"
                            )));
                        }
                    }
                }
                if let Some(state) = provider_state_from_extra(message.extra_content.clone())? {
                    parts.push(CanonicalContent::ProviderState { state });
                }
                messages.push(CanonicalMessage { role, parts });
                continue;
            }
            let output = chat_tool_output_to_json(message.content)?;
            let mut parts = vec![CanonicalContent::ToolResult {
                id,
                name: message.name,
                output,
            }];
            if let Some(state) = provider_state_from_extra(message.extra_content)? {
                parts.push(CanonicalContent::ProviderState { state });
            }
            messages.push(CanonicalMessage { role, parts });
            continue;
        }

        if message
            .extra_content
            .as_ref()
            .and_then(|extra| extra.google.as_ref())
            .is_some()
            && !message.tool_calls.is_empty()
        {
            return Err(RelayConvertError::Unsupported(
                "message-level Google thought signature cannot be associated with tool calls"
                    .to_owned(),
            ));
        }
        let mut parts = anthropic_parts.unwrap_or(chat_content_to_canonical(message.content)?);
        if let Some(state) = provider_state_from_extra(message.extra_content.clone())? {
            parts.push(CanonicalContent::ProviderState { state });
        }
        if !has_anthropic_parts {
            if let Some(reasoning) = message.reasoning_content {
                parts.push(CanonicalContent::Reasoning { text: reasoning });
            }
        }
        if !has_anthropic_parts {
            for (call_index, call) in message.tool_calls.into_iter().enumerate() {
                let id = if call.id.is_empty() {
                    record_synthetic(&mut loss, "SYNTHETIC_TOOL_CALL_ID");
                    let id = synthetic_tool_call_id(message_index, call_index, &call.function.name);
                    observed_call_ids.push(id.clone());
                    id
                } else {
                    let id = call.id;
                    observed_call_ids.push(id.clone());
                    id
                };
                parts.push(CanonicalContent::ToolCall {
                    id,
                    name: call.function.name,
                    arguments: call.function.arguments,
                });
                if let Some(state) = provider_state_from_extra(call.extra_content)? {
                    parts.push(CanonicalContent::ProviderState { state });
                }
            }
        }
        messages.push(CanonicalMessage { role, parts });
    }

    let tools = request
        .tools
        .into_iter()
        .map(|tool| CanonicalTool {
            name: tool.function.name,
            description: tool.function.description,
            input_schema: tool.function.parameters.unwrap_or(JsonData::Null),
            strict: tool.function.strict,
        })
        .collect();
    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(parse_tool_choice)
        .transpose()?;

    Ok(Converted {
        value: CanonicalRequest {
            model: request.model,
            instructions,
            messages,
            max_output_tokens: request.max_completion_tokens.or(request.max_tokens),
            temperature: request.temperature,
            stream: request.stream,
            tools,
            tool_choice,
            options: RequestOptions {
                top_p: request.top_p,
                reasoning_effort: request.reasoning_effort,
                response_format: request.response_format,
                parallel_tool_calls: request.parallel_tool_calls,
                user: request.user,
                store: request.store,
                metadata: request.metadata,
                stream_options: request.stream_options,
                top_logprobs: request.top_logprobs,
                safety_identifier: request.safety_identifier,
                prompt_cache_retention: request.prompt_cache_retention,
                prompt_cache_key: request.prompt_cache_key,
                service_tier: request.service_tier,
                enable_thinking: request.enable_thinking,
                thinking_budget: request.thinking_budget,
            },
        },
        loss,
    })
}

pub fn canonical_request_to_openai_chat(
    request: CanonicalRequest,
) -> Result<Converted<OpenAiChatRequest>, RelayConvertError> {
    let options = request.options;
    let mut loss = LossReport::default();
    let mut messages: Vec<OpenAiChatMessage> = request
        .instructions
        .into_iter()
        .map(|content| OpenAiChatMessage {
            role: "system".to_owned(),
            content: Some(StringOrParts::String(content)),
            reasoning_content: None,
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
            extra_content: None,
        })
        .collect::<Vec<_>>();

    for message in request.messages {
        let message_parts = message.parts;
        let has_anthropic_extension = message_parts.iter().any(|part| {
            matches!(
                part,
                CanonicalContent::ClaudeThinking { .. }
                    | CanonicalContent::RedactedThinking { .. }
            )
        });
        if message.role == Role::Model {
            loss.normalized_fields
                .push("messages[].role model -> assistant");
        }
        if message.role == Role::Tool {
            for part in &message_parts {
                match part {
                    CanonicalContent::Text { .. } => {
                        record_dropped(&mut loss, "messages[].tool.text")
                    }
                    CanonicalContent::Image { .. } => {
                        record_dropped(&mut loss, "messages[].tool.image")
                    }
                    CanonicalContent::ToolCall { .. } => {
                        record_dropped(&mut loss, "messages[].tool.tool_call")
                    }
                    CanonicalContent::Reasoning { .. } => {
                        record_dropped(&mut loss, "messages[].tool.reasoning")
                    }
                    CanonicalContent::ClaudeThinking { .. } => {
                        // A Claude tool round is carried in the explicit
                        // Anthropic extension below; do not silently erase it.
                    }
                    CanonicalContent::RedactedThinking { .. } => {
                        // See the signed thinking arm above.
                    }
                    CanonicalContent::ProviderState { .. } => {
                        record_dropped(&mut loss, "messages[].tool.provider_state")
                    }
                    CanonicalContent::ToolResult { .. } => {}
                }
            }
        }
        let mut text = Vec::new();
        let mut tool_calls: Vec<OpenAiToolCall> = Vec::new();
        let mut reasoning = Vec::new();
        let mut message_extra_content: Option<OpenAiExtraContent> = None;
        let mut previous_was_tool_call = false;
        let mut previous_tool_result_message: Option<usize> = None;
        let mut first_tool_result_message: Option<usize> = None;
        for part in message_parts.iter().cloned() {
            match part {
                CanonicalContent::Text { text: value } => {
                    text.push(OpenAiChatContentPart {
                        kind: "text".to_owned(),
                        text: Some(value),
                        image_url: None,
                    });
                    previous_was_tool_call = false;
                    previous_tool_result_message = None;
                }
                CanonicalContent::Image { url, detail } => {
                    text.push(OpenAiChatContentPart {
                        kind: "image_url".to_owned(),
                        text: None,
                        image_url: Some(match detail {
                            Some(detail) => JsonData::Object(
                                [
                                    ("url".to_owned(), JsonData::String(url)),
                                    ("detail".to_owned(), JsonData::String(detail)),
                                ]
                                .into_iter()
                                .collect(),
                            ),
                            None => JsonData::String(url),
                        }),
                    });
                    previous_was_tool_call = false;
                    previous_tool_result_message = None;
                }
                CanonicalContent::ProviderState { state } => {
                    let extra = provider_state_to_extra(&state)?;
                    if previous_was_tool_call {
                        let Some(tool_call) = tool_calls.last_mut() else {
                            return Err(RelayConvertError::Unsupported(
                                "provider state has no preceding tool call".to_owned(),
                            ));
                        };
                        tool_call.extra_content = Some(merge_extra_content(
                            tool_call.extra_content.take(),
                            extra,
                        ));
                    } else if let Some(index) = previous_tool_result_message {
                        messages[index].extra_content = Some(merge_extra_content(
                            messages[index].extra_content.take(),
                            extra,
                        ));
                    } else {
                        message_extra_content = Some(merge_extra_content(
                            message_extra_content.take(),
                            extra,
                        ));
                    }
                }
                CanonicalContent::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    tool_calls.push(OpenAiToolCall {
                        id,
                        kind: "function".to_owned(),
                        function: OpenAiFunction {
                            name,
                            arguments,
                            description: None,
                            parameters: None,
                            strict: None,
                        },
                        extra_content: None,
                    });
                    previous_was_tool_call = true;
                    previous_tool_result_message = None;
                }
                CanonicalContent::ToolResult { id, name, output } => {
                    messages.push(OpenAiChatMessage {
                        role: "tool".to_owned(),
                        content: Some(StringOrParts::String(output.compact_string()?)),
                        reasoning_content: None,
                        name,
                        tool_call_id: Some(id),
                        tool_calls: Vec::new(),
                        extra_content: None,
                    });
                    let message_index = messages.len() - 1;
                    if first_tool_result_message.is_none() {
                        first_tool_result_message = Some(message_index);
                    }
                    previous_tool_result_message = Some(message_index);
                    previous_was_tool_call = false;
                }
                CanonicalContent::Reasoning { text } => {
                    reasoning.push(text);
                    previous_was_tool_call = false;
                    previous_tool_result_message = None;
                }
                CanonicalContent::ClaudeThinking {
                    thinking,
                    signature,
                    model,
                    provenance,
                } => {
                    let source_model = model.clone();
                    let block = anthropic_thinking_block(thinking, signature, model, provenance);
                    let extra = message_anthropic_extra(vec![block], source_model);
                    if let Some(index) = previous_tool_result_message {
                        messages[index].extra_content = Some(merge_extra_content(
                            messages[index].extra_content.take(),
                            extra,
                        ));
                    } else {
                        message_extra_content = Some(merge_extra_content(
                            message_extra_content.take(),
                            extra,
                        ));
                    }
                    previous_was_tool_call = false;
                }
                CanonicalContent::RedactedThinking {
                    data,
                    model,
                    provenance,
                } => {
                    let block = anthropic_redacted_block(data, model, provenance);
                    let extra = message_anthropic_extra(vec![block], None);
                    if let Some(index) = previous_tool_result_message {
                        messages[index].extra_content = Some(merge_extra_content(
                            messages[index].extra_content.take(),
                            extra,
                        ));
                    } else {
                        message_extra_content = Some(merge_extra_content(
                            message_extra_content.take(),
                            extra,
                        ));
                    }
                    previous_was_tool_call = false;
                }
            }
        }
        if message.role == Role::Tool {
            if has_anthropic_extension {
                let Some(index) = first_tool_result_message.or(previous_tool_result_message) else {
                    return Err(RelayConvertError::Unsupported(
                        "tool message extension has no generated tool result".to_owned(),
                    ));
                };
                set_anthropic_extra(
                    &mut messages[index].extra_content,
                    message_anthropic_extra(
                        canonical_parts_to_anthropic_blocks(&message_parts)?,
                        None,
                    ),
                );
            }
            if !has_anthropic_extension {
                if let Some(extra) = message_extra_content {
                    let Some(index) = first_tool_result_message.or(previous_tool_result_message) else {
                        return Err(RelayConvertError::Unsupported(
                            "tool message extension has no generated tool result".to_owned(),
                        ));
                    };
                    messages[index].extra_content = Some(merge_extra_content(
                        messages[index].extra_content.take(),
                        extra,
                    ));
                }
            }
            continue;
        }
        if has_anthropic_extension {
            set_anthropic_extra(
                &mut message_extra_content,
                message_anthropic_extra(
                    canonical_parts_to_anthropic_blocks(&message_parts)?,
                    None,
                ),
            );
        }
        let content = if text.is_empty() {
            None
        } else if text.len() == 1 && text[0].kind == "text" {
            Some(StringOrParts::String(
                text[0].text.clone().unwrap_or_default(),
            ))
        } else {
            Some(StringOrParts::Parts(text))
        };
        messages.push(OpenAiChatMessage {
            role: role_to_chat(message.role).to_owned(),
            content,
            reasoning_content: (!reasoning.is_empty()).then(|| reasoning.join("\n")),
            name: None,
            tool_call_id: None,
            tool_calls,
            extra_content: message_extra_content,
        });
    }

    let tools = request
        .tools
        .into_iter()
        .map(|tool| OpenAiChatTool {
            kind: "function".to_owned(),
            function: OpenAiFunction {
                name: tool.name,
                arguments: String::new(),
                description: tool.description,
                parameters: Some(tool.input_schema),
                strict: tool.strict,
            },
        })
        .collect();
    Ok(Converted {
        value: OpenAiChatRequest {
            model: request.model,
            messages,
            stream: request.stream,
            max_tokens: None,
            max_completion_tokens: request.max_output_tokens,
            temperature: request.temperature,
            tools,
            tool_choice: request.tool_choice.map(tool_choice_to_chat),
            top_p: options.top_p,
            n: None,
            reasoning_effort: options.reasoning_effort,
            response_format: options.response_format,
            parallel_tool_calls: options.parallel_tool_calls,
            user: options.user,
            store: options.store,
            metadata: options.metadata,
            stream_options: options.stream_options,
            top_logprobs: options.top_logprobs,
            safety_identifier: options.safety_identifier,
            prompt_cache_retention: options.prompt_cache_retention,
            prompt_cache_key: options.prompt_cache_key,
            service_tier: options.service_tier,
            enable_thinking: options.enable_thinking,
            thinking_budget: options.thinking_budget,
        },
        loss,
    })
}

pub fn openai_responses_request_to_canonical(
    request: OpenAiResponsesRequest,
) -> Result<Converted<CanonicalRequest>, RelayConvertError> {
    if request.model.trim().is_empty() {
        return Err(RelayConvertError::Missing("model"));
    }
    preflight_openai_responses_request_for_canonical(&request)?;

    let mut messages = Vec::new();
    let input = match request.input {
        None => Vec::new(),
        Some(ResponsesInput::String(text)) => {
            messages.push(CanonicalMessage {
                role: Role::User,
                parts: vec![CanonicalContent::Text { text }],
            });
            Vec::new()
        }
        Some(ResponsesInput::Items(items)) => items,
        Some(ResponsesInput::Json(_)) => {
            return Err(unsupported_responses_feature(
                "input_shape",
                "input",
                None,
            ));
        }
    };
    for (item_index, item) in input.into_iter().enumerate() {
        match item.kind.as_deref() {
            Some("function_call") => messages.push(CanonicalMessage {
                role: Role::Assistant,
                parts: vec![CanonicalContent::ToolCall {
                    id: item.call_id.ok_or_else(|| {
                        unsupported_responses_feature(
                            "function_call_id",
                            format!("input[{item_index}].call_id"),
                            Some("LOSS_TOOL_CALL_ID"),
                        )
                    })?,
                    name: item
                        .name
                        .ok_or_else(|| {
                            unsupported_responses_feature(
                                "function_call_name",
                                format!("input[{item_index}].name"),
                                None,
                            )
                        })?,
                    arguments: item.arguments.unwrap_or_default(),
                }],
            }),
            Some("function_call_output") => {
                messages.push(CanonicalMessage {
                    role: Role::Tool,
                    parts: vec![CanonicalContent::ToolResult {
                        id: item.call_id.ok_or_else(|| {
                            unsupported_responses_feature(
                                "function_call_output_id",
                                format!("input[{item_index}].call_id"),
                                Some("LOSS_TOOL_CALL_ID"),
                            )
                        })?,
                        name: None,
                        output: item.output.unwrap_or(JsonData::String(String::new())),
                    }],
                });
            }
            Some("message") => messages.push(CanonicalMessage {
                role: role_from_wire(item.role.as_deref().unwrap_or("user"))?,
                parts: responses_content_to_canonical(item.content)?,
            }),
            None => messages.push(CanonicalMessage {
                role: role_from_wire(item.role.as_deref().unwrap_or("user"))?,
                parts: responses_content_to_canonical(item.content)?,
            }),
            Some("custom_tool_call" | "custom_tool_call_output") => {
                return Err(unsupported_responses_feature(
                    "custom_tool",
                    format!("input[{item_index}]"),
                    Some("LOSS_CUSTOM_TOOL"),
                ));
            }
            Some(kind) => {
                return Err(unsupported_responses_feature(
                    format!("responses_input_{kind}"),
                    format!("input[{item_index}]"),
                    None,
                ));
            }
        }
    }
    let tools = request
        .tools
        .into_iter()
        .enumerate()
        .map(|(index, tool)| {
            let name = tool.name.ok_or_else(|| {
                unsupported_responses_feature(
                    "function_tool_name",
                    format!("tools[{index}].name"),
                    None,
                )
            })?;
            Ok(CanonicalTool {
                name,
                description: tool.description,
                input_schema: tool.parameters.unwrap_or(JsonData::Null),
                strict: tool.strict,
            })
        })
        .collect::<Result<Vec<_>, RelayConvertError>>()?;
    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(parse_tool_choice)
        .transpose()?;
    let mut loss = LossReport::default();
    let reasoning_effort = request
        .reasoning
        .as_ref()
        .and_then(|value| value.effort.clone());
    let prompt_cache_key = match request.prompt_cache_key {
        Some(JsonData::String(value)) => Some(value),
        Some(_) => {
            return Err(RelayConvertError::Unsupported(
                "prompt_cache_key must be a string when converting Responses to Chat".to_owned(),
            ));
        }
        None => None,
    };

    Ok(Converted {
        value: CanonicalRequest {
            model: request.model,
            instructions: request.instructions.into_iter().collect(),
            messages,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            stream: request.stream,
            tools,
            tool_choice,
            options: RequestOptions {
                top_p: request.top_p,
                reasoning_effort,
                response_format: responses_text_to_chat_format(request.text),
                parallel_tool_calls: request.parallel_tool_calls,
                user: request.user,
                store: request.store,
                metadata: request.metadata,
                stream_options: request.stream_options,
                top_logprobs: request.top_logprobs,
                safety_identifier: request.safety_identifier,
                prompt_cache_retention: request.prompt_cache_retention,
                prompt_cache_key,
                service_tier: request.service_tier,
                enable_thinking: request.enable_thinking,
                thinking_budget: request.thinking_budget,
            },
        },
        loss,
    })
}

pub fn canonical_request_to_openai_responses(
    request: CanonicalRequest,
) -> Result<Converted<OpenAiResponsesRequest>, RelayConvertError> {
    let options = request.options;
    let mut loss = LossReport::default();
    if !request.instructions.is_empty() {
        loss.normalized_fields
            .push("messages.system -> instructions");
    }
    let mut input = Vec::new();
    for message in request.messages {
        if message.role == Role::Model {
            loss.normalized_fields
                .push("messages[].role model -> assistant");
        }
        let tool_call_count = message
            .parts
            .iter()
            .filter(|part| matches!(part, CanonicalContent::ToolCall { .. }))
            .count();
        let mut content = Vec::new();
        for part in message.parts {
            match part {
                CanonicalContent::Text { text } => content.push(ResponsesContentPart {
                    kind: if message.role == Role::Assistant {
                        "output_text"
                    } else {
                        "input_text"
                    }
                    .to_owned(),
                    text: Some(text),
                    image_url: None,
                    extra: BTreeMap::new(),
                }),
                CanonicalContent::Image { url, detail } => {
                    if detail.is_some() {
                        record_dropped(&mut loss, "messages[].image.detail");
                    }
                    content.push(ResponsesContentPart {
                        kind: "input_image".to_owned(),
                        text: None,
                        image_url: Some(url),
                        extra: BTreeMap::new(),
                    });
                }
                CanonicalContent::ToolCall {
                    id,
                    name,
                    arguments,
                } => input.push(ResponsesInputItem {
                    kind: Some("function_call".to_owned()),
                    role: None,
                    content: None,
                    call_id: Some(id),
                    name: Some(name),
                    arguments: Some(arguments),
                    output: None,
                    extra: BTreeMap::new(),
                }),
                CanonicalContent::ToolResult {
                    id,
                    name: _,
                    output,
                } => {
                    input.push(ResponsesInputItem {
                        kind: Some("function_call_output".to_owned()),
                        role: None,
                        content: None,
                        call_id: Some(id),
                        name: None,
                        arguments: None,
                        output: Some(output),
                        extra: BTreeMap::new(),
                    });
                }
                CanonicalContent::Reasoning { .. } => {
                    record_dropped(&mut loss, "messages[].reasoning_content");
                }
                CanonicalContent::ClaudeThinking { .. } => {
                    record_dropped(&mut loss, "messages[].claude_thinking");
                }
                CanonicalContent::RedactedThinking { .. } => {
                    record_dropped(&mut loss, "messages[].redacted_thinking");
                }
                CanonicalContent::ProviderState { .. } => {
                    record_dropped(&mut loss, "messages[].provider_state");
                }
            }
        }
        if message.role != Role::Tool || !content.is_empty() {
            input.insert(
                input.len().saturating_sub(tool_call_count),
                ResponsesInputItem {
                    kind: None,
                    role: Some(role_to_responses(message.role).to_owned()),
                    content: Some(
                        if content.is_empty()
                            || (content.len() == 1 && content[0].kind == "input_text")
                        {
                            StringOrParts::String(
                                content
                                    .first()
                                    .and_then(|part| part.text.clone())
                                    .unwrap_or_default(),
                            )
                        } else {
                            StringOrParts::Parts(content)
                        },
                    ),
                    call_id: None,
                    name: None,
                    arguments: None,
                    output: None,
                    extra: BTreeMap::new(),
                },
            );
        }
    }
    let tools = request
        .tools
        .into_iter()
        .map(|tool| ResponsesTool {
            kind: "function".to_owned(),
            name: Some(tool.name),
            description: tool.description,
            parameters: Some(tool.input_schema),
            strict: tool.strict,
            extra: BTreeMap::new(),
        })
        .collect();
    Ok(Converted {
        value: OpenAiResponsesRequest {
            model: request.model,
            input: Some(ResponsesInput::Items(input)),
            instructions: (!request.instructions.is_empty())
                .then(|| request.instructions.join("\n\n")),
            max_output_tokens: request.max_output_tokens,
            stream: request.stream,
            temperature: request.temperature,
            tools,
            tool_choice: request.tool_choice.map(tool_choice_to_responses),
            top_p: options.top_p,
            reasoning: options.reasoning_effort.map(|effort| ReasoningConfig {
                effort: Some(effort),
                summary: None,
                mode: None,
                context: None,
                extra: BTreeMap::new(),
            }),
            text: chat_format_to_responses_text(options.response_format),
            parallel_tool_calls: options.parallel_tool_calls,
            user: options.user,
            store: options.store,
            metadata: options.metadata,
            stream_options: options.stream_options,
            top_logprobs: options.top_logprobs,
            safety_identifier: options.safety_identifier,
            prompt_cache_retention: options.prompt_cache_retention,
            prompt_cache_key: options.prompt_cache_key.map(JsonData::String),
            service_tier: options.service_tier,
            enable_thinking: options.enable_thinking,
            thinking_budget: options.thinking_budget,
            conversation: None,
            previous_response_id: None,
            prompt: None,
            context_management: None,
            include: None,
            moderation: None,
            max_tool_calls: None,
            client_metadata: None,
            extra: BTreeMap::new(),
        },
        loss,
    })
}

pub fn openai_chat_response_to_canonical(
    response: OpenAiChatResponse,
) -> Result<Converted<CanonicalResponse>, RelayConvertError> {
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(RelayConvertError::Missing("choices[0]"))?;
    let message = choice.message;
    if message
        .extra_content
        .as_ref()
        .and_then(|extra| extra.google.as_ref())
        .is_some()
        && !message.tool_calls.is_empty()
    {
        return Err(RelayConvertError::Unsupported(
            "message-level Google thought signature cannot be associated with tool calls"
                .to_owned(),
        ));
    }
    let has_anthropic_parts = message.extra_content.as_ref().is_some_and(|extra| {
        extra
            .anthropic
            .as_ref()
            .is_some_and(|anthropic| !anthropic.blocks.is_empty())
    });
    let mut output = anthropic_content_from_extra(&message.extra_content)?
        .unwrap_or(chat_content_to_canonical(message.content)?);
    if let Some(state) = provider_state_from_extra(message.extra_content.clone())? {
        output.push(CanonicalContent::ProviderState { state });
    }
    if !has_anthropic_parts {
        if let Some(reasoning) = message.reasoning_content {
            output.push(CanonicalContent::Reasoning { text: reasoning });
        }
    }
    if !has_anthropic_parts {
        for call in message.tool_calls {
            output.push(CanonicalContent::ToolCall {
                id: call.id,
                name: call.function.name,
                arguments: call.function.arguments,
            });
            if let Some(state) = provider_state_from_extra(call.extra_content)? {
                output.push(CanonicalContent::ProviderState { state });
            }
        }
    }
    Ok(Converted {
        value: CanonicalResponse {
            id: response.id,
            model: response.model,
            created_at: response.created,
            output,
            finish_reason: choice.finish_reason.as_deref().map(finish_reason),
            usage: response.usage.as_ref().map(wire_usage),
        },
        loss: LossReport::default(),
    })
}

pub fn openai_responses_response_to_canonical(
    response: ResponsesResponse,
) -> Result<Converted<CanonicalResponse>, RelayConvertError> {
    preflight_openai_responses_response_for_canonical(&response)?;
    let mut output = Vec::new();
    for (item_index, item) in response.output.iter().enumerate() {
        match item.kind.as_str() {
            "message" => {
                output.extend(
                    item.content
                        .iter()
                        .flatten()
                        .map(|part| CanonicalContent::Text {
                            text: part.text.clone(),
                        }),
                )
            }
            "reasoning" => output.extend(
                item.summary
                    .iter()
                    .chain(item.content.iter())
                    .flatten()
                    .map(|part| CanonicalContent::Reasoning {
                        text: part.text.clone(),
                    }),
            ),
            "function_call" => output.push(CanonicalContent::ToolCall {
                id: item.call_id.clone().ok_or_else(|| {
                    unsupported_responses_feature(
                        "function_call_id",
                        format!("output[{item_index}].call_id"),
                        Some("LOSS_TOOL_CALL_ID"),
                    )
                })?,
                name: item.name.clone().unwrap_or_default(),
                arguments: item.arguments.clone().unwrap_or_default(),
            }),
            kind => {
                return Err(unsupported_responses_feature(
                    format!("responses_output_{kind}"),
                    format!("output[{item_index}]"),
                    None,
                ));
            }
        }
    }
    let reason = match response.status.as_str() {
        "incomplete" => match response
            .incomplete_details
            .as_ref()
            .map(|details| details.reason.as_str())
        {
            Some("content_filter") => FinishReason::ContentFilter,
            Some("max_output_tokens") => FinishReason::Length,
            _ => FinishReason::Other,
        },
        "cancelled" => FinishReason::Cancelled,
        "failed" => FinishReason::Error,
        _ if output
            .iter()
            .any(|part| matches!(part, CanonicalContent::ToolCall { .. })) =>
        {
            FinishReason::ToolCalls
        }
        _ => FinishReason::Stop,
    };
    Ok(Converted {
        value: CanonicalResponse {
            id: response.id,
            model: response.model,
            created_at: response.created_at,
            output,
            finish_reason: Some(reason),
            usage: response.usage.as_ref().map(wire_usage),
        },
        loss: LossReport::default(),
    })
}

/// Pre-scans a Responses response before converting it to Chat.  Native
/// Responses HTTP relays do not call this function: they retain the raw body
/// and therefore preserve future fields and event kinds unchanged.
pub fn preflight_openai_responses_response_to_openai_chat(
    response: &ResponsesResponse,
) -> Result<(), RelayConvertError> {
    if let Some(error) = first_extra_path("", &response.extra) {
        return Err(error);
    }
    for (index, item) in response.output.iter().enumerate() {
        let path = format!("output[{index}]");
        match item.kind.as_str() {
            "message" => {
                if item.summary.is_some() {
                    return Err(unsupported_responses_feature(
                        "message_summary",
                        format!("{path}.summary"),
                        Some("LOSS_OPAQUE_REASONING"),
                    ));
                }
                preflight_responses_output_content(item.content.as_ref(), &path)?;
                if let Some(error) = first_extra_path(&path, &item.extra) {
                    return Err(error);
                }
            }
            "reasoning" => {
                preflight_responses_output_content(item.content.as_ref(), &path)?;
                preflight_responses_output_content(item.summary.as_ref(), &path)?;
                if let Some(error) = first_extra_path(&path, &item.extra) {
                    return Err(error);
                }
            }
            "function_call" => {
                if item.call_id.as_deref().is_none_or(str::is_empty) {
                    return Err(unsupported_responses_feature(
                        "function_call_id",
                        format!("{path}.call_id"),
                        Some("LOSS_TOOL_CALL_ID"),
                    ));
                }
                if item.name.as_deref().is_none_or(str::is_empty) {
                    return Err(unsupported_responses_feature(
                        "function_call_name",
                        format!("{path}.name"),
                        None,
                    ));
                }
                if let Some(error) = first_extra_path(&path, &item.extra) {
                    return Err(error);
                }
            }
            "custom_tool_call" => {
                return Err(unsupported_responses_feature(
                    "custom_tool",
                    path,
                    Some("LOSS_CUSTOM_TOOL"),
                ));
            }
            _ => {
                return Err(unsupported_responses_feature(
                    "unknown_output_item_type",
                    path,
                    None,
                ));
            }
        }
    }
    Ok(())
}

/// Target-aware response preflight counterpart to
/// [`preflight_openai_responses_request_for_target`].
pub fn preflight_openai_responses_response_for_target(
    response: &ResponsesResponse,
    target: Protocol,
) -> Result<(), RelayConvertError> {
    if target == Protocol::OpenAiResponses {
        return Ok(());
    }
    preflight_openai_responses_response_to_openai_chat(response)
        .map_err(|error| retarget_unsupported_error(error, target))
}

/// Provider-neutral IR response preflight with an explicit target label.
pub fn preflight_openai_responses_response_for_canonical(
    response: &ResponsesResponse,
) -> Result<(), RelayConvertError> {
    preflight_openai_responses_response_to_openai_chat(response)
        .map_err(|error| retarget_unsupported_error_format(error, "provider_neutral_ir"))
}

fn preflight_responses_output_content(
    content: Option<&Vec<super::ResponsesOutputContent>>,
    path: &str,
) -> Result<(), RelayConvertError> {
    let Some(content) = content else {
        return Ok(());
    };
    for (index, part) in content.iter().enumerate() {
        let part_path = format!("{path}.content[{index}]");
        if !matches!(part.kind.as_str(), "output_text" | "summary_text" | "text") {
            return Err(unsupported_responses_feature(
                "unknown_output_content_part",
                part_path,
                None,
            ));
        }
        if part.text.is_none() {
            return Err(unsupported_responses_feature(
                "text_content",
                format!("{part_path}.text"),
                None,
            ));
        }
        if let Some(error) = first_extra_path(&part_path, &part.extra) {
            return Err(error);
        }
    }
    Ok(())
}

pub fn canonical_response_to_openai_chat(
    response: CanonicalResponse,
) -> Converted<OpenAiChatResponse> {
    let mut text = String::new();
    let mut reasoning = Vec::new();
    let mut tool_calls: Vec<OpenAiToolCall> = Vec::new();
    let mut message_extra_content = None;
    let mut previous_was_tool_call = false;
    let mut loss = LossReport::default();
    let output_parts = response.output;
    let has_anthropic_extension = output_parts.iter().any(|part| {
        matches!(
            part,
            CanonicalContent::ClaudeThinking { .. } | CanonicalContent::RedactedThinking { .. }
        )
    });
    for part in output_parts.iter().cloned() {
        match part {
            CanonicalContent::Text { text: part } => {
                text.push_str(&part);
                previous_was_tool_call = false;
            }
            CanonicalContent::Reasoning { text } => {
                reasoning.push(text);
                previous_was_tool_call = false;
            }
            CanonicalContent::ClaudeThinking {
                thinking,
                signature,
                model,
                provenance,
            } => {
                let block = anthropic_thinking_block(thinking, signature, model, provenance);
                message_extra_content = Some(merge_extra_content(
                    message_extra_content.take(),
                    message_anthropic_extra(vec![block], None),
                ));
                previous_was_tool_call = false;
            }
            CanonicalContent::RedactedThinking {
                data,
                model,
                provenance,
            } => {
                let block = anthropic_redacted_block(data, model, provenance);
                message_extra_content = Some(merge_extra_content(
                    message_extra_content.take(),
                    message_anthropic_extra(vec![block], None),
                ));
                previous_was_tool_call = false;
            }
            CanonicalContent::ProviderState { state } => {
                let extra = match provider_state_to_extra(&state) {
                    Ok(extra) => extra,
                    Err(error) => {
                        record_dropped(&mut loss, "output[].provider_state");
                        let _ = error;
                        continue;
                    }
                };
                if previous_was_tool_call {
                    let Some(tool_call) = tool_calls.last_mut() else {
                        record_dropped(&mut loss, "output[].provider_state");
                        continue;
                    };
                    tool_call.extra_content = Some(merge_extra_content(
                        tool_call.extra_content.take(),
                        extra,
                    ));
                } else {
                    message_extra_content = Some(merge_extra_content(
                        message_extra_content.take(),
                        extra,
                    ));
                }
            }
            CanonicalContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                tool_calls.push(OpenAiToolCall {
                    id,
                    kind: "function".to_owned(),
                    function: OpenAiFunction {
                        name,
                        arguments,
                        description: None,
                        parameters: None,
                        strict: None,
                    },
                    extra_content: None,
                });
                previous_was_tool_call = true;
            }
            CanonicalContent::Image { .. } => {
                record_dropped(&mut loss, "output[].image");
            }
            CanonicalContent::ToolResult { .. } => {
                record_dropped(&mut loss, "output[].tool_result");
                previous_was_tool_call = false;
            }
        }
    }
    if has_anthropic_extension {
        match canonical_parts_to_anthropic_blocks(&output_parts) {
            Ok(blocks) => set_anthropic_extra(
                &mut message_extra_content,
                message_anthropic_extra(blocks, None),
            ),
            Err(_) => record_dropped(&mut loss, "output[].anthropic_extension"),
        }
    }
    Converted {
        value: OpenAiChatResponse {
            id: response.id,
            model: response.model,
            object: "chat.completion".to_owned(),
            created: response.created_at,
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiChatMessage {
                    role: "assistant".to_owned(),
                    content: Some(StringOrParts::String(text)),
                    reasoning_content: (!reasoning.is_empty()).then(|| reasoning.join("\n")),
                    name: None,
                    tool_call_id: None,
                    tool_calls,
                    extra_content: message_extra_content,
                },
                finish_reason: response.finish_reason.map(finish_reason_to_chat),
            }],
            usage: response.usage.as_ref().map(token_usage_to_wire),
        },
        loss,
    }
}

pub fn canonical_response_to_openai_responses(
    response: CanonicalResponse,
) -> Converted<ResponsesResponse> {
    let finish_reason = response.finish_reason;
    let item_status = if matches!(
        finish_reason,
        Some(FinishReason::Length | FinishReason::ContentFilter)
    ) {
        "incomplete"
    } else {
        "completed"
    };
    let mut output = Vec::new();
    let mut loss = LossReport::default();
    let mut message_index = 0_usize;
    let mut reasoning_index = 0_usize;
    for part in response.output {
        match part {
            CanonicalContent::Text { text } => {
                output.push(ResponsesOutputItem {
                    kind: "message".to_owned(),
                    id: format!("{}_msg_{message_index}", response.id),
                    status: item_status.to_owned(),
                    role: "assistant".to_owned(),
                    content: Some(vec![ResponsesOutputContent {
                        kind: "output_text".to_owned(),
                        text,
                        annotations: Some(Vec::new()),
                        extra: BTreeMap::new(),
                    }]),
                    summary: None,
                    quality: String::new(),
                    size: String::new(),
                    call_id: None,
                    name: None,
                    arguments: None,
                    extra: BTreeMap::new(),
                });
                message_index += 1;
            }
            CanonicalContent::Reasoning { text } => {
                output.push(ResponsesOutputItem {
                    kind: "reasoning".to_owned(),
                    id: format!("{}_reasoning_{reasoning_index}", response.id),
                    status: item_status.to_owned(),
                    role: String::new(),
                    content: Some(vec![ResponsesOutputContent {
                        kind: "summary_text".to_owned(),
                        text,
                        annotations: None,
                        extra: BTreeMap::new(),
                    }]),
                    summary: None,
                    quality: String::new(),
                    size: String::new(),
                    call_id: None,
                    name: None,
                    arguments: None,
                    extra: BTreeMap::new(),
                });
                reasoning_index += 1;
            }
            CanonicalContent::ClaudeThinking { .. } => {
                record_dropped(&mut loss, "output[].claude_thinking");
            }
            CanonicalContent::RedactedThinking { .. } => {
                record_dropped(&mut loss, "output[].redacted_thinking");
            }
            CanonicalContent::ToolCall {
                id,
                name,
                arguments,
            } => output.push(ResponsesOutputItem {
                kind: "function_call".to_owned(),
                id: id.clone(),
                status: item_status.to_owned(),
                role: String::new(),
                content: None,
                summary: None,
                quality: String::new(),
                size: String::new(),
                call_id: Some(id),
                name: Some(name),
                arguments: Some(arguments),
                extra: BTreeMap::new(),
            }),
            CanonicalContent::Image { .. } => {
                record_dropped(&mut loss, "output[].image");
            }
            CanonicalContent::ToolResult { .. } => {
                record_dropped(&mut loss, "output[].tool_result");
            }
            CanonicalContent::ProviderState { .. } => {
                record_dropped(&mut loss, "output[].provider_state");
            }
        }
    }
    Converted {
        value: ResponsesResponse {
            id: response.id,
            object: "response".to_owned(),
            created_at: response.created_at,
            status: match finish_reason {
                Some(FinishReason::Cancelled) => "cancelled",
                Some(FinishReason::Error) => "failed",
                Some(FinishReason::Length | FinishReason::ContentFilter) => "incomplete",
                _ => "completed",
            }
            .to_owned(),
            instructions: None,
            max_output_tokens: 0,
            model: response.model,
            output,
            parallel_tool_calls: false,
            previous_response_id: None,
            reasoning: None,
            store: false,
            temperature: 0.0,
            tool_choice: None,
            tools: None,
            top_p: 0.0,
            truncation: None,
            usage: response.usage.as_ref().map(token_usage_to_wire),
            user: None,
            metadata: None,
            incomplete_details: match finish_reason {
                Some(FinishReason::Length) => Some(super::IncompleteDetails {
                    reason: "max_output_tokens".to_owned(),
                }),
                Some(FinishReason::ContentFilter) => Some(super::IncompleteDetails {
                    reason: "content_filter".to_owned(),
                }),
                _ => None,
            },
            error: None,
            extra: BTreeMap::new(),
        },
        loss,
    }
}

pub fn openai_chat_response_to_responses(
    response: OpenAiChatResponse,
) -> Result<Converted<ResponsesResponse>, RelayConvertError> {
    let canonical = openai_chat_response_to_canonical(response)?;
    let mut target = canonical_response_to_openai_responses(canonical.value);
    // The legacy Chat -> Responses converter did not propagate Chat's
    // `created` timestamp into the Responses DTO.
    target.value.created_at = 0;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(canonical.loss, target.loss),
    })
}

pub fn openai_responses_response_to_chat(
    response: ResponsesResponse,
) -> Result<Converted<OpenAiChatResponse>, RelayConvertError> {
    let canonical = openai_responses_response_to_canonical(response)?;
    let target = canonical_response_to_openai_chat(canonical.value);
    Ok(Converted {
        value: target.value,
        loss: merge_loss(canonical.loss, target.loss),
    })
}

pub fn openai_stream_to_canonical(snapshot: &OpenAiStreamSnapshot) -> Vec<CanonicalStreamEvent> {
    let mut out = Vec::new();
    let mut started = false;
    let mut response_id = String::new();
    let mut next_output_index = 0_usize;
    let mut text_output_index = None;
    let mut reasoning_output_index = None;
    let mut tool_output_indices = BTreeMap::new();
    let mut closed = BTreeSet::new();
    let mut response_ended = false;
    for chunk in &snapshot.events {
        if !started && (!chunk.id.is_empty() || !chunk.model.is_empty()) {
            response_id.clone_from(&chunk.id);
            out.push(CanonicalStreamEvent::ResponseStart {
                id: chunk.id.clone(),
                model: chunk.model.clone(),
            });
            started = true;
        }
        if let Some(error) = &chunk.error {
            out.push(CanonicalStreamEvent::Error {
                code: error.code.clone(),
                message: error.message.clone(),
            });
        }
        if chunk.cancelled {
            out.push(CanonicalStreamEvent::Cancelled);
        }
        for choice in &chunk.choices {
            if let Some(reasoning) = &choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                let index = *reasoning_output_index.get_or_insert_with(|| {
                    let index = next_output_index;
                    next_output_index += 1;
                    out.push(CanonicalStreamEvent::ContentStart {
                        index,
                        kind: StreamContentKind::Reasoning,
                    });
                    index
                });
                out.push(CanonicalStreamEvent::ReasoningDelta {
                    index,
                    delta: reasoning.clone(),
                });
            }
            if let Some(content) = &choice.delta.content
                && !content.is_empty()
            {
                let index = *text_output_index.get_or_insert_with(|| {
                    let index = next_output_index;
                    next_output_index += 1;
                    out.push(CanonicalStreamEvent::ContentStart {
                        index,
                        kind: StreamContentKind::Text,
                    });
                    index
                });
                out.push(CanonicalStreamEvent::TextDelta {
                    index,
                    delta: content.clone(),
                });
            }
            for call in &choice.delta.tool_calls {
                let (index, is_new) = match tool_output_indices.get(&call.index) {
                    Some(index) => (*index, false),
                    None => {
                        let index = next_output_index;
                        next_output_index += 1;
                        tool_output_indices.insert(call.index, index);
                        out.push(CanonicalStreamEvent::ContentStart {
                            index,
                            kind: StreamContentKind::ToolCall,
                        });
                        (index, true)
                    }
                };
                if is_new {
                    out.push(CanonicalStreamEvent::ToolCallStart {
                        index,
                        id: call
                            .id
                            .clone()
                            .filter(|id| !id.trim().is_empty())
                            .unwrap_or_else(|| format!("{response_id}_call_{}", call.index)),
                        name: call
                            .function
                            .as_ref()
                            .and_then(|function| function.name.clone())
                            .unwrap_or_default(),
                    });
                }
                if let Some(delta) = call
                    .function
                    .as_ref()
                    .and_then(|function| function.arguments.clone())
                {
                    out.push(CanonicalStreamEvent::ToolArgumentsDelta { index, delta });
                }
            }
            if let Some(reason) = &choice.finish_reason
                && !response_ended
            {
                if let Some(index) = text_output_index.filter(|index| closed.insert(*index)) {
                    out.push(CanonicalStreamEvent::ContentEnd { index });
                }
                if let Some(index) = reasoning_output_index.filter(|index| closed.insert(*index)) {
                    out.push(CanonicalStreamEvent::ContentEnd { index });
                }
                for index in tool_output_indices.values().copied() {
                    if closed.insert(index) {
                        out.push(CanonicalStreamEvent::ContentEnd { index });
                    }
                }
                out.push(CanonicalStreamEvent::ResponseEnd {
                    finish_reason: finish_reason(reason),
                    usage: chunk
                        .usage
                        .as_ref()
                        .map(wire_usage)
                        .or_else(|| Some(wire_usage(&snapshot.usage))),
                    model: (!chunk.model.is_empty()).then(|| chunk.model.clone()),
                });
                response_ended = true;
            }
        }
        if chunk.choices.is_empty() && chunk.usage.is_some() {
            // OpenAI's `stream_options.include_usage` emits a trailing usage-only
            // chunk. The usage is already attached to the preceding ResponseEnd.
        }
    }
    out
}

pub fn responses_stream_to_canonical(
    snapshot: &ResponsesStreamSnapshot,
) -> Vec<CanonicalStreamEvent> {
    let mut out = Vec::new();
    let mut ended = BTreeSet::new();
    let mut saw_tool_call = false;
    for event in &snapshot.events {
        let payload = &event.payload;
        match payload.kind.as_str() {
            "response.created" => {
                if let Some(response) = &payload.response {
                    out.push(CanonicalStreamEvent::ResponseStart {
                        id: response.id.clone(),
                        model: response.model.clone(),
                    });
                }
            }
            "response.output_item.added" => {
                if let Some(item) = &payload.item {
                    let index = payload.output_index.unwrap_or_default();
                    let kind = match item.kind.as_str() {
                        "function_call" | "custom_tool_call" => StreamContentKind::ToolCall,
                        "reasoning" => StreamContentKind::Reasoning,
                        _ => StreamContentKind::Text,
                    };
                    out.push(CanonicalStreamEvent::ContentStart { index, kind });
                    if matches!(item.kind.as_str(), "function_call" | "custom_tool_call") {
                        saw_tool_call = true;
                        out.push(CanonicalStreamEvent::ToolCallStart {
                            index,
                            id: item.call_id.clone().unwrap_or_else(|| item.id.clone()),
                            name: item.name.clone().unwrap_or_default(),
                        });
                    }
                }
            }
            "response.output_text.delta" => out.push(CanonicalStreamEvent::TextDelta {
                index: payload.output_index.unwrap_or_default(),
                delta: payload.delta.clone().unwrap_or_default(),
            }),
            "response.function_call_arguments.delta" => {
                out.push(CanonicalStreamEvent::ToolArgumentsDelta {
                    index: payload.output_index.unwrap_or_default(),
                    delta: payload.delta.clone().unwrap_or_default(),
                });
            }
            "response.custom_tool_call_input.delta" => {
                out.push(CanonicalStreamEvent::ToolArgumentsDelta {
                    index: payload.output_index.unwrap_or_default(),
                    delta: payload.delta.clone().unwrap_or_default(),
                });
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                out.push(CanonicalStreamEvent::ReasoningDelta {
                    index: payload.output_index.unwrap_or_default(),
                    delta: payload.delta.clone().unwrap_or_default(),
                });
            }
            "response.output_text.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.done"
            | "response.function_call_arguments.done"
            | "response.custom_tool_call_input.done"
            | "response.output_item.done" => {
                let index = payload.output_index.unwrap_or_default();
                if ended.insert(index) {
                    out.push(CanonicalStreamEvent::ContentEnd { index });
                }
            }
            "response.completed" | "response.done" | "response.incomplete" => {
                let response = payload.response.as_ref();
                let finish_reason = if payload.kind == "response.incomplete" {
                    incomplete_finish_reason(response)
                } else if saw_tool_call
                    || response.is_some_and(|value| {
                        value.output.iter().any(|item| {
                            matches!(item.kind.as_str(), "function_call" | "custom_tool_call")
                        })
                    })
                {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                };
                out.push(CanonicalStreamEvent::ResponseEnd {
                    finish_reason,
                    usage: payload
                        .response
                        .as_ref()
                        .and_then(|response| response.usage.as_ref())
                        .map(wire_usage)
                        .or_else(|| Some(wire_usage(&snapshot.usage))),
                    model: response
                        .filter(|response| !response.model.is_empty())
                        .map(|response| response.model.clone()),
                });
            }
            "response.failed" | "response.error" => {
                let error = payload.error.as_ref().or_else(|| {
                    payload
                        .response
                        .as_ref()
                        .and_then(|response| response.error.as_ref())
                });
                out.push(CanonicalStreamEvent::Error {
                    code: error.and_then(|value| value.code.clone()),
                    message: error.map_or_else(
                        || "response failed".to_owned(),
                        |value| value.message.clone(),
                    ),
                });
            }
            "response.cancelled" => out.push(CanonicalStreamEvent::Cancelled),
            _ => {}
        }
    }
    out
}

/// Gemini model families whose tool-loop signature requirements differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeminiModelFamily {
    /// Gemini 2.5 accepts the documented synthetic history signature.
    Gemini25,
    /// Gemini 3 requires an authentic or explicitly synthetic signature on
    /// the first functionCall Part of each tool-loop step sent as history.
    Gemini3,
    /// A model that is not covered by the known Gemini signature policy.
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeminiSignaturePolicy {
    Request { synthetic_history: bool },
    Response,
}

/// Classifies a Gemini model name for signature policy decisions.
pub fn gemini_model_family(model: &str) -> GeminiModelFamily {
    let model = model.to_ascii_lowercase();
    if model.contains("gemini-3") {
        GeminiModelFamily::Gemini3
    } else if model.contains("gemini-2.5") || model.contains("gemini_2_5") {
        GeminiModelFamily::Gemini25
    } else {
        GeminiModelFamily::Unknown
    }
}

/// Converts a Gemini GenerateContent request into the provider-neutral model.
///
/// Gemini does not carry its model in the JSON body, so callers that know the
/// route model should use [`gemini_request_to_canonical_for_model`].
pub fn gemini_request_to_canonical(
    request: GeminiRequest,
) -> Result<Converted<CanonicalRequest>, RelayConvertError> {
    gemini_request_to_canonical_for_model(request, "gemini")
}

/// Converts a Gemini request into canonical content while retaining IDs and
/// thought signatures at their original Part positions.
pub fn gemini_request_to_canonical_for_model(
    request: GeminiRequest,
    model: &str,
) -> Result<Converted<CanonicalRequest>, RelayConvertError> {
    let mut loss = LossReport::default();
    let mut messages = Vec::new();
    let mut ids = GeminiIdAllocator::default();
    if let Some(system) = request.system_instruction {
        let parts = gemini_parts_to_canonical(system.parts, model, &mut loss, &mut ids)?;
        messages.push(CanonicalMessage {
            role: Role::System,
            parts,
        });
    }
    for content in request.contents {
        let parts = gemini_parts_to_canonical(content.parts, model, &mut loss, &mut ids)?;
        let role = content
            .role
            .as_deref()
            .map(role_from_wire)
            .transpose()?
            .unwrap_or(Role::User);
        if parts
            .iter()
            .any(|part| matches!(part, CanonicalContent::ToolResult { .. }))
        {
            messages.push(CanonicalMessage {
                role: Role::Tool,
                parts,
            });
        } else {
            if parts.is_empty() && content.role.is_some() {
                loss.normalized_fields
                    .push("contents[].empty_parts_preserved_as_empty_message");
            }
            messages.push(CanonicalMessage { role, parts });
        }
    }
    let tools = request
        .tools
        .into_iter()
        .flat_map(|tool| tool.function_declarations)
        .map(|tool| CanonicalTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.parameters,
            strict: None,
        })
        .collect();
    let tool_choice = request
        .tool_config
        .as_ref()
        .map(
            |config| match config.function_calling_config.mode.as_str() {
                "AUTO" => Ok(CanonicalToolChoice::Auto),
                "ANY" => Ok(CanonicalToolChoice::Required),
                "NONE" => Ok(CanonicalToolChoice::None),
                mode => Err(RelayConvertError::Unsupported(format!(
                    "unsupported Gemini function calling mode {mode:?}"
                ))),
            },
        )
        .transpose()?;
    if !request.safety_settings.is_empty() {
        record_dropped(&mut loss, "safetySettings");
    }
    Ok(Converted {
        value: CanonicalRequest {
            model: model.to_owned(),
            instructions: Vec::new(),
            messages,
            max_output_tokens: request
                .generation_config
                .as_ref()
                .and_then(|config| config.max_output_tokens),
            temperature: request
                .generation_config
                .as_ref()
                .and_then(|config| config.temperature),
            stream: false,
            tools,
            tool_choice,
            options: RequestOptions::default(),
        },
        loss,
    })
}

/// Converts a canonical request to Gemini using a conservative, no-synthetic
/// default.  OpenAI history should use
/// [`openai_chat_request_to_gemini_for_model`], which opts into the documented
/// Gemini 2.5 compatibility dummy and Gemini 3 validation.
pub fn canonical_request_to_gemini(
    request: CanonicalRequest,
) -> Result<Converted<GeminiRequest>, RelayConvertError> {
    let model = request.model.clone();
    canonical_request_to_gemini_for_model(request, &model, false)
}

/// Converts canonical content into a Gemini request and applies the model's
/// tool-loop signature policy before the request can reach the upstream.
pub fn canonical_request_to_gemini_for_model(
    request: CanonicalRequest,
    model: &str,
    synthetic_history: bool,
) -> Result<Converted<GeminiRequest>, RelayConvertError> {
    let family = gemini_model_family(model);
    let mut loss = LossReport::default();
    let mut system_parts = request
        .instructions
        .into_iter()
        .map(|text| GeminiPart {
            text: Some(text),
            inline_data: None,
            function_call: None,
            function_response: None,
            thought_signature: None,
        })
        .collect::<Vec<_>>();
    let mut contents = Vec::new();
    let mut call_names = BTreeMap::new();
    for message in request.messages {
        if matches!(message.role, Role::System | Role::Developer) {
            append_canonical_gemini_parts(
                &mut system_parts,
                &message.parts,
                model,
                family,
                GeminiSignaturePolicy::Request { synthetic_history },
                &mut loss,
                &mut call_names,
            )?;
            continue;
        }
        let role = if message.role == Role::Tool {
            "user"
        } else if matches!(message.role, Role::Assistant | Role::Model) {
            "model"
        } else {
            "user"
        };
        let mut parts = Vec::new();
        append_canonical_gemini_parts(
            &mut parts,
            &message.parts,
            model,
            family,
            GeminiSignaturePolicy::Request { synthetic_history },
            &mut loss,
            &mut call_names,
        )?;
        contents.push(GeminiContent {
            role: Some(role.to_owned()),
            parts,
        });
    }
    let declarations = request
        .tools
        .into_iter()
        .map(|tool| GeminiFunctionDeclaration {
            name: tool.name,
            description: tool.description,
            parameters: tool.input_schema,
        })
        .collect::<Vec<_>>();
    let tools = if declarations.is_empty() {
        Vec::new()
    } else {
        vec![GeminiTool {
            function_declarations: declarations,
        }]
    };
    let tool_config = request.tool_choice.map(|choice| GeminiToolConfig {
        function_calling_config: GeminiFunctionCallingConfig {
            mode: match choice {
                CanonicalToolChoice::Auto => "AUTO",
                CanonicalToolChoice::None => "NONE",
                CanonicalToolChoice::Required | CanonicalToolChoice::Function { .. } => "ANY",
            }
            .to_owned(),
        },
    });
    Ok(Converted {
        value: GeminiRequest {
            contents,
            system_instruction: (!system_parts.is_empty()).then_some(GeminiContent {
                role: None,
                parts: system_parts,
            }),
            generation_config: (request.max_output_tokens.is_some()
                || request.temperature.is_some())
            .then_some(GeminiGenerationConfig {
                max_output_tokens: request.max_output_tokens,
                temperature: request.temperature,
            }),
            safety_settings: Vec::new(),
            tools,
            tool_config,
        },
        loss,
    })
}

/// Converts an OpenAI Chat request to Gemini with explicit target model
/// policy.  IDs remain exact; missing IDs receive deterministic synthetic IDs.
pub fn openai_chat_request_to_gemini_for_model(
    request: OpenAiChatRequest,
    model: &str,
) -> Result<Converted<GeminiRequest>, RelayConvertError> {
    let source = openai_chat_request_to_canonical(request)?;
    let target = canonical_request_to_gemini_for_model(source.value, model, true)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts an OpenAI Chat request to Gemini.  Use the `_for_model` variant
/// when the upstream model differs from the request's source model.
pub fn openai_chat_request_to_gemini(
    request: OpenAiChatRequest,
) -> Result<Converted<GeminiRequest>, RelayConvertError> {
    let model = request.model.clone();
    openai_chat_request_to_gemini_for_model(request, &model)
}

/// Converts a Gemini request to an OpenAI-compatible Chat request.
pub fn gemini_request_to_openai_chat(
    request: GeminiRequest,
) -> Result<Converted<OpenAiChatRequest>, RelayConvertError> {
    let source = gemini_request_to_canonical(request)?;
    let target = canonical_request_to_openai_chat(source.value)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Alias using the protocol-first naming used by route adapters.
pub fn gemini_to_openai_chat_request(
    request: GeminiRequest,
) -> Result<Converted<OpenAiChatRequest>, RelayConvertError> {
    gemini_request_to_openai_chat(request)
}

/// Alias using the protocol-first naming used by route adapters.
pub fn openai_chat_to_gemini_request(
    request: OpenAiChatRequest,
) -> Result<Converted<GeminiRequest>, RelayConvertError> {
    openai_chat_request_to_gemini(request)
}

/// Converts a Gemini request to OpenAI Chat while retaining the route model
/// in the generated request.
pub fn gemini_request_to_openai_chat_for_model(
    request: GeminiRequest,
    model: &str,
) -> Result<Converted<OpenAiChatRequest>, RelayConvertError> {
    let source = gemini_request_to_canonical_for_model(request, model)?;
    let target = canonical_request_to_openai_chat(source.value)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts a Gemini GenerateContent response into canonical output.
pub fn gemini_response_to_canonical(
    response: GeminiResponse,
) -> Result<Converted<CanonicalResponse>, RelayConvertError> {
    gemini_response_to_canonical_for_model(response, "gemini")
}

/// Converts a Gemini response while associating authentic signatures with
/// their original Parts.
pub fn gemini_response_to_canonical_for_model(
    response: GeminiResponse,
    model: &str,
) -> Result<Converted<CanonicalResponse>, RelayConvertError> {
    let candidate = response
        .candidates
        .into_iter()
        .next()
        .ok_or(RelayConvertError::Missing("candidates[0]"))?;
    let mut loss = LossReport::default();
    let mut ids = GeminiIdAllocator::default();
    let output = gemini_parts_to_canonical(
        candidate.content.parts,
        model,
        &mut loss,
        &mut ids,
    )?;
    Ok(Converted {
        value: CanonicalResponse {
            id: "gemini-response".to_owned(),
            model: model.to_owned(),
            created_at: 0,
            output,
            finish_reason: candidate.finish_reason.as_deref().map(gemini_finish_reason),
            usage: response.usage_metadata.as_ref().map(gemini_usage),
        },
        loss,
    })
}

/// Aggregates incremental Gemini GenerateContent responses into one canonical
/// response.  Each snapshot event is treated as a delta: Parts are consumed
/// in arrival order and a candidate `finishReason` only finalizes the
/// aggregate after all Parts from that event have been retained.  This keeps
/// a late thought signature attached to its original Part instead of
/// finalizing the response early.
pub fn gemini_stream_to_canonical(
    snapshot: &GeminiStreamSnapshot,
    model: &str,
) -> Result<Converted<CanonicalResponse>, RelayConvertError> {
    let mut loss = LossReport::default();
    let mut ids = GeminiIdAllocator::default();
    let mut output = Vec::new();
    let mut finish_reason = None;
    let mut usage = None;
    let mut finished = false;

    for event in &snapshot.events {
        for candidate in &event.candidates {
            if finished && !candidate.content.parts.is_empty() {
                return Err(RelayConvertError::Unsupported(
                    "Gemini stream emitted content after finishReason".to_owned(),
                ));
            }
            output.extend(gemini_parts_to_canonical(
                candidate.content.parts.clone(),
                model,
                &mut loss,
                &mut ids,
            )?);
            if let Some(reason) = candidate.finish_reason.as_deref() {
                finish_reason = Some(gemini_finish_reason(reason));
                finished = true;
            }
        }
        if let Some(event_usage) = event.usage_metadata.as_ref() {
            usage = Some(gemini_usage(event_usage));
        }
    }
    if usage.is_none() && snapshot.usage != WireUsage::default() {
        usage = Some(wire_usage(&snapshot.usage));
    }

    Ok(Converted {
        value: CanonicalResponse {
            id: "gemini-stream-response".to_owned(),
            model: model.to_owned(),
            created_at: 0,
            output,
            finish_reason,
            usage,
        },
        loss,
    })
}

/// Converts canonical response output to a Gemini response candidate.
pub fn canonical_response_to_gemini(
    response: CanonicalResponse,
) -> Result<Converted<GeminiResponse>, RelayConvertError> {
    let mut loss = LossReport::default();
    let mut parts = Vec::new();
    let mut call_names = BTreeMap::new();
    append_canonical_gemini_parts(
        &mut parts,
        &response.output,
        &response.model,
        gemini_model_family(&response.model),
        GeminiSignaturePolicy::Response,
        &mut loss,
        &mut call_names,
    )?;
    Ok(Converted {
        value: GeminiResponse {
            candidates: vec![GeminiCandidate {
                index: Some(0),
                finish_reason: response.finish_reason.map(gemini_finish_reason_to_wire),
                content: GeminiContent {
                    role: Some("model".to_owned()),
                    parts,
                },
                safety_ratings: None,
            }],
            usage_metadata: response.usage.as_ref().map(token_usage_to_gemini),
        },
        loss,
    })
}

/// Converts a Gemini response to OpenAI Chat, preserving call IDs and Google
/// extensions.  Gemini's body has no response ID/model, so deterministic
/// placeholders are used and recorded as normalized metadata.
pub fn gemini_response_to_openai_chat(
    response: GeminiResponse,
) -> Result<Converted<OpenAiChatResponse>, RelayConvertError> {
    let source = gemini_response_to_canonical(response)?;
    let mut target = canonical_response_to_openai_chat(source.value);
    target
        .loss
        .normalized_fields
        .push("gemini.response_id.synthetic");
    target
        .loss
        .normalized_fields
        .push("gemini.model.from_route");
    target.value.id = "gemini-response".to_owned();
    target.value.model = "gemini".to_owned();
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Alias using the protocol-first naming used by route adapters.
pub fn gemini_to_openai_chat_response(
    response: GeminiResponse,
) -> Result<Converted<OpenAiChatResponse>, RelayConvertError> {
    gemini_response_to_openai_chat(response)
}

/// Converts an OpenAI Chat response into a Gemini response.
pub fn openai_chat_response_to_gemini(
    response: OpenAiChatResponse,
) -> Result<Converted<GeminiResponse>, RelayConvertError> {
    let source = openai_chat_response_to_canonical(response)?;
    let target = canonical_response_to_gemini(source.value)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Alias using the protocol-first naming used by route adapters.
pub fn openai_chat_to_gemini_response(
    response: OpenAiChatResponse,
) -> Result<Converted<GeminiResponse>, RelayConvertError> {
    openai_chat_response_to_gemini(response)
}

/// Converts an OpenAI Chat response to Gemini using a known target model.
pub fn openai_chat_response_to_gemini_for_model(
    response: OpenAiChatResponse,
    model: &str,
) -> Result<Converted<GeminiResponse>, RelayConvertError> {
    let source = openai_chat_response_to_canonical(response)?;
    let mut canonical = source.value;
    canonical.model = model.to_owned();
    let target = canonical_response_to_gemini(canonical)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts a Gemini response to OpenAI Chat using the supplied route model
/// for the generated response metadata.
pub fn gemini_response_to_openai_chat_for_model(
    response: GeminiResponse,
    model: &str,
) -> Result<Converted<OpenAiChatResponse>, RelayConvertError> {
    let source = gemini_response_to_canonical_for_model(response, model)?;
    let mut target = canonical_response_to_openai_chat(source.value);
    target.value.model = model.to_owned();
    target
        .loss
        .normalized_fields
        .push("gemini.response_id.synthetic");
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts a Claude Messages request into the provider-neutral model while
/// retaining every ordered content block, including signed and redacted
/// thinking.
pub fn claude_request_to_canonical(
    request: ClaudeRequest,
) -> Result<Converted<CanonicalRequest>, RelayConvertError> {
    let mut instructions = Vec::new();
    let mut messages = Vec::new();
    if let Some(system) = request.system {
        match system {
            StringOrParts::String(text) => instructions.push(text),
            StringOrParts::Parts(parts) => messages.push(CanonicalMessage {
                role: Role::System,
                parts: claude_blocks_to_canonical(parts, Some(&request.model))?,
            }),
        }
    }
    for message in request.messages {
        let role = role_from_wire(&message.role)?;
        let parts = match message.content {
            StringOrParts::String(text) => vec![CanonicalContent::Text { text }],
            StringOrParts::Parts(parts) => {
                claude_blocks_to_canonical(parts, Some(&request.model))?
            }
        };
        let role = if parts
            .iter()
            .any(|part| matches!(part, CanonicalContent::ToolResult { .. }))
        {
            Role::Tool
        } else {
            role
        };
        messages.push(CanonicalMessage {
            role,
            parts,
        });
    }
    let tools = request
        .tools
        .into_iter()
        .map(|tool| CanonicalTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
            strict: None,
        })
        .collect();
    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(claude_tool_choice_to_canonical)
        .transpose()?;
    Ok(Converted {
        value: CanonicalRequest {
            model: request.model,
            instructions,
            messages,
            max_output_tokens: Some(request.max_tokens),
            temperature: request.temperature,
            stream: request.stream,
            tools,
            tool_choice,
            options: RequestOptions::default(),
        },
        loss: LossReport::default(),
    })
}

/// Converts canonical request content to Claude's ordered Messages blocks.
pub fn canonical_request_to_claude(
    request: CanonicalRequest,
) -> Result<Converted<ClaudeRequest>, RelayConvertError> {
    let mut loss = LossReport::default();
    let mut system_parts = Vec::new();
    for text in request.instructions {
        system_parts.push(claude_text_block(text));
    }
    let mut messages = Vec::new();
    for message in request.messages {
        if message.role == Role::System || message.role == Role::Developer {
            system_parts.extend(canonical_parts_to_claude(
                &message.parts,
                &mut loss,
                false,
            )?);
            continue;
        }
        let has_tool_call = message
            .parts
            .iter()
            .any(|part| matches!(part, CanonicalContent::ToolCall { .. }));
        let parts = canonical_parts_to_claude(&message.parts, &mut loss, has_tool_call)?;
        messages.push(ClaudeMessage {
            role: match message.role {
                Role::Model => "assistant".to_owned(),
                Role::Tool => "user".to_owned(),
                role => role_to_chat(role).to_owned(),
            },
            content: StringOrParts::Parts(parts),
        });
    }
    let options = request.options;
    let tools = request
        .tools
        .into_iter()
        .map(|tool| ClaudeTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
        })
        .collect();
    let tool_choice = request
        .tool_choice
        .map(canonical_tool_choice_to_claude);
    Ok(Converted {
        value: ClaudeRequest {
            model: request.model,
            max_tokens: request.max_output_tokens.unwrap_or(1),
            stream: request.stream,
            system: (!system_parts.is_empty()).then_some(StringOrParts::Parts(system_parts)),
            messages,
            temperature: request.temperature,
            tools,
            tool_choice,
        },
        loss,
    })
}

/// Converts a Claude response into canonical ordered output.
pub fn claude_response_to_canonical(
    response: ClaudeResponse,
) -> Result<Converted<CanonicalResponse>, RelayConvertError> {
    let output = claude_blocks_to_canonical(response.content, Some(&response.model))?;
    Ok(Converted {
        value: CanonicalResponse {
            id: response.id,
            model: response.model,
            created_at: 0,
            output,
            finish_reason: response.stop_reason.as_deref().map(finish_reason),
            usage: response.usage.as_ref().map(claude_usage),
        },
        loss: LossReport::default(),
    })
}

/// Converts canonical response output to Claude's typed response blocks.
pub fn canonical_response_to_claude(
    response: CanonicalResponse,
) -> Result<Converted<ClaudeResponse>, RelayConvertError> {
    let mut loss = LossReport::default();
    let content = canonical_parts_to_claude(&response.output, &mut loss, false)?;
    Ok(Converted {
        value: ClaudeResponse {
            id: response.id,
            kind: "message".to_owned(),
            role: "assistant".to_owned(),
            model: response.model,
            content,
            stop_reason: response.finish_reason.map(claude_finish_reason),
            usage: response.usage.as_ref().map(token_usage_to_claude),
        },
        loss,
    })
}

/// Converts Claude request history to OpenAI Chat.  Claude-only blocks are
/// carried in `extra_content.anthropic.blocks` in their exact source order.
pub fn claude_request_to_openai_chat(
    request: ClaudeRequest,
) -> Result<Converted<OpenAiChatRequest>, RelayConvertError> {
    let source = claude_request_to_canonical(request)?;
    let mut target = canonical_request_to_openai_chat(source.value.clone())?;
    attach_anthropic_request_extensions(&source.value, &mut target.value, &mut target.loss)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts OpenAI Chat request history to Claude.  An Anthropic extension is
/// authoritative when present, so signed/redacted blocks are not flattened.
pub fn openai_chat_request_to_claude(
    request: OpenAiChatRequest,
) -> Result<Converted<ClaudeRequest>, RelayConvertError> {
    let source = openai_chat_request_to_canonical(request)?;
    let target = canonical_request_to_claude(source.value)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts a Claude response to OpenAI Chat with an ordered Anthropic
/// extension on the assistant message.
pub fn claude_response_to_openai_chat(
    response: ClaudeResponse,
) -> Result<Converted<OpenAiChatResponse>, RelayConvertError> {
    let source = claude_response_to_canonical(response)?;
    let mut target = canonical_response_to_openai_chat(source.value.clone());
    attach_anthropic_response_extension(&source.value, &mut target.value, &mut target.loss)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Converts an OpenAI Chat response to Claude while honoring the explicit
/// Anthropic extension when clients return Claude history.
pub fn openai_chat_response_to_claude(
    response: OpenAiChatResponse,
) -> Result<Converted<ClaudeResponse>, RelayConvertError> {
    let source = openai_chat_response_to_canonical(response)?;
    let target = canonical_response_to_claude(source.value)?;
    Ok(Converted {
        value: target.value,
        loss: merge_loss(source.loss, target.loss),
    })
}

/// Protocol-first aliases used by adapters that name the source protocol.
pub fn claude_to_openai_chat_request(
    request: ClaudeRequest,
) -> Result<Converted<OpenAiChatRequest>, RelayConvertError> {
    claude_request_to_openai_chat(request)
}

pub fn openai_chat_to_claude_request(
    request: OpenAiChatRequest,
) -> Result<Converted<ClaudeRequest>, RelayConvertError> {
    openai_chat_request_to_claude(request)
}

pub fn claude_to_openai_chat_response(
    response: ClaudeResponse,
) -> Result<Converted<OpenAiChatResponse>, RelayConvertError> {
    claude_response_to_openai_chat(response)
}

pub fn openai_chat_to_claude_response(
    response: OpenAiChatResponse,
) -> Result<Converted<ClaudeResponse>, RelayConvertError> {
    openai_chat_response_to_claude(response)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenValidation {
    pub kind: FixtureKind,
    pub target: Protocol,
    pub event_count: usize,
    pub usage: Option<TokenUsage>,
}

pub fn validate_golden(
    kind: FixtureKind,
    target: Protocol,
    json: &str,
) -> Result<GoldenValidation, RelayConvertError> {
    let (event_count, usage) = match (kind, target) {
        (FixtureKind::Request, Protocol::OpenAi) => {
            serde_json::from_str::<OpenAiChatRequest>(json)?;
            (0, None)
        }
        (FixtureKind::Request, Protocol::OpenAiResponses) => {
            serde_json::from_str::<OpenAiResponsesRequest>(json)?;
            (0, None)
        }
        (FixtureKind::Request, Protocol::Claude) => {
            serde_json::from_str::<ClaudeRequest>(json)?;
            (0, None)
        }
        (FixtureKind::Request, Protocol::Gemini) => {
            serde_json::from_str::<GeminiRequest>(json)?;
            (0, None)
        }
        (FixtureKind::Response, Protocol::OpenAi) => {
            let value = serde_json::from_str::<OpenAiChatResponse>(json)?;
            (0, value.usage.as_ref().map(wire_usage))
        }
        (FixtureKind::Response, Protocol::OpenAiResponses) => {
            let value = serde_json::from_str::<ResponsesResponse>(json)?;
            (0, value.usage.as_ref().map(wire_usage))
        }
        (FixtureKind::Response, Protocol::Claude) => {
            let value = serde_json::from_str::<ClaudeResponse>(json)?;
            (0, value.usage.as_ref().map(claude_usage))
        }
        (FixtureKind::Response, Protocol::Gemini) => {
            let value = serde_json::from_str::<GeminiResponse>(json)?;
            (0, value.usage_metadata.as_ref().map(gemini_usage))
        }
        (FixtureKind::Stream, Protocol::OpenAi) => {
            let value = serde_json::from_str::<OpenAiStreamSnapshot>(json)?;
            (value.events.len(), Some(wire_usage(&value.usage)))
        }
        (FixtureKind::Stream, Protocol::OpenAiResponses) => {
            let value = serde_json::from_str::<ResponsesStreamSnapshot>(json)?;
            (value.events.len(), Some(wire_usage(&value.usage)))
        }
        (FixtureKind::Stream, Protocol::Claude) => {
            let value = serde_json::from_str::<ClaudeStreamSnapshot>(json)?;
            (value.events.len(), Some(wire_usage(&value.usage)))
        }
        (FixtureKind::Stream, Protocol::Gemini) => {
            let value = serde_json::from_str::<GeminiStreamSnapshot>(json)?;
            (value.events.len(), Some(wire_usage(&value.usage)))
        }
    };
    Ok(GoldenValidation {
        kind,
        target,
        event_count,
        usage,
    })
}

fn record_dropped(loss: &mut LossReport, field: &'static str) {
    if !loss.dropped_fields.contains(&field) {
        loss.dropped_fields.push(field);
    }
}

fn record_synthetic(loss: &mut LossReport, field: &'static str) {
    if !loss.synthetic_fields.contains(&field) {
        loss.synthetic_fields.push(field);
    }
}

fn synthetic_tool_call_id(message_index: usize, call_index: usize, name: &str) -> String {
    format!("call_synthetic_{message_index}_{call_index}_{name}")
}

fn synthetic_tool_result_id(message_index: usize) -> String {
    format!("call_result_synthetic_{message_index}")
}

fn provider_state_from_extra(
    extra: Option<OpenAiExtraContent>,
) -> Result<Option<OpaqueProviderState>, RelayConvertError> {
    let Some(extra) = extra else {
        return Ok(None);
    };
    let Some(google) = extra.google else {
        return Ok(None);
    };
    let Some(signature) = google.thought_signature else {
        return Err(RelayConvertError::Missing(
            "extra_content.google.thought_signature",
        ));
    };
    Ok(Some(OpaqueProviderState {
        provider: "google".to_owned(),
        kind: "thought_signature".to_owned(),
        raw: JsonData::String(signature),
        provenance: if google.synthetic.unwrap_or(false) {
            OpaqueStateProvenance::Synthetic
        } else {
            OpaqueStateProvenance::Authentic
        },
        model: None,
    }))
}

fn provider_state_to_extra(
    state: &OpaqueProviderState,
) -> Result<OpenAiExtraContent, RelayConvertError> {
    if state.provider != "google" || state.kind != "thought_signature" {
        return Err(RelayConvertError::Unsupported(format!(
            "provider state {}:{} has no OpenAI-compatible extension",
            state.provider, state.kind
        )));
    }
    let JsonData::String(signature) = &state.raw else {
        return Err(RelayConvertError::Unsupported(
            "Google thought_signature must be a string".to_owned(),
        ));
    };
    Ok(OpenAiExtraContent {
        google: Some(OpenAiGoogleExtraContent {
            thought_signature: Some(signature.clone()),
            synthetic: match state.provenance {
                OpaqueStateProvenance::Authentic => None,
                OpaqueStateProvenance::Synthetic => Some(true),
            },
        }),
        anthropic: None,
    })
}

fn anthropic_content_from_extra(
    extra: &Option<OpenAiExtraContent>,
) -> Result<Option<Vec<CanonicalContent>>, RelayConvertError> {
    let Some(anthropic) = extra.as_ref().and_then(|value| value.anthropic.as_ref()) else {
        return Ok(None);
    };
    if anthropic.blocks.is_empty() {
        return Ok(None);
    }
    anthropic
        .blocks
        .iter()
        .map(|block| anthropic_block_to_canonical(block, anthropic.model.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn validate_anthropic_extension_block_shape(
    block: &OpenAiAnthropicBlock,
) -> Result<(), RelayConvertError> {
    let reject = |field: &str| {
        Err(RelayConvertError::Unsupported(format!(
            "Anthropic extension {} block carries incompatible field {field:?}",
            block.kind
        )))
    };
    match block.kind.as_str() {
        "text" | "reasoning" => {
            if block.text.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].text",
                ));
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.synthetic.is_some() {
                return reject("synthetic");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
            if block.image_url.is_some() {
                return reject("image_url");
            }
        }
        "thinking" => {
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].thinking",
                ));
            }
            if block.signature.is_none() && block.synthetic == Some(true) {
                return reject("synthetic without signature");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
            if block.image_url.is_some() {
                return reject("image_url");
            }
        }
        "redacted_thinking" => {
            if block.data.is_some() && block.content.is_some() {
                return reject("content");
            }
            if block.data.is_none() && block.content.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].data",
                ));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.image_url.is_some() {
                return reject("image_url");
            }
        }
        "tool_use" => {
            if block.id.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].id",
                ));
            }
            if block.name.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].name",
                ));
            }
            if block.input.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].input",
                ));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.synthetic.is_some() {
                return reject("synthetic");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
            if block.image_url.is_some() {
                return reject("image_url");
            }
        }
        "tool_result" => {
            if block.tool_use_id.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].tool_use_id",
                ));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.synthetic.is_some() {
                return reject("synthetic");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.image_url.is_some() {
                return reject("image_url");
            }
        }
        "image" => {
            if block.image_url.is_none() {
                return Err(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].image_url",
                ));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.synthetic.is_some() {
                return reject("synthetic");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
        }
        kind => {
            return Err(RelayConvertError::Unsupported(format!(
                "unsupported Anthropic extension block type {kind:?}"
            )))
        }
    }
    Ok(())
}

fn anthropic_block_to_canonical(
    block: &OpenAiAnthropicBlock,
    model: Option<String>,
) -> Result<CanonicalContent, RelayConvertError> {
    if !block.extra.is_empty() {
        return Err(RelayConvertError::Unsupported(
            "unknown Anthropic extension block fields cannot cross the canonical boundary"
            .to_owned(),
        ));
    }
    validate_anthropic_extension_block_shape(block)?;
    match block.kind.as_str() {
        "text" => Ok(CanonicalContent::Text {
            text: block.text.clone().unwrap_or_default(),
        }),
        "reasoning" => Ok(CanonicalContent::Reasoning {
            text: block.text.clone().unwrap_or_default(),
        }),
        "thinking" => Ok(CanonicalContent::ClaudeThinking {
            thinking: block
                .thinking
                .clone()
                .or_else(|| block.text.clone())
                .unwrap_or_default(),
            signature: block.signature.clone(),
            model,
            provenance: if block.synthetic.unwrap_or(false) {
                OpaqueStateProvenance::Synthetic
            } else {
                OpaqueStateProvenance::Authentic
            },
        }),
        "redacted_thinking" => Ok(CanonicalContent::RedactedThinking {
            data: block
                .data
                .clone()
                .or_else(|| block.content.clone())
                .ok_or(RelayConvertError::Missing(
                    "extra_content.anthropic.blocks[].data",
                ))?,
            model,
            provenance: if block.synthetic.unwrap_or(false) {
                OpaqueStateProvenance::Synthetic
            } else {
                OpaqueStateProvenance::Authentic
            },
        }),
        "tool_use" => Ok(CanonicalContent::ToolCall {
            id: block
                .id
                .clone()
                .ok_or(RelayConvertError::Missing("anthropic tool_use.id"))?,
            name: block
                .name
                .clone()
                .ok_or(RelayConvertError::Missing("anthropic tool_use.name"))?,
            arguments: block
                .input
                .as_ref()
                .ok_or(RelayConvertError::Missing("anthropic tool_use.input"))?
                .compact_string()?,
        }),
        "tool_result" => Ok(CanonicalContent::ToolResult {
            id: block
                .tool_use_id
                .clone()
                .ok_or(RelayConvertError::Missing("anthropic tool_result.tool_use_id"))?,
            name: block.name.clone(),
            output: block.content.clone().unwrap_or(JsonData::Null),
        }),
        "image" => {
            let image = block
                .image_url
                .clone()
                .ok_or(RelayConvertError::Missing("anthropic image.image_url"))?;
            let url = match image {
                JsonData::String(url) => url,
                JsonData::Object(mut object) => match object.remove("url") {
                    Some(JsonData::String(url)) => url,
                    _ => return Err(RelayConvertError::Missing("anthropic image.url")),
                },
                _ => return Err(RelayConvertError::Missing("anthropic image.url")),
            };
            Ok(CanonicalContent::Image { url, detail: None })
        }
        kind => Err(RelayConvertError::Unsupported(format!(
            "unsupported Anthropic extension block type {kind:?}"
        ))),
    }
}

fn anthropic_thinking_block(
    thinking: String,
    signature: Option<String>,
    _model: Option<String>,
    provenance: OpaqueStateProvenance,
) -> OpenAiAnthropicBlock {
    OpenAiAnthropicBlock {
        kind: "thinking".to_owned(),
        text: None,
        thinking: Some(thinking),
        signature,
        synthetic: match provenance {
            OpaqueStateProvenance::Authentic => None,
            OpaqueStateProvenance::Synthetic => Some(true),
        },
        data: None,
        id: None,
        name: None,
        input: None,
        tool_use_id: None,
        content: None,
        image_url: None,
        extra: BTreeMap::new(),
    }
}

fn anthropic_redacted_block(
    data: JsonData,
    _model: Option<String>,
    provenance: OpaqueStateProvenance,
) -> OpenAiAnthropicBlock {
    OpenAiAnthropicBlock {
        kind: "redacted_thinking".to_owned(),
        text: None,
        thinking: None,
        signature: None,
        synthetic: match provenance {
            OpaqueStateProvenance::Authentic => None,
            OpaqueStateProvenance::Synthetic => Some(true),
        },
        data: Some(data),
        id: None,
        name: None,
        input: None,
        tool_use_id: None,
        content: None,
        image_url: None,
        extra: BTreeMap::new(),
    }
}

fn message_anthropic_extra(
    blocks: Vec<OpenAiAnthropicBlock>,
    model: Option<String>,
) -> OpenAiExtraContent {
    OpenAiExtraContent {
        google: None,
        anthropic: Some(OpenAiAnthropicExtraContent { blocks, model }),
    }
}

fn merge_extra_content(
    existing: Option<OpenAiExtraContent>,
    incoming: OpenAiExtraContent,
) -> OpenAiExtraContent {
    let mut merged = existing.unwrap_or(OpenAiExtraContent {
        google: None,
        anthropic: None,
    });
    if incoming.google.is_some() {
        merged.google = incoming.google;
    }
    if let Some(incoming_anthropic) = incoming.anthropic {
        if let Some(existing_anthropic) = merged.anthropic.as_mut() {
            existing_anthropic.blocks.extend(incoming_anthropic.blocks);
            if incoming_anthropic.model.is_some() {
                existing_anthropic.model = incoming_anthropic.model;
            }
        } else {
            merged.anthropic = Some(incoming_anthropic);
        }
    }
    merged
}

fn canonical_part_to_anthropic_block(
    part: &CanonicalContent,
) -> Result<OpenAiAnthropicBlock, RelayConvertError> {
    let empty = || OpenAiAnthropicBlock {
        kind: String::new(),
        text: None,
        thinking: None,
        signature: None,
        synthetic: None,
        data: None,
        id: None,
        name: None,
        input: None,
        tool_use_id: None,
        content: None,
        image_url: None,
        extra: BTreeMap::new(),
    };
    let block = match part {
        CanonicalContent::Text { text } => OpenAiAnthropicBlock {
            kind: "text".to_owned(),
            text: Some(text.clone()),
            ..empty()
        },
        CanonicalContent::Image { url, detail } => OpenAiAnthropicBlock {
            kind: "image".to_owned(),
            image_url: Some(match detail {
                Some(detail) => JsonData::Object(
                    [("url".to_owned(), JsonData::String(url.clone())),
                        ("detail".to_owned(), JsonData::String(detail.clone()))]
                        .into_iter()
                        .collect(),
                ),
                None => JsonData::String(url.clone()),
            }),
            ..empty()
        },
        CanonicalContent::ToolCall {
            id,
            name,
            arguments,
        } => OpenAiAnthropicBlock {
            kind: "tool_use".to_owned(),
            id: Some(id.clone()),
            name: Some(name.clone()),
            input: Some(parse_function_arguments(arguments)?),
            ..empty()
        },
        CanonicalContent::ToolResult { id, name, output } => OpenAiAnthropicBlock {
            kind: "tool_result".to_owned(),
            name: name.clone(),
            tool_use_id: Some(id.clone()),
            content: Some(output.clone()),
            ..empty()
        },
        CanonicalContent::Reasoning { text } => OpenAiAnthropicBlock {
            kind: "reasoning".to_owned(),
            text: Some(text.clone()),
            ..empty()
        },
        CanonicalContent::ClaudeThinking {
            thinking,
            signature,
            model,
            provenance,
        } => anthropic_thinking_block(
            thinking.clone(),
            signature.clone(),
            model.clone(),
            *provenance,
        ),
        CanonicalContent::RedactedThinking {
            data,
            model,
            provenance,
        } => anthropic_redacted_block(data.clone(), model.clone(), *provenance),
        CanonicalContent::ProviderState { .. } => {
            return Err(RelayConvertError::Unsupported(
                "provider state has no Anthropic extension representation".to_owned(),
            ));
        }
    };
    Ok(block)
}

fn canonical_parts_to_anthropic_blocks(
    parts: &[CanonicalContent],
) -> Result<Vec<OpenAiAnthropicBlock>, RelayConvertError> {
    parts
        .iter()
        .map(canonical_part_to_anthropic_block)
        .collect()
}

fn attach_anthropic_request_extensions(
    request: &CanonicalRequest,
    target: &mut OpenAiChatRequest,
    _loss: &mut LossReport,
) -> Result<(), RelayConvertError> {
    // `instructions` are emitted as leading system messages by the generic
    // Chat converter; skip those before matching Claude's explicit system
    // message blocks.
    let mut target_index = request.instructions.len();
    for message in &request.messages {
        let expected_role = role_to_chat(message.role.clone());
        let Some(index) = target.messages[target_index..]
            .iter()
            .position(|candidate| candidate.role == expected_role)
            .map(|index| target_index + index)
        else {
            return Err(RelayConvertError::Unsupported(
                "cannot locate generated Chat message for Claude history".to_owned(),
            ));
        };
        target_index = index + 1;
        let blocks = canonical_parts_to_anthropic_blocks(&message.parts)?;
        let model = blocks
            .iter()
            .find(|block| block.kind == "thinking" || block.kind == "redacted_thinking")
            .map(|_| request.model.clone());
        set_anthropic_extra(
            &mut target.messages[index].extra_content,
            message_anthropic_extra(blocks, model),
        );
    }
    Ok(())
}

fn attach_anthropic_response_extension(
    response: &CanonicalResponse,
    target: &mut OpenAiChatResponse,
    _loss: &mut LossReport,
) -> Result<(), RelayConvertError> {
    let Some(choice) = target.choices.first_mut() else {
        return Err(RelayConvertError::Missing("choices[0]"));
    };
    let blocks = canonical_parts_to_anthropic_blocks(&response.output)?;
    set_anthropic_extra(
        &mut choice.message.extra_content,
        message_anthropic_extra(blocks, Some(response.model.clone())),
    );
    Ok(())
}

fn set_anthropic_extra(
    target: &mut Option<OpenAiExtraContent>,
    incoming: OpenAiExtraContent,
) {
    let mut merged = target.take().unwrap_or(OpenAiExtraContent {
        google: None,
        anthropic: None,
    });
    merged.anthropic = incoming.anthropic;
    *target = Some(merged);
}

fn claude_blocks_to_canonical(
    blocks: Vec<ClaudeContentBlock>,
    model: Option<&str>,
) -> Result<Vec<CanonicalContent>, RelayConvertError> {
    blocks
        .into_iter()
        .map(|block| {
            if !block.extra.is_empty() {
                return Err(RelayConvertError::Unsupported(
                    "unknown Claude block fields cannot cross the canonical boundary".to_owned(),
                ));
            }
            validate_claude_block_shape(&block)?;
            match block.kind.as_str() {
            "text" => Ok(CanonicalContent::Text {
                text: block.text.unwrap_or_default(),
            }),
            "thinking" => Ok(CanonicalContent::ClaudeThinking {
                thinking: block
                    .thinking
                    .or(block.text)
                    .unwrap_or_default(),
                signature: block.signature,
                model: model.map(str::to_owned),
                provenance: OpaqueStateProvenance::Authentic,
            }),
            "redacted_thinking" => Ok(CanonicalContent::RedactedThinking {
                data: block
                    .data
                    .or(block.content)
                    .ok_or(RelayConvertError::Missing("redacted_thinking.data"))?,
                model: model.map(str::to_owned),
                provenance: OpaqueStateProvenance::Authentic,
            }),
            "tool_use" => Ok(CanonicalContent::ToolCall {
                id: block
                    .id
                    .ok_or(RelayConvertError::Missing("tool_use.id"))?,
                name: block
                    .name
                    .ok_or(RelayConvertError::Missing("tool_use.name"))?,
                arguments: block
                    .input
                    .ok_or(RelayConvertError::Missing("tool_use.input"))?
                    .compact_string()?,
            }),
            "tool_result" => Ok(CanonicalContent::ToolResult {
                id: block
                    .tool_use_id
                    .ok_or(RelayConvertError::Missing("tool_result.tool_use_id"))?,
                name: block.name,
                output: block.content.unwrap_or(JsonData::Null),
            }),
            "image" => {
                let image = block
                    .source
                    .ok_or(RelayConvertError::Missing("image.source"))?;
                if image.kind != "url" {
                    return Err(RelayConvertError::Unsupported(
                        "Claude non-URL image source cannot be represented by canonical image URL"
                            .to_owned(),
                    ));
                }
                Ok(CanonicalContent::Image {
                    url: image.data,
                    detail: None,
                })
            }
                kind => Err(RelayConvertError::Unsupported(format!(
                    "unsupported Claude content block type {kind:?}"
                ))),
            }
        })
        .collect()
}

fn validate_claude_block_shape(
    block: &ClaudeContentBlock,
) -> Result<(), RelayConvertError> {
    let reject = |field: &str| {
        Err(RelayConvertError::Unsupported(format!(
            "Claude {} block carries incompatible field {field:?}",
            block.kind
        )))
    };
    match block.kind.as_str() {
        "text" => {
            if block.text.is_none() {
                return Err(RelayConvertError::Missing("text.text"));
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
            if block.source.is_some() {
                return reject("source");
            }
        }
        "thinking" => {
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_none() {
                return Err(RelayConvertError::Missing("thinking.thinking"));
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
            if block.source.is_some() {
                return reject("source");
            }
        }
        "redacted_thinking" => {
            if block.data.is_some() && block.content.is_some() {
                return reject("content");
            }
            if block.data.is_none() && block.content.is_none() {
                return Err(RelayConvertError::Missing("redacted_thinking.data"));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.source.is_some() {
                return reject("source");
            }
        }
        "tool_use" => {
            if block.id.is_none() {
                return Err(RelayConvertError::Missing("tool_use.id"));
            }
            if block.name.is_none() {
                return Err(RelayConvertError::Missing("tool_use.name"));
            }
            if block.input.is_none() {
                return Err(RelayConvertError::Missing("tool_use.input"));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
            if block.source.is_some() {
                return reject("source");
            }
        }
        "tool_result" => {
            if block.tool_use_id.is_none() {
                return Err(RelayConvertError::Missing("tool_result.tool_use_id"));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.source.is_some() {
                return reject("source");
            }
        }
        "image" => {
            if block.source.is_none() {
                return Err(RelayConvertError::Missing("image.source"));
            }
            if block.text.is_some() {
                return reject("text");
            }
            if block.thinking.is_some() {
                return reject("thinking");
            }
            if block.signature.is_some() {
                return reject("signature");
            }
            if block.data.is_some() {
                return reject("data");
            }
            if block.id.is_some() {
                return reject("id");
            }
            if block.name.is_some() {
                return reject("name");
            }
            if block.input.is_some() {
                return reject("input");
            }
            if block.tool_use_id.is_some() {
                return reject("tool_use_id");
            }
            if block.content.is_some() {
                return reject("content");
            }
        }
        kind => {
            return Err(RelayConvertError::Unsupported(format!(
                "unsupported Claude content block type {kind:?}"
            )))
        }
    }
    Ok(())
}

fn claude_text_block(text: String) -> ClaudeContentBlock {
    ClaudeContentBlock {
        kind: "text".to_owned(),
        text: Some(text),
        thinking: None,
        signature: None,
        data: None,
        id: None,
        name: None,
        input: None,
        tool_use_id: None,
        content: None,
        source: None,
        extra: BTreeMap::new(),
    }
}

fn canonical_parts_to_claude(
    parts: &[CanonicalContent],
    loss: &mut LossReport,
    has_tool_call: bool,
) -> Result<Vec<ClaudeContentBlock>, RelayConvertError> {
    let mut output = Vec::new();
    for part in parts {
        match part {
            CanonicalContent::Text { text } => output.push(claude_text_block(text.clone())),
            CanonicalContent::Image { .. } => {
                return Err(RelayConvertError::Unsupported(
                    "canonical image URL cannot be encoded as a Claude image source".to_owned(),
                ));
            }
            CanonicalContent::Reasoning { text } => {
                // OpenAI reasoning_content and canonical summaries are not
                // legal Claude signed thinking.  Keep the text visible as a
                // normal block rather than fabricating a signature.
                record_dropped(loss, "ordinary_reasoning->claude_text");
                output.push(claude_text_block(text.clone()));
            }
            CanonicalContent::ClaudeThinking {
                thinking,
                signature,
                provenance,
                ..
            } => {
                if *provenance == OpaqueStateProvenance::Synthetic {
                    return Err(RelayConvertError::Unsupported(
                        "synthetic Claude thinking cannot be sent upstream".to_owned(),
                    ));
                }
                if has_tool_call && signature.is_none() {
                    return Err(RelayConvertError::Unsupported(
                        "Claude tool round is missing authentic thinking signature".to_owned(),
                    ));
                }
                output.push(ClaudeContentBlock {
                    kind: "thinking".to_owned(),
                    text: None,
                    thinking: Some(thinking.clone()),
                    signature: signature.clone(),
                    data: None,
                    id: None,
                    name: None,
                    input: None,
                    tool_use_id: None,
                    content: None,
                    source: None,
                    extra: BTreeMap::new(),
                });
            }
            CanonicalContent::RedactedThinking {
                data,
                provenance,
                ..
            } => {
                if *provenance == OpaqueStateProvenance::Synthetic {
                    return Err(RelayConvertError::Unsupported(
                        "synthetic Claude redacted thinking cannot be sent upstream".to_owned(),
                    ));
                }
                output.push(ClaudeContentBlock {
                    kind: "redacted_thinking".to_owned(),
                    text: None,
                    thinking: None,
                    signature: None,
                    data: Some(data.clone()),
                    id: None,
                    name: None,
                    input: None,
                    tool_use_id: None,
                    content: None,
                    source: None,
                    extra: BTreeMap::new(),
                });
            }
            CanonicalContent::ToolCall {
                id,
                name,
                arguments,
            } => output.push(ClaudeContentBlock {
                kind: "tool_use".to_owned(),
                text: None,
                thinking: None,
                signature: None,
                data: None,
                id: Some(id.clone()),
                name: Some(name.clone()),
                input: Some(parse_function_arguments(arguments)?),
                tool_use_id: None,
                content: None,
                source: None,
                extra: BTreeMap::new(),
            }),
            CanonicalContent::ToolResult { id, name, output } => {
                output.push(ClaudeContentBlock {
                    kind: "tool_result".to_owned(),
                    text: None,
                    thinking: None,
                    signature: None,
                    data: None,
                    id: None,
                    name: name.clone(),
                    input: None,
                    tool_use_id: Some(id.clone()),
                    content: Some(output.clone()),
                    source: None,
                    extra: BTreeMap::new(),
                });
            }
            CanonicalContent::ProviderState { .. } => {
                return Err(RelayConvertError::Unsupported(
                    "opaque provider state has no Claude block representation".to_owned(),
                ));
            }
        }
    }
    Ok(output)
}

fn claude_tool_choice_to_canonical(
    choice: &ClaudeToolChoice,
) -> Result<CanonicalToolChoice, RelayConvertError> {
    match choice.kind.as_str() {
        "auto" => Ok(CanonicalToolChoice::Auto),
        "any" => Ok(CanonicalToolChoice::Required),
        "tool" => Ok(CanonicalToolChoice::Function {
            name: choice
                .name
                .clone()
                .ok_or(RelayConvertError::Missing("tool_choice.name"))?,
        }),
        kind => Err(RelayConvertError::Unsupported(format!(
            "unsupported Claude tool_choice type {kind:?}"
        ))),
    }
}

fn canonical_tool_choice_to_claude(choice: CanonicalToolChoice) -> ClaudeToolChoice {
    match choice {
        CanonicalToolChoice::Auto => ClaudeToolChoice {
            kind: "auto".to_owned(),
            name: None,
        },
        CanonicalToolChoice::Required => ClaudeToolChoice {
            kind: "any".to_owned(),
            name: None,
        },
        CanonicalToolChoice::Function { name } => ClaudeToolChoice {
            kind: "tool".to_owned(),
            name: Some(name),
        },
        CanonicalToolChoice::None => ClaudeToolChoice {
            kind: "auto".to_owned(),
            name: None,
        },
    }
}

fn claude_finish_reason(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "end_turn",
        FinishReason::Length => "max_tokens",
        FinishReason::ToolCalls => "tool_use",
        FinishReason::ContentFilter => "refusal",
        FinishReason::Cancelled => "cancelled",
        FinishReason::Error => "error",
        FinishReason::Other => "stop_sequence",
    }
    .to_owned()
}

fn token_usage_to_claude(usage: &TokenUsage) -> super::ClaudeUsage {
    super::ClaudeUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_input_tokens: usage.cached_input_tokens,
        ..super::ClaudeUsage::default()
    }
}

/// Lossless semantic representation of Claude SSE events.  In particular,
/// `signature_delta` is a separate event and can never be mistaken for a
/// newline/text delta.
#[derive(Clone, Debug, PartialEq)]
pub enum ClaudeStreamSemanticEvent {
    MessageStart { message: Option<ClaudeResponse> },
    ContentBlockStart {
        index: usize,
        block: ClaudeContentBlock,
    },
    TextDelta { index: usize, delta: String },
    ThinkingDelta { index: usize, delta: String },
    SignatureDelta { index: usize, signature: String },
    RedactedThinking { index: usize, data: JsonData },
    ToolInputJsonDelta { index: usize, delta: String },
    ContentBlockStop { index: usize },
    MessageDelta {
        stop_reason: Option<String>,
        usage: Option<super::ClaudeUsage>,
    },
    MessageStop,
    Ping,
    Error { code: Option<String>, message: String },
    Unknown { kind: String, fields: JsonData },
    Cancelled,
}

/// Small state machine for Claude streaming.  It validates block ordering and
/// exposes cancellation without pretending a cancellation is a provider
/// text event.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClaudeStreamState {
    started: bool,
    ended: bool,
    cancelled: bool,
    last_started_index: Option<usize>,
    open_blocks: BTreeMap<usize, ClaudeStreamBlockKind>,
    closed_blocks: BTreeSet<usize>,
    signature_seen: BTreeSet<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ClaudeStreamBlockKind {
    Text,
    Thinking,
    RedactedThinking,
    ToolUse,
    Other(String),
}

impl ClaudeStreamState {
    pub fn apply(
        &mut self,
        event: &ClaudeStreamSemanticEvent,
    ) -> Result<(), RelayConvertError> {
        match event {
            ClaudeStreamSemanticEvent::MessageStart { .. } => {
                if self.started || self.ended || self.cancelled {
                    return Err(RelayConvertError::Unsupported(
                        "Claude stream contains duplicate message_start".to_owned(),
                    ));
                }
                self.started = true;
            }
            ClaudeStreamSemanticEvent::ContentBlockStart { index, block } => {
                self.require_started()?;
                if self.ended
                    || self.cancelled
                    || !self.open_blocks.is_empty()
                    || self
                        .last_started_index
                        .is_some_and(|last_index| *index <= last_index)
                    || self.closed_blocks.contains(index)
                {
                    return Err(RelayConvertError::Unsupported(
                        "Claude stream content block started out of order".to_owned(),
                    ));
                }
                self.last_started_index = Some(*index);
                self.open_blocks
                    .insert(*index, ClaudeStreamBlockKind::from_block(block));
                if block.signature.is_some() {
                    self.signature_seen.insert(*index);
                }
            }
            ClaudeStreamSemanticEvent::TextDelta { index, .. } => {
                self.require_started()?;
                self.require_open_block(*index, ClaudeStreamBlockKind::Text)?;
            }
            ClaudeStreamSemanticEvent::ThinkingDelta { index, .. } => {
                self.require_started()?;
                self.require_open_block(*index, ClaudeStreamBlockKind::Thinking)?;
            }
            ClaudeStreamSemanticEvent::SignatureDelta { index, .. } => {
                self.require_started()?;
                self.require_open_block(*index, ClaudeStreamBlockKind::Thinking)?;
                self.signature_seen.insert(*index);
            }
            ClaudeStreamSemanticEvent::RedactedThinking { index, .. } => {
                self.require_started()?;
                self.require_open_block(*index, ClaudeStreamBlockKind::RedactedThinking)?;
            }
            ClaudeStreamSemanticEvent::ToolInputJsonDelta { index, .. } => {
                self.require_started()?;
                self.require_open_block(*index, ClaudeStreamBlockKind::ToolUse)?;
            }
            ClaudeStreamSemanticEvent::ContentBlockStop { index } => {
                self.require_started()?;
                if self.ended || self.cancelled {
                    return Err(RelayConvertError::Unsupported(
                        "Claude stream content block stopped out of order".to_owned(),
                    ));
                }
                let Some(kind) = self.open_blocks.remove(index) else {
                    return Err(RelayConvertError::Unsupported(
                        "Claude stream content block stopped out of order".to_owned(),
                    ));
                };
                if kind == ClaudeStreamBlockKind::Thinking
                    && !self.signature_seen.contains(index)
                {
                    return Err(RelayConvertError::Unsupported(
                        "Claude thinking block stopped before signature_delta".to_owned(),
                    ));
                }
                self.closed_blocks.insert(*index);
            }
            ClaudeStreamSemanticEvent::MessageDelta { .. } => {
                self.require_started()?;
                if self.ended || self.cancelled || !self.open_blocks.is_empty() {
                    return Err(RelayConvertError::Unsupported(
                        "Claude message_delta arrived out of order".to_owned(),
                    ));
                }
            }
            ClaudeStreamSemanticEvent::MessageStop => {
                self.require_started()?;
                if self.ended || self.cancelled || !self.open_blocks.is_empty() {
                    return Err(RelayConvertError::Unsupported(
                        "Claude message_stop arrived before content_block_stop".to_owned(),
                    ));
                }
                self.ended = true;
            }
            ClaudeStreamSemanticEvent::Ping
            | ClaudeStreamSemanticEvent::Error { .. }
            | ClaudeStreamSemanticEvent::Unknown { .. } => {}
            ClaudeStreamSemanticEvent::Cancelled => {
                if !self.started || self.ended || self.cancelled {
                    return Err(RelayConvertError::Unsupported(
                        "Claude stream cancellation arrived out of order".to_owned(),
                    ));
                }
                self.cancelled = true;
                self.ended = true;
            }
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> ClaudeStreamSemanticEvent {
        self.cancelled = true;
        self.ended = true;
        ClaudeStreamSemanticEvent::Cancelled
    }

    fn require_started(&self) -> Result<(), RelayConvertError> {
        if self.started {
            Ok(())
        } else {
            Err(RelayConvertError::Unsupported(
                "Claude stream event arrived before message_start".to_owned(),
            ))
        }
    }

    pub fn has_open_blocks(&self) -> bool {
        !self.open_blocks.is_empty()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn require_open_block(
        &self,
        index: usize,
        expected: ClaudeStreamBlockKind,
    ) -> Result<(), RelayConvertError> {
        if self.ended || self.cancelled {
            return Err(RelayConvertError::Unsupported(
                "Claude stream delta arrived after stream end".to_owned(),
            ));
        }
        match self.open_blocks.get(&index) {
            Some(kind) if kind == &expected => Ok(()),
            Some(kind) => Err(RelayConvertError::Unsupported(format!(
                "Claude stream delta does not match block kind {kind:?}"
            ))),
            None => Err(RelayConvertError::Unsupported(
                "Claude stream delta has no open content block".to_owned(),
            )),
        }
    }
}

impl ClaudeStreamBlockKind {
    fn from_block(block: &ClaudeContentBlock) -> Self {
        match block.kind.as_str() {
            "text" => Self::Text,
            "thinking" => Self::Thinking,
            "redacted_thinking" => Self::RedactedThinking,
            "tool_use" => Self::ToolUse,
            kind => Self::Other(kind.to_owned()),
        }
    }
}

/// Decodes typed Claude stream DTOs into explicit semantic events.  Unknown
/// provider events and pings are retained; malformed typed deltas fail rather
/// than being silently converted into text.
pub fn claude_stream_to_semantic_events(
    snapshot: &ClaudeStreamSnapshot,
) -> Result<Vec<ClaudeStreamSemanticEvent>, RelayConvertError> {
    let mut events = Vec::new();
    for event in &snapshot.events {
        let semantic = match event.kind.as_str() {
            "message_start" => ClaudeStreamSemanticEvent::MessageStart {
                message: event.message.clone(),
            },
            "content_block_start" => ClaudeStreamSemanticEvent::ContentBlockStart {
                index: event.index.ok_or(RelayConvertError::Missing(
                    "content_block_start.index",
                ))?,
                block: event.content_block.clone().ok_or(RelayConvertError::Missing(
                    "content_block_start.content_block",
                ))?,
            },
            "content_block_delta" => {
                let index = event
                    .index
                    .ok_or(RelayConvertError::Missing("content_block_delta.index"))?;
                let delta = event
                    .delta
                    .as_ref()
                    .ok_or(RelayConvertError::Missing("content_block_delta.delta"))?;
                match delta.kind.as_deref() {
                    Some("text_delta") => ClaudeStreamSemanticEvent::TextDelta {
                        index,
                        delta: delta
                            .text
                            .clone()
                            .ok_or(RelayConvertError::Missing("text_delta.text"))?,
                    },
                    Some("thinking_delta") => ClaudeStreamSemanticEvent::ThinkingDelta {
                        index,
                        delta: delta
                            .thinking
                            .clone()
                            .ok_or(RelayConvertError::Missing("thinking_delta.thinking"))?,
                    },
                    Some("signature_delta") => ClaudeStreamSemanticEvent::SignatureDelta {
                        index,
                        signature: delta.signature.clone().ok_or(
                            RelayConvertError::Missing("signature_delta.signature"),
                        )?,
                    },
                    Some("input_json_delta") => ClaudeStreamSemanticEvent::ToolInputJsonDelta {
                        index,
                        delta: delta.partial_json.clone().ok_or(
                            RelayConvertError::Missing("input_json_delta.partial_json"),
                        )?,
                    },
                    Some("redacted_thinking_delta") => {
                        ClaudeStreamSemanticEvent::RedactedThinking {
                            index,
                            data: delta.data.clone().ok_or(RelayConvertError::Missing(
                                "redacted_thinking_delta.data",
                            ))?,
                        }
                    }
                    Some(kind) => {
                        return Err(RelayConvertError::Unsupported(format!(
                            "unsupported Claude content delta type {kind:?}"
                        )));
                    }
                    None => {
                        return Err(RelayConvertError::Missing("content_block_delta.delta.type"));
                    }
                }
            }
            "content_block_stop" => ClaudeStreamSemanticEvent::ContentBlockStop {
                index: event
                    .index
                    .ok_or(RelayConvertError::Missing("content_block_stop.index"))?,
            },
            "message_delta" => ClaudeStreamSemanticEvent::MessageDelta {
                stop_reason: event
                    .delta
                    .as_ref()
                    .and_then(|delta| delta.stop_reason.clone()),
                usage: event
                    .usage
                    .clone()
                    .or_else(|| event.delta.as_ref().and_then(|delta| delta.usage.clone())),
            },
            "message_stop" => ClaudeStreamSemanticEvent::MessageStop,
            "ping" => ClaudeStreamSemanticEvent::Ping,
            "error" => {
                let error = event.error.as_ref().ok_or(RelayConvertError::Missing(
                    "error.error",
                ))?;
                ClaudeStreamSemanticEvent::Error {
                    code: error.code.clone(),
                    message: error.message.clone(),
                }
            }
            kind => ClaudeStreamSemanticEvent::Unknown {
                kind: kind.to_owned(),
                fields: claude_stream_event_fields(event)?,
            },
        };
        events.push(semantic);
    }
    Ok(events)
}

fn claude_stream_event_fields(
    event: &ClaudeStreamEvent,
) -> Result<JsonData, RelayConvertError> {
    let value = serde_json::to_value(event)?;
    Ok(serde_json::from_value(value)?)
}

pub fn claude_stream_cancelled() -> ClaudeStreamSemanticEvent {
    ClaudeStreamSemanticEvent::Cancelled
}

fn merge_loss(mut left: LossReport, right: LossReport) -> LossReport {
    for field in right.dropped_fields {
        record_dropped(&mut left, field);
    }
    for field in right.normalized_fields {
        if !left.normalized_fields.contains(&field) {
            left.normalized_fields.push(field);
        }
    }
    for field in right.synthetic_fields {
        if !left.synthetic_fields.contains(&field) {
            left.synthetic_fields.push(field);
        }
    }
    left
}

fn chat_content_to_canonical(
    content: Option<StringOrParts<OpenAiChatContentPart>>,
) -> Result<Vec<CanonicalContent>, RelayConvertError> {
    match content {
        Some(StringOrParts::String(text)) => Ok(vec![CanonicalContent::Text { text }]),
        Some(StringOrParts::Parts(parts)) => parts
            .into_iter()
            .map(|part| match part.kind.as_str() {
                "text" => Ok(CanonicalContent::Text {
                    text: part.text.unwrap_or_default(),
                }),
                "image_url" => {
                    let image = part
                        .image_url
                        .ok_or(RelayConvertError::Missing("image_url"))?;
                    let (url, detail) = match image {
                        JsonData::String(url) => (url, None),
                        JsonData::Object(mut object) => {
                            let url = match object.remove("url") {
                                Some(JsonData::String(url)) => url,
                                _ => return Err(RelayConvertError::Missing("image_url.url")),
                            };
                            let detail = match object.remove("detail") {
                                Some(JsonData::String(detail)) => Some(detail),
                                Some(_) => {
                                    return Err(RelayConvertError::Unsupported(
                                        "image_url.detail must be a string".to_owned(),
                                    ));
                                }
                                None => None,
                            };
                            (url, detail)
                        }
                        _ => return Err(RelayConvertError::Missing("image_url.url")),
                    };
                    Ok(CanonicalContent::Image { url, detail })
                }
                kind => Err(RelayConvertError::Unsupported(format!(
                    "unsupported Chat content part type {kind:?}"
                ))),
            })
            .collect(),
        None => Ok(Vec::new()),
    }
}

fn chat_tool_output_to_json(
    content: Option<StringOrParts<OpenAiChatContentPart>>,
) -> Result<JsonData, RelayConvertError> {
    match content {
        Some(StringOrParts::String(value)) => Ok(JsonData::String(value)),
        Some(StringOrParts::Parts(parts)) => Ok(JsonData::Array(
            parts
                .into_iter()
                .map(|part| JsonData::String(part.text.unwrap_or_default()))
                .collect(),
        )),
        None => Ok(JsonData::String(String::new())),
    }
}

fn responses_content_to_canonical(
    content: Option<StringOrParts<ResponsesContentPart>>,
) -> Result<Vec<CanonicalContent>, RelayConvertError> {
    match content {
        Some(StringOrParts::String(text)) => Ok(vec![CanonicalContent::Text { text }]),
        Some(StringOrParts::Parts(parts)) => parts
            .into_iter()
            .map(|part| match part.kind.as_str() {
                "input_text" | "output_text" | "text" => Ok(CanonicalContent::Text {
                    text: part.text.unwrap_or_default(),
                }),
                "input_image" => Ok(CanonicalContent::Image {
                    url: part
                        .image_url
                        .ok_or(RelayConvertError::Missing("image_url"))?,
                    detail: None,
                }),
                kind => Err(RelayConvertError::Unsupported(format!(
                    "unsupported Responses content part type {kind:?}"
                ))),
            })
            .collect(),
        None => Ok(Vec::new()),
    }
}

#[derive(Default)]
struct GeminiIdAllocator {
    next: usize,
    pending_by_name: BTreeMap<String, VecDeque<String>>,
    pending_by_id: BTreeMap<String, String>,
}

impl GeminiIdAllocator {
    fn call_id(&mut self, name: &str) -> String {
        let id = loop {
            let candidate = format!("gemini_call_synthetic_{}_{}", self.next, name);
            self.next += 1;
            if !self.pending_by_id.contains_key(&candidate) {
                break candidate;
            }
        };
        self.pending_by_name
            .entry(name.to_owned())
            .or_default()
            .push_back(id.clone());
        self.pending_by_id.insert(id.clone(), name.to_owned());
        id
    }

    fn remember_call(
        &mut self,
        name: &str,
        id: &str,
    ) -> Result<(), RelayConvertError> {
        if self.pending_by_id.contains_key(id) {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionCall id {id:?} is duplicated before its result"
            )));
        }
        self.pending_by_name
            .entry(name.to_owned())
            .or_default()
            .push_back(id.to_owned());
        self.pending_by_id.insert(id.to_owned(), name.to_owned());
        Ok(())
    }

    fn result_id(
        &mut self,
        name: &str,
        loss: &mut LossReport,
    ) -> Result<String, RelayConvertError> {
        let Some(pending) = self.pending_by_name.get_mut(name) else {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionResponse for {name:?} has no outstanding functionCall"
            )));
        };
        if pending.len() > 1 {
            record_synthetic(loss, "SYNTHETIC_TOOL_RESULT_ID_AMBIGUOUS");
        }
        let Some(id) = pending.pop_front() else {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionResponse for {name:?} has no outstanding functionCall"
            )));
        };
        self.pending_by_id.remove(&id);
        Ok(id)
    }

    fn remove_explicit_result(
        &mut self,
        name: &str,
        id: &str,
    ) -> Result<(), RelayConvertError> {
        let Some(expected_name) = self.pending_by_id.get(id).cloned() else {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionResponse id {id:?} has no outstanding functionCall"
            )));
        };
        if expected_name != name {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionResponse id {id:?} names {name:?}, expected {expected_name:?}"
            )));
        }
        let Some(pending) = self.pending_by_name.get_mut(name) else {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionResponse id {id:?} has no name queue"
            )));
        };
        let Some(index) = pending.iter().position(|candidate| candidate == id) else {
            return Err(RelayConvertError::Unsupported(format!(
                "Gemini functionResponse id {id:?} is not in its name queue"
            )));
        };
        pending.remove(index);
        self.pending_by_id.remove(id);
        Ok(())
    }

}

fn gemini_parts_to_canonical(
    parts: Vec<GeminiPart>,
    model: &str,
    loss: &mut LossReport,
    ids: &mut GeminiIdAllocator,
) -> Result<Vec<CanonicalContent>, RelayConvertError> {
    let mut output = Vec::new();
    for part in parts {
        let GeminiPart {
            text,
            inline_data,
            function_call,
            function_response,
            thought_signature,
        } = part;
        let had_text = text.is_some();
        let had_function_call = function_call.is_some();
        let had_function_response = function_response.is_some();
        if had_text && (had_function_call || had_function_response) {
            return Err(RelayConvertError::Unsupported(
                "Gemini Part contains multiple content payloads".to_owned(),
            ));
        }
        if inline_data.is_some() {
            return Err(RelayConvertError::Unsupported(
                "Gemini inlineData cannot be represented by the current canonical image URL"
                    .to_owned(),
            ));
        }
        if function_call.is_some() && function_response.is_some() {
            return Err(RelayConvertError::Unsupported(
                "Gemini Part cannot contain both functionCall and functionResponse".to_owned(),
            ));
        }
        if let Some(text) = text {
            output.push(CanonicalContent::Text { text });
        }
        if let Some(call) = function_call {
            let name = call.name;
            let id = match call.id.filter(|id| !id.is_empty()) {
                Some(id) => {
                    ids.remember_call(&name, &id)?;
                    id
                }
                None => {
                    record_synthetic(loss, "SYNTHETIC_TOOL_CALL_ID");
                    ids.call_id(&name)
                }
            };
            output.push(CanonicalContent::ToolCall {
                id,
                name,
                arguments: call
                    .args
                    .map(|args| args.compact_string())
                    .transpose()?
                    .unwrap_or_else(|| "{}".to_owned()),
            });
        }
        if let Some(result) = function_response {
            let name = result.name;
            let id = match result.id.filter(|id| !id.is_empty()) {
                Some(id) => {
                    ids.remove_explicit_result(&name, &id)?;
                    id
                }
                None => {
                    record_synthetic(loss, "SYNTHETIC_TOOL_RESULT_ID");
                    ids.result_id(&name, loss)?
                }
            };
            output.push(CanonicalContent::ToolResult {
                id,
                name: Some(name),
                output: result.response,
            });
        }
        if let Some(signature) = thought_signature {
            if !had_text && !had_function_call && !had_function_response {
                // A signature-only Part is still anchored to its own empty
                // text Part.  Append both in this order so a late signature
                // cannot be moved before the preceding Part.
                output.push(CanonicalContent::Text {
                    text: String::new(),
                });
            }
            output.push(CanonicalContent::ProviderState {
                state: OpaqueProviderState {
                    provider: "google".to_owned(),
                    kind: "thought_signature".to_owned(),
                    raw: JsonData::String(signature),
                    provenance: OpaqueStateProvenance::Authentic,
                    model: Some(model.to_owned()),
                },
            });
        }
    }
    Ok(output)
}

fn append_canonical_gemini_parts(
    destination: &mut Vec<GeminiPart>,
    parts: &[CanonicalContent],
    model: &str,
    family: GeminiModelFamily,
    policy: GeminiSignaturePolicy,
    loss: &mut LossReport,
    call_names: &mut BTreeMap<String, String>,
) -> Result<(), RelayConvertError> {
    let (synthetic_history, reject_gemini3_missing_signature) = match policy {
        GeminiSignaturePolicy::Request { synthetic_history } => (synthetic_history, true),
        GeminiSignaturePolicy::Response => (false, false),
    };
    let mut saw_tool_call = false;
    for (index, part) in parts.iter().enumerate() {
        match part {
            CanonicalContent::Text { text } => destination.push(GeminiPart {
                text: Some(text.clone()),
                inline_data: None,
                function_call: None,
                function_response: None,
                thought_signature: None,
            }),
            CanonicalContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                let first_tool_call = !saw_tool_call;
                saw_tool_call = true;
                if call_names.contains_key(id) {
                    return Err(RelayConvertError::Unsupported(format!(
                        "Gemini functionCall id {id:?} is duplicated before its result"
                    )));
                }
                call_names.insert(id.clone(), name.clone());
                let args = parse_function_arguments(arguments)?;
                destination.push(GeminiPart {
                    text: None,
                    inline_data: None,
                    function_call: Some(GeminiFunctionCall {
                        id: Some(id.clone()),
                        name: name.clone(),
                        args: Some(args),
                    }),
                    function_response: None,
                    thought_signature: None,
                });
                let next_state = parts.get(index + 1).and_then(|next| match next {
                    CanonicalContent::ProviderState { state }
                        if state.provider == "google" && state.kind == "thought_signature" =>
                    {
                        Some(state)
                    }
                    _ => None,
                });
                if next_state.is_some() && !first_tool_call {
                    return Err(RelayConvertError::Unsupported(
                        "Gemini parallel functionCall signature must remain on the first call Part"
                            .to_owned(),
                    ));
                }
                if next_state.is_none() && first_tool_call {
                    if family == GeminiModelFamily::Gemini3
                        && reject_gemini3_missing_signature
                    {
                        return Err(RelayConvertError::Unsupported(format!(
                            "Gemini 3 tool call {id:?} is missing thoughtSignature"
                        )));
                    }
                    if family == GeminiModelFamily::Gemini25 && synthetic_history {
                        let Some(last) = destination.last_mut() else {
                            return Err(RelayConvertError::Unsupported(
                                "Gemini tool call Part was not emitted".to_owned(),
                            ));
                        };
                        last.thought_signature =
                            Some(GEMINI_SYNTHETIC_THOUGHT_SIGNATURE.to_owned());
                        record_synthetic(loss, "SYNTHETIC_THOUGHT_SIGNATURE");
                    }
                }
            }
            CanonicalContent::ToolResult { id, name, output } => {
                let Some(expected_name) = call_names.get(id).cloned() else {
                    return Err(RelayConvertError::Unsupported(format!(
                        "Gemini functionResponse id {id:?} has no outstanding functionCall"
                    )));
                };
                if let Some(name) = name.as_deref() {
                    if name != expected_name {
                        return Err(RelayConvertError::Unsupported(format!(
                            "Gemini functionResponse id {id:?} names {name:?}, expected {expected_name:?}"
                        )));
                    }
                }
                destination.push(GeminiPart {
                    text: None,
                    inline_data: None,
                    function_call: None,
                    function_response: Some(GeminiFunctionResponse {
                        id: Some(id.clone()),
                        name: name.clone().unwrap_or(expected_name),
                        response: output.clone(),
                    }),
                    thought_signature: None,
                });
                call_names.remove(id);
            }
            CanonicalContent::Reasoning { .. } => {
                return Err(RelayConvertError::Unsupported(
                    "ordinary reasoning cannot be encoded as a Gemini thoughtSignature".to_owned(),
                ));
            }
            CanonicalContent::ClaudeThinking { .. } => {
                return Err(RelayConvertError::Unsupported(
                    "Claude authentic thinking cannot be encoded as Gemini content".to_owned(),
                ));
            }
            CanonicalContent::RedactedThinking { .. } => {
                return Err(RelayConvertError::Unsupported(
                    "Claude redacted thinking cannot be encoded as Gemini content".to_owned(),
                ));
            }
            CanonicalContent::Image { .. } => {
                return Err(RelayConvertError::Unsupported(
                    "canonical image URL cannot be encoded as Gemini inlineData".to_owned(),
                ));
            }
            CanonicalContent::ProviderState { state } => {
                if state.provenance == OpaqueStateProvenance::Synthetic {
                    if !synthetic_history {
                        return Err(RelayConvertError::Unsupported(
                            "synthetic Gemini thoughtSignature requires synthetic history mode"
                                .to_owned(),
                        ));
                    }
                    if state.raw
                        != JsonData::String(GEMINI_SYNTHETIC_THOUGHT_SIGNATURE.to_owned())
                    {
                        return Err(RelayConvertError::Unsupported(
                            "unsupported synthetic Gemini thoughtSignature value".to_owned(),
                        ));
                    }
                    record_synthetic(loss, "SYNTHETIC_THOUGHT_SIGNATURE");
                }
                let signature = gemini_signature_from_state(state)?;
                let Some(last) = destination.last_mut() else {
                    return Err(RelayConvertError::Unsupported(
                        "Gemini provider state has no preceding Part".to_owned(),
                    ));
                };
                if last.thought_signature.is_some() {
                    return Err(RelayConvertError::Unsupported(
                        "Gemini Part has duplicate thoughtSignature provider state".to_owned(),
                    ));
                }
                last.thought_signature = Some(signature);
            }
        }
    }
    let _ = model;
    Ok(())
}

fn gemini_signature_from_state(state: &OpaqueProviderState) -> Result<String, RelayConvertError> {
    if state.provider != "google" || state.kind != "thought_signature" {
        return Err(RelayConvertError::Unsupported(format!(
            "provider state {}:{} cannot be encoded for Gemini",
            state.provider, state.kind
        )));
    }
    match &state.raw {
        JsonData::String(value) => Ok(value.clone()),
        _ => Err(RelayConvertError::Unsupported(
            "Gemini thoughtSignature must be a string".to_owned(),
        )),
    }
}

fn parse_function_arguments(arguments: &str) -> Result<JsonData, RelayConvertError> {
    if arguments.trim().is_empty() {
        return Ok(JsonData::Object(BTreeMap::new()));
    }
    Ok(serde_json::from_str(arguments)?)
}

fn gemini_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "STOP" | "FINISH_REASON_STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" => FinishReason::ContentFilter,
        "MALFORMED_FUNCTION_CALL" => FinishReason::Error,
        _ => FinishReason::Other,
    }
}

fn gemini_finish_reason_to_wire(reason: FinishReason) -> String {
    match reason {
        FinishReason::Length => "MAX_TOKENS",
        FinishReason::ContentFilter => "SAFETY",
        FinishReason::Error => "MALFORMED_FUNCTION_CALL",
        FinishReason::Cancelled => "OTHER",
        FinishReason::Stop | FinishReason::ToolCalls | FinishReason::Other => "STOP",
    }
    .to_owned()
}

fn token_usage_to_gemini(usage: &TokenUsage) -> GeminiUsage {
    GeminiUsage {
        prompt_token_count: usage.input_tokens,
        candidates_token_count: usage.output_tokens,
        thoughts_token_count: usage.reasoning_tokens,
        total_token_count: usage.total_tokens,
        cached_content_token_count: usage.cached_input_tokens,
        ..GeminiUsage::default()
    }
}

fn role_from_wire(role: &str) -> Result<Role, RelayConvertError> {
    match role {
        "system" => Ok(Role::System),
        "developer" => Ok(Role::Developer),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" | "function" => Ok(Role::Tool),
        "model" => Ok(Role::Model),
        value => Err(RelayConvertError::Unsupported(format!(
            "unsupported message role {value:?}"
        ))),
    }
}

fn role_to_chat(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::Developer => "developer",
        Role::User => "user",
        Role::Assistant | Role::Model => "assistant",
        Role::Tool => "tool",
    }
}

fn role_to_responses(role: Role) -> &'static str {
    match role {
        Role::Model => "assistant",
        other => role_to_chat(other),
    }
}

fn parse_tool_choice(value: &JsonData) -> Result<CanonicalToolChoice, RelayConvertError> {
    match value {
        JsonData::String(choice) => match choice.as_str() {
            "auto" => Ok(CanonicalToolChoice::Auto),
            "none" => Ok(CanonicalToolChoice::None),
            "required" | "any" => Ok(CanonicalToolChoice::Required),
            other => Err(RelayConvertError::Unsupported(format!(
                "unsupported tool choice {other:?}"
            ))),
        },
        JsonData::Object(object) => {
            let name = object.get("name").or_else(|| {
                object.get("function").and_then(|function| match function {
                    JsonData::Object(function) => function.get("name"),
                    _ => None,
                })
            });
            match name {
                Some(JsonData::String(name)) => {
                    Ok(CanonicalToolChoice::Function { name: name.clone() })
                }
                _ => Err(RelayConvertError::Missing("tool_choice.name")),
            }
        }
        _ => Err(RelayConvertError::Unsupported(
            "tool_choice must be a string or object".to_owned(),
        )),
    }
}

fn tool_choice_to_chat(choice: CanonicalToolChoice) -> JsonData {
    match choice {
        CanonicalToolChoice::Auto => JsonData::String("auto".to_owned()),
        CanonicalToolChoice::None => JsonData::String("none".to_owned()),
        CanonicalToolChoice::Required => JsonData::String("required".to_owned()),
        CanonicalToolChoice::Function { name } => JsonData::Object(
            [
                ("type".to_owned(), JsonData::String("function".to_owned())),
                (
                    "function".to_owned(),
                    JsonData::Object(
                        [("name".to_owned(), JsonData::String(name))]
                            .into_iter()
                            .collect(),
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    }
}

fn tool_choice_to_responses(choice: CanonicalToolChoice) -> JsonData {
    match choice {
        CanonicalToolChoice::Function { name } => JsonData::Object(
            [
                ("type".to_owned(), JsonData::String("function".to_owned())),
                ("name".to_owned(), JsonData::String(name)),
            ]
            .into_iter()
            .collect(),
        ),
        other => tool_choice_to_chat(other),
    }
}

fn chat_format_to_responses_text(format: Option<JsonData>) -> Option<JsonData> {
    format.map(|value| {
        JsonData::Object(
            [("format".to_owned(), value)]
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
        )
    })
}

fn responses_text_to_chat_format(text: Option<JsonData>) -> Option<JsonData> {
    match text {
        Some(JsonData::Object(mut object)) => object.remove("format"),
        _ => None,
    }
}

fn finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" | "end_turn" | "completed" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "tool_use" => FinishReason::ToolCalls,
        "content_filter" | "safety" => FinishReason::ContentFilter,
        "cancelled" => FinishReason::Cancelled,
        "error" | "failed" => FinishReason::Error,
        _ => FinishReason::Other,
    }
}

fn finish_reason_to_chat(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Cancelled => "cancelled",
        FinishReason::Error => "error",
        FinishReason::Other => "stop",
    }
    .to_owned()
}

fn incomplete_finish_reason(response: Option<&ResponsesResponse>) -> FinishReason {
    match response
        .and_then(|value| value.incomplete_details.as_ref())
        .map(|details| details.reason.as_str())
    {
        Some("max_output_tokens") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

fn wire_usage(usage: &WireUsage) -> TokenUsage {
    let input = usage.input_tokens.max(usage.prompt_tokens);
    let output = usage.output_tokens.max(usage.completion_tokens);
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: usage.total_tokens.max(input + output),
        cached_input_tokens: usage
            .input_tokens_details
            .as_ref()
            .or(usage.prompt_tokens_details.as_ref())
            .map_or(0, |details| details.cached_tokens),
        reasoning_tokens: usage
            .completion_tokens_details
            .as_ref()
            .map_or(0, |details| details.reasoning_tokens),
    }
}

fn claude_usage(usage: &super::ClaudeUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.input_tokens + usage.output_tokens,
        cached_input_tokens: usage.cache_read_input_tokens,
        reasoning_tokens: 0,
    }
}

fn gemini_usage(usage: &super::GeminiUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.prompt_token_count,
        output_tokens: usage.candidates_token_count,
        total_tokens: usage.total_token_count,
        cached_input_tokens: usage.cached_content_token_count,
        reasoning_tokens: usage.thoughts_token_count,
    }
}

fn token_usage_to_wire(usage: &TokenUsage) -> WireUsage {
    WireUsage {
        prompt_tokens: usage.input_tokens,
        completion_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        prompt_tokens_details: Some(TokenDetails {
            cached_tokens: usage.cached_input_tokens,
            ..TokenDetails::default()
        }),
        completion_tokens_details: Some(TokenDetails {
            reasoning_tokens: usage.reasoning_tokens,
            ..TokenDetails::default()
        }),
        input_tokens_details: Some(TokenDetails {
            cached_tokens: usage.cached_input_tokens,
            ..TokenDetails::default()
        }),
        ..WireUsage::default()
    }
}

pub fn response_events_to_openai_chunks(events: &[CanonicalStreamEvent]) -> Vec<OpenAiStreamChunk> {
    let mut id = String::new();
    let mut model = String::new();
    let mut out = Vec::new();
    for event in events {
        let (delta, finish_reason, usage, error, cancelled) = match event {
            CanonicalStreamEvent::ResponseStart {
                id: event_id,
                model: event_model,
            } => {
                id.clone_from(event_id);
                model.clone_from(event_model);
                (
                    OpenAiStreamDelta {
                        role: Some("assistant".to_owned()),
                        content: Some(String::new()),
                        reasoning_content: None,
                        tool_calls: Vec::new(),
                    },
                    None,
                    None,
                    None,
                    false,
                )
            }
            CanonicalStreamEvent::ContentStart { .. } => continue,
            CanonicalStreamEvent::TextDelta { delta, .. } => (
                OpenAiStreamDelta {
                    content: Some(delta.clone()),
                    ..OpenAiStreamDelta::default()
                },
                None,
                None,
                None,
                false,
            ),
            CanonicalStreamEvent::ReasoningDelta { delta, .. } => (
                OpenAiStreamDelta {
                    reasoning_content: Some(delta.clone()),
                    ..OpenAiStreamDelta::default()
                },
                None,
                None,
                None,
                false,
            ),
            CanonicalStreamEvent::ToolCallStart { index, id, name } => (
                OpenAiStreamDelta {
                    tool_calls: vec![super::OpenAiStreamToolCall {
                        index: *index,
                        id: Some(id.clone()),
                        kind: Some("function".to_owned()),
                        function: Some(super::OpenAiStreamFunction {
                            name: Some(name.clone()),
                            arguments: Some(String::new()),
                        }),
                    }],
                    ..OpenAiStreamDelta::default()
                },
                None,
                None,
                None,
                false,
            ),
            CanonicalStreamEvent::ToolArgumentsDelta { index, delta } => (
                OpenAiStreamDelta {
                    tool_calls: vec![super::OpenAiStreamToolCall {
                        index: *index,
                        id: None,
                        kind: None,
                        function: Some(super::OpenAiStreamFunction {
                            name: None,
                            arguments: Some(delta.clone()),
                        }),
                    }],
                    ..OpenAiStreamDelta::default()
                },
                None,
                None,
                None,
                false,
            ),
            CanonicalStreamEvent::ResponseEnd {
                finish_reason,
                usage,
                model: final_model,
            } => (
                {
                    if let Some(final_model) = final_model {
                        model.clone_from(final_model);
                    }
                    OpenAiStreamDelta::default()
                },
                Some(finish_reason_to_chat(*finish_reason)),
                usage.as_ref().map(token_usage_to_wire),
                None,
                false,
            ),
            CanonicalStreamEvent::Error { code, message } => (
                OpenAiStreamDelta::default(),
                None,
                None,
                Some(WireError {
                    code: code.clone(),
                    message: message.clone(),
                }),
                false,
            ),
            CanonicalStreamEvent::Cancelled => {
                (OpenAiStreamDelta::default(), None, None, None, true)
            }
            CanonicalStreamEvent::ContentEnd { .. } => continue,
        };
        let choices = if error.is_some() || cancelled {
            Vec::new()
        } else {
            vec![super::OpenAiStreamChoice {
                delta,
                logprobs: None,
                finish_reason,
                index: 0,
            }]
        };
        out.push(OpenAiStreamChunk {
            id: id.clone(),
            object: "chat.completion.chunk".to_owned(),
            created: 0,
            model: model.clone(),
            system_fingerprint: None,
            choices,
            usage,
            error,
            cancelled,
        });
    }
    out
}

pub fn response_events_to_responses(events: &[CanonicalStreamEvent]) -> Vec<ResponsesStreamEvent> {
    let mut id = String::new();
    let mut model = String::new();
    let mut out = Vec::new();
    let mut items = BTreeMap::<usize, ResponsesOutputItem>::new();
    let mut open_items = BTreeSet::new();
    for event in events {
        let mut payloads = Vec::new();
        match event {
            CanonicalStreamEvent::ResponseStart {
                id: event_id,
                model: event_model,
            } => {
                id.clone_from(event_id);
                model.clone_from(event_model);
                let response = empty_responses_stream_response(&id, &model, "in_progress", None);
                let mut payload = responses_payload("response.created");
                payload.response = Some(response);
                payloads.push(payload);
            }
            CanonicalStreamEvent::ContentStart { index, kind } => {
                ensure_responses_item(
                    &mut payloads,
                    &mut items,
                    &mut open_items,
                    &id,
                    *index,
                    *kind,
                );
            }
            CanonicalStreamEvent::TextDelta { index, delta } => {
                ensure_responses_item(
                    &mut payloads,
                    &mut items,
                    &mut open_items,
                    &id,
                    *index,
                    StreamContentKind::Text,
                );
                let mut payload = responses_payload("response.output_text.delta");
                payload.delta = Some(delta.clone());
                payload.output_index = Some(*index);
                payload.content_index = Some(0);
                payload.item_id = Some(format!("{id}_msg_{index}"));
                payloads.push(payload);
                if let Some(item) = items.get_mut(index) {
                    let content = item.content.get_or_insert_default();
                    if let Some(part) = content.first_mut() {
                        part.text.push_str(delta);
                    } else {
                        content.push(ResponsesOutputContent {
                            kind: "output_text".to_owned(),
                            text: delta.clone(),
                            annotations: Some(Vec::new()),
                            extra: BTreeMap::new(),
                        });
                    }
                }
            }
            CanonicalStreamEvent::ReasoningDelta { index, delta } => {
                ensure_responses_item(
                    &mut payloads,
                    &mut items,
                    &mut open_items,
                    &id,
                    *index,
                    StreamContentKind::Reasoning,
                );
                let mut payload = responses_payload("response.reasoning_summary_text.delta");
                payload.delta = Some(delta.clone());
                payload.output_index = Some(*index);
                payload.summary_index = Some(0);
                payload.item_id = Some(format!("{id}_reasoning_{index}"));
                payloads.push(payload);
                if let Some(item) = items.get_mut(index) {
                    let summary = item.summary.get_or_insert_default();
                    if let Some(part) = summary.first_mut() {
                        part.text.push_str(delta);
                    } else {
                        summary.push(ResponsesOutputContent {
                            kind: "summary_text".to_owned(),
                            text: delta.clone(),
                            annotations: None,
                            extra: BTreeMap::new(),
                        });
                    }
                }
            }
            CanonicalStreamEvent::ToolCallStart {
                index,
                id: call_id,
                name,
            } => {
                let item = ResponsesOutputItem {
                    kind: "function_call".to_owned(),
                    id: format!("{id}_tool_{index}"),
                    status: "in_progress".to_owned(),
                    role: String::new(),
                    content: None,
                    summary: None,
                    quality: String::new(),
                    size: String::new(),
                    call_id: Some(call_id.clone()),
                    name: Some(name.clone()),
                    arguments: Some(String::new()),
                    extra: BTreeMap::new(),
                };
                items.insert(*index, item.clone());
                open_items.insert(*index);
                let mut payload = responses_payload("response.output_item.added");
                payload.output_index = Some(*index);
                payload.item = Some(item);
                payloads.push(payload);
            }
            CanonicalStreamEvent::ToolArgumentsDelta { index, delta } => {
                let mut payload = responses_payload("response.function_call_arguments.delta");
                payload.delta = Some(delta.clone());
                payload.output_index = Some(*index);
                payload.item_id = Some(
                    items
                        .get(index)
                        .map_or_else(|| format!("{id}_tool_{index}"), |item| item.id.clone()),
                );
                payloads.push(payload);
                if let Some(item) = items.get_mut(index) {
                    item.arguments.get_or_insert_default().push_str(delta);
                }
            }
            CanonicalStreamEvent::ContentEnd { index } => {
                finish_responses_item(&mut payloads, &mut items, &mut open_items, *index);
            }
            CanonicalStreamEvent::ResponseEnd {
                finish_reason,
                usage,
                model: _,
            } => {
                for index in open_items.clone() {
                    finish_responses_item(&mut payloads, &mut items, &mut open_items, index);
                }
                let (event_kind, status, incomplete_details) = match finish_reason {
                    FinishReason::Length => (
                        "response.incomplete",
                        "incomplete",
                        Some(super::IncompleteDetails {
                            reason: "max_output_tokens".to_owned(),
                        }),
                    ),
                    FinishReason::ContentFilter => (
                        "response.incomplete",
                        "incomplete",
                        Some(super::IncompleteDetails {
                            reason: "content_filter".to_owned(),
                        }),
                    ),
                    FinishReason::Cancelled => ("response.cancelled", "cancelled", None),
                    FinishReason::Error => ("response.failed", "failed", None),
                    _ => ("response.completed", "completed", None),
                };
                let mut response = empty_responses_stream_response(
                    &id,
                    &model,
                    status,
                    usage.as_ref().map(token_usage_to_wire),
                );
                response.output = items.values().cloned().collect();
                response.incomplete_details = incomplete_details;
                let mut payload = responses_payload(event_kind);
                payload.response = Some(response);
                payloads.push(payload);
            }
            CanonicalStreamEvent::Error { code, message } => {
                let mut payload = responses_payload("response.error");
                payload.error = Some(WireError {
                    code: code.clone(),
                    message: message.clone(),
                });
                payloads.push(payload);
            }
            CanonicalStreamEvent::Cancelled => {
                payloads.push(responses_payload("response.cancelled"));
            }
        }
        out.extend(payloads.into_iter().map(|payload| ResponsesStreamEvent {
            kind: payload.kind.clone(),
            payload,
            extra: BTreeMap::new(),
        }));
    }
    out
}

fn responses_payload(kind: &str) -> super::ResponsesEventPayload {
    super::ResponsesEventPayload {
        kind: kind.to_owned(),
        response: None,
        item: None,
        delta: None,
        text: None,
        arguments: None,
        output_index: None,
        content_index: None,
        item_id: None,
        summary_index: None,
        part: None,
        error: None,
        extra: BTreeMap::new(),
    }
}

fn ensure_responses_item(
    payloads: &mut Vec<super::ResponsesEventPayload>,
    items: &mut BTreeMap<usize, ResponsesOutputItem>,
    open_items: &mut BTreeSet<usize>,
    response_id: &str,
    index: usize,
    kind: StreamContentKind,
) {
    if items.contains_key(&index) {
        return;
    }
    let item = match kind {
        StreamContentKind::Text => ResponsesOutputItem {
            kind: "message".to_owned(),
            id: format!("{response_id}_msg_{index}"),
            status: "in_progress".to_owned(),
            role: "assistant".to_owned(),
            content: Some(Vec::new()),
            summary: None,
            quality: String::new(),
            size: String::new(),
            call_id: None,
            name: None,
            arguments: None,
            extra: BTreeMap::new(),
        },
        StreamContentKind::Reasoning => ResponsesOutputItem {
            kind: "reasoning".to_owned(),
            id: format!("{response_id}_reasoning_{index}"),
            status: "in_progress".to_owned(),
            role: String::new(),
            content: None,
            summary: Some(Vec::new()),
            quality: String::new(),
            size: String::new(),
            call_id: None,
            name: None,
            arguments: None,
            extra: BTreeMap::new(),
        },
        StreamContentKind::ToolCall => return,
    };
    items.insert(index, item.clone());
    open_items.insert(index);
    let mut payload = responses_payload("response.output_item.added");
    payload.output_index = Some(index);
    payload.item = Some(item);
    payloads.push(payload);
}

fn finish_responses_item(
    payloads: &mut Vec<super::ResponsesEventPayload>,
    items: &mut BTreeMap<usize, ResponsesOutputItem>,
    open_items: &mut BTreeSet<usize>,
    index: usize,
) {
    if !open_items.remove(&index) {
        return;
    }
    let Some(item) = items.get_mut(&index) else {
        return;
    };
    if item.kind == "function_call" {
        let mut arguments_done = responses_payload("response.function_call_arguments.done");
        arguments_done.output_index = Some(index);
        arguments_done.item_id = Some(item.id.clone());
        payloads.push(arguments_done);
    } else if item.kind == "reasoning" {
        let mut reasoning_done = responses_payload("response.reasoning_summary_text.done");
        reasoning_done.output_index = Some(index);
        reasoning_done.item_id = Some(item.id.clone());
        reasoning_done.summary_index = Some(0);
        reasoning_done.part = item
            .summary
            .as_ref()
            .and_then(|summary| summary.first())
            .cloned();
        payloads.push(reasoning_done);
    } else if item.kind == "message" {
        let mut text_done = responses_payload("response.output_text.done");
        text_done.output_index = Some(index);
        text_done.item_id = Some(item.id.clone());
        text_done.content_index = Some(0);
        payloads.push(text_done);
    }
    item.status = "completed".to_owned();
    let mut done = responses_payload("response.output_item.done");
    done.output_index = Some(index);
    done.item = Some(item.clone());
    payloads.push(done);
}

fn empty_responses_stream_response(
    id: &str,
    model: &str,
    status: &str,
    usage: Option<WireUsage>,
) -> ResponsesResponse {
    ResponsesResponse {
        id: id.to_owned(),
        object: "response".to_owned(),
        created_at: 0,
        status: status.to_owned(),
        instructions: None,
        max_output_tokens: 0,
        model: model.to_owned(),
        output: Vec::new(),
        parallel_tool_calls: false,
        previous_response_id: None,
        reasoning: None,
        store: false,
        temperature: 0.0,
        tool_choice: None,
        tools: None,
        top_p: 0.0,
        truncation: None,
        usage,
        user: None,
        metadata: None,
        incomplete_details: None,
        error: None,
        extra: BTreeMap::new(),
    }
}
