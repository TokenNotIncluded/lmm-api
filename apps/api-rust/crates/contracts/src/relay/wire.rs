//! Provider wire DTOs used by the three migrated relay routes.
//!
//! Legacy provenance:
//! `relaykit/dto/openai_request.go`, `openai_response.go`, `claude.go`,
//! `gemini.go`, plus the response stream DTOs used by `relayconvert`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::JsonData;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrParts<T> {
    String(String),
    Parts(Vec<T>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiChatRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<OpenAiChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAiChatTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_retention: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<StringOrParts<OpenAiChatContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OpenAiToolCall>,
    /// Provider extensions carried through an OpenAI-compatible envelope.
    /// Google thought signatures live at message level when the source Part
    /// was not a function call (including an empty text Part).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<OpenAiExtraContent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiChatContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: OpenAiFunction,
    /// Provider extensions attached to this exact tool call Part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<OpenAiExtraContent>,
}

/// OpenAI-compatible provider extension envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiExtraContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google: Option<OpenAiGoogleExtraContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<OpenAiAnthropicExtraContent>,
}

/// Google extension fields used to preserve opaque Gemini state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiGoogleExtraContent {
    #[serde(
        rename = "thought_signature",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub thought_signature: Option<String>,
    /// Explicitly records that the value was generated for synthetic history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthetic: Option<bool>,
}

/// Ordered Claude blocks carried through an OpenAI-compatible message.  Chat
/// has no native representation for signed or redacted thinking, nor for
/// interleaving those blocks with text and tool use, so Claude conversions
/// use this explicit extension instead of silently flattening them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiAnthropicExtraContent {
    pub blocks: Vec<OpenAiAnthropicBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One ordered Claude content block in the OpenAI-compatible extension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiAnthropicBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthetic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<JsonData>,
    /// Unknown fields are retained so a Claude extension can make a typed
    /// round-trip even when a newer block carries provider metadata.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

