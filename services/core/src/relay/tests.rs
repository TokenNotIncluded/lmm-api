use super::{
    ClientProtocol, Event, Limits, RelayError, Request, Route, UpstreamProtocol, Usage,
    decode::Decoder, encode::Encoder, sse::Frame,
};
use serde_json::{Value, json};

fn input(client: ClientProtocol, stream: bool) -> Request {
    let body = match client {
        ClientProtocol::Chat => {
            json!({"model":"alias","messages":[{"role":"user","content":"hello"}],"stream":stream})
        }
        ClientProtocol::Responses => json!({"model":"alias","input":"hello","stream":stream}),

        ClientProtocol::Anthropic | ClientProtocol::Gemini => {
            unreachable!("native clients require native fixtures")
        }
    };
    Request::parse(
        client,
        &serde_json::to_vec(&body).unwrap(),
        &Limits::default(),
    )
    .unwrap()
}
fn frame(decoder: &mut Decoder, v: Value) -> Result<Vec<Event>, RelayError> {
    decoder.frame(Frame {
        event: v["type"].as_str().map(str::to_owned),
        data: v.to_string(),
    })
}
fn chat_request(body: Value) -> Request {
    Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&body).unwrap(),
        &Limits::default(),
    )
    .unwrap()
}

#[test]
fn chat_gemini_image_config_preserves_bounded_top_level_value() {
    let mut body = json!({
        "model":"gemini-3.1-flash-image-preview",
        "messages":[{"role":"user","content":"draw"}],
        "imageConfig":{"imageSize":"4K","aspectRatio":"16:9"}
    });
    let upstream = chat_request(body)
        .upstream_to(UpstreamProtocol::Gemini, "image-model")
        .unwrap();
    assert_eq!(
        upstream["generationConfig"]["imageConfig"],
        json!({"imageSize":"4K","aspectRatio":"16:9"})
    );
    assert!(upstream.get("imageConfig").is_none());
}

#[test]
fn chat_image_config_rejects_unknown_fields_and_non_gemini_routes() {
    let mut invalid = json!({"model":"image-model","messages":[{"role":"user","content":"draw"}]});
    invalid["imageConfig"] = json!({"unknown":"value"});
    assert!(matches!(
        Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&invalid).unwrap(),
            &Limits::default(),
        ),
        Err(RelayError::Unsupported(_))
    ));

    let request = chat_request(json!({
        "model":"gemini-3.1-flash-image-preview",
        "messages":[{"role":"user","content":"draw"}],
        "imageConfig":{"imageSize":"4K"}
    }));
    assert!(matches!(
        request.upstream_to(UpstreamProtocol::Chat, "chat-model"),
        Err(RelayError::Unsupported(_))
    ));
}

#[test]
fn responses_cache_lifetime_is_bounded_and_not_silently_lost_in_conversion() {
    for ttl in ["30m", "12h"] {
        let options = json!({"mode":"implicit", "ttl":ttl});
        let request = Request::parse(
            ClientProtocol::Responses,
            &serde_json::to_vec(&json!({
                "model":"alias", "input":"hello", "prompt_cache_options": options
            }))
            .unwrap(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            request
                .upstream_to(UpstreamProtocol::Responses, "real")
                .unwrap()["prompt_cache_options"],
            options
        );
        for protocol in [
            UpstreamProtocol::Chat,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ] {
            assert!(matches!(
                request.upstream_to(protocol, "real"),
                Err(RelayError::Unsupported(_))
            ));
        }
    }
    for options in [
        json!({}),
        json!([]),
        json!({"ttl":"1h"}),
        json!({"ttl":12}),
        json!({"ttl":"12h","mode":"explicit"}),
        json!({"ttl":"12h","price":0}),
        json!({"ttl":"12h","prewarm":true}),
    ] {
        assert!(
            Request::parse(
                ClientProtocol::Responses,
                &serde_json::to_vec(&json!({
                    "model":"alias", "input":"hello", "prompt_cache_options":options
                }))
                .unwrap(),
                &Limits::default()
            )
            .is_err()
        );
    }
    assert!(
        Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&json!({
                "model":"alias", "messages":[{"role":"user","content":"hello"}],
                "prompt_cache_options":{"ttl":"12h"}
            }))
            .unwrap(),
            &Limits::default()
        )
        .is_err()
    );
}

