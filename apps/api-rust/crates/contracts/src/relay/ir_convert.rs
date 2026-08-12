//! Direct v2 conversions through the ordered relay IR.
//!
//! This module is intentionally independent from the legacy conversion
//! module.  A direct route has exactly one source decode/map and one target
//! encode/map.  In particular, Gemini and Claude never use an OpenAI DTO as
//! an intermediate representation here.

use std::{collections::{BTreeMap, VecDeque}, error::Error, fmt};

use serde::{Deserialize, Serialize};

use super::{
    CacheUsage, ClaudeContentBlock, ClaudeMediaSource, ClaudeMessage, ClaudeRequest,
    ClaudeResponse, ClaudeTool, ClaudeToolChoice, ClaudeUsage, ConversionPlan, Envelope,
    Feature, Fidelity, FunctionData, FunctionKind, GenerationControls, GeminiCandidate,
    GeminiContent, GeminiFunctionCall, GeminiFunctionDeclaration, GeminiFunctionResponse,
    GeminiGenerationConfig, GeminiInlineData, GeminiPart, GeminiRequest, GeminiResponse,
    GeminiTool, GeminiToolConfig, GeminiFunctionCallingConfig, GeminiUsage, Item, ItemKind,
    JsonData, Loss, LossCode, LossLedger, LossSeverity, Media, MediaKind, OpaqueId,
    OpaqueIdProvenance, OpaqueProviderState, OpaqueStateProvenance, OpenAiAnthropicBlock,
    OpenAiAnthropicExtraContent, OpenAiChatContentPart, OpenAiChatMessage, OpenAiChatRequest,
    OpenAiChatResponse, OpenAiChatTool, OpenAiChoice, OpenAiExtraContent, OpenAiFunction,
    OpenAiGoogleExtraContent, OpenAiToolCall, Part, PartKind, Protocol, Provenance, Role,
    SemanticBillingUsage, SemanticUsage, StateRefs, StringOrParts, Tool, ToolChoice, ToolKind,
    WireUsage,
};

/// The default gate for candidate v2 direct routes.  The functions in this
/// module are available for offline differential tests, but production route
/// ownership remains with the existing registry until an explicit rollout.
pub const DIRECT_IR_V2_DEFAULT_ENABLED: bool = false;

/// A machine-readable route descriptor for one direct IR candidate route.
///
/// The descriptor is deliberately not a [`crate::relay::RuntimeCatalog`]
/// entry.  It makes the candidate's disabled-by-default state and one-hop
/// shape inspectable without changing runtime registration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectIrRouteDescriptor {
    /// Source wire protocol.
    pub source: Protocol,
    /// Target wire protocol.
    pub target: Protocol,
    /// Candidate route is disabled until an explicit rollout enables it.
    pub enabled: bool,
    /// Request converter identifier.
    pub request_converter_id: String,
    /// Response converter identifier.
    pub response_converter_id: String,
    /// Direct routes always have one cross-protocol hop.
    pub hop_count: usize,
    /// Candidate route fidelity before input-specific losses.
    pub fidelity: Fidelity,
}

/// Returns the disabled-by-default descriptor for a direct IR route.
#[must_use]
pub fn direct_ir_route_descriptor(
    source: Protocol,
    target: Protocol,
) -> DirectIrRouteDescriptor {
    DirectIrRouteDescriptor {
        source,
        target,
        enabled: DIRECT_IR_V2_DEFAULT_ENABLED,
        request_converter_id: format!("{}-to-{}-ir-v2-request", protocol_name(source), protocol_name(target)),
        response_converter_id: format!("{}-to-{}-ir-v2-response", protocol_name(source), protocol_name(target)),
        hop_count: if source == target { 0 } else { 1 },
        fidelity: if source == target { Fidelity::Exact } else { Fidelity::Normalized },
    }
}

/// Alias retained for route-oriented callers.
#[must_use]
pub fn direct_ir_route(source: Protocol, target: Protocol) -> DirectIrRouteDescriptor {
    direct_ir_route_descriptor(source, target)
}

/// A direct conversion result with enough information for audit, policy, and
/// offline differential comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectIrConversion<T> {
    /// Encoded target wire value (or the Envelope for decode functions).
    pub value: T,
    /// Ordered intermediate envelope used by the conversion.
    pub envelope: Envelope,
    /// One-hop conversion decision for this source/target pair.
    pub plan: ConversionPlan,
    /// De-duplicated losses, including synthetic-field markers.
    pub losses: LossLedger,
}

impl<T> DirectIrConversion<T> {
    fn new(value: T, envelope: Envelope, plan: ConversionPlan) -> Self {
        let mut losses = envelope.loss_ledger().clone();
        losses.extend(plan.losses.iter().cloned());
        Self {
            value,
            envelope,
            plan,
            losses,
        }
    }

    /// Returns the audit ledger under the conventional name used by relay
    /// policy code.
    #[must_use]
    pub fn loss_ledger(&self) -> &LossLedger {
        &self.losses
    }
}

/// Safe, typed failure from a direct source/target conversion.
///
/// The error carries only structural routing information.  Source body text,
/// argument bytes, and provider payloads are intentionally absent from its
/// [`Display`] implementation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectIrError {
    /// Stable error code.
    pub code: &'static str,
    /// Source wire protocol.
    pub source: Protocol,
    /// Target wire protocol.
    pub target: Protocol,
    /// Stable feature name, such as `function_call_id`.
    pub feature: String,
    /// Structural source path, such as `contents[1].parts[0]`.
    pub path: String,
    /// Direct conversion failures are not fixed by retrying unchanged input.
    pub retryable: bool,
    /// A safe machine-readable reason, never raw provider content.
    pub reason: DirectIrReason,
}

