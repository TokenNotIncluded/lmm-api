//! Source-level acceptance coverage for the protocol IR, converters, SSE
//! framing, rollout boundary, and bounded observability.
//!
//! The corpus is deliberately embedded in Rust so this test remains useful in
//! an offline checkout.  It is semantic coverage, rather than a replacement
//! for the Go oracle or a network integration test.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use lmm_api_rs::{
    conversion_observability::{
        ClientAbortGuard, ConversionObserver, ConversionResult, ConverterVersion, FeatureClass,
        MetricKind, MetricLabels, StreamTiming,
    },
    migration_routes::sse::{
        DEFAULT_MAX_FRAME_BYTES, SseError, SseFrameParser, UnknownEventAction, UnknownEventClass,
        json_events_from_frames, parse_sse_frames, unknown_event_decision,
    },
    protocol_rollout::{
        LocalConversionError, LocalConversionSummary, LocalRequest, ShadowDifference, ShadowRunner,
    },
    protocol_runtime_registry::current_support_matrix,
    route_ownership::{
        DifferentialClass, MIN_REVIEW_CANARY_BASIS_POINTS, OwnershipDecision, OwnershipEvidence,
        OwnershipGate, RouteOwnershipScope,
    },
};
use lmm_contracts::relay::{
    CacheUsage, CanonicalContent, ClaudeRequest, ClaudeResponse, ClaudeStreamSemanticEvent,
    ClaudeStreamSnapshot, Envelope, Feature, FinishReason, FunctionData, GeminiRequest,
    GeminiResponse, GeminiStreamSnapshot, Item, ItemKind, JsonData, Loss, LossCode, Media,
    MediaKind, Money, OpaqueId, OpaqueIdProvenance, OpaqueProviderState, OpenAiChatRequest,
    OpenAiChatResponse, OpenAiResponsesRequest, OpenAiStreamSnapshot, Part, PartKind, Protocol,
    Provenance, ResponsesResponse, ResponsesStreamSnapshot, Role, SemanticBillingUsage,
    SemanticUsage, TokenUsage, Tool, ToolChoice, canonical_request_to_claude,
    canonical_request_to_gemini_for_model, canonical_request_to_openai_chat,
    canonical_response_to_claude, canonical_response_to_gemini, claude_request_to_canonical,
    claude_response_to_canonical, claude_stream_to_semantic_events,
    gemini_request_to_canonical_for_model, gemini_response_to_canonical_for_model,
    gemini_stream_to_canonical, openai_chat_request_to_canonical,
    openai_chat_response_to_canonical, openai_responses_request_to_canonical,
    openai_responses_response_to_canonical, openai_stream_to_canonical,
    responses_stream_to_canonical,
};

const CHAT_REQUEST: &str = r#"{
  "model":"gpt-test",
  "messages":[
    {"role":"user","content":"hello"},
    {"role":"assistant","tool_calls":[{"id":"call-chat","type":"function","function":{"name":"lookup","arguments":"{\"q\":\"x\"}"}}]},
    {"role":"tool","tool_call_id":"call-chat","content":"ok"}
  ]
}"#;

const RESPONSES_REQUEST: &str = r#"{
  "model":"responses-test",
  "input":[
    {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]},
    {"type":"function_call","call_id":"call-responses","name":"lookup","arguments":"{}"},
    {"type":"function_call_output","call_id":"call-responses","output":"ok"}
  ]
}"#;

const GEMINI_REQUEST: &str = r#"{
  "contents":[
    {"role":"model","parts":[{"functionCall":{"id":"call-gemini","name":"lookup","args":{"q":"x"}},"thoughtSignature":"sig-gemini"}]},
    {"role":"user","parts":[{"functionResponse":{"id":"call-gemini","name":"lookup","response":{"ok":true}}}]}
  ]
}"#;

const CLAUDE_REQUEST: &str = r#"{
  "model":"claude-test",
  "max_tokens":128,
  "messages":[
    {"role":"assistant","content":[{"type":"thinking","thinking":"plan","signature":"sig-claude"},{"type":"tool_use","id":"call-claude","name":"lookup","input":{}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-claude","content":"ok"}]}
  ]
}"#;

const CHAT_RESPONSE: &str = r#"{
  "id":"chat-response",
  "object":"chat.completion",
  "created":1,
  "model":"gpt-test",
  "choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],
  "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3,"prompt_tokens_details":{"cached_tokens":1}}
}"#;

const RESPONSES_RESPONSE: &str = r#"{
  "id":"responses-response",
  "object":"response",
  "status":"completed",
  "model":"responses-test",
  "output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"answer"}]}],
  "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}
}"#;

const CLAUDE_RESPONSE: &str = r#"{
  "id":"claude-response",
  "type":"message",
  "role":"assistant",
  "model":"claude-test",
  "content":[{"type":"thinking","thinking":"plan","signature":"sig-claude"},{"type":"text","text":"answer"}],
  "stop_reason":"end_turn",
  "usage":{"input_tokens":2,"output_tokens":1}
}"#;