fn function_kind() -> String {
    "function".to_owned()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiFunction {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiChatTool {
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: OpenAiFunction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesRequest {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ResponsesInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ResponsesTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_retention: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_management: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderation: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<JsonData>,
    /// Provider additions are retained by the typed boundary.  Cross-
    /// protocol converters must inspect this map and reject fields they
    /// cannot express; native OpenAI Responses relays forward the raw body.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    String(String),
    Items(Vec<ResponsesInputItem>),
    /// A syntactically valid but unsupported input shape is retained so a
    /// converter can return a path-aware feature error instead of exposing a
    /// generic serde type error.
    Json(JsonData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<JsonData>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesInputItem {
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<StringOrParts<ResponsesContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<JsonData>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesTool {
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    /// Function tools require a non-empty name, while OpenAI built-in and
    /// future tool kinds may legitimately omit it.  Keeping this optional is
    /// what lets preflight return a typed `tools[i]` error for those kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeRequest {
    pub model: String,
    pub max_tokens: u64,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<StringOrParts<ClaudeContentBlock>>,
    #[serde(default)]
    pub messages: Vec<ClaudeMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ClaudeTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ClaudeToolChoice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeMessage {
    pub role: String,
    pub content: StringOrParts<ClaudeContentBlock>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Present on Claude `thinking` blocks, including an empty thinking
    /// string.  It is distinct from `text` so an empty signed block survives
    /// typed and raw round-trips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Opaque payload of a Claude `redacted_thinking` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ClaudeMediaSource>,
    /// Forward-compatible provider fields are kept verbatim rather than
    /// discarded by serde when Claude adds a new block extension.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeMediaSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: JsonData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeToolChoice {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiRequest {
    #[serde(default)]
    pub contents: Vec<GeminiContent>,
    #[serde(
        rename = "systemInstruction",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub system_instruction: Option<GeminiContent>,
    #[serde(
        rename = "generationConfig",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub generation_config: Option<GeminiGenerationConfig>,
    #[serde(
        rename = "safetySettings",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub safety_settings: Vec<GeminiSafetySetting>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<GeminiTool>,
    #[serde(
        rename = "toolConfig",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_config: Option<GeminiToolConfig>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub parts: Vec<GeminiPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(
        rename = "inlineData",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub inline_data: Option<GeminiInlineData>,
    #[serde(
        rename = "functionCall",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub function_call: Option<GeminiFunctionCall>,
    #[serde(
        rename = "functionResponse",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub function_response: Option<GeminiFunctionResponse>,
    #[serde(
        rename = "thoughtSignature",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub thought_signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiFunctionCall {
    /// Gemini's opaque function-call identifier.  Official Gemini IDs are
    /// strings and must be copied byte-for-byte across a tool loop.  The typed
    /// boundary rejects non-string IDs rather than silently reformatting them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiFunctionResponse {
    /// Exact identifier of the function call this result answers.  As with
    /// [`GeminiFunctionCall::id`], only the official string form is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub response: JsonData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiGenerationConfig {
    #[serde(
        rename = "maxOutputTokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiSafetySetting {
    pub category: String,
    pub threshold: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiTool {
    #[serde(rename = "functionDeclarations", default)]
    pub function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiFunctionDeclaration {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: JsonData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiFunctionCallingConfig {
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub model: String,
    pub object: String,
    pub created: i64,
    #[serde(default)]
    pub choices: Vec<OpenAiChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<WireUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiChoice {
    pub index: usize,
    pub message: OpenAiChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesResponse {
    pub id: String,
    pub object: String,
    #[serde(default)]
    pub created_at: i64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    pub model: String,
    #[serde(default)]
    pub output: Vec<ResponsesOutputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<WireUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncompleteDetails {
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesOutputItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ResponsesOutputContent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Vec<ResponsesOutputContent>>,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub size: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesOutputContent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<JsonData>>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub role: String,
    pub model: String,
    #[serde(default)]
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ClaudeUsage>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub claude_cache_creation_5_m_tokens: u64,
    #[serde(default)]
    pub claude_cache_creation_1_h_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_usage: Option<Box<BillingUsage>>,
    /// Provider usage counters introduced after this DTO was published.
    /// Keeping them verbatim prevents a typed round-trip from silently
    /// deleting accounting data that the relay does not understand yet.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiResponse {
    #[serde(default)]
    pub candidates: Vec<GeminiCandidate>,
    #[serde(
        rename = "usageMetadata",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub usage_metadata: Option<GeminiUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiCandidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(
        rename = "finishReason",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub finish_reason: Option<String>,
    pub content: GeminiContent,
    #[serde(
        rename = "safetyRatings",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub safety_ratings: Option<JsonData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GeminiUsage {
    #[serde(rename = "promptTokenCount", default)]
    pub prompt_token_count: u64,
    #[serde(rename = "candidatesTokenCount", default)]
    pub candidates_token_count: u64,
    #[serde(rename = "thoughtsTokenCount", default)]
    pub thoughts_token_count: u64,
    #[serde(rename = "totalTokenCount", default)]
    pub total_token_count: u64,
    #[serde(rename = "cachedContentTokenCount", default)]
    pub cached_content_token_count: u64,
    #[serde(rename = "toolUsePromptTokenCount", default)]
    pub tool_use_prompt_token_count: u64,
    #[serde(
        rename = "promptTokensDetails",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt_tokens_details: Option<JsonData>,
    #[serde(
        rename = "candidatesTokensDetails",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub candidates_tokens_details: Option<JsonData>,
    #[serde(
        rename = "toolUsePromptTokensDetails",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_use_prompt_tokens_details: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_usage: Option<Box<BillingUsage>>,
    /// Provider usage counters introduced after this DTO was published.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<TokenDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<TokenDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<TokenDetails>,
    #[serde(default)]
    pub claude_cache_creation_5_m_tokens: u64,
    #[serde(default)]
    pub claude_cache_creation_1_h_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_semantic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_usage: Option<Box<BillingUsage>>,
    /// Provider usage/cache fields not known by this version of the DTO.
    /// Direct same-protocol paths can serialize these values unchanged;
    /// cross-protocol paths must account for them in the loss ledger.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenDetails {
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub cached_creation_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub text_tokens: u64,
    #[serde(default)]
    pub audio_tokens: u64,
    #[serde(default)]
    pub image_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    /// Provider token dimensions introduced after this DTO was published.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BillingUsage {
    pub source: String,
    pub semantic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_usage: Option<Box<WireUsage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_usage: Option<ClaudeUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_usage_metadata: Option<GeminiUsage>,
    /// Provider billing fields not normalized by the relay yet.
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiStreamSnapshot {
    pub events: Vec<OpenAiStreamChunk>,
    pub usage: WireUsage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiStreamChunk {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<OpenAiStreamChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<WireUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cancelled: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiStreamChoice {
    pub delta: OpenAiStreamDelta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    pub index: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenAiStreamDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OpenAiStreamToolCall>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiStreamToolCall {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<OpenAiStreamFunction>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenAiStreamFunction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesStreamSnapshot {
    pub events: Vec<ResponsesStreamEvent>,
    pub usage: WireUsage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesStreamEvent {
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(rename = "Payload")]
    pub payload: ResponsesEventPayload,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsesEventPayload {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponsesResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ResponsesOutputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<ResponsesOutputContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeStreamSnapshot {
    pub events: Vec<ClaudeStreamEvent>,
    pub usage: WireUsage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<ClaudeResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_block: Option<ClaudeContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<ClaudeStreamDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ClaudeUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaudeStreamDelta {
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Claude `signature_delta` is opaque data.  It must never be converted
    /// to a text/newline delta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ClaudeUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: BTreeMap<String, JsonData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiStreamSnapshot {
    pub events: Vec<GeminiResponse>,
    pub usage: WireUsage,
}
