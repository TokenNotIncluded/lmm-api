//! Search receipt and native wire regressions. Fixtures describe supplier
//! evidence; they do not give callers permission to select prices or limits.
use super::{
    ClientProtocol, Event, Limits, RelayError, Request, Route, UpstreamProtocol, Usage,
    decode::Decoder,
    encode::Encoder,
    search::{SearchBinding, SearchMeter, unknown_usage},
    sse::Frame,
};
use crate::routing::{CacheCreationMode, SearchLimits, SearchProvider};
use serde_json::{Value, json};

fn bounds(provider: SearchProvider) -> SearchLimits {
    match provider {
        SearchProvider::GeminiGroundingV1 => SearchLimits {
            queries: Some(8),
            grounded_requests: Some(1),
            ..SearchLimits::default()
        },
        SearchProvider::ZhipuChatV1 => SearchLimits {
            grounded_requests: Some(1),
            ..SearchLimits::default()
        },
        SearchProvider::XaiResponsesV1 => SearchLimits {
            calls: Some(4),
            x_posts_fetched: Some(100),
            x_users_fetched: Some(20),
            ..SearchLimits::default()
        },
        _ => SearchLimits {
            calls: Some(4),
            ..SearchLimits::default()
        },
    }
}
fn decoder(provider: SearchProvider) -> Decoder {
    let limits = Limits::default();
    Decoder::new(super::search::provider_protocol(provider), &limits).with_search_contract(
        Some(SearchBinding {
            provider,
            limits: bounds(provider),
        }),
        &limits,
    )
}
fn meter(provider: SearchProvider) -> SearchMeter {
    SearchMeter::new(
        SearchBinding {
            provider,
            limits: bounds(provider),
        },
        &Limits::default(),
    )
}
fn parse(client: ClientProtocol, value: Value) -> Result<Request, RelayError> {
    Request::parse(
        client,
        &serde_json::to_vec(&value).unwrap(),
        &Limits::default(),
    )
}
fn request(provider: SearchProvider, stream: bool) -> Request {
    match super::search::provider_protocol(provider) {
        UpstreamProtocol::Responses => parse(ClientProtocol::Responses, json!({
            "model":"alias", "input":"find the source", "stream":stream,
            "tools":[{"type":"web_search"}]
        })).unwrap(),
        UpstreamProtocol::Anthropic => parse(ClientProtocol::Anthropic, json!({
            "model":"alias", "max_tokens":32, "messages":[{"role":"user","content":"find the source"}],
            "stream":stream, "tools":[{"type":"web_search_20250305","name":"web_search"}]
        })).unwrap(),
        UpstreamProtocol::Gemini => super::native_gemini::parse("alias", stream,
            &serde_json::to_vec(&json!({"contents":[{"parts":[{"text":"find the source"}]}],"tools":[{"googleSearch":{}}]})).unwrap(),
            &Limits::default()).unwrap(),
        UpstreamProtocol::Chat => parse(ClientProtocol::Chat, json!({
            "model":"alias", "messages":[{"role":"user","content":"find the source"}], "stream":stream,
            "tools":[{"type":"web_search","web_search":{"enable":true,"search_engine":"search_std","search_result":true}}]
        })).unwrap(),
    }
}
fn route(provider: SearchProvider) -> Route {
    Route::new(
        "source".into(),
        super::search::provider_protocol(provider),
        "https://supplier.example.test/v1",
        "actual-model".into(),
        "test-key",
        1,
    )
    .unwrap()
    .with_search_contract(provider, bounds(provider))
    .unwrap()
}
fn frame(decoder: &mut Decoder, value: Value) -> Result<Vec<Event>, RelayError> {
    decoder.frame(Frame {
        event: value["type"].as_str().map(str::to_owned),
        data: value.to_string(),
    })
}
fn search_item(id: &str, action: &str) -> Value {
    json!({"id":id,"type":"web_search_call","status":"completed","action":{"type":action}})
}
fn annotation() -> Value {
    json!({"type":"url_citation","start_index":0,"end_index":6,"title":"Source","url":"https://example.test/source"})
}
fn response(output: Vec<Value>) -> Value {
    json!({"id":"resp_up","status":"completed","output":output,"usage":{
        "input_tokens":10,"output_tokens":3,"input_tokens_details":{"cached_tokens":0},
        "cache_creation_input_tokens":0,"output_tokens_details":{"reasoning_tokens":0}
    }})
}
fn render(request: &Request, events: Vec<Event>) -> (Usage, Vec<Value>) {
    let mut encoder = Encoder::new(request, "search-test", &Limits::default());
    let mut wire = Vec::new();
    for event in events {
        for packet in encoder.consume(event).unwrap() {
            wire.extend(packet);
        }
    }
    for packet in encoder.terminal().unwrap() {
        wire.extend(packet);
    }
    let packets = if request.stream {
        std::str::from_utf8(&wire)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|line| *line != "[DONE]")
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    } else {
        vec![serde_json::from_slice(&wire).unwrap()]
    };
    (encoder.usage(), packets)
}