const GEMINI_RESPONSE: &str = r#"{
  "candidates":[{"finishReason":"STOP","content":{"role":"model","parts":[{"text":"answer","thoughtSignature":"sig-gemini"}]}}],
  "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1,"totalTokenCount":3}
}"#;

const CHAT_STREAM: &str = r#"{
  "events":[
    {"id":"chat-stream","model":"gpt-test","choices":[{"index":0,"delta":{"role":"assistant","content":"a"}}]},
    {"id":"chat-stream","model":"gpt-test","choices":[{"index":0,"delta":{"content":"b"}}]},
    {"id":"chat-stream","model":"gpt-test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}
  ],
  "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}
}"#;

const RESPONSES_STREAM: &str = r#"{
  "events":[
    {"Type":"response.created","Payload":{"type":"response.created","response":{"id":"responses-stream","object":"response","status":"in_progress","model":"responses-test","output":[]}}},
    {"Type":"response.output_text.delta","Payload":{"type":"response.output_text.delta","delta":"ab"}},
    {"Type":"response.completed","Payload":{"type":"response.completed","response":{"id":"responses-stream","object":"response","status":"completed","model":"responses-test","output":[],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}}
  ],
  "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}
}"#;

const GEMINI_STREAM: &str = r#"{
  "events":[
    {"candidates":[{"content":{"role":"model","parts":[{"text":"ab","thoughtSignature":"sig-stream"}]}}]},
    {"candidates":[{"finishReason":"STOP","content":{"role":"model","parts":[]}}]}
  ],
  "usage":{}
}"#;

const CLAUDE_STREAM: &str = r#"{
  "events":[
    {"type":"message_start","message":{"id":"claude-stream","type":"message","role":"assistant","model":"claude-test","content":[]}},
    {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":"sig-stream"}},
    {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}},
    {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-stream"}},
    {"type":"content_block_stop","index":0},
    {"type":"ping"},
    {"type":"message_delta","delta":{"stop_reason":"end_turn"}},
    {"type":"future_event","index":7,"provider_field":"retained"},
    {"type":"message_stop"}
  ],
  "usage":{}
}"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CorpusMetadata {
    source_url: &'static str,
    source_vendor: &'static str,
    retrieved_date: &'static str,
    model_family: &'static str,
    feature: &'static str,
    expected_fidelity: &'static str,
}

const CORPUS_METADATA: [CorpusMetadata; 4] = [
    CorpusMetadata {
        source_url: "https://platform.openai.com/docs/api-reference/chat",
        source_vendor: "OpenAI",
        retrieved_date: "2026-08-12",
        model_family: "openai-chat",
        feature: "ordered tool calls and usage",
        expected_fidelity: "exact typed subset",
    },
    CorpusMetadata {
        source_url: "https://platform.openai.com/docs/api-reference/responses",
        source_vendor: "OpenAI",
        retrieved_date: "2026-08-12",
        model_family: "openai-responses",
        feature: "ordered input items and call_id",
        expected_fidelity: "exact typed subset",
    },
    CorpusMetadata {
        source_url: "https://ai.google.dev/api/generate-content",
        source_vendor: "Google",
        retrieved_date: "2026-08-12",
        model_family: "gemini-2.5",
        feature: "function IDs and thought signatures",
        expected_fidelity: "exact with opaque state",
    },
    CorpusMetadata {
        source_url: "https://docs.anthropic.com/en/api/messages",
        source_vendor: "Anthropic",
        retrieved_date: "2026-08-12",
        model_family: "claude-messages",
        feature: "ordered thinking signatures and tool use",
        expected_fidelity: "exact with opaque state",
    },
];

const ARBITRARY_SSE_BYTES: &[u8] = &[
    0, 255, 10, 13, b':', b'd', b'a', b't', b'a', b':', 0, b'\n', b'\n', 0xef, 0xbb, 0xbf, b'e',
    b'v', b'e', b'n', b't', b':', b'f', b'u', b't', b'u', b'r', b'e', b'\r', b'\n', b'd', b'a',
    b't', b'a', b':', b' ', 0xff, b'\n', b'\n', b'r', b'e', b't', b'r', b'y', b':', b' ', b'+',
    b'1', b'\n', b'\n',
];

#[test]
fn corpus_metadata_is_complete_and_deterministic() {
    assert_eq!(CORPUS_METADATA.len(), 4);
    for metadata in CORPUS_METADATA {
        assert!(metadata.source_url.starts_with("https://"));
        assert!(!metadata.source_vendor.is_empty());
        assert_eq!(metadata.retrieved_date.len(), 10);
        assert!(metadata.retrieved_date.as_bytes()[4] == b'-');
        assert!(metadata.retrieved_date.as_bytes()[7] == b'-');
        assert!(!metadata.model_family.is_empty());
        assert!(!metadata.feature.is_empty());
        assert!(!metadata.expected_fidelity.is_empty());
    }
}

