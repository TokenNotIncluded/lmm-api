//! Offline microbench calibration for request, response, and stream paths.
//!
//! This intentionally uses only the standard library harness (`Instant` and
//! `black_box`) plus the production contract APIs.  It reports measured
//! throughput and average latency; it does not invent a p95 without a sample
//! distribution and an accepted baseline.  The tests are ignored by default
//! so a normal test run does not become a benchmark run.

use std::{hint::black_box, time::Instant};

use lmm_api_rs::{
    migration_routes::sse::SseFrameParser,
    protocol_runtime_registry::validated_current_registry,
};
use lmm_contracts::relay::{
    CanonicalStreamEvent, OpenAiChatRequest, OpenAiChatResponse, OpenAiStreamSnapshot,
    openai_chat_request_to_canonical, openai_chat_response_to_canonical,
    openai_stream_to_canonical,
};

const ITERATIONS: usize = 128;

const REQUEST_CORPUS: &[u8] = br#"{
  "model":"gpt-bench",
  "messages":[{"role":"user","content":"calibrate"}],
  "stream":false
}"#;

const RESPONSE_CORPUS: &[u8] = br#"{
  "id":"bench-response",
  "object":"chat.completion",
  "created":1,
  "model":"gpt-bench",
  "choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],
  "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}
}"#;

const STREAM_CORPUS: &[u8] = br#"{
  "events":[
    {"id":"bench-stream","model":"gpt-bench","choices":[{"index":0,"delta":{"role":"assistant","content":"a"}}]},
    {"id":"bench-stream","model":"gpt-bench","choices":[{"index":0,"delta":{"content":"b"}}]},
    {"id":"bench-stream","model":"gpt-bench","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}
  ],
  "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}
}"#;

const SSE_CORPUS: &[u8] = b"event: update\r\ndata: one\r\ndata: two\r\n\r\ndata: [DONE]\n\n";

const NATIVE_PASSTHROUGH_CORPUS: &[u8] =
    b"data: provider bytes stay opaque\n\ndata: [DONE]\n\n";

fn report(label: &str, bytes: usize, elapsed_nanos: u128, checksum: u64) {
    let elapsed_nanos = elapsed_nanos.max(1);
    let total_bytes = (bytes as u128).saturating_mul(ITERATIONS as u128);
    let bytes_per_second = match total_bytes
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_div(elapsed_nanos))
    {
        Some(value) => value,
        None => 0,
    };
    let nanos_per_iteration = match elapsed_nanos.checked_div(ITERATIONS as u128) {
        Some(value) => value,
        None => 0,
    };
    eprintln!(
        "protocol_hotpath label={label} iterations={ITERATIONS} bytes_per_second={bytes_per_second} nanos_per_iteration={nanos_per_iteration} checksum={checksum}"
    );
    assert_ne!(checksum, 0, "benchmark semantic checksum must be non-zero");
}

#[test]
#[ignore = "run explicitly as an offline calibration benchmark"]
fn request_conversion_hotpath_calibration() {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..ITERATIONS {
        let request: OpenAiChatRequest =
            serde_json::from_slice(black_box(REQUEST_CORPUS)).expect("request corpus");
        let converted = black_box(
            openai_chat_request_to_canonical(request).expect("request conversion"),
        );
        checksum = checksum
            .wrapping_add(converted.value.model.len() as u64)
            .wrapping_add(converted.value.messages.len() as u64);
    }
    report("request", REQUEST_CORPUS.len(), started.elapsed().as_nanos(), checksum);
}

#[test]
#[ignore = "run explicitly as an offline calibration benchmark"]
fn response_conversion_hotpath_calibration() {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..ITERATIONS {
        let response: OpenAiChatResponse =
            serde_json::from_slice(black_box(RESPONSE_CORPUS)).expect("response corpus");
        let converted = black_box(
            openai_chat_response_to_canonical(response).expect("response conversion"),
        );
        checksum = checksum
            .wrapping_add(converted.value.id.len() as u64)
            .wrapping_add(converted.value.output.len() as u64)
            .wrapping_add(
                converted
                    .value
                    .usage
                    .as_ref()
                    .map_or(0, |usage| usage.total_tokens),
            );
    }
    report("response", RESPONSE_CORPUS.len(), started.elapsed().as_nanos(), checksum);
}

#[test]
#[ignore = "run explicitly as an offline calibration benchmark"]
fn stream_conversion_hotpath_calibration() {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..ITERATIONS {
        let snapshot: OpenAiStreamSnapshot =
            serde_json::from_slice(black_box(STREAM_CORPUS)).expect("stream corpus");
        let events = black_box(openai_stream_to_canonical(&snapshot));
        checksum = events.iter().fold(checksum, |value, event| match event {
            CanonicalStreamEvent::TextDelta { delta, .. }
            | CanonicalStreamEvent::ReasoningDelta { delta, .. }
            | CanonicalStreamEvent::ToolArgumentsDelta { delta, .. } => {
                value.wrapping_add(delta.len() as u64)
            }
            _ => value.wrapping_add(1),
        });
    }
    report("stream", STREAM_CORPUS.len(), started.elapsed().as_nanos(), checksum);
}

#[test]
#[ignore = "run explicitly as an offline calibration benchmark"]
fn sse_frame_parser_hotpath_calibration() {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..ITERATIONS {
        let mut parser = SseFrameParser::new(1024);
        let frames = black_box(parser.feed(SSE_CORPUS).expect("SSE corpus"));
        let tail = black_box(parser.finish().expect("SSE EOF"));
        checksum = checksum
            .wrapping_add(frames.iter().map(|frame| frame.raw.len() as u64).sum::<u64>())
            .wrapping_add(tail.iter().map(|frame| frame.raw.len() as u64).sum::<u64>());
    }
    report("sse", SSE_CORPUS.len(), started.elapsed().as_nanos(), checksum);
}

#[test]
#[ignore = "run explicitly as an offline calibration benchmark"]
fn native_passthrough_hotpath_calibration() {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..ITERATIONS {
        let bytes = black_box(NATIVE_PASSTHROUGH_CORPUS);
        checksum = bytes.iter().fold(checksum, |value, byte| {
            value.wrapping_mul(257).wrapping_add(u64::from(*byte))
        });
    }
    report(
        "native_passthrough",
        NATIVE_PASSTHROUGH_CORPUS.len(),
        started.elapsed().as_nanos(),
        checksum,
    );
}

#[test]
#[ignore = "run explicitly as an offline calibration benchmark"]
fn plan_compile_hotpath_calibration() {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..ITERATIONS {
        let registry = black_box(validated_current_registry().expect("runtime registry"));
        checksum = checksum
            .wrapping_add(registry.support_matrix().routes.len() as u64)
            .wrapping_add(registry.runtime_catalog_version().len() as u64);
    }
    report("plan_compile", 1, started.elapsed().as_nanos(), checksum);
}