#[test]
fn hosted_tools_require_native_frozen_contracts_before_dispatch() {
    for provider in [
        SearchProvider::OpenAiResponsesV1,
        SearchProvider::AzureResponsesV1,
        SearchProvider::XaiResponsesV1,
        SearchProvider::DashscopeResponsesV1,
        SearchProvider::AnthropicMessagesV1,
        SearchProvider::DashscopeAnthropicV1,
        SearchProvider::GeminiGroundingV1,
        SearchProvider::ZhipuChatV1,
    ] {
        let request = request(provider, false);
        assert!(request.has_hosted_search());
        let route = route(provider);
        let wire = request.upstream(&route).unwrap();
        assert!(
            wire["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty())
        );
        let bare = Route::new(
            "source".into(),
            route.protocol,
            "https://supplier.example.test/v1",
            "actual-model".into(),
            "test-key",
            1,
        )
        .unwrap();
        assert!(request.upstream(&bare).is_err());
        for protocol in [
            UpstreamProtocol::Chat,
            UpstreamProtocol::Responses,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ] {
            if protocol != route.protocol {
                assert!(request.upstream_to(protocol, "other").is_err());
            }
        }
        match provider {
            SearchProvider::OpenAiResponsesV1 => assert_eq!(wire["max_tool_calls"], 4),
            SearchProvider::AnthropicMessagesV1 | SearchProvider::DashscopeAnthropicV1 => {
                assert_eq!(wire["tools"][0]["max_uses"], 4);
                if provider == SearchProvider::DashscopeAnthropicV1 {
                    assert!(
                        wire["system"][0]["text"]
                            .as_str()
                            .unwrap()
                            .contains("cc_entrypoint=cli")
                    );
                }
            }
            _ => assert!(wire.get("max_tool_calls").is_none()),
        }
    }
    let function = parse(
        ClientProtocol::Responses,
        json!({"model":"m","input":"q",
        "tools":[{"type":"function","name":"web_search","parameters":{"type":"object"}}]}),
    )
    .unwrap();
    assert!(!function.has_hosted_search());
    assert!(!function.has_x_search());
    let x = parse(
        ClientProtocol::Responses,
        json!({"model":"m","input":"q","tools":[{"type":"x_search"}]}),
    )
    .unwrap();
    assert!(x.has_x_search());
    assert!(x.upstream(&route(SearchProvider::XaiResponsesV1)).is_ok());
    assert!(
        x.upstream(&route(SearchProvider::OpenAiResponsesV1))
            .is_err()
    );
    let stream = request(SearchProvider::ZhipuChatV1, true);
    assert!(
        stream
            .upstream(&route(SearchProvider::ZhipuChatV1))
            .is_err()
    );
}

#[test]
fn invalid_or_unpreservable_search_controls_fail_preflight() {
    for change in [
        json!({"max_tool_calls":0}),
        json!({"max_tool_calls":1.5}),
        json!({"max_tool_calls":-1}),
        json!({"max_tool_calls":10001}),
        json!({"tools":[{"type":"web_search"},{"type":"web_search"}]}),
        json!({"tools":[{"type":"web_search","unpriced_option":true}]}),
        json!({"tool_choice":{"type":"x_search"}}),
    ] {
        let mut value = json!({"model":"m","input":"q","tools":[{"type":"web_search"}]});
        value
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(parse(ClientProtocol::Responses, value).is_err());
    }
    let capped = parse(
        ClientProtocol::Responses,
        json!({"model":"m","input":"q",
        "tools":[{"type":"web_search"}],"max_tool_calls":5}),
    )
    .unwrap();
    assert!(
        capped
            .upstream(&route(SearchProvider::OpenAiResponsesV1))
            .is_err()
    );
    let capped = parse(
        ClientProtocol::Responses,
        json!({"model":"m","input":"q",
        "tools":[{"type":"web_search"}],"max_tool_calls":2}),
    )
    .unwrap();
    assert_eq!(
        capped
            .upstream(&route(SearchProvider::OpenAiResponsesV1))
            .unwrap()["max_tool_calls"],
        2
    );
    for provider in [
        SearchProvider::AzureResponsesV1,
        SearchProvider::XaiResponsesV1,
        SearchProvider::DashscopeResponsesV1,
    ] {
        assert!(capped.upstream(&route(provider)).is_err());
    }
}

#[test]
fn openai_json_counts_only_search_actions_and_keeps_native_order_and_citations() {
    let output = vec![
        search_item("search-1", "search"),
        search_item("open-1", "open_page"),
        search_item("find-1", "find_in_page"),
        json!({"type":"message","role":"assistant","content":[{
            "type":"output_text","text":"answer","annotations":[annotation()]
        }]}),
    ];
    let mut d = decoder(SearchProvider::OpenAiResponsesV1);
    let events = d.json(&response(output.clone())).unwrap();
    let (usage, packets) = render(&request(SearchProvider::OpenAiResponsesV1, false), events);
    assert_eq!(usage.search.unwrap().calls, Some(1));
    assert_eq!(&packets[0]["output"].as_array().unwrap()[..3], &output[..3]);
    assert_eq!(
        packets[0]["output"][3]["content"][0]["annotations"],
        json!([annotation()])
    );
    assert!(packets[0]["usage"].get("search").is_none());
    assert!(
        Decoder::new(UpstreamProtocol::Responses, &Limits::default())
            .json(&response(output))
            .is_err()
    );
}

#[test]
fn openai_sse_search_ids_are_deduplicated_and_checkpointed_before_terminal() {
    let provider = SearchProvider::OpenAiResponsesV1;
    let mut d = decoder(provider);
    let mut events = frame(
        &mut d,
        json!({"type":"response.created","response":{"id":"resp_up"}}),
    )
    .unwrap();
    let started = json!({"id":"search-1","type":"web_search_call","status":"in_progress"});
    events.extend(
        frame(
            &mut d,
            json!({"type":"response.output_item.added","output_index":0,"item":started}),
        )
        .unwrap(),
    );
    for _ in 0..2 {
        let partial = frame(&mut d, json!({"type":"response.output_item.done","output_index":0,"item":search_item("search-1","search")})).unwrap();
        assert!(partial.iter().any(|event| matches!(event,Event::Usage(usage) if usage.search.is_some_and(|s| s.calls==Some(1)))));
        events.extend(partial);
    }
    let terminal = response(vec![
        search_item("search-1", "search"),
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer","annotations":[annotation()]}]}),
    ]);
    events.extend(
        frame(
            &mut d,
            json!({"type":"response.completed","response":terminal}),
        )
        .unwrap(),
    );
    let (usage, packets) = render(&request(provider, true), events);
    assert_eq!(usage.search.unwrap().calls, Some(1));
    assert_eq!(
        packets
            .iter()
            .filter(|v| v["type"] == "response.output_item.added"
                && v["item"]["type"] == "web_search_call")
            .count(),
        1
    );
    assert_eq!(
        packets
            .iter()
            .filter(|v| v["type"] == "response.output_item.done"
                && v["item"]["type"] == "web_search_call")
            .count(),
        1
    );
    let final_response = &packets.last().unwrap()["response"];
    assert_eq!(final_response["output"][0]["id"], "search-1");
    assert_eq!(
        final_response["output"][1]["content"][0]["annotations"],
        json!([annotation()])
    );
    assert!(
        frame(
            &mut d,
            json!({"type":"response.completed","response":terminal})
        )
        .is_err()
    );
}

#[test]
fn openai_missing_actions_changed_ids_or_lost_final_items_cannot_settle() {
    let provider = SearchProvider::OpenAiResponsesV1;
    let mut m = meter(provider);
    assert_eq!(m.observe(&json!({}), false).unwrap().calls, None);
    m.item(&search_item("s", "search")).unwrap();
    assert!(m.item(&search_item("s", "open_page")).is_err());
    assert!(m.observe(&response(vec![]), true).is_err());
    let mut d = decoder(provider);
    assert!(
        d.json(&response(vec![
            json!({"id":"s","type":"web_search_call","status":"completed"})
        ]))
        .is_err()
    );
    let mut d = decoder(provider);
    let events = d.json(&response(vec![])).unwrap();
    let (usage, _) = render(&request(provider, false), events);
    assert_eq!(usage.search.unwrap().calls, Some(0));
}

#[test]
fn suppliers_override_item_counts_without_fallback_or_double_counting() {
    for (provider, fields, expected) in [
        (
            SearchProvider::AzureResponsesV1,
            json!({"tool_usage":{"web_search":{"num_requests":0}}}),
            0,
        ),
        (
            SearchProvider::XaiResponsesV1,
            json!({"usage":{"server_side_tool_usage_details":{"web_search_calls":2,"x_posts_fetched":7,"x_users_fetched":3}}}),
            2,
        ),
        (
            SearchProvider::DashscopeResponsesV1,
            json!({"usage":{"x_tools":{"web_search":{"count":3}}}}),
            3,
        ),
    ] {
        let mut receipt = response(vec![search_item("s", "search")]);
        for (key, value) in fields.as_object().unwrap() {
            if key == "usage" {
                receipt[key]
                    .as_object_mut()
                    .unwrap()
                    .extend(value.as_object().unwrap().clone());
            } else {
                receipt[key] = value.clone();
            }
        }
        let events = decoder(provider).json(&receipt).unwrap();
        let (usage, packets) = render(&request(provider, false), events);
        let search = usage.search.unwrap();
        assert_eq!(search.calls, Some(expected));
        if provider == SearchProvider::XaiResponsesV1 {
            assert_eq!(search.x_posts_fetched, Some(7));
            assert_eq!(search.x_users_fetched, Some(3));
            assert_eq!(
                packets[0]["usage"]["server_side_tool_usage_details"]["x_posts_fetched"],
                7
            );
        }
        if provider == SearchProvider::AzureResponsesV1 {
            assert_eq!(packets[0]["tool_usage"]["web_search"]["num_requests"], 0);
        }
        if provider == SearchProvider::DashscopeResponsesV1 {
            assert_eq!(packets[0]["usage"]["x_tools"]["web_search"]["count"], 3);
        }
    }
}

#[test]
fn supplier_final_absence_is_unknown_and_invalid_counts_are_never_clamped() {
    let provider = SearchProvider::AzureResponsesV1;
    let mut m = meter(provider);
    assert_eq!(
        m.observe(
            &json!({"tool_usage":{"web_search":{"num_requests":0}}}),
            false
        )
        .unwrap()
        .calls,
        Some(0)
    );
    m.observe(&json!({}), true).unwrap();
    assert!(m.complete().is_err());
    for value in [
        json!(-1),
        json!(0.5),
        json!("1"),
        json!(true),
        json!(10001),
        json!(u64::MAX),
        json!(5),
    ] {
        assert!(
            meter(provider)
                .observe(
                    &json!({"tool_usage":{"web_search":{"num_requests":value}}}),
                    true
                )
                .is_err()
        );
    }
    for receipt in [
        json!({"tool_usage":[]}),
        json!({"tool_usage":{"web_search":false}}),
    ] {
        assert!(meter(provider).observe(&receipt, true).is_err());
    }
    let mut m = meter(provider);
    m.observe(
        &json!({"tool_usage":{"web_search":{"num_requests":2}}}),
        false,
    )
    .unwrap();
    assert!(
        m.observe(
            &json!({"tool_usage":{"web_search":{"num_requests":1}}}),
            true
        )
        .is_err()
    );
    assert!(
        super::search::reject_uncontracted_usage(
            &json!({"usage":{"server_tool_use":{"web_search_requests":1}}})
        )
        .is_err()
    );
}

#[test]
fn gemini_query_sets_accumulate_once_and_grounding_sources_keep_their_indices() {
    let provider = SearchProvider::GeminiGroundingV1;
    let mut d = decoder(provider);
    let mut events = Vec::new();
    for (text, queries, image_queries, uri) in [
        (
            "first ",
            json!([" alpha ", "alpha", ""]),
            json!([]),
            "https://example.test/a",
        ),
        (
            "answer",
            json!(["beta", "alpha"]),
            json!(["gamma", " beta "]),
            "https://example.test/b",
        ),
    ] {
        events.extend(frame(&mut d, json!({"responseId":"g-up","candidates":[{"index":0,"content":{"parts":[{"text":text}]},"groundingMetadata":{
            "webSearchQueries":queries,"imageSearchQueries":image_queries,
            "groundingChunks":[{"web":{"uri":uri,"title":"source"}}],
            "groundingSupports":[{"segment":{"text":text},"groundingChunkIndices":[0],"confidenceScores":[0.75]}]
        }}]})).unwrap());
    }
    events.extend(frame(&mut d, json!({"responseId":"g-up","candidates":[{"index":0,"finishReason":"STOP"}],"usageMetadata":{
        "promptTokenCount":10,"candidatesTokenCount":3,"thoughtsTokenCount":0,"cachedContentTokenCount":0
    }})).unwrap());
    events.extend(d.eof().unwrap());
    let (usage, packets) = render(&request(provider, true), events);
    let search = usage.search.unwrap();
    assert_eq!(search.queries, Some(3));
    assert_eq!(search.grounded_requests, Some(1));
    assert_eq!(search.calls, None);
    let metadata = &packets.last().unwrap()["candidates"][0]["groundingMetadata"];
    assert_eq!(metadata["groundingChunks"].as_array().unwrap().len(), 2);
    assert_eq!(
        metadata["groundingSupports"][0]["groundingChunkIndices"],
        json!([0])
    );
    assert_eq!(
        metadata["groundingSupports"][1]["groundingChunkIndices"],
        json!([1])
    );
    assert_eq!(
        metadata["groundingSupports"][0]["confidenceScores"],
        json!([0.75])
    );
    assert_eq!(
        metadata["groundingSupports"][1]["confidenceScores"],
        json!([0.75])
    );
    assert!(
        packets.last().unwrap()["usageMetadata"]
            .get("search")
            .is_none()
    );
}

#[test]
fn gemini_explicit_empty_queries_are_zero_but_absent_metadata_is_unknown() {
    let provider = SearchProvider::GeminiGroundingV1;
    let mut absent = meter(provider);
    assert_eq!(absent.observe(&json!({}), false).unwrap().queries, None);
    assert!(absent.complete().is_err());
    let mut empty = meter(provider);
    let usage = empty.observe(&json!({"candidates":[{"groundingMetadata":{"webSearchQueries":[],"imageSearchQueries":[]}}]}), false).unwrap();
    assert_eq!(usage.queries, Some(0));
    assert_eq!(usage.grounded_requests, Some(0));
    assert_eq!(empty.complete().unwrap(), usage);
    for metadata in [
        json!({"webSearchQueries":null}),
        json!({"webSearchQueries":[42]}),
        json!({"groundingChunks":false}),
    ] {
        assert!(
            meter(provider)
                .observe(
                    &json!({"candidates":[{"groundingMetadata":metadata}]}),
                    false
                )
                .is_err()
        );
    }
}

fn anthropic_usage(calls: u64) -> Value {
    json!({"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,
        "server_tool_use":{"web_search_requests":calls}})
}
fn anthropic_blocks() -> Vec<Value> {
    vec![
        json!({"type":"server_tool_use","id":"srv_1","name":"web_search","input":{"query":"source"}}),
        json!({"type":"web_search_tool_result","tool_use_id":"srv_1","content":[{"type":"web_search_result","url":"https://example.test/","title":"Source","encrypted_content":"opaque"}]}),
        json!({"type":"text","text":"answer","citations":[{"type":"web_search_result_location","url":"https://example.test/","title":"Source","encrypted_index":"opaque-index","cited_text":"answer"}]}),
    ]
}
#[test]
fn anthropic_native_search_results_citations_and_receipts_survive_json_and_sse() {
    for provider in [
        SearchProvider::AnthropicMessagesV1,
        SearchProvider::DashscopeAnthropicV1,
    ] {
        for stream in [false, true] {
            let blocks = anthropic_blocks();
            let mut d = decoder(provider);
            let events = if stream {
                let mut events = frame(&mut d, json!({"type":"message_start","message":{"id":"a-up","usage":anthropic_usage(0)}})).unwrap();
                for (index, block) in blocks.iter().enumerate() {
                    events.extend(frame(&mut d, json!({"type":"content_block_start","index":index,"content_block":block})).unwrap());
                    events.extend(
                        frame(&mut d, json!({"type":"content_block_stop","index":index})).unwrap(),
                    );
                }
                events.extend(frame(&mut d, json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":anthropic_usage(1)})).unwrap());
                events.extend(frame(&mut d, json!({"type":"message_stop"})).unwrap());
                events
            } else {
                d.json(&json!({"id":"a-up","content":blocks,"stop_reason":"end_turn","usage":anthropic_usage(1)})).unwrap()
            };
            let (usage, packets) = render(&request(provider, stream), events);
            assert_eq!(usage.search.unwrap().calls, Some(1));
            if stream {
                let starts: Vec<_> = packets
                    .iter()
                    .filter(|packet| packet["type"] == "content_block_start")
                    .collect();
                assert_eq!(starts[0]["content_block"], blocks[0]);
                assert_eq!(starts[1]["content_block"], blocks[1]);
                assert_eq!(starts[2]["index"], 2);
                assert!(
                    packets
                        .iter()
                        .any(|packet| packet["delta"]["type"] == "citations_delta")
                );
                let final_usage = &packets
                    .iter()
                    .find(|packet| packet["type"] == "message_delta")
                    .unwrap()["usage"];
                assert_eq!(final_usage["server_tool_use"]["web_search_requests"], 1);
            } else {
                assert_eq!(packets[0]["content"], json!(blocks));
            }
        }
    }
}

#[test]
fn zhipu_results_prove_one_grounded_request_not_result_count_or_forced_intent() {
    let provider = SearchProvider::ZhipuChatV1;
    let results = json!([{"title":"one","link":"https://example.test/1"},{"title":"two","link":"https://example.test/2"}]);
    let receipt = json!({"id":"z-up","choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":10,"completion_tokens":3},"web_search":results});
    let (usage, packets) = render(
        &request(provider, false),
        decoder(provider).json(&receipt).unwrap(),
    );
    assert_eq!(usage.search.unwrap().grounded_requests, Some(1));
    assert_eq!(usage.search.unwrap().calls, None);
    assert_eq!(packets[0]["web_search"], results);
    for receipt in [
        json!({}),
        json!({"web_search":[]}),
        json!({"search_intent":true}),
    ] {
        let mut m = meter(provider);
        assert_eq!(m.observe(&receipt, true).unwrap().grounded_requests, None);
        assert!(m.complete().is_err());
    }
}

#[test]
fn search_evidence_round_trips_and_merge_preserves_unknown_and_rejects_decreases_atomically() {
    let old = json!({"input_tokens":10,"output_tokens":3,"cached_input_tokens":null,"cache_creation_input_tokens":null,"reasoning_output_tokens":null});
    let old_usage: Usage = serde_json::from_value(old).unwrap();
    assert_eq!(old_usage.search, None);
    assert!(
        serde_json::to_value(old_usage)
            .unwrap()
            .get("search")
            .is_none()
    );
    for mode in [None, Some(CacheCreationMode::GenericRemainderV1)] {
        let mut evidence = unknown_usage(SearchProvider::XaiResponsesV1);
        evidence.calls = Some(1);
        evidence.x_posts_fetched = Some(0);
        let mut usage = Usage {
            cache_creation_mode: mode,
            search: Some(evidence),
            ..Usage::default()
        };
        let persisted = serde_json::to_value(usage).unwrap();
        assert_eq!(persisted["search"]["x_posts_fetched"], 0);
        assert!(persisted["search"].get("x_users_fetched").is_none());
        assert_eq!(
            serde_json::from_value::<Usage>(persisted.clone()).unwrap(),
            usage
        );
        usage
            .merge(Usage {
                cache_creation_mode: mode,
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(usage.search, Some(evidence));
        for field in ["calls", "x_posts_fetched"] {
            for invalid in [json!(-1), json!(0.5), json!("1")] {
                let mut bad = persisted.clone();
                bad["search"][field] = invalid;
                assert!(serde_json::from_value::<Usage>(bad).is_err());
            }
        }
        for next in [
            crate::routing::SearchUsage {
                calls: Some(0),
                ..evidence
            },
            crate::routing::SearchUsage {
                provider: SearchProvider::OpenAiResponsesV1,
                ..evidence
            },
        ] {
            let before = usage;
            assert!(
                usage
                    .merge(Usage {
                        cache_creation_mode: mode,
                        search: Some(next),
                        ..Usage::default()
                    })
                    .is_err()
            );
            assert_eq!(usage, before);
        }
    }
}

#[test]
fn duplicated_or_gapped_annotation_events_do_not_replace_existing_citations() {
    let provider = SearchProvider::OpenAiResponsesV1;
    let mut d = decoder(provider);
    let mut events = frame(
        &mut d,
        json!({"type":"response.created","response":{"id":"resp_up"}}),
    )
    .unwrap();
    events.extend(frame(&mut d, json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"answer","annotations":[]}})).unwrap());
    let event = json!({"type":"response.output_text.annotation.added","output_index":0,"content_index":0,"annotation_index":0,"annotation":annotation()});
    for _ in 0..2 {
        events.extend(frame(&mut d, event.clone()).unwrap());
    }
    let mut gap = event.clone();
    gap["annotation_index"] = json!(2);
    assert!(frame(&mut d, gap).is_err());
    let mut changed = event;
    changed["annotation"]["url"] = json!("https://example.test/replaced");
    assert!(frame(&mut d, changed).is_err());
    let terminal = response(vec![
        json!({"type":"message","content":[{"type":"output_text","text":"answer","annotations":[annotation()]}]}),
    ]);
    events.extend(
        frame(
            &mut d,
            json!({"type":"response.completed","response":terminal}),
        )
        .unwrap(),
    );
    let (_, packets) = render(&request(provider, true), events);
    assert_eq!(
        packets
            .iter()
            .filter(|v| v["type"] == "response.output_text.annotation.added")
            .count(),
        1
    );
}

#[test]
fn ambiguous_receipt_keys_and_changed_hosted_payloads_are_rejected() {
    let mut d = decoder(SearchProvider::AzureResponsesV1);
    assert!(d.frame(Frame { event:Some("response.created".into()), data:r#"{"type":"response.created","response":{"id":"resp_up","tool_usage":{"web_search":{"num_requests":2,"num_requests":0}}}}"#.into() }).is_err());
    assert!(
        super::native_anthropic::unambiguous_json(
            br#"{"usage":{"x_tools":{"web_search":{"count":2,"count":0}}}}"#
        )
        .is_err()
    );
    assert!(
        Request::parse(
            ClientProtocol::Responses,
            br#"{"model":"m","input":"q","tools":[{"type":"web_search","type":"web_search"}]}"#,
            &Limits::default()
        )
        .is_err()
    );
    let previous = json!({"id":"s","type":"web_search_call","status":"completed","action":{"type":"search","query":"first"}});
    let mut changed = previous.clone();
    changed["action"]["query"] = json!("different");
    assert!(super::search::merge_response_output(&previous, &changed).is_err());
    let enriched = super::search::merge_response_output(
        &previous,
        &json!({"id":"s","type":"web_search_call","status":"completed"}),
    )
    .unwrap();
    assert_eq!(enriched, previous);
}

#[test]
fn provider_sse_final_receipt_cannot_inherit_an_earlier_zero() {
    for (provider, reported) in [
        (
            SearchProvider::AzureResponsesV1,
            json!({"tool_usage":{"web_search":{"num_requests":0}}}),
        ),
        (
            SearchProvider::XaiResponsesV1,
            json!({"usage":{"server_side_tool_usage_details":{"web_search_calls":0,"x_posts_fetched":0,"x_users_fetched":0}}}),
        ),
        (
            SearchProvider::DashscopeResponsesV1,
            json!({"usage":{"x_tools":{"web_search":{"count":0}}}}),
        ),
    ] {
        let mut d = decoder(provider);
        let mut initial = reported;
        initial["id"] = json!("resp_up");
        frame(
            &mut d,
            json!({"type":"response.created","response":initial}),
        )
        .unwrap();
        assert!(
            frame(
                &mut d,
                json!({"type":"response.completed","response":response(vec![])})
            )
            .is_err()
        );
    }
}

#[test]
fn strict_receipt_reader_preserves_decimal_metadata_and_integer_boundaries() {
    let value = super::native_anthropic::unambiguous_json(br#"{"confidence":0.1234567890123456789,"large":18446744073709551616,"zero":0,"array":[1.5]}"#).unwrap();
    assert!(value["confidence"].is_number());
    assert_eq!(value["confidence"].to_string(), "0.1234567890123456789");
    assert_eq!(value["large"].to_string(), "18446744073709551616");
    assert_eq!(value["large"].as_u64(), None);
    assert_eq!(value["zero"].as_u64(), Some(0));
    assert_eq!(value["array"][0], json!(1.5));
    assert!(
        super::native_anthropic::unambiguous_json(
            br#"{"calls":{"$serde_json::private::Number":"0"}}"#
        )
        .is_err()
    );
    assert!(
        super::native_anthropic::unambiguous_json(br#"{"nested":[{"count":1,"count":0}]}"#)
            .is_err()
    );
}