#[test]
fn request_conformance_preserves_order_ids_and_provider_signatures() {
    let chat: OpenAiChatRequest = serde_json::from_str(CHAT_REQUEST).expect("Chat corpus");
    let chat = openai_chat_request_to_canonical(chat).expect("Chat conversion");
    assert_eq!(chat.value.model, "gpt-test");
    assert_eq!(chat.value.messages.len(), 3);
    assert!(
        chat.value
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .any(|part| matches!(part, CanonicalContent::ToolCall { id, .. } if id == "call-chat"))
    );
    assert!(
        chat.value
            .messages
            .iter()
            .any(|message| message.role == lmm_contracts::relay::Role::Tool)
    );

    let responses: OpenAiResponsesRequest =
        serde_json::from_str(RESPONSES_REQUEST).expect("Responses corpus");
    let responses = openai_responses_request_to_canonical(responses).expect("Responses conversion");
    assert_eq!(responses.value.messages.len(), 3);
    assert!(responses.value.messages.iter().flat_map(|message| &message.parts).any(
        |part| matches!(part, CanonicalContent::ToolCall { id, .. } if id == "call-responses")
    ));

    let gemini: GeminiRequest = serde_json::from_str(GEMINI_REQUEST).expect("Gemini corpus");
    let gemini =
        gemini_request_to_canonical_for_model(gemini, "gemini-2.5-pro").expect("Gemini conversion");
    assert_eq!(gemini.value.messages.len(), 2);
    assert!(
        gemini
            .value
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .any(
                |part| matches!(part, CanonicalContent::ToolCall { id, .. } if id == "call-gemini")
            )
    );
    assert!(gemini.value.messages.iter().flat_map(|message| &message.parts).any(
        |part| matches!(part, CanonicalContent::ProviderState { state } if state.raw == JsonData::String("sig-gemini".to_owned()))
    ));

    let claude: ClaudeRequest = serde_json::from_str(CLAUDE_REQUEST).expect("Claude corpus");
    let claude = claude_request_to_canonical(claude).expect("Claude conversion");
    assert_eq!(claude.value.messages.len(), 2);
    assert!(matches!(
        claude.value.messages[0].parts.first(),
        Some(CanonicalContent::ClaudeThinking { thinking, signature: Some(signature), .. })
            if thinking == "plan" && signature == "sig-claude"
    ));
    assert!(
        claude
            .value
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .any(
                |part| matches!(part, CanonicalContent::ToolCall { id, .. } if id == "call-claude")
            )
    );
}

#[test]
fn request_round_trip_keeps_authentic_tool_and_signature_data() {
    let chat: OpenAiChatRequest = serde_json::from_str(CHAT_REQUEST).expect("Chat corpus");
    let canonical = openai_chat_request_to_canonical(chat).expect("Chat conversion");
    let round_trip = canonical_request_to_openai_chat(canonical.value).expect("Chat round trip");
    assert_eq!(round_trip.value.messages[1].tool_calls[0].id, "call-chat");
    assert!(round_trip.loss.is_lossless());

    let gemini: GeminiRequest = serde_json::from_str(GEMINI_REQUEST).expect("Gemini corpus");
    let canonical =
        gemini_request_to_canonical_for_model(gemini, "gemini-2.5-pro").expect("Gemini conversion");
    let round_trip =
        canonical_request_to_gemini_for_model(canonical.value, "gemini-2.5-pro", false)
            .expect("Gemini round trip");
    let function_call = round_trip.value.contents[0].parts[0]
        .function_call
        .as_ref()
        .expect("function call");
    assert_eq!(function_call.id.as_deref(), Some("call-gemini"));
    assert_eq!(
        round_trip.value.contents[0].parts[0]
            .thought_signature
            .as_deref(),
        Some("sig-gemini")
    );

    let claude: ClaudeRequest = serde_json::from_str(CLAUDE_REQUEST).expect("Claude corpus");
    let canonical = claude_request_to_canonical(claude).expect("Claude conversion");
    let round_trip = canonical_request_to_claude(canonical.value).expect("Claude round trip");
    let first_message = &round_trip.value.messages[0];
    let blocks = match &first_message.content {
        lmm_contracts::relay::StringOrParts::Parts(parts) => parts,
        lmm_contracts::relay::StringOrParts::String(_) => panic!("Claude blocks flattened"),
    };
    assert_eq!(blocks[0].signature.as_deref(), Some("sig-claude"));
    assert_eq!(blocks[1].id.as_deref(), Some("call-claude"));
}

