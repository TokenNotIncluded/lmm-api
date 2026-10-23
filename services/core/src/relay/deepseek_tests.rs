use super::{ClientProtocol, Event, Limits, Request, Route, UpstreamProtocol};
use super::{decode::Decoder, encode::Encoder, sse::Frame};
use serde_json::{Value, json};

fn request(client: ClientProtocol, extra: Value) -> Request {
    let mut value = match client {
        ClientProtocol::Chat => {
            json!({"model":"public-max","messages":[{"role":"user","content":"hello"}],"max_tokens":128})
        }
        ClientProtocol::Responses => {
            json!({"model":"public-max","input":"hello","max_output_tokens":128})
        }
        ClientProtocol::Anthropic => {
            json!({"model":"public-max","messages":[{"role":"user","content":"hello"}],"max_tokens":128})
        }
        _ => unreachable!(),
    };
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    Request::parse(
        client,
        &serde_json::to_vec(&value).unwrap(),
        &Limits::default(),
    )
    .unwrap()
}
fn route(protocol: UpstreamProtocol, deepseek: bool) -> Route {
    let route = Route::new(
        "deepseek-test".into(),
        protocol,
        "http://127.0.0.1:12345/v1/test",
        "mapped-model-max".into(),
        "test-private-key",
        1,
    )
    .unwrap();
    if deepseek {
        route.with_deepseek_parameters().unwrap()
    } else {
        route
    }
}
fn frame(value: Value) -> Frame {
    Frame {
        event: value["type"].as_str().map(str::to_owned),
        data: value.to_string(),
    }
}

#[test]
fn explicit_supplier_contract_maps_documented_efforts_without_model_suffix_guessing() {
    for (requested, mapped) in [
        ("none", "none"),
        ("minimal", "low"),
        ("low", "low"),
        ("medium", "high"),
        ("high", "high"),
        ("xhigh", "high"),
        ("max", "max"),
        ("ultra", "max"),
    ] {
        for (client, protocol) in [
            (ClientProtocol::Chat, UpstreamProtocol::Chat),
            (ClientProtocol::Responses, UpstreamProtocol::Responses),
        ] {
            let extra = if client == ClientProtocol::Chat {
                json!({"reasoning_effort":requested})
            } else {
                json!({"reasoning":{"effort":requested}})
            };
            let request = request(client, extra);
            let value = request.upstream(&route(protocol, true)).unwrap();
            assert_eq!(value["model"], "mapped-model-max");
            if client == ClientProtocol::Chat {
                assert_eq!(value["reasoning_effort"], mapped);
                assert_eq!(
                    value["thinking"]["type"],
                    if mapped == "none" {
                        "disabled"
                    } else {
                        "enabled"
                    }
                );
                assert_eq!(value["max_tokens"], 128);
                assert!(value.get("max_completion_tokens").is_none());
            } else {
                assert_eq!(value["reasoning"]["effort"], mapped);
                assert_eq!(value["max_output_tokens"], 128);
            }
            if !["low", "medium", "high"].contains(&requested) {
                assert!(request.upstream(&route(protocol, false)).is_err());
                assert!(request.upstream_to(protocol, "mapped").is_err());
            }
        }
    }
    let unchanged = request(ClientProtocol::Chat, json!({"reasoning_effort":"medium"}));
    assert_eq!(
        unchanged
            .upstream(&route(UpstreamProtocol::Chat, false))
            .unwrap()["reasoning_effort"],
        "medium"
    );
}