/// Stable reasons for a direct conversion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectIrReason {
    /// A required field was absent.
    MissingField,
    /// An opaque identifier was empty.
    EmptyId,
    /// An identifier was duplicated.
    DuplicateId,
    /// A result did not match a prior call.
    Mismatch,
    /// A result had no prior call.
    Orphan,
    /// The source shape is not valid for this protocol.
    InvalidShape,
    /// The source feature cannot be represented by the target.
    Unsupported,
    /// A provider signature or opaque state was malformed.
    InvalidOpaqueState,
    /// A JSON argument/schema field could not be decoded as JSON.
    InvalidJsonField,
}

impl DirectIrError {
    fn new(
        source: Protocol,
        target: Protocol,
        feature: impl Into<String>,
        path: impl Into<String>,
        reason: DirectIrReason,
    ) -> Self {
        Self {
            code: "direct_ir_conversion_error",
            source,
            target,
            feature: feature.into(),
            path: path.into(),
            retryable: false,
            reason,
        }
    }

    fn missing(source: Protocol, target: Protocol, feature: &str, path: &str) -> Self {
        Self::new(source, target, feature, path, DirectIrReason::MissingField)
    }

    fn unsupported(
        source: Protocol,
        target: Protocol,
        feature: &str,
        path: &str,
    ) -> Self {
        Self::new(source, target, feature, path, DirectIrReason::Unsupported)
    }
}

impl fmt::Display for DirectIrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {:?} -> {:?}, feature {}, path {}",
            self.code, self.source, self.target, self.feature, self.path
        )
    }
}

impl Error for DirectIrError {}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAi => "openai-chat",
        Protocol::OpenAiResponses => "openai-responses",
        Protocol::Claude => "claude",
        Protocol::Gemini => "gemini",
    }
}

fn plan_for(
    source: Protocol,
    target: Protocol,
    model: &str,
    envelope: &Envelope,
) -> ConversionPlan {
    let descriptor = direct_ir_route_descriptor(source, target);
    let mut plan = ConversionPlan {
        source,
        target,
        model_family: model.to_owned(),
        converter_ids: vec![descriptor.request_converter_id, descriptor.response_converter_id],
        hop_count: descriptor.hop_count,
        fidelity: descriptor.fidelity,
        unsupported: Vec::new(),
        losses: envelope.loss_ledger().as_slice().to_vec(),
        synthetic: Vec::new(),
    };
    for loss in envelope.loss_ledger().as_slice() {
        match loss.code {
            LossCode::SyntheticToolCallId => {
                plan.add_synthetic(super::SyntheticField::ToolCallId);
            }
            LossCode::SyntheticThoughtSignature => {
                plan.add_synthetic(super::SyntheticField::ThoughtSignature);
            }
            _ => {}
        }
    }
    if !plan.losses.is_empty() {
        plan.fidelity = Fidelity::Lossy;
    }
    plan
}

fn finish<T>(
    value: T,
    envelope: Envelope,
    source: Protocol,
    target: Protocol,
) -> DirectIrConversion<T> {
    let plan = plan_for(source, target, &envelope.model, &envelope);
    DirectIrConversion::new(value, envelope, plan)
}