#[test]
fn response_conformance_preserves_usage_finish_order_and_opaque_state() {
    let chat: OpenAiChatResponse = serde_json::from_str(CHAT_RESPONSE).expect("Chat response");
    let chat = openai_chat_response_to_canonical(chat).expect("Chat response conversion");
    assert_eq!(chat.value.id, "chat-response");
    assert_eq!(chat.value.finish_reason, Some(FinishReason::Stop));
    assert_eq!(
        chat.value.usage.as_ref().map(|usage| usage.total_tokens),
        Some(3)
    );
    assert!(
        matches!(chat.value.output.first(), Some(CanonicalContent::Text { text }) if text == "answer")
    );
    let chat_wire = lmm_contracts::relay::canonical_response_to_openai_chat(chat.value)
        .expect("Chat response round trip");
    assert_eq!(chat_wire.value.id, "chat-response");

    let responses: ResponsesResponse =
        serde_json::from_str(RESPONSES_RESPONSE).expect("Responses response");
    let responses =
        openai_responses_response_to_canonical(responses).expect("Responses response conversion");
    assert_eq!(responses.value.finish_reason, Some(FinishReason::Stop));
    assert_eq!(
        responses
            .value
            .usage
            .as_ref()
            .map(|usage| usage.total_tokens),
        Some(3)
    );

    let claude: ClaudeResponse = serde_json::from_str(CLAUDE_RESPONSE).expect("Claude response");
    let claude = claude_response_to_canonical(claude).expect("Claude response conversion");
    assert!(claude.value.output.iter().any(
        |part| matches!(part, CanonicalContent::ClaudeThinking { signature: Some(signature), .. } if signature == "sig-claude")
    ));
    assert_eq!(
        claude.value.usage.as_ref().map(|usage| usage.total_tokens),
        Some(3)
    );
    let claude_wire =
        canonical_response_to_claude(claude.value).expect("Claude response round trip");
    assert_eq!(
        claude_wire.value.content[0].signature.as_deref(),
        Some("sig-claude")
    );

    let gemini: GeminiResponse = serde_json::from_str(GEMINI_RESPONSE).expect("Gemini response");
    let gemini = gemini_response_to_canonical_for_model(gemini, "gemini-2.5-pro")
        .expect("Gemini response conversion");
    assert_eq!(gemini.value.finish_reason, Some(FinishReason::Stop));
    assert_eq!(
        gemini.value.usage.as_ref().map(|usage| usage.total_tokens),
        Some(3)
    );
    assert!(gemini.value.output.iter().any(
        |part| matches!(part, CanonicalContent::ProviderState { state } if state.raw == JsonData::String("sig-gemini".to_owned()))
    ));
    let gemini_wire =
        canonical_response_to_gemini(gemini.value).expect("Gemini response round trip");
    assert_eq!(
        gemini_wire.value.candidates[0].content.parts[0]
            .thought_signature
            .as_deref(),
        Some("sig-gemini")
    );
}

fn semantic_usage_from_token_usage(usage: &TokenUsage) -> SemanticUsage {
    SemanticUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cache: CacheUsage {
            read_input_tokens: usage.cached_input_tokens,
            write_input_tokens: 0,
            creation_input_tokens: 0,
            extensions: Default::default(),
        },
        billing: None,
        extensions: Default::default(),
    }
}

#[test]
fn usage_cache_billing_differential_matches_legacy_and_new_semantics() {
    let chat: OpenAiChatResponse = serde_json::from_str(CHAT_RESPONSE).expect("Chat response");
    let chat = openai_chat_response_to_canonical(chat)
        .expect("Chat response conversion")
        .value;
    let responses: ResponsesResponse =
        serde_json::from_str(RESPONSES_RESPONSE).expect("Responses response");
    let responses = openai_responses_response_to_canonical(responses)
        .expect("Responses response conversion")
        .value;
    let claude: ClaudeResponse = serde_json::from_str(CLAUDE_RESPONSE).expect("Claude response");
    let claude = claude_response_to_canonical(claude)
        .expect("Claude response conversion")
        .value;
    let gemini: GeminiResponse = serde_json::from_str(GEMINI_RESPONSE).expect("Gemini response");
    let gemini = gemini_response_to_canonical_for_model(gemini, "gemini-2.5-pro")
        .expect("Gemini response conversion")
        .value;

    let cases = [
        ("openai-chat", chat.usage.expect("Chat usage"), (2, 1, 3, 1)),
        (
            "openai-responses",
            responses.usage.expect("Responses usage"),
            (2, 1, 3, 0),
        ),
        ("claude", claude.usage.expect("Claude usage"), (2, 1, 3, 0)),
        ("gemini", gemini.usage.expect("Gemini usage"), (2, 1, 3, 0)),
    ];
    for (name, new_usage, legacy_usage) in cases {
        let new_tuple = (
            new_usage.input_tokens,
            new_usage.output_tokens,
            new_usage.total_tokens,
        );
        assert_eq!(
            new_tuple,
            (legacy_usage.0, legacy_usage.1, legacy_usage.2),
            "legacy/new token usage: {name}"
        );
        let semantic = semantic_usage_from_token_usage(&new_usage);
        assert_eq!(
            semantic.input_tokens, legacy_usage.0,
            "semantic input: {name}"
        );
        assert_eq!(
            semantic.output_tokens, legacy_usage.1,
            "semantic output: {name}"
        );
        assert_eq!(
            semantic.total_tokens, legacy_usage.2,
            "semantic total: {name}"
        );
        assert_eq!(
            new_usage.cached_input_tokens, legacy_usage.3,
            "legacy cache: {name}"
        );
        assert_eq!(
            semantic.cache.read_input_tokens, legacy_usage.3,
            "semantic cache: {name}"
        );
        assert!(semantic.billing.is_none(), "billing absent: {name}");
    }
}