#[test]
fn thinking_conflicts_budgets_ignored_sampling_and_cross_protocols_fail_before_send() {
    for extra in [
        json!({"thinking":{"type":"enabled"},"reasoning_effort":"none"}),
        json!({"thinking":{"type":"disabled"},"reasoning_effort":"max"}),
        json!({"top_p":0.9}),
        json!({"temperature":0.5}),
        json!({"parallel_tool_calls":false}),
    ] {
        assert!(
            request(ClientProtocol::Chat, extra)
                .upstream(&route(UpstreamProtocol::Chat, true))
                .is_err()
        );
    }
    let disabled = request(
        ClientProtocol::Chat,
        json!({"thinking":{"type":"disabled"},"temperature":0.5}),
    );
    assert_eq!(
        disabled
            .upstream(&route(UpstreamProtocol::Chat, true))
            .unwrap()["temperature"],
        0.5
    );
    assert!(
        disabled
            .upstream(&route(UpstreamProtocol::Responses, true))
            .is_err()
    );
    assert!(
        disabled
            .upstream(&route(UpstreamProtocol::Chat, false))
            .is_err()
    );
    for control in [
        json!({"type":"enabled","budget_tokens":1024}),
        json!({"type":"adaptive"}),
        json!({"type":"disabled","ignored":true}),
    ] {
        let body =
            json!({"model":"m","messages":[{"role":"user","content":"hi"}],"thinking":control});
        assert!(
            Request::parse(
                ClientProtocol::Chat,
                &serde_json::to_vec(&body).unwrap(),
                &Limits::default()
            )
            .is_err()
        );
    }
    for body in [br#"{"model":"m","messages":[{"role":"user","content":"hi"}],"thinking":{"type":"enabled","type":"disabled"}}"#.as_slice(),br#"{"model":"m","input":"hi","reasoning":{"effort":"low","effort":"max"}}"#.as_slice()] {
        let client=if body.windows(8).any(|text|text==b"messages") {ClientProtocol::Chat} else {ClientProtocol::Responses};
        assert!(Request::parse(client,body,&Limits::default()).is_err());
    }
}