fn map_validation<T>(
    result: Result<T, super::IrValidationError>,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<T, DirectIrError> {
    result.map_err(|_| {
        DirectIrError::new(
            source,
            target,
            "ordered_item",
            path,
            DirectIrReason::InvalidShape,
        )
    })
}

fn push_item(
    envelope: &mut Envelope,
    item: Item,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<(), DirectIrError> {
    map_validation(envelope.push_item(item), source, target, path)
}

fn record_loss(
    envelope: &mut Envelope,
    code: LossCode,
    feature: Option<Feature>,
    path: &str,
    message: &str,
) {
    envelope.record_loss(
        Loss::new(code, feature)
            .at(path)
            .with_message(message),
    );
}

fn loss_for_unrepresented(
    envelope: &mut Envelope,
    feature: Feature,
    path: &str,
    message: &str,
) {
    record_loss(
        envelope,
        LossCode::LossUnknownEvent,
        Some(feature),
        path,
        message,
    );
}

fn role_from_wire(
    role: Option<&str>,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<Role, DirectIrError> {
    match role {
        None => Ok(Role::User),
        Some("system") => Ok(Role::System),
        Some("developer") => Ok(Role::Developer),
        Some("user") => Ok(Role::User),
        Some("assistant") => Ok(Role::Assistant),
        Some("tool") | Some("function") => Ok(Role::Tool),
        Some("model") => Ok(Role::Model),
        Some(_) => Err(DirectIrError::unsupported(
            source,
            target,
            "message_role",
            path,
        )),
    }
}

fn chat_role(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::Developer => "developer",
        Role::User => "user",
        Role::Assistant | Role::Model => "assistant",
        Role::Tool => "tool",
    }
}

fn provider_state(
    provider: &str,
    kind: &str,
    raw: JsonData,
    provenance: OpaqueStateProvenance,
    model: Option<String>,
) -> OpaqueProviderState {
    OpaqueProviderState {
        provider: provider.to_owned(),
        kind: kind.to_owned(),
        raw,
        provenance,
        model,
    }
}

fn object(entries: impl IntoIterator<Item = (String, JsonData)>) -> JsonData {
    JsonData::Object(entries.into_iter().collect())
}

fn json_string(value: &JsonData) -> Option<&str> {
    match value {
        JsonData::String(value) => Some(value),
        _ => None,
    }
}

fn json_object(value: &JsonData) -> Option<&BTreeMap<String, JsonData>> {
    match value {
        JsonData::Object(value) => Some(value),
        _ => None,
    }
}

fn parse_json_field(
    text: &str,
    source: Protocol,
    target: Protocol,
    feature: &str,
    path: &str,
) -> Result<JsonData, DirectIrError> {
    let source_text = if text.trim().is_empty() { "{}" } else { text };
    serde_json::from_str(source_text).map_err(|_| {
        DirectIrError::new(
            source,
            target,
            feature,
            path,
            DirectIrReason::InvalidJsonField,
        )
    })
}

fn compact_json(value: &JsonData) -> Result<String, DirectIrError> {
    value
        .compact_string()
        .map_err(|_| DirectIrError::new(
            Protocol::OpenAi,
            Protocol::OpenAi,
            "json_field",
            "envelope",
            DirectIrReason::InvalidJsonField,
        ))
}

fn empty_extensions() -> BTreeMap<String, JsonData> {
    BTreeMap::new()
}

fn authentic_id(
    value: Option<String>,
    source: Protocol,
    target: Protocol,
    feature: &str,
    path: &str,
) -> Result<Option<OpaqueId>, DirectIrError> {
    match value {
        None => Ok(None),
        Some(value) if value.is_empty() => Err(DirectIrError::new(
            source,
            target,
            feature,
            path,
            DirectIrReason::EmptyId,
        )),
        Some(value) => Ok(Some(OpaqueId::authentic(value, source))),
    }
}

fn synthetic_gemini_id(kind: &str, index: usize, part_index: usize, name: &str) -> String {
    format!("gemini_call_synthetic_{kind}_{index}_{part_index}_{name}")
}

fn id_value(id: &OpaqueId) -> Result<String, DirectIrError> {
    if id.is_empty() {
        return Err(DirectIrError::new(
            id.source(),
            id.source(),
            "function_call_id",
            "envelope.items[].call_id",
            DirectIrReason::EmptyId,
        ));
    }
    Ok(id.value.clone())
}

fn function_call_part(
    name: String,
    arguments: JsonData,
) -> Part {
    Part::function(FunctionData {
        kind: FunctionKind::Call,
        name: Some(name),
        arguments: Some(arguments),
        output: None,
        extensions: empty_extensions(),
    })
}

fn function_result_part(
    name: Option<String>,
    output: JsonData,
) -> Part {
    Part::function(FunctionData {
        kind: FunctionKind::Result,
        name,
        arguments: None,
        output: Some(output),
        extensions: empty_extensions(),
    })
}

fn text_item(role: Role, text: String, source: Protocol, path: &str) -> Item {
    let mut item = Item::new(ItemKind::Message, role, Provenance::new(source));
    item.provenance.source_path = Some(path.to_owned());
    item.push_part(Part::text(text));
    item
}

fn opaque_item(
    role: Role,
    state: OpaqueProviderState,
    source: Protocol,
    path: &str,
) -> Item {
    let mut item = Item::new(ItemKind::Reasoning, role, Provenance::new(source));
    item.provenance.source_path = Some(path.to_owned());
    item.push_part(Part::opaque(state));
    item
}

fn media_item(role: Role, media: Media, source: Protocol, path: &str) -> Item {
    let mut item = Item::new(ItemKind::Message, role, Provenance::new(source));
    item.provenance.source_path = Some(path.to_owned());
    item.push_part(Part::media(media));
    item
}

fn tool_call_item(
    call_id: OpaqueId,
    name: String,
    arguments: JsonData,
    source: Protocol,
    path: &str,
) -> Item {
    let mut item = Item::tool_call(
        call_id,
        vec![function_call_part(name, arguments)],
        Provenance::new(source),
    );
    item.provenance.source_path = Some(path.to_owned());
    item
}

fn tool_result_item(
    call_id: OpaqueId,
    name: Option<String>,
    output: JsonData,
    source: Protocol,
    path: &str,
) -> Item {
    let mut item = Item::tool_result(
        call_id,
        vec![function_result_part(name, output)],
        Provenance::new(source),
    );
    item.provenance.source_path = Some(path.to_owned());
    item
}

fn chat_content_to_parts(
    content: Option<StringOrParts<OpenAiChatContentPart>>,
    role: Role,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<Vec<Part>, DirectIrError> {
    let Some(content) = content else {
        return Ok(Vec::new());
    };
    match content {
        StringOrParts::String(text) => Ok(vec![Part::text(text)]),
        StringOrParts::Parts(parts) => parts
            .into_iter()
            .enumerate()
            .map(|(index, part)| {
                let part_path = format!("{path}[{index}]");
                match part.kind.as_str() {
                    "text" => part
                        .text
                        .map(Part::text)
                        .ok_or_else(|| DirectIrError::missing(
                            source,
                            target,
                            "text",
                            &format!("{part_path}.text"),
                        )),
                    "image_url" => {
                        let image = part.image_url.ok_or_else(|| {
                            DirectIrError::missing(
                                source,
                                target,
                                "image",
                                &format!("{part_path}.image_url"),
                            )
                        })?;
                        let (uri, data) = match image {
                            JsonData::String(uri) => (Some(uri), None),
                            JsonData::Object(mut object) => {
                                let uri = object.remove("url").and_then(|value| match value {
                                    JsonData::String(value) => Some(value),
                                    _ => None,
                                });
                                (uri, Some(JsonData::Object(object)))
                            }
                            value => (None, Some(value)),
                        };
                        let mut media = Media::new(MediaKind::Image);
                        media.uri = uri;
                        media.data = data;
                        Ok(Part::media(media))
                    }
                    _ => Err(DirectIrError::unsupported(
                        source,
                        target,
                        "content_part",
                        &part_path,
                    )),
                }
            })
            .collect(),
    }
}

fn chat_content_to_json(
    content: Option<StringOrParts<OpenAiChatContentPart>>,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<JsonData, DirectIrError> {
    let Some(content) = content else {
        return Ok(JsonData::String(String::new()));
    };
    match content {
        StringOrParts::String(value) => Ok(JsonData::String(value)),
        StringOrParts::Parts(parts) => {
            let values = parts
                .into_iter()
                .enumerate()
                .map(|(index, part)| {
                    let mut entries = BTreeMap::new();
                    entries.insert("type".to_owned(), JsonData::String(part.kind));
                    if let Some(text) = part.text {
                        entries.insert("text".to_owned(), JsonData::String(text));
                    }
                    if let Some(image) = part.image_url {
                        entries.insert("image_url".to_owned(), image);
                    }
                    if entries.len() == 1 {
                        Err(DirectIrError::missing(
                            source,
                            target,
                            "content_part",
                            &format!("{path}[{index}]"),
                        ))
                    } else {
                        Ok(JsonData::Object(entries))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(JsonData::Array(values))
        }
    }
}

fn chat_parts_to_content(
    parts: &[Part],
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<Option<StringOrParts<OpenAiChatContentPart>>, DirectIrError> {
    if parts.is_empty() {
        return Ok(None);
    }
    if parts.iter().all(|part| matches!(part.kind, PartKind::Text)) {
        let mut text = String::new();
        for part in parts {
            let Some(value) = part.text.as_ref() else {
                return Err(DirectIrError::new(
                    source,
                    target,
                    "text",
                    path,
                    DirectIrReason::InvalidShape,
                ));
            };
            text.push_str(value);
        }
        return Ok(Some(StringOrParts::String(text)));
    }
    let mut output = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        match &part.kind {
            PartKind::Text => output.push(OpenAiChatContentPart {
                kind: "text".to_owned(),
                text: part.text.clone(),
                image_url: None,
            }),
            PartKind::Media => {
                let Some(media) = part.media.as_ref() else {
                    return Err(DirectIrError::new(
                        source,
                        target,
                        "media",
                        &format!("{path}[{index}]"),
                        DirectIrReason::InvalidShape,
                    ));
                };
                if !matches!(media.kind, MediaKind::Image) {
                    return Err(DirectIrError::unsupported(
                        source,
                        target,
                        "media",
                        &format!("{path}[{index}]"),
                    ));
                }
                let image_url = if let Some(uri) = media.uri.as_ref() {
                    if let Some(JsonData::Object(extra)) = media.data.as_ref() {
                        let mut value = extra.clone();
                        value.insert("url".to_owned(), JsonData::String(uri.clone()));
                        JsonData::Object(value)
                    } else {
                        JsonData::String(uri.clone())
                    }
                } else if let Some(data) = media.data.as_ref() {
                    data.clone()
                } else {
                    return Err(DirectIrError::missing(
                        source,
                        target,
                        "image_url",
                        &format!("{path}[{index}]"),
                    ));
                };
                output.push(OpenAiChatContentPart {
                    kind: "image_url".to_owned(),
                    text: None,
                    image_url: Some(image_url),
                });
            }
            PartKind::Unknown(_) | PartKind::Opaque | PartKind::Function => {
                return Err(DirectIrError::unsupported(
                    source,
                    target,
                    "content_part",
                    &format!("{path}[{index}]"),
                ));
            }
        }
    }
    Ok(Some(StringOrParts::Parts(output)))
}

fn tool_choice_from_json(
    choice: Option<JsonData>,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<ToolChoice, DirectIrError> {
    let Some(choice) = choice else {
        return Ok(ToolChoice::Auto);
    };
    match choice {
        JsonData::String(value) => match value.as_str() {
            "auto" => Ok(ToolChoice::Auto),
            "none" => Ok(ToolChoice::None),
            "required" | "any" => Ok(ToolChoice::Required),
            _ => Err(DirectIrError::unsupported(
                source,
                target,
                "tool_choice",
                path,
            )),
        },
        JsonData::Object(object) => {
            let name = object.get("name").and_then(json_string).or_else(|| {
                object.get("function").and_then(json_object).and_then(|value| {
                    value.get("name").and_then(json_string)
                })
            });
            name.map(|name| ToolChoice::Named {
                name: name.to_owned(),
            })
            .ok_or_else(|| DirectIrError::missing(source, target, "tool_choice.name", path))
        }
        _ => Err(DirectIrError::unsupported(
            source,
            target,
            "tool_choice",
            path,
        )),
    }
}

fn tool_choice_to_json(choice: &ToolChoice) -> JsonData {
    match choice {
        ToolChoice::Auto => JsonData::String("auto".to_owned()),
        ToolChoice::None => JsonData::String("none".to_owned()),
        ToolChoice::Required => JsonData::String("required".to_owned()),
        ToolChoice::Named { name } => object([
            ("type".to_owned(), JsonData::String("function".to_owned())),
            (
                "function".to_owned(),
                object([("name".to_owned(), JsonData::String(name.clone()))]),
            ),
        ]),
        ToolChoice::Provider { raw } => raw.clone(),
    }
}

fn tool_to_ir(
    kind: String,
    name: String,
    description: Option<String>,
    schema: Option<JsonData>,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<Tool, DirectIrError> {
    let kind = match kind.as_str() {
        "function" => ToolKind::Function,
        "builtin" => ToolKind::Builtin,
        "mcp" => ToolKind::Mcp,
        "custom" => ToolKind::Custom,
        _ => {
            return Err(DirectIrError::unsupported(
                source,
                target,
                "tool_type",
                path,
            ));
        }
    };
    Ok(Tool {
        kind,
        name: Some(name),
        description,
        input_schema: schema,
        extensions: empty_extensions(),
    })
}

fn openai_request_controls(
    request: &OpenAiChatRequest,
    envelope: &mut Envelope,
) {
    envelope.controls = GenerationControls {
        max_output_tokens: request.max_completion_tokens.or(request.max_tokens),
        temperature: request.temperature,
        top_p: request.top_p,
        stream: request.stream,
        reasoning_effort: request.reasoning_effort.clone(),
        response_format: request.response_format.clone(),
        parallel_tool_calls: request.parallel_tool_calls,
        extensions: BTreeMap::new(),
    };
    if request.max_tokens.is_some() {
        envelope.extensions.insert(
            "openai.max_tokens".to_owned(),
            JsonData::Number(serde_json::Number::from(request.max_tokens.unwrap_or(0))),
        );
    }
    if request.max_completion_tokens.is_some() {
        envelope.extensions.insert(
            "openai.max_completion_tokens".to_owned(),
            JsonData::Number(serde_json::Number::from(
                request.max_completion_tokens.unwrap_or(0),
            )),
        );
    }
    let extensions = [
        ("user", request.user.clone()),
        ("store", request.store.clone()),
        ("metadata", request.metadata.clone()),
        ("stream_options", request.stream_options.clone()),
        ("top_logprobs", request.top_logprobs.map(|value| JsonData::Number(value.into()))),
        ("safety_identifier", request.safety_identifier.clone()),
        ("prompt_cache_retention", request.prompt_cache_retention.clone()),
        (
            "prompt_cache_key",
            request
                .prompt_cache_key
                .clone()
                .map(JsonData::String),
        ),
        (
            "service_tier",
            request.service_tier.clone().map(JsonData::String),
        ),
        ("enable_thinking", request.enable_thinking.clone()),
        ("thinking_budget", request.thinking_budget.clone()),
        ("n", request.n.map(|value| JsonData::Number(value.into()))),
    ];
    for (name, value) in extensions {
        if let Some(value) = value {
            envelope.controls.extensions.insert(name.to_owned(), value);
        }
    }
}

fn chat_extra_google_state(
    extra: Option<OpenAiExtraContent>,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<Option<OpaqueProviderState>, DirectIrError> {
    let Some(extra) = extra else {
        return Ok(None);
    };
    let Some(google) = extra.google else {
        return Ok(None);
    };
    let Some(signature) = google.thought_signature else {
        return Err(DirectIrError::missing(
            source,
            target,
            "thought_signature",
            path,
        ));
    };
    if signature.is_empty() {
        return Err(DirectIrError::new(
            source,
            target,
            "thought_signature",
            path,
            DirectIrReason::EmptyId,
        ));
    }
    Ok(Some(provider_state(
        "google",
        "thought_signature",
        JsonData::String(signature),
        if google.synthetic.unwrap_or(false) {
            OpaqueStateProvenance::Synthetic
        } else {
            OpaqueStateProvenance::Authentic
        },
        None,
    )))
}

fn chat_anthropic_blocks_to_items(
    blocks: Vec<OpenAiAnthropicBlock>,
    role: Role,
    source: Protocol,
    target: Protocol,
    path: &str,
    envelope: &mut Envelope,
) -> Result<(), DirectIrError> {
    for (index, block) in blocks.into_iter().enumerate() {
        let block_path = format!("{path}[{index}]");
        match block.kind.as_str() {
            "text" => {
                let text = block.text.ok_or_else(|| {
                    DirectIrError::missing(source, target, "text", &block_path)
                })?;
                push_item(
                    envelope,
                    text_item(role.clone(), text, source, &block_path),
                    source,
                    target,
                    &block_path,
                )?;
            }
            "thinking" => {
                let thinking = block.thinking.ok_or_else(|| {
                    DirectIrError::missing(source, target, "thinking", &block_path)
                })?;
                let raw = object([
                    ("type".to_owned(), JsonData::String("thinking".to_owned())),
                    ("thinking".to_owned(), JsonData::String(thinking)),
                    (
                        "signature".to_owned(),
                        block
                            .signature
                            .map(JsonData::String)
                            .unwrap_or(JsonData::Null),
                    ),
                    (
                        "extra".to_owned(),
                        JsonData::Object(block.extra),
                    ),
                ]);
                let state = provider_state(
                    "anthropic",
                    "thinking",
                    raw,
                    if block.synthetic.unwrap_or(false) {
                        OpaqueStateProvenance::Synthetic
                    } else {
                        OpaqueStateProvenance::Authentic
                    },
                    block.model.clone(),
                );
                push_item(
                    envelope,
                    opaque_item(role.clone(), state, source, &block_path),
                    source,
                    target,
                    &block_path,
                )?;
            }
            "redacted_thinking" => {
                let data = block.data.or(block.content).ok_or_else(|| {
                    DirectIrError::missing(source, target, "redacted_thinking.data", &block_path)
                })?;
                let state = provider_state(
                    "anthropic",
                    "redacted_thinking",
                    object([
                        (
                            "type".to_owned(),
                            JsonData::String("redacted_thinking".to_owned()),
                        ),
                        ("data".to_owned(), data),
                        (
                            "extra".to_owned(),
                            JsonData::Object(block.extra),
                        ),
                    ]),
                    if block.synthetic.unwrap_or(false) {
                        OpaqueStateProvenance::Synthetic
                    } else {
                        OpaqueStateProvenance::Authentic
                    },
                    block.model.clone(),
                );
                push_item(
                    envelope,
                    opaque_item(role.clone(), state, source, &block_path),
                    source,
                    target,
                    &block_path,
                )?;
            }
            "tool_use" => {
                let id = block.id.ok_or_else(|| {
                    DirectIrError::missing(source, target, "tool_use.id", &block_path)
                })?;
                let call_id = authentic_id(
                    Some(id),
                    source,
                    target,
                    "function_call_id",
                    &format!("{block_path}.id"),
                )?
                .ok_or_else(|| DirectIrError::missing(source, target, "tool_use.id", &block_path))?;
                let name = block.name.ok_or_else(|| {
                    DirectIrError::missing(source, target, "tool_use.name", &block_path)
                })?;
                let input = block.input.ok_or_else(|| {
                    DirectIrError::missing(source, target, "tool_use.input", &block_path)
                })?;
                push_item(
                    envelope,
                    tool_call_item(call_id, name, input, source, &block_path),
                    source,
                    target,
                    &block_path,
                )?;
            }
            "tool_result" => {
                let id = block.tool_use_id.ok_or_else(|| {
                    DirectIrError::missing(source, target, "tool_result.tool_use_id", &block_path)
                })?;
                let call_id = authentic_id(
                    Some(id),
                    source,
                    target,
                    "function_call_id",
                    &format!("{block_path}.tool_use_id"),
                )?
                .ok_or_else(|| DirectIrError::missing(source, target, "tool_result.tool_use_id", &block_path))?;
                push_item(
                    envelope,
                    tool_result_item(
                        call_id,
                        block.name,
                        block.content.unwrap_or(JsonData::Null),
                        source,
                        &block_path,
                    ),
                    source,
                    target,
                    &block_path,
                )?;
            }
            "image" => {
                let image = block.image_url.ok_or_else(|| {
                    DirectIrError::missing(source, target, "image.image_url", &block_path)
                })?;
                let (uri, data) = match image {
                    JsonData::String(uri) => (Some(uri), None),
                    value => (None, Some(value)),
                };
                let mut media = Media::new(MediaKind::Image);
                media.uri = uri;
                media.data = data;
                push_item(
                    envelope,
                    media_item(role.clone(), media, source, &block_path),
                    source,
                    target,
                    &block_path,
                )?;
            }
            _ => {
                return Err(DirectIrError::unsupported(
                    source,
                    target,
                    "anthropic_block",
                    &block_path,
                ));
            }
        }
    }
    Ok(())
}

fn chat_message_to_envelope(
    message: OpenAiChatMessage,
    index: usize,
    envelope: &mut Envelope,
    source: Protocol,
    target: Protocol,
) -> Result<(), DirectIrError> {
    let path = format!("messages[{index}]");
    let role = role_from_wire(Some(&message.role), source, target, &format!("{path}.role"))?;
    let has_anthropic_blocks = message
        .extra_content
        .as_ref()
        .and_then(|extra| extra.anthropic.as_ref())
        .is_some_and(|value| !value.blocks.is_empty());
    if has_anthropic_blocks {
        let blocks = message
            .extra_content
            .as_ref()
            .and_then(|extra| extra.anthropic.as_ref())
            .map(|value| value.blocks.clone())
            .unwrap_or_default();
        chat_anthropic_blocks_to_items(
            blocks,
            role.clone(),
            source,
            target,
            &format!("{path}.extra_content.anthropic.blocks"),
            envelope,
        )?;
    } else {
        let parts = chat_content_to_parts(
            message.content,
            role.clone(),
            source,
            target,
            &format!("{path}.content"),
        )?;
        let mut item = Item::new(ItemKind::Message, role.clone(), Provenance::new(source));
        item.provenance.source_path = Some(path.clone());
        for part in parts {
            item.push_part(part);
        }
        if !item.ordered_parts().is_empty() || message.tool_calls.is_empty() {
            push_item(envelope, item, source, target, &path)?;
        }
        if let Some(reasoning) = message.reasoning_content {
            push_item(
                envelope,
                {
                    let mut item = Item::new(ItemKind::Reasoning, role.clone(), Provenance::new(source));
                    item.provenance.source_path = Some(format!("{path}.reasoning_content"));
                    item.push_part(Part::text(reasoning));
                    item
                },
                source,
                target,
                &format!("{path}.reasoning_content"),
            )?;
        }
    }
    for (call_index, call) in message.tool_calls.into_iter().enumerate() {
        if call.id.is_empty() {
            return Err(DirectIrError::new(
                source,
                target,
                "function_call_id",
                &format!("{path}.tool_calls[{call_index}].id"),
                DirectIrReason::EmptyId,
            ));
        }
        let arguments = parse_json_field(
            &call.function.arguments,
            source,
            target,
            "function_arguments",
            &format!("{path}.tool_calls[{call_index}].function.arguments"),
        )?;
        push_item(
            envelope,
            tool_call_item(
                OpaqueId::authentic(call.id, source),
                call.function.name,
                arguments,
                source,
                &format!("{path}.tool_calls[{call_index}]"),
            ),
            source,
            target,
            &format!("{path}.tool_calls[{call_index}]"),
        )?;
    }
    if let Some(state) = chat_extra_google_state(
        message.extra_content,
        source,
        target,
        &format!("{path}.extra_content.google.thought_signature"),
    )? {
        if state.provenance == OpaqueStateProvenance::Synthetic {
            record_loss(
                envelope,
                LossCode::SyntheticThoughtSignature,
                Some(Feature::OpaqueReasoningSignature),
                &format!("{path}.extra_content.google.thought_signature"),
                "synthetic provider signature retained explicitly",
            );
        }
        push_item(
            envelope,
            opaque_item(role, state, source, &format!("{path}.extra_content.google")),
            source,
            target,
            &format!("{path}.extra_content.google"),
        )?;
    }
    Ok(())
}

/// Decodes an OpenAI Chat request into an ordered IR Envelope.
pub fn openai_chat_request_to_envelope_v2(
    request: OpenAiChatRequest,
) -> Result<DirectIrConversion<Envelope>, DirectIrError> {
    let model = request.model.clone();
    let source = Protocol::OpenAi;
    let target = Protocol::OpenAi;
    let mut envelope = Envelope::new(source, model);
    openai_request_controls(&request, &mut envelope);
    envelope.tool_choice = tool_choice_from_json(
        request.tool_choice.clone(),
        source,
        target,
        "tool_choice",
    )?;
    for (index, tool) in request.tools.iter().enumerate() {
        envelope.tools.push(tool_to_ir(
            tool.kind.clone(),
            tool.function.name.clone(),
            tool.function.description.clone(),
            tool.function.parameters.clone(),
            source,
            target,
            &format!("tools[{index}]"),
        )?);
    }
    for (index, message) in request.messages.into_iter().enumerate() {
        chat_message_to_envelope(message, index, &mut envelope, source, target)?;
    }
    map_validation(envelope.validate(), source, target, "envelope")?;
    Ok(finish(envelope.clone(), envelope, source, target))
}

fn controls_extension(envelope: &Envelope, key: &str) -> Option<JsonData> {
    envelope.controls.extensions.get(key).cloned()
}

fn number_u64(value: &JsonData) -> Option<u64> {
    match value {
        JsonData::Number(number) => number.as_u64(),
        _ => None,
    }
}

fn openai_message_from_item(
    item: &Item,
    envelope: &Envelope,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<OpenAiChatMessage, DirectIrError> {
    let mut message = OpenAiChatMessage {
        role: chat_role(item.role.clone()).to_owned(),
        content: None,
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: Vec::new(),
        extra_content: None,
    };
    match item.kind {
        ItemKind::Message => {
            message.content = chat_parts_to_content(
                item.ordered_parts(),
                source,
                target,
                &format!("{path}.content"),
            )?;
        }
        ItemKind::Reasoning => {
            let Some(part) = item.ordered_parts().first() else {
                return Err(DirectIrError::missing(source, target, "reasoning", path));
            };
            match &part.kind {
                PartKind::Text => {
                    message.reasoning_content = part.text.clone();
                }
                PartKind::Opaque => {
                    let Some(state) = part.opaque.as_ref() else {
                        return Err(DirectIrError::new(
                            source,
                            target,
                            "opaque_state",
                            path,
                            DirectIrReason::InvalidOpaqueState,
                        ));
                    };
                    if state.provider == "google" && state.kind == "thought_signature" {
                        let signature = json_string(&state.raw).ok_or_else(|| {
                            DirectIrError::new(
                                source,
                                target,
                                "thought_signature",
                                path,
                                DirectIrReason::InvalidOpaqueState,
                            )
                        })?;
                        let google = OpenAiGoogleExtraContent {
                            thought_signature: Some(signature.to_owned()),
                            synthetic: (state.provenance == OpaqueStateProvenance::Synthetic)
                                .then_some(true),
                        };
                        message.extra_content = Some(OpenAiExtraContent {
                            google: Some(google),
                            anthropic: None,
                        });
                    } else if state.provider == "anthropic" {
                        let block = opaque_state_to_anthropic_block(state, source, target, path)?;
                        message.extra_content = Some(OpenAiExtraContent {
                            google: None,
                            anthropic: Some(OpenAiAnthropicExtraContent {
                                blocks: vec![block],
                                model: state.model.clone(),
                            }),
                        });
                    } else {
                        return Err(DirectIrError::unsupported(
                            source,
                            target,
                            "opaque_state",
                            path,
                        ));
                    }
                }
                _ => {
                    return Err(DirectIrError::unsupported(
                        source,
                        target,
                        "reasoning_part",
                        path,
                    ));
                }
            }
        }
        ItemKind::ToolCall => {
            let call_id = item.call_id.as_ref().ok_or_else(|| {
                DirectIrError::missing(source, target, "function_call_id", path)
            })?;
            let function = item
                .ordered_parts()
                .first()
                .and_then(|part| part.function.as_ref())
                .ok_or_else(|| DirectIrError::missing(source, target, "function", path))?;
            let name = function.name.clone().ok_or_else(|| {
                DirectIrError::missing(source, target, "function.name", path)
            })?;
            let arguments = function.arguments.clone().unwrap_or(object([]));
            message.tool_calls.push(OpenAiToolCall {
                id: id_value(call_id)?,
                kind: "function".to_owned(),
                function: OpenAiFunction {
                    name,
                    arguments: compact_json(&arguments).map_err(|_| {
                        DirectIrError::new(
                            source,
                            target,
                            "function_arguments",
                            path,
                            DirectIrReason::InvalidJsonField,
                        )
                    })?,
                    description: None,
                    parameters: None,
                    strict: None,
                },
                extra_content: None,
            });
        }
        ItemKind::ToolResult => {
            let call_id = item.call_id.as_ref().ok_or_else(|| {
                DirectIrError::missing(source, target, "function_call_id", path)
            })?;
            let function = item
                .ordered_parts()
                .first()
                .and_then(|part| part.function.as_ref())
                .ok_or_else(|| DirectIrError::missing(source, target, "function", path))?;
            message.tool_call_id = Some(id_value(call_id)?);
            message.content = Some(StringOrParts::String(compact_json(
                &function.output.clone().unwrap_or(JsonData::Null),
            )?));
        }
        ItemKind::Unknown(_) => {
            return Err(DirectIrError::unsupported(source, target, "item_kind", path));
        }
    }
    Ok(message)
}

fn opaque_state_to_anthropic_block(
    state: &OpaqueProviderState,
    source: Protocol,
    target: Protocol,
    path: &str,
) -> Result<OpenAiAnthropicBlock, DirectIrError> {
    let object = json_object(&state.raw).ok_or_else(|| {
        DirectIrError::new(
            source,
            target,
            "opaque_state",
            path,
            DirectIrReason::InvalidOpaqueState,
        )
    })?;
    let kind = object
        .get("type")
        .and_then(json_string)
        .ok_or_else(|| DirectIrError::missing(source, target, "opaque_state.type", path))?;
    let extra = object
        .get("extra")
        .and_then(json_object)
        .cloned()
        .unwrap_or_default();
    match kind {
        "thinking" => Ok(OpenAiAnthropicBlock {
            kind: "thinking".to_owned(),
            text: None,
            thinking: object
                .get("thinking")
                .and_then(json_string)
                .map(str::to_owned),
            signature: object.get("signature").and_then(json_string).map(str::to_owned),
            synthetic: (state.provenance == OpaqueStateProvenance::Synthetic).then_some(true),
            data: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            image_url: None,
            extra,
        }),
        "redacted_thinking" => Ok(OpenAiAnthropicBlock {
            kind: "redacted_thinking".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            synthetic: (state.provenance == OpaqueStateProvenance::Synthetic).then_some(true),
            data: object.get("data").cloned(),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            image_url: None,
            extra,
        }),
        _ => Err(DirectIrError::unsupported(
            source,
            target,
            "anthropic_opaque_state",
            path,
        )),
    }
}

/// Encodes an ordered Envelope as an OpenAI Chat request.
pub fn envelope_to_openai_chat_request_v2(
    envelope: Envelope,
) -> Result<DirectIrConversion<OpenAiChatRequest>, DirectIrError> {
    let source = envelope.source;
    let target = Protocol::OpenAi;
    let mut output = OpenAiChatRequest {
        model: envelope.model.clone(),
        messages: Vec::new(),
        stream: envelope.controls.stream,
        max_tokens: controls_extension(&envelope, "openai.max_tokens").and_then(|value| number_u64(&value)),
        max_completion_tokens: controls_extension(&envelope, "openai.max_completion_tokens")
            .and_then(|value| number_u64(&value)),
        temperature: envelope.controls.temperature,
        tools: Vec::new(),
        tool_choice: Some(tool_choice_to_json(&envelope.tool_choice)),
        top_p: envelope.controls.top_p,
        n: controls_extension(&envelope, "n").and_then(|value| number_u64(&value)),
        reasoning_effort: envelope.controls.reasoning_effort.clone(),
        response_format: envelope.controls.response_format.clone(),
        parallel_tool_calls: envelope.controls.parallel_tool_calls,
        user: controls_extension(&envelope, "user"),
        store: controls_extension(&envelope, "store"),
        metadata: controls_extension(&envelope, "metadata"),
        stream_options: controls_extension(&envelope, "stream_options"),
        top_logprobs: controls_extension(&envelope, "top_logprobs")
            .and_then(|value| match value {
                JsonData::Number(number) => number.as_i64(),
                _ => None,
            }),
        safety_identifier: controls_extension(&envelope, "safety_identifier"),
        prompt_cache_retention: controls_extension(&envelope, "prompt_cache_retention"),
        prompt_cache_key: controls_extension(&envelope, "prompt_cache_key")
            .and_then(|value| json_string(&value).map(str::to_owned)),
        service_tier: controls_extension(&envelope, "service_tier")
            .and_then(|value| json_string(&value).map(str::to_owned)),
        enable_thinking: controls_extension(&envelope, "enable_thinking"),
        thinking_budget: controls_extension(&envelope, "thinking_budget"),
    };
    if let Some(max_output_tokens) = envelope.controls.max_output_tokens {
        if output.max_tokens.is_none() && output.max_completion_tokens.is_none() {
            output.max_completion_tokens = Some(max_output_tokens);
        }
    }
    for tool in &envelope.tools {
        let Some(name) = tool.name.clone() else {
            return Err(DirectIrError::missing(source, target, "tool.name", "tools[]"));
        };
        if !matches!(tool.kind, ToolKind::Function) {
            return Err(DirectIrError::unsupported(source, target, "tool_type", "tools[]"));
        }
        output.tools.push(OpenAiChatTool {
            kind: "function".to_owned(),
            function: OpenAiFunction {
                name,
                arguments: String::new(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
                strict: None,
            },
        });
    }
    for (index, item) in envelope.ordered_items().iter().enumerate() {
        output.messages.push(openai_message_from_item(
            item,
            &envelope,
            source,
            target,
            &format!("items[{index}]"),
        )?);
    }
    Ok(finish(output, envelope, source, target))
}

/// Protocol-first alias for the OpenAI Chat request encoder.
pub fn envelope_to_openai_chat_request(
    envelope: Envelope,
) -> Result<DirectIrConversion<OpenAiChatRequest>, DirectIrError> {
    envelope_to_openai_chat_request_v2(envelope)
}