#[test]
fn stream_conformance_preserves_order_unknown_events_and_cancel_class() {
    let chat: OpenAiStreamSnapshot = serde_json::from_str(CHAT_STREAM).expect("Chat stream");
    let chat_events = openai_stream_to_canonical(&chat);
    let chat_text = chat_events
        .iter()
        .filter_map(|event| match event {
            lmm_contracts::relay::CanonicalStreamEvent::TextDelta { delta, .. } => {
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(chat_text, vec!["a", "b"]);
    assert!(chat_events.iter().any(|event| matches!(
        event,
        lmm_contracts::relay::CanonicalStreamEvent::ResponseEnd {
            finish_reason: FinishReason::Stop,
            ..
        }
    )));

    let responses: ResponsesStreamSnapshot =
        serde_json::from_str(RESPONSES_STREAM).expect("Responses stream");
    let responses_events = responses_stream_to_canonical(&responses);
    assert!(responses_events.iter().any(|event| matches!(event, lmm_contracts::relay::CanonicalStreamEvent::TextDelta { delta, .. } if delta == "ab")));
    assert!(responses_events.iter().any(|event| matches!(
        event,
        lmm_contracts::relay::CanonicalStreamEvent::ResponseEnd {
            finish_reason: FinishReason::Stop,
            ..
        }
    )));

    let gemini: GeminiStreamSnapshot = serde_json::from_str(GEMINI_STREAM).expect("Gemini stream");
    let gemini =
        gemini_stream_to_canonical(&gemini, "gemini-2.5-pro").expect("Gemini stream conversion");
    assert!(
        gemini
            .value
            .output
            .iter()
            .any(|part| matches!(part, CanonicalContent::Text { text } if text == "ab"))
    );
    assert!(gemini.value.output.iter().any(|part| matches!(part, CanonicalContent::ProviderState { state } if state.raw == JsonData::String("sig-stream".to_owned()))));

    let claude: ClaudeStreamSnapshot = serde_json::from_str(CLAUDE_STREAM).expect("Claude stream");
    let claude_events =
        claude_stream_to_semantic_events(&claude).expect("Claude stream conversion");
    assert!(claude_events.iter().any(|event| matches!(event, ClaudeStreamSemanticEvent::SignatureDelta { signature, .. } if signature == "sig-stream")));
    assert!(claude_events.iter().any(|event| matches!(event, ClaudeStreamSemanticEvent::Unknown { kind, .. } if kind == "future_event")));
    assert!(
        claude_events
            .iter()
            .any(|event| matches!(event, ClaudeStreamSemanticEvent::Ping))
    );
    assert!(
        claude_events
            .iter()
            .any(|event| matches!(event, ClaudeStreamSemanticEvent::MessageStop))
    );

    let same_protocol = unknown_event_decision(true, Some("future_event"));
    assert_eq!(same_protocol.class, UnknownEventClass::Content);
    assert_eq!(same_protocol.action, UnknownEventAction::Preserve);
    assert_eq!(same_protocol.loss_code, None);
    let metadata = unknown_event_decision(false, Some("message_metadata"));
    assert_eq!(metadata.class, UnknownEventClass::Metadata);
    assert_eq!(metadata.action, UnknownEventAction::RecordLossAndContinue);
    assert!(metadata.loss_code.is_some());
    let termination = unknown_event_decision(false, Some("future_complete"));
    assert_eq!(termination.class, UnknownEventClass::Termination);
    assert_eq!(termination.action, UnknownEventAction::DegradedOrError);
}

#[test]
fn sse_regression_corpus_is_incremental_bounded_and_panic_free() {
    const CORPUS: &[&[u8]] = &[
        b"data: one\ndata: two\n\n",
        b"\xef\xbb\xbfevent: update\r\ndata: [DONE]\r\n\r\n",
        b"retry: +1\nid: bad\0id\ndata: {\"ok\":true}\n\n",
        b"event: future_content\ndata: raw\n\n",
        b"data: {\xff}\n\n",
        b"data: unfinished\n",
        b"\r\n\n",
    ];

    for input in CORPUS {
        for width in 1..=3 {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let mut parser = SseFrameParser::new(64);
                let mut offset = 0;
                while offset < input.len() {
                    let end = offset.saturating_add(width).min(input.len());
                    let _ = parser.feed(&input[offset..end]);
                    assert!(parser.buffered_frame_bytes() <= 64);
                    offset = end;
                }
                let _ = parser.finish();
                assert!(parser.buffered_frame_bytes() <= 64);
            }));
            assert!(result.is_ok(), "SSE parser panicked for width {width}");
        }
    }

    let mut parser = SseFrameParser::new(128);
    let mut frames = Vec::new();
    for byte in b"event: future\ndata: {\"x\":1}\n\n" {
        frames.extend(
            parser
                .feed(std::slice::from_ref(byte))
                .expect("valid byte feed"),
        );
    }
    frames.extend(parser.finish().expect("valid EOF"));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event_name(), Some("future"));
    assert_eq!(frames[0].raw, b"event: future\ndata: {\"x\":1}\n\n");
    assert!(json_events_from_frames(&frames).is_ok());
    let done =
        parse_sse_frames(b"data: [DONE]\n\n", DEFAULT_MAX_FRAME_BYTES).expect("terminal SSE frame");
    assert!(done[0].is_done());
    assert!(
        parse_sse_frames(b"data: unfinished\n", 64)
            .expect("strict EOF")
            .is_empty()
    );
    assert_eq!(
        parse_sse_frames(b"data: 123\n\n", 5),
        Err(SseError::FrameTooLarge {
            limit: 5,
            observed: 6,
        })
    );
    assert!(matches!(
        json_events_from_frames(&[lmm_api_rs::migration_routes::sse::SseFrame {
            event: None,
            id: Some("id".to_owned()),
            retry: None,
            data: "{}".to_owned(),
            comments: Vec::new(),
            unknown_fields: Vec::new(),
            has_data: true,
            raw: b"id: id\ndata: {}\n\n".to_vec(),
        }]),
        Err(SseError::UnsupportedMetadata { field: "id", .. })
    ));
}