#[test]
fn native_anthropic_deepseek_uses_output_effort_and_keeps_unsigned_thinking_history() {
    let request = request(
        ClientProtocol::Anthropic,
        json!({"thinking":{"type":"enabled"},"output_config":{"effort":"ultra"},"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"prior"},{"type":"text","text":"done"}]},{"role":"user","content":"continue"}]}),
    );
    assert!(
        request
            .upstream(&route(UpstreamProtocol::Anthropic, false))
            .is_err()
    );
    let value = request
        .upstream(&route(UpstreamProtocol::Anthropic, true))
        .unwrap();
    assert_eq!(value["output_config"], json!({"effort":"max"}));
    assert_eq!(value["thinking"], json!({"type":"enabled"}));
    assert_eq!(
        value["messages"][0]["content"][0],
        json!({"type":"thinking","thinking":"prior"})
    );
    let mut decoder = Decoder::new(UpstreamProtocol::Anthropic, &Limits::default())
        .with_deepseek_parameters(true);
    let events=decoder.json(&json!({"id":"d1","type":"message","content":[{"type":"thinking","thinking":"new"},{"type":"text","text":"answer"}],"stop_reason":"end_turn","usage":{"input_tokens":2,"output_tokens":3,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}})).unwrap();
    let mut encoder = Encoder::new(&request, "unsigned", &Limits::default());
    encoder.set_deepseek_parameters(true).unwrap();
    for event in events {
        encoder.consume(event).unwrap();
    }
    let value: Value = serde_json::from_slice(&encoder.terminal().unwrap().concat()).unwrap();
    assert_eq!(
        value["content"][0],
        json!({"type":"thinking","thinking":"new"})
    );
}

fn response() -> Value {
    json!({"id":"deepseek_response","status":"completed","output":[{"type":"reasoning","id":"rs_native","status":"completed","summary":[],"content":[{"type":"reasoning_text","text":"think carefully"}]},{"type":"message","id":"msg_native","role":"assistant","status":"completed","content":[{"type":"output_text","text":"answer","annotations":[]}]}],"usage":{"input_tokens":8,"input_tokens_details":{"cached_tokens":3},"output_tokens":5,"output_tokens_details":{"reasoning_tokens":2},"total_tokens":13}})
}

#[test]
fn deepseek_responses_plaintext_json_roundtrips_as_content_and_native_history() {
    let request = request(
        ClientProtocol::Responses,
        json!({"reasoning":{"effort":"max"}}),
    );
    assert!(
        Decoder::new(UpstreamProtocol::Responses, &Limits::default())
            .json(&response())
            .is_err()
    );
    let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default())
        .with_deepseek_parameters(true);
    let mut encoder = Encoder::new(&request, "plain", &Limits::default());
    encoder.set_deepseek_parameters(true).unwrap();
    for event in decoder.json(&response()).unwrap() {
        encoder.consume(event).unwrap();
    }
    let value: Value = serde_json::from_slice(&encoder.terminal().unwrap().concat()).unwrap();
    assert_eq!(
        value["output"][0]["content"],
        json!([{"type":"reasoning_text","text":"think carefully"}])
    );
    assert_eq!(value["output"][0]["summary"], json!([]));
    assert_eq!(value["usage"]["input_tokens_details"]["cached_tokens"], 3);
    let replay = self::request(
        ClientProtocol::Responses,
        json!({"input":[value["output"][0].clone(),{"type":"message","role":"user","content":"continue"}]}),
    );
    assert!(
        replay
            .upstream(&route(UpstreamProtocol::Responses, false))
            .is_err()
    );
    assert_eq!(
        replay
            .upstream(&route(UpstreamProtocol::Responses, true))
            .unwrap()["input"][0],
        value["output"][0]
    );
}

#[test]
fn deepseek_responses_sse_keeps_plaintext_lifecycle_and_terminal_usage() {
    let request = request(
        ClientProtocol::Responses,
        json!({"stream":true,"reasoning":{"effort":"max"}}),
    );
    let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default())
        .with_deepseek_parameters(true);
    let mut encoder = Encoder::new(&request, "plain_sse", &Limits::default());
    encoder.set_deepseek_parameters(true).unwrap();
    let mut wire = Vec::new();
    let events = [
        json!({"type":"response.created","response":{"id":"deepseek_response","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_native","content":[],"summary":[]}}),
        json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"reasoning_text","text":""}}),
        json!({"type":"response.reasoning_text.delta","output_index":0,"content_index":0,"delta":"think carefully"}),
        json!({"type":"response.reasoning_text.done","output_index":0,"content_index":0,"text":"think carefully"}),
        json!({"type":"response.content_part.done","output_index":0,"content_index":0,"part":{"type":"reasoning_text","text":"think carefully"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":response()["output"][0]}),
        json!({"type":"response.content_part.added","output_index":1,"content_index":0,"part":{"type":"output_text","text":"answer","annotations":[]}}),
        json!({"type":"response.content_part.done","output_index":1,"content_index":0,"part":{"type":"output_text","text":"answer","annotations":[]}}),
        json!({"type":"response.completed","response":response()}),
    ];
    for value in events {
        for event in decoder.frame(frame(value)).unwrap() {
            wire.extend(encoder.consume(event).unwrap().concat());
        }
    }
    assert!(decoder.eof().is_ok());
    let before = String::from_utf8(wire.clone()).unwrap();
    assert!(before.contains("response.reasoning_text.delta"));
    assert!(!before.contains("response.completed"));
    wire.extend(encoder.terminal().unwrap().concat());
    let wire = String::from_utf8(wire).unwrap();
    assert!(wire.contains("response.reasoning_text.done"));
    assert!(wire.contains("response.completed"));
    assert!(!wire.contains("reasoning_summary_"));
    assert_eq!(encoder.usage().output_tokens, Some(5));
    assert_eq!(encoder.usage().reasoning_output_tokens, Some(2));
}

#[test]
fn deepseek_chat_cache_aliases_are_receipts_not_defaults() {
    let native = |usage: Value| json!({"id":"cache","choices":[{"index":0,"message":{"content":"ok"},"finish_reason":"stop"}],"usage":usage});
    let observe = |usage: Value| {
        Decoder::new(UpstreamProtocol::Chat, &Limits::default())
            .with_deepseek_parameters(true)
            .json(&native(usage))
    };
    let events=observe(json!({"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":6})).unwrap();
    let observed = events
        .into_iter()
        .find_map(|event| match event {
            Event::Usage(usage) => Some(usage),
            _ => None,
        })
        .unwrap();
    assert_eq!(observed.cached_input_tokens, Some(4));
    for bad in [
        json!({"prompt_tokens":10,"completion_tokens":2}),
        json!({"prompt_tokens":10,"prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":7}),
        json!({"prompt_tokens":10,"prompt_cache_hit_tokens":4,"prompt_tokens_details":{"cached_tokens":3}}),
        json!({"prompt_tokens":10,"prompt_cache_hit_tokens":0.5}),
    ] {
        assert!(observe(bad).is_err());
    }
    assert!(observe(json!({"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":10})).is_ok());
}