#[test]
fn chat_reasoning_summary_survives_responses_json_and_stream_lifecycle() {
    let summary = json!([{"type":"summary_text","text":"think carefully"}]);
    for finish in ["stop", "length"] {
        for stream in [false, true] {
            let limits = Limits::default();
            let mut decoder = Decoder::new(UpstreamProtocol::Chat, &limits);
            let events = if stream {
                let mut events = Vec::new();
                for delta in [
                    json!({"reasoning_content":"think "}),
                    json!({"reasoning_content":"carefully"}),
                    json!({"content":"answer"}),
                ] {
                    events.extend(
                        frame(
                            &mut decoder,
                            json!({"id":"chat_1","choices":[{"index":0,"delta":delta}]}),
                        )
                        .unwrap(),
                    );
                }
                events.extend(frame(&mut decoder, json!({"id":"chat_1","choices":[{"index":0,"delta":{},"finish_reason":finish}]})).unwrap());
                events.extend(
                    decoder
                        .frame(Frame {
                            event: None,
                            data: "[DONE]".into(),
                        })
                        .unwrap(),
                );
                events
            } else {
                decoder.json(&json!({"id":"chat_1","choices":[{"index":0,"message":{"role":"assistant","reasoning_content":"think carefully","content":"answer"},"finish_reason":finish}]})).unwrap()
            };
            let mut encoder = Encoder::new(
                &input(ClientProtocol::Responses, stream),
                "reasoning_summary",
                &limits,
            );
            let mut bytes = Vec::new();
            for event in events {
                for packet in encoder.consume(event).unwrap() {
                    bytes.extend(packet);
                }
            }
            for packet in encoder.terminal().unwrap() {
                bytes.extend(packet);
            }
            let body = String::from_utf8(bytes).unwrap();
            let packets: Vec<Value> = if stream {
                body.lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            } else {
                vec![serde_json::from_str(&body).unwrap()]
            };
            let response = if stream {
                &packets.last().unwrap()["response"]
            } else {
                &packets[0]
            };
            let item = &response["output"][0];
            assert_eq!(item["type"], "reasoning");
            assert_eq!(item["summary"], summary);
            assert!(item.get("content").is_none());
            assert_eq!(response["output"][1]["content"][0]["text"], "answer");
            assert_eq!(
                response["status"],
                if finish == "length" {
                    "incomplete"
                } else {
                    "completed"
                }
            );
            if stream {
                let lifecycle: Vec<_> = packets
                    .iter()
                    .filter(|p| {
                        p["item"]["type"] == "reasoning"
                            || p["type"]
                                .as_str()
                                .is_some_and(|kind| kind.starts_with("response.reasoning_summary_"))
                    })
                    .collect();
                assert_eq!(
                    lifecycle
                        .iter()
                        .map(|p| p["type"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    [
                        "response.output_item.added",
                        "response.reasoning_summary_part.added",
                        "response.reasoning_summary_text.delta",
                        "response.reasoning_summary_text.delta",
                        "response.reasoning_summary_text.done",
                        "response.reasoning_summary_part.done",
                        "response.output_item.done",
                    ]
                );
                assert_eq!(lifecycle[0]["item"]["summary"], json!([]));
                assert_eq!(lifecycle[0]["item"]["id"], item["id"]);
                assert!(lifecycle[0]["item"].get("content").is_none());
                for event in &lifecycle {
                    assert_eq!(event["output_index"], 0);
                    if event.get("item_id").is_some() {
                        assert_eq!(event["item_id"], item["id"]);
                        assert_eq!(event["summary_index"], 0);
                    }
                }
                assert_eq!(
                    lifecycle[1]["part"],
                    json!({"type":"summary_text","text":""})
                );
                assert_eq!(lifecycle[2]["delta"], "think ");
                assert_eq!(lifecycle[3]["delta"], "carefully");
                assert_eq!(lifecycle[4]["text"], "think carefully");
                assert_eq!(lifecycle[5]["part"], summary[0]);
                assert_eq!(&lifecycle[6]["item"], item);
            }
        }
    }
}

#[test]
fn extra_tool_result_fields_are_not_silently_discarded() {
    for field in ["reasoning_content", "refusal", "provider_metadata"] {
        let mut body = json!({"model":"alias","messages":[
            {"role":"assistant","tool_calls":[{"type":"function","id":"old_call","function":{"name":"weather","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"old_call","content":"result"}]});
        body["messages"][1][field] = json!("must not disappear");
        assert!(matches!(
            Request::parse(
                ClientProtocol::Chat,
                &serde_json::to_vec(&body).unwrap(),
                &Limits::default()
            ),
            Err(RelayError::Unsupported(_))
        ));
    }
}
#[test]
fn a_tool_call_id_is_only_valid_on_a_tool_message() {
    let body = json!({"model":"alias","messages":[{"role":"user","content":"hello","tool_call_id":"unexpected"}]});
    assert!(matches!(
        Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default()
        ),
        Err(RelayError::Invalid(_))
    ));
}
#[test]
fn responses_input_type_must_be_a_string_when_present() {
    for kind in [Value::Null, json!(17), json!(true)] {
        let body = json!({"model":"alias","input":[{"type":kind,"role":"user","content":"hello"}]});
        assert!(matches!(
            Request::parse(
                ClientProtocol::Responses,
                &serde_json::to_vec(&body).unwrap(),
                &Limits::default()
            ),
            Err(RelayError::Invalid(_))
        ));
    }
}
#[test]
fn signatures_cannot_be_dropped_from_anthropic_tool_history_or_cross_providers() {
    let body = json!({"model":"alias","messages":[{"role":"assistant","tool_calls":[
        {"type":"function","id":"old_call","function":{"name":"weather","arguments":"{}"},"provider_metadata":{"protocol":"anthropic","signature":"opaque"}}]}]});
    let request = Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&body).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    for provider in [UpstreamProtocol::Anthropic, UpstreamProtocol::Gemini] {
        let route = Route::new(
            "r".into(),
            provider,
            "https://example.test/operation",
            "real".into(),
            "test-only",
            1,
        )
        .unwrap();
        assert!(matches!(
            request.upstream(&route),
            Err(RelayError::Unsupported(_))
        ));
    }
}
#[test]
fn gemini_plain_text_signatures_survive_both_client_formats_and_output_modes() {
    for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
        for stream in [false, true] {
            let source = json!({"responseId":"g_1","candidates":[{"index":0,"content":{"parts":[{"text":"answer","thoughtSignature":"opaque_signature"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":3}});
            let mut decoder = Decoder::new(UpstreamProtocol::Gemini, &Limits::default());
            let events = if stream {
                let mut events = frame(&mut decoder, source).unwrap();
                events.extend(decoder.eof().unwrap());
                events
            } else {
                decoder.json(&source).unwrap()
            };
            let mut encoder =
                Encoder::new(&input(client, stream), "signed_text", &Limits::default());
            let mut bytes = Vec::new();
            for event in events {
                for packet in encoder.consume(event).unwrap() {
                    bytes.extend(packet);
                }
            }
            for packet in encoder.terminal().unwrap() {
                bytes.extend(packet);
            }
            let body = String::from_utf8(bytes).unwrap();
            assert!(body.contains("opaque_signature"));
            assert!(!body.contains("reasoning_content"));
            let packets: Vec<Value> = if stream {
                body.lines()
                    .filter_map(|l| l.strip_prefix("data: "))
                    .filter(|s| *s != "[DONE]")
                    .map(|s| serde_json::from_str(s).unwrap())
                    .collect()
            } else {
                vec![serde_json::from_str(&body).unwrap()]
            };
            match client {
                ClientProtocol::Chat if stream => assert!(packets.iter().any(|p| p["choices"][0]
                    ["delta"]["content_details"][0]["signature"]
                    == "opaque_signature")),
                ClientProtocol::Chat => assert_eq!(
                    packets[0]["choices"][0]["message"]["content_details"][0]["signature"],
                    "opaque_signature"
                ),
                ClientProtocol::Responses => {
                    let response = if stream {
                        &packets.last().unwrap()["response"]
                    } else {
                        &packets[0]
                    };
                    assert_eq!(
                        response["output"][0]["provider_metadata"]["signature"],
                        "opaque_signature"
                    );
                    assert_eq!(
                        response["output"][0]["provider_metadata"]["protocol"],
                        "gemini"
                    );
                }

                ClientProtocol::Anthropic | ClientProtocol::Gemini => {
                    unreachable!("native clients require native fixtures")
                }
            }
        }
    }
}
#[test]
fn annotated_responses_fail_instead_of_losing_citations() {
    let part = json!({"type":"output_text","text":"answer","annotations":[{"type":"url_citation","url":"https://example.test/"}]});
    let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
    assert!(matches!(decoder.json(&json!({"id":"r_1","status":"completed","output":[{"id":"m_1","type":"message","content":[part.clone()]}]})),Err(RelayError::Protocol(_))));
    let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
    frame(
        &mut decoder,
        json!({"type":"response.created","response":{"id":"r_1"}}),
    )
    .unwrap();
    assert!(matches!(
        frame(
            &mut decoder,
            json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":part})
        ),
        Err(RelayError::Protocol(_))
    ));
}
#[test]
fn encrypted_reasoning_is_rejected_at_added_done_and_terminal_events() {
    let item = json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"});
    for kind in [
        "response.output_item.added",
        "response.output_item.done",
        "response.completed",
    ] {
        let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
        frame(
            &mut decoder,
            json!({"type":"response.created","response":{"id":"r_1"}}),
        )
        .unwrap();
        let event = if kind == "response.completed" {
            json!({"type":kind,"response":{"id":"r_1","status":"completed","output":[item.clone()]}})
        } else {
            json!({"type":kind,"output_index":0,"item":item.clone()})
        };
        assert!(matches!(
            frame(&mut decoder, event),
            Err(RelayError::Protocol(_))
        ));
    }
}
#[test]
fn huge_sparse_tool_indexes_cannot_allocate_unbounded_output_blocks() {
    let limits = Limits {
        max_blocks: 1,
        ..Limits::default()
    };
    let mut decoder = Decoder::new(UpstreamProtocol::Chat, &limits);
    let event = json!({"id":"r_1","choices":[{"index":0,"delta":{"tool_calls":[
        {"index":0,"id":"call_1","type":"function","function":{"name":"weather","arguments":"{}"}},
        {"index":u64::MAX,"id":"call_2","type":"function","function":{"name":"weather","arguments":"{}"}}]}}]});
    assert!(matches!(
        frame(&mut decoder, event),
        Err(RelayError::Limit(_))
    ));
}
#[test]
fn cumulative_usage_cannot_decrease_or_overflow() {
    let mut usage = Usage {
        input_tokens: Some(7),
        output_tokens: Some(3),
        ..Usage::default()
    };
    assert!(
        usage
            .merge(Usage {
                input_tokens: Some(6),
                ..Usage::default()
            })
            .is_err()
    );
    let mut usage = Usage {
        input_tokens: Some(u64::MAX),
        ..Usage::default()
    };
    assert!(
        usage
            .merge(Usage {
                output_tokens: Some(1),
                ..Usage::default()
            })
            .is_err()
    );
}

#[test]
fn invalid_usage_does_not_partially_replace_the_last_valid_snapshot() {
    let before = Usage {
        input_tokens: Some(7),
        output_tokens: Some(3),
        ..Usage::default()
    };
    let mut current = before;
    assert!(
        current
            .merge(Usage {
                input_tokens: Some(8),
                output_tokens: Some(2),
                ..Usage::default()
            })
            .is_err()
    );
    assert_eq!(current, before);
    let before = Usage {
        input_tokens: Some(u64::MAX),
        ..Usage::default()
    };
    let mut current = before;
    assert!(
        current
            .merge(Usage {
                output_tokens: Some(1),
                ..Usage::default()
            })
            .is_err()
    );
    assert_eq!(current, before);
}