#[test]
fn arbitrary_sse_bytes_are_bounded_without_panics() {
    for width in 1..=7 {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut parser = SseFrameParser::new(64);
            let mut offset = 0;
            while offset < ARBITRARY_SSE_BYTES.len() {
                let end = offset.saturating_add(width).min(ARBITRARY_SSE_BYTES.len());
                let _ = parser.feed(&ARBITRARY_SSE_BYTES[offset..end]);
                assert!(parser.buffered_frame_bytes() <= 64);
                offset = end;
            }
            let _ = parser.finish();
            assert!(parser.buffered_frame_bytes() <= 64);
        }));
        assert!(
            result.is_ok(),
            "arbitrary SSE bytes panicked at width {width}"
        );
    }
}

#[test]
fn observer_and_parser_are_race_ready_with_per_thread_parser_state() {
    let observer = Arc::new(ConversionObserver::with_max_series(8));
    let mut handles = Vec::new();
    for _worker in 0..4 {
        let observer = Arc::clone(&observer);
        handles.push(thread::spawn(move || {
            let labels =
                MetricLabels::native_raw(Protocol::OpenAi, true, ConversionResult::Success);
            for _ in 0..64 {
                observer.record_events(labels, 1);
                let mut parser = SseFrameParser::new(64);
                let _ = parser.feed(b"data: chunk\n\n");
                assert!(parser.buffered_frame_bytes() <= 64);
                let _ = parser.finish();
            }
        }));
    }
    for handle in handles {
        handle.join().expect("worker completed");
    }
    let snapshot = observer.snapshot();
    assert!(snapshot.samples.iter().any(|sample| {
        sample.metric == MetricKind::ConversionEventsTotal && sample.value == 256
    }));
}

#[test]
fn deterministic_generated_cases_keep_each_data_frame_in_order() {
    for seed in 0_u16..32 {
        let body = format!("data: generated-{seed}\n\n");
        let frames = parse_sse_frames(body.as_bytes(), 128).expect("generated frame");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, format!("generated-{seed}"));
        assert_eq!(frames[0].raw, body.as_bytes());
    }
}

#[test]
fn ordered_ir_keeps_authentic_state_and_requires_structured_loss_for_mutation() {
    let mut envelope = Envelope::new(Protocol::OpenAiResponses, "responses-test");
    let mut message = Item::new(
        ItemKind::Message,
        Role::User,
        Provenance::new(Protocol::OpenAiResponses),
    );
    message.push_part(Part::text("first"));
    message.push_part(Part::opaque(
        OpaqueProviderState::authentic_gemini_thought_signature(
            "sig-authentic".to_owned(),
            Some("gemini-2.5-pro".to_owned()),
        ),
    ));
    envelope.push_item(message).expect("valid ordered message");
    assert_eq!(
        envelope.ordered_items()[0].ordered_parts()[0]
            .text
            .as_deref(),
        Some("first")
    );
    assert!(envelope.validate_exact_round_trip().is_ok());

    let mut second = Item::new(
        ItemKind::Message,
        Role::User,
        Provenance::new(Protocol::OpenAiResponses),
    );
    second.push_part(Part::text("second"));
    envelope.push_item(second).expect("second ordered message");
    assert!(envelope.reorder_items(0, 1, None).is_err());
    envelope
        .reorder_items(0, 1, Some(Loss::new(LossCode::LossContentOrder, None)))
        .expect("explicit order loss");
    assert_eq!(
        envelope.ordered_items()[0].ordered_parts()[0]
            .text
            .as_deref(),
        Some("second")
    );
    assert_eq!(envelope.loss_ledger().len(), 1);
    assert!(envelope.validate_exact_round_trip().is_err());
}

#[test]
fn ordered_ir_tool_links_keep_item_id_distinct_from_call_id() {
    let mut envelope = Envelope::new(Protocol::OpenAi, "gpt-test");
    let mut call = Item::tool_call(
        OpaqueId::authentic("call-authentic", Protocol::OpenAi),
        vec![Part::function(FunctionData::call("lookup", JsonData::Null))],
        Provenance::new(Protocol::OpenAi),
    );
    call.id = Some(OpaqueId::authentic("item-id", Protocol::OpenAi));
    let result = Item::tool_result(
        OpaqueId::authentic("call-authentic", Protocol::OpenAi),
        vec![Part::function(FunctionData::result(
            None,
            JsonData::String("ok".to_owned()),
        ))],
        Provenance::new(Protocol::OpenAi),
    );
    envelope.push_item(call).expect("tool call");
    envelope.push_item(result).expect("linked tool result");
    assert_eq!(
        envelope.ordered_items()[0]
            .id
            .as_ref()
            .map(OpaqueId::as_str),
        Some("item-id")
    );
    assert_eq!(
        envelope.ordered_items()[0]
            .call_id
            .as_ref()
            .map(OpaqueId::as_str),
        Some("call-authentic")
    );
    assert_eq!(
        envelope.ordered_items()[1]
            .call_id
            .as_ref()
            .map(OpaqueId::as_str),
        Some("call-authentic")
    );
    assert!(envelope.validate().is_ok());

    let authentic = OpaqueId::authentic("same-bytes", Protocol::OpenAi);
    let synthetic = OpaqueId::synthetic("same-bytes", Protocol::OpenAi);
    assert_eq!(authentic.value, synthetic.value);
    assert_eq!(authentic.provenance, OpaqueIdProvenance::Authentic);
    assert_eq!(synthetic.provenance, OpaqueIdProvenance::Synthetic);
    let mut synthetic_item = Item::new(
        ItemKind::Message,
        Role::User,
        Provenance::new(Protocol::OpenAi),
    );
    synthetic_item.id = Some(synthetic);
    let mut synthetic_envelope = Envelope::new(Protocol::OpenAi, "gpt-test");
    synthetic_envelope
        .push_item(synthetic_item)
        .expect("synthetic id is structurally valid");
    assert!(synthetic_envelope.validate_exact_round_trip().is_err());
}

#[test]
fn usage_cache_billing_and_extensions_round_trip_without_float_loss() {
    let usage = SemanticUsage {
        input_tokens: 10,
        output_tokens: 5,
        total_tokens: 15,
        reasoning_tokens: 2,
        cache: CacheUsage {
            read_input_tokens: 3,
            write_input_tokens: 4,
            creation_input_tokens: 5,
            extensions: Default::default(),
        },
        billing: Some(SemanticBillingUsage {
            source: Some("gateway".to_owned()),
            semantic: Some("provider_reported".to_owned()),
            input_tokens: 10,
            output_tokens: 5,
            cached_input_tokens: 3,
            total_tokens: 15,
            cost: Some(Money {
                amount: "0.000000123456789".to_owned(),
                currency: "USD".to_owned(),
            }),
            extensions: Default::default(),
        }),
        extensions: Default::default(),
    };
    let encoded = serde_json::to_string(&usage).expect("usage serialization");
    let decoded: SemanticUsage = serde_json::from_str(&encoded).expect("usage deserialization");
    assert_eq!(decoded, usage);
    let mut changed_cache = usage.clone();
    changed_cache.cache.read_input_tokens = 4;
    assert_ne!(changed_cache, usage);

    let mut envelope = Envelope::new(Protocol::Claude, "claude-test");
    envelope
        .extensions
        .insert("future_usage_field".to_owned(), JsonData::Bool(true));
    envelope.usage = Some(usage);
    assert!(envelope.validate_exact_round_trip().is_err());
    let encoded = serde_json::to_string(&envelope).expect("envelope serialization");
    assert!(encoded.contains("future_usage_field"));
}

#[test]
fn registry_runtime_matrix_is_complete_and_fail_closed() {
    let matrix = current_support_matrix().expect("validated runtime support matrix");
    let protocols = lmm_contracts::relay::protocols();
    assert_eq!(matrix.routes.len(), protocols.len() * protocols.len());
    for source in protocols {
        for target in protocols {
            let route = matrix.route(source, target).expect("matrix route");
            if route.supports_any_direction() {
                assert!(route.runtime_adaptor().is_some());
                assert!(!route.converter_ids().is_empty());
                assert!(route.stream().finalizer_id.is_some());
            } else {
                assert!(route.runtime_adaptor().is_none());
                assert!(route.converter_ids().is_empty());
                assert!(route.stream().finalizer_id.is_none());
            }
        }
    }
    assert!(matrix.to_json().is_ok());
}

fn local_summary(converter_id: &str, fingerprint: u8) -> LocalConversionSummary {
    LocalConversionSummary {
        converter_id: converter_id.to_owned(),
        plan_fingerprint: [fingerprint; 32],
        semantic_fingerprint: [fingerprint.wrapping_add(1); 32],
        losses: Vec::new(),
        synthetic: Vec::new(),
    }
}

#[test]
fn ownership_gate_defaults_closed_and_requires_all_five_differentials() {
    let scope = RouteOwnershipScope {
        source: Protocol::OpenAi,
        target: Protocol::OpenAi,
        stream: true,
    };
    let gate = OwnershipGate::default();
    assert!(matches!(
        gate.evaluate(&OwnershipEvidence::closed(scope)),
        OwnershipDecision::ClosedByDefault { .. }
    ));

    let mut complete = OwnershipEvidence::closed(scope);
    for class in DifferentialClass::all() {
        complete.mark_green(*class);
    }
    complete.set_shadow_identical(true);
    complete
        .set_canary_basis_points(MIN_REVIEW_CANARY_BASIS_POINTS)
        .expect("bounded canary");
    complete.approve_rollout();
    assert_eq!(
        gate.evaluate(&complete),
        OwnershipDecision::EligibleForOwnershipReview { scope }
    );

    for missing in DifferentialClass::all() {
        let mut green = complete.green_classes().clone();
        green.remove(missing);
        let mut evidence = OwnershipEvidence::closed(scope);
        for class in green {
            evidence.mark_green(class);
        }
        evidence.set_shadow_identical(true);
        evidence
            .set_canary_basis_points(MIN_REVIEW_CANARY_BASIS_POINTS)
            .expect("bounded canary");
        evidence.approve_rollout();
        assert!(matches!(
            gate.evaluate(&evidence),
            OwnershipDecision::ClosedByDefault { .. }
        ));
    }

    let mut below_canary = complete.clone();
    below_canary
        .set_canary_basis_points(99)
        .expect("bounded canary");
    assert!(matches!(
        gate.evaluate(&below_canary),
        OwnershipDecision::ClosedByDefault { .. }
    ));
    let mut shadow_mismatch = complete;
    shadow_mismatch.set_shadow_identical(false);
    assert!(matches!(
        gate.evaluate(&shadow_mismatch),
        OwnershipDecision::ClosedByDefault { .. }
    ));
}

#[test]
fn shadow_comparison_is_body_free_and_client_abort_is_bounded() {
    let runner = ShadowRunner::new(
        |_request: &LocalRequest<'_>| {
            Ok::<LocalConversionSummary, LocalConversionError>(local_summary("old", 7))
        },
        |_request: &LocalRequest<'_>| {
            Ok::<LocalConversionSummary, LocalConversionError>(local_summary("old", 7))
        },
        Protocol::OpenAi,
        Protocol::OpenAiResponses,
        false,
    );
    let identical = runner.compare(&LocalRequest::new(b"private request body"));
    assert!(identical.is_identical());
    assert!(identical.differences.is_empty());

    let mismatch = ShadowRunner::new(
        |_request: &LocalRequest<'_>| {
            Ok::<LocalConversionSummary, LocalConversionError>(local_summary("old", 7))
        },
        |_request: &LocalRequest<'_>| {
            Ok::<LocalConversionSummary, LocalConversionError>(local_summary("new", 7))
        },
        Protocol::OpenAi,
        Protocol::OpenAiResponses,
        true,
    )
    .compare(&LocalRequest::new(b"private request body"));
    assert!(
        mismatch
            .differences
            .contains(&ShadowDifference::ConverterId)
    );
    assert!(!mismatch.differences.is_empty());

    let observer = ConversionObserver::with_max_series(2);
    let labels = MetricLabels::new(
        Protocol::OpenAi,
        Protocol::OpenAiResponses,
        ConverterVersion::OpenAiChatV1,
        1000,
        true,
        FeatureClass::Stream,
        ConversionResult::Success,
    );
    {
        let _abort = ClientAbortGuard::new(observer.clone(), labels);
    }
    observer.record_unknown_event(labels);
    observer.record_client_abort(labels);
    let snapshot = observer.snapshot();
    assert!(snapshot.samples.len() <= 2);
    assert!(snapshot.samples.iter().any(|sample| {
        sample.metric == MetricKind::StreamClientAbortTotal
            && sample.labels.result == ConversionResult::Cancelled
            && sample.value == 2
    }));

    let now = Instant::now();
    let mut timing = StreamTiming::default();
    timing.mark_upstream_event_at(now);
    timing.mark_downstream_write_at(now + Duration::from_millis(1));
    assert!(timing.gateway_ttft_tax().is_some());
}

#[test]
fn typed_mutation_and_provider_types_are_reachable_from_public_api() {
    let tool = Tool::function("lookup", JsonData::Null);
    assert_eq!(tool.name.as_deref(), Some("lookup"));
    assert_eq!(ToolChoice::default(), ToolChoice::Auto);
    let media = Media::new(MediaKind::Image);
    let media_part = Part::media(media);
    assert_eq!(media_part.kind, PartKind::Media);
    assert!(!Feature::all().is_empty());
}
