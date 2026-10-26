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
fn legacy_policy(flags: [bool; 5]) -> crate::routing::PayloadPolicy {
    crate::routing::PayloadPolicy::LegacyOpenai {
        allow_service_tier: flags[0],
        allow_inference_geo: flags[1],
        allow_speed: flags[2],
        allow_safety_identifier: flags[3],
        allow_include_obfuscation: flags[4],
    }
}

fn legacy_route(protocol: UpstreamProtocol, flags: [bool; 5]) -> Route {
    Route::new(
        "synthetic".into(),
        protocol,
        "https://provider.invalid/v1",
        "real".into(),
        "synthetic",
        1,
    )
    .unwrap()
    .with_payload_policy(legacy_policy(flags))
}

#[test]
fn legacy_openai_all_flags_preserve_or_remove_exact_payloads() {
    for (client, upstream) in [
        (ClientProtocol::Chat, UpstreamProtocol::Chat),
        (ClientProtocol::Responses, UpstreamProtocol::Responses),
    ] {
        for stream in [false, true] {
            let mut body = match client {
                ClientProtocol::Chat => {
                    json!({"model":"alias","messages":[{"role":"user","content":"hello"}],"stream":stream})
                }
                _ => json!({"model":"alias","input":"hello","stream":stream}),
            };
            let baseline = input(client, stream).upstream_to(upstream, "real").unwrap();
            // Each field is tested alone as well as together, so one flag cannot
            // accidentally authorize another field.
            for mask in 0..32_u8 {
                let flags = std::array::from_fn(|i| mask & (1 << i) != 0);
                for (name, value) in [
                    ("service_tier", json!("priority")),
                    ("inference_geo", json!("us")),
                    ("speed", json!("fast")),
                    ("safety_identifier", json!("synthetic-user")),
                ] {
                    body[name] = value;
                }
                body["stream_options"] = json!({"include_obfuscation":false});
                let bytes = serde_json::to_vec(&body).unwrap();
                assert!(Request::parse(client, &bytes, &Limits::default()).is_err());
                let request =
                    Request::parse_deferred_openai(client, &bytes, &Limits::default()).unwrap();
                assert!(request.upstream_to(upstream, "real").is_err());
                let ordinary = Route::new(
                    "strict".into(),
                    upstream,
                    "https://provider.invalid/v1",
                    "real".into(),
                    "synthetic",
                    1,
                )
                .unwrap();
                assert!(request.upstream(&ordinary).is_err());
                let encoded = request.upstream(&legacy_route(upstream, flags));
                if !stream && flags[4] {
                    assert!(encoded.is_err());
                    continue;
                }
                let mut expected = baseline.clone();
                for (i, name) in [
                    "service_tier",
                    "inference_geo",
                    "speed",
                    "safety_identifier",
                ]
                .into_iter()
                .enumerate()
                {
                    if flags[i] {
                        expected[name] = body[name].clone();
                    }
                }
                if flags[4] {
                    if expected.get("stream_options").is_none() {
                        expected["stream_options"] = json!({});
                    }
                    expected["stream_options"]["include_obfuscation"] = json!(false);
                }
                assert_eq!(
                    encoded.unwrap(),
                    expected,
                    "client={client:?}, stream={stream}, mask={mask}"
                );
            }
            // Absent request fields never get synthesized, even with all flags on.
            assert_eq!(
                input(client, stream)
                    .upstream(&legacy_route(upstream, [true; 5]))
                    .unwrap(),
                baseline
            );
        }
    }
}

#[test]
fn legacy_openai_disabled_values_are_removed_but_enabled_values_are_validated() {
    for (client, upstream, base) in [
        (
            ClientProtocol::Chat,
            UpstreamProtocol::Chat,
            json!({"model":"alias","messages":[{"role":"user","content":"hello"}],"stream":true}),
        ),
        (
            ClientProtocol::Responses,
            UpstreamProtocol::Responses,
            json!({"model":"alias","input":"hello","stream":true}),
        ),
    ] {
        for (index, name) in [
            "service_tier",
            "inference_geo",
            "speed",
            "safety_identifier",
            "include_obfuscation",
        ]
        .into_iter()
        .enumerate()
        {
            let invalid = if index == 4 {
                vec![json!(null), json!("true"), json!(0), json!({})]
            } else {
                vec![
                    json!(null),
                    json!(true),
                    json!(0),
                    json!({}),
                    json!(""),
                    json!("bad\nvalue"),
                    json!("x".repeat(if index == 3 { 129 } else { 257 })),
                ]
            };
            for value in invalid {
                if value.is_null() && matches!(name, "service_tier" | "safety_identifier") {
                    continue;
                }
                let mut body = base.clone();
                if index == 4 {
                    body["stream_options"] = json!({name:value});
                } else {
                    body[name] = value;
                }
                let request = Request::parse_deferred_openai(
                    client,
                    &serde_json::to_vec(&body).unwrap(),
                    &Limits::default(),
                )
                .unwrap();
                assert_eq!(
                    request
                        .upstream(&legacy_route(upstream, [false; 5]))
                        .unwrap(),
                    input(client, true).upstream_to(upstream, "real").unwrap()
                );
                let mut flags = [false; 5];
                flags[index] = true;
                assert!(
                    request.upstream(&legacy_route(upstream, flags)).is_err(),
                    "{name}"
                );
            }
        }
        let mut body = base.clone();
        body["service_tier"] = json!("unreviewed-tier");
        let request = Request::parse_deferred_openai(
            client,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        assert!(
            request
                .upstream(&legacy_route(upstream, [true; 5]))
                .is_err()
        );
        body.as_object_mut().unwrap().remove("service_tier");
        body["safety_identifier"] = json!("界".repeat(128));
        body["stream_options"] = json!({"include_obfuscation":true});
        let request = Request::parse_deferred_openai(
            client,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        let payload = request
            .upstream(&legacy_route(upstream, [true; 5]))
            .unwrap();
        assert_eq!(payload["safety_identifier"], body["safety_identifier"]);
        assert_eq!(payload["stream_options"]["include_obfuscation"], true);
    }
}

#[test]
fn legacy_openai_preserves_nullable_fields_and_protocol_specific_tiers() {
    for (client, upstream) in [
        (ClientProtocol::Chat, UpstreamProtocol::Chat),
        (ClientProtocol::Responses, UpstreamProtocol::Responses),
    ] {
        let mut body = match client {
            ClientProtocol::Chat => {
                json!({"model":"alias","messages":[{"role":"user","content":"hello"}]})
            }
            _ => json!({"model":"alias","input":"hello"}),
        };
        body["service_tier"] = Value::Null;
        body["safety_identifier"] = Value::Null;
        let request = Request::parse_deferred_openai(
            client,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        let mut expected = input(client, false).upstream_to(upstream, "real").unwrap();
        expected["service_tier"] = Value::Null;
        expected["safety_identifier"] = Value::Null;
        assert_eq!(
            request
                .upstream(&legacy_route(upstream, [true; 5]))
                .unwrap(),
            expected
        );
        assert_eq!(
            request
                .upstream(&legacy_route(upstream, [false; 5]))
                .unwrap(),
            input(client, false).upstream_to(upstream, "real").unwrap()
        );
        body["service_tier"] = json!("ultrafast");
        let request = Request::parse_deferred_openai(
            client,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        let result = request.upstream(&legacy_route(upstream, [true; 5]));
        if client == ClientProtocol::Responses {
            expected["service_tier"] = json!("ultrafast");
            assert_eq!(result.unwrap(), expected);
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn legacy_openai_admission_rejects_duplicates_unknowns_and_control_keys_before_filtering() {
    for (client, base) in [
        (
            ClientProtocol::Chat,
            r#""model":"alias","messages":[{"role":"user","content":"hello"}]"#,
        ),
        (
            ClientProtocol::Responses,
            r#""model":"alias","input":"hello""#,
        ),
    ] {
        for extra in [
            r#""service_tier":0,"service_tier":1"#,
            r#""service_tier":0,"SERVICE_TIER":1"#,
            r#""stream_options":{"include_obfuscation":false,"include_obfuscation":true}"#,
            r#""stream_options":{"include_obfuscation":false,"Include_Obfuscation":true}"#,
            r#""stream_options":{"include_usage":true,"INCLUDE_USAGE":false,"include_obfuscation":false}"#,
            r#""stream_options":{"include_obfuscation":false,"unknown":1}"#,
            r#""stream_options":[],"speed":"fast""#,
            r#""unknown":1,"speed":"fast""#,
            r#""payload_policy":{"legacy_openai":{}},"speed":"fast""#,
            r#""allow_speed":true,"speed":"fast""#,
            r#""upstream_id":7,"speed":"fast""#,
            r#""SERVICE_TIER":"priority""#,
        ] {
            let body = format!("{{{base},{extra}}}");
            assert!(
                Request::parse_deferred_openai(client, body.as_bytes(), &Limits::default())
                    .is_err(),
                "{body}"
            );
        }
        let body = format!("{{{base},\"speed\":\"fast\"}}");
        let limits = Limits {
            request_bytes: body.len() - 1,
            ..Limits::default()
        };
        assert!(Request::parse_deferred_openai(client, body.as_bytes(), &limits).is_err());
    }
}

#[test]
fn openai_request_parsing_bounds_nested_stream_options_and_values() {
    let limits = Limits::default();
    for (client, base) in [
        (
            ClientProtocol::Chat,
            r#""model":"alias","messages":[{"role":"user","content":"hello"}]"#,
        ),
        (
            ClientProtocol::Responses,
            r#""model":"alias","input":"hello""#,
        ),
    ] {
        for depth in [1, 256, 4096] {
            let nested = format!(
                "{}null{}",
                "{\"stream_options\":".repeat(depth),
                "}".repeat(depth)
            );
            let body = format!("{{{base},\"stream_options\":{nested}}}");
            assert!(body.len() < limits.request_bytes);
            for result in [
                Request::parse(client, body.as_bytes(), &limits),
                Request::parse_deferred_openai(client, body.as_bytes(), &limits),
            ] {
                assert!(matches!(
                    result,
                    Err(RelayError::Invalid("invalid or ambiguous JSON body"))
                ));
            }
        }
        let deep = format!("{}null{}", "[".repeat(256), "]".repeat(256));
        for extra in [
            format!("\"stream_options\":{{\"include_obfuscation\":{deep}}}"),
            format!("\"stream_options\":{{\"include_usage\":{deep}}}"),
            format!("\"service_tier\":{deep}"),
            r#""stream_options":{"include_usage":true,"include_usage":false}"#.into(),
            r#""stream_options":{"include_usage":true,"INCLUDE_USAGE":false}"#.into(),
            r#""stream_options":{"include_obfuscation":true,"include_obfuscation":false}"#.into(),
            r#""stream_options":{"include_obfuscation":true,"INCLUDE_OBFUSCATION":false}"#.into(),
        ] {
            let body = format!("{{{base},{extra}}}");
            for result in [
                Request::parse(client, body.as_bytes(), &limits),
                Request::parse_deferred_openai(client, body.as_bytes(), &limits),
            ] {
                assert!(matches!(
                    result,
                    Err(RelayError::Invalid("invalid or ambiguous JSON body"))
                ));
            }
        }
    }
}

#[test]
fn legacy_openai_empty_stream_options_match_omitted_options() {
    for (client, upstream, base) in [
        (
            ClientProtocol::Chat,
            UpstreamProtocol::Chat,
            json!({"model":"alias","messages":[{"role":"user","content":"hello"}]}),
        ),
        (
            ClientProtocol::Responses,
            UpstreamProtocol::Responses,
            json!({"model":"alias","input":"hello"}),
        ),
    ] {
        for stream in [false, true] {
            let mut body = base.clone();
            body["stream"] = json!(stream);
            body["stream_options"] = json!({});
            let request = Request::parse_deferred_openai(
                client,
                &serde_json::to_vec(&body).unwrap(),
                &Limits::default(),
            )
            .unwrap();
            assert_eq!(
                request.upstream_to(upstream, "real").unwrap(),
                input(client, stream).upstream_to(upstream, "real").unwrap()
            );
        }
    }
}

#[test]
fn legacy_openai_never_authorizes_cross_protocol_or_passthrough_encoding() {
    let bytes =
        br#"{"model":"alias","messages":[{"role":"user","content":"hello"}],"speed":"fast"}"#;
    let request =
        Request::parse_deferred_openai(ClientProtocol::Chat, bytes, &Limits::default()).unwrap();
    for protocol in [
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        assert!(
            request
                .upstream(&legacy_route(protocol, [false; 5]))
                .is_err()
        );
        assert!(
            input(ClientProtocol::Chat, false)
                .upstream(&legacy_route(protocol, [false; 5]))
                .is_err()
        );
    }
    let passthrough = legacy_route(UpstreamProtocol::Chat, [false; 5])
        .with_payload_policy(crate::routing::PayloadPolicy::Passthrough);
    assert!(request.upstream(&passthrough).is_err());
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
    let body = json!({
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

    let empty = chat_request(json!({
        "model":"gemini-3.1-flash-image-preview",
        "messages":[{"role":"user","content":"draw"}],
        "imageConfig":{}
    }))
    .upstream_to(UpstreamProtocol::Gemini, "image-model")
    .unwrap();
    assert!(empty["generationConfig"].get("imageConfig").is_none());
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

    for value in ["", " 4K", "4K\n"] {
        let invalid = json!({
            "model":"image-model",
            "messages":[{"role":"user","content":"draw"}],
            "imageConfig":{"imageSize":value}
        });
        assert!(matches!(
            Request::parse(
                ClientProtocol::Chat,
                &serde_json::to_vec(&invalid).unwrap(),
                &Limits::default(),
            ),
            Err(RelayError::Invalid(_))
        ));
    }
    let invalid = json!({
        "model":"image-model",
        "messages":[{"role":"user","content":"draw"}],
        "imageConfig":{"imageSize":"x".repeat(33)}
    });
    assert!(matches!(
        Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&invalid).unwrap(),
            &Limits::default(),
        ),
        Err(RelayError::Invalid(_))
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
fn deferred_openai_image_config_survives_reencoding_and_selected_route_checks() {
    let mut body = json!({
        "model":"image-model",
        "messages":[{"role":"user","content":"draw"}],
        "imageConfig":{"imageSize":"4K","aspectRatio":"16:9"}
    });
    let parse = |body: &Value| {
        Request::parse_deferred_openai(
            ClientProtocol::Chat,
            &serde_json::to_vec(body).unwrap(),
            &Limits::default(),
        )
        .unwrap()
    };
    let request = parse(&body);
    let gemini = Route::new(
        "gemini".into(),
        UpstreamProtocol::Gemini,
        "https://provider.invalid/v1",
        "image-model".into(),
        "synthetic",
        1,
    )
    .unwrap();
    assert_eq!(
        request.upstream(&gemini).unwrap()["generationConfig"]["imageConfig"],
        body["imageConfig"]
    );
    for protocol in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
    ] {
        let route = Route::new(
            "other".into(),
            protocol,
            "https://provider.invalid/v1",
            "real".into(),
            "synthetic",
            1,
        )
        .unwrap();
        assert!(matches!(
            request.upstream(&route),
            Err(RelayError::Unsupported(_))
        ));
    }
    body["speed"] = json!("fast");
    let request = parse(&body);
    for flags in [[false; 5], [true; 5]] {
        assert!(matches!(
            request.upstream(&legacy_route(UpstreamProtocol::Chat, flags)),
            Err(RelayError::Unsupported(_))
        ));
    }
    assert!(matches!(
        request.upstream(&gemini),
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

fn gemini_client(stream: bool) -> Request {
    let mut request = super::native_gemini::parse(
        "gemini-test",
        stream,
        br#"{"contents":[{"parts":[{"text":"hello"}]}]}"#,
        &Limits::default(),
    )
    .unwrap();
    request.client = ClientProtocol::Gemini;
    request.stream = stream;
    request
}

fn render_gemini(client: ClientProtocol, stream: bool, events: Vec<Event>) -> String {
    let request = if client == ClientProtocol::Gemini {
        gemini_client(stream)
    } else {
        input(client, stream)
    };
    let mut encoder = Encoder::new(&request, "long_decode", &Limits::default());
    let mut bytes = Vec::new();
    for event in events {
        for packet in encoder.consume(event).unwrap() {
            bytes.extend(packet);
        }
    }
    for packet in encoder.terminal().unwrap() {
        bytes.extend(packet);
    }
    String::from_utf8(bytes).unwrap()
}

fn json_packets(stream: bool, body: &str) -> Vec<Value> {
    if stream {
        body.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .map(|data| serde_json::from_str(data).unwrap())
            .collect()
    } else {
        vec![serde_json::from_str(body).unwrap()]
    }
}

fn gemini_upstream(reason: &str, token: Option<&str>) -> Value {
    let mut candidate = json!({
        "index": 0,
        "content": {"role": "model", "parts": [{"text": "partial"}]},
        "finishReason": reason
    });
    if let Some(token) = token {
        candidate["continuationToken"] = json!(token);
    }
    json!({
        "responseId": "g_1",
        "candidates": [candidate],
        "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 4}
    })
}

fn decode_gemini(stream: bool, frames: &[Value]) -> Vec<Event> {
    let mut decoder = Decoder::new(UpstreamProtocol::Gemini, &Limits::default());
    if stream {
        let mut events = Vec::new();
        for frame_value in frames {
            events.extend(frame(&mut decoder, frame_value.clone()).unwrap());
        }
        events.extend(decoder.eof().unwrap());
        events
    } else {
        decoder.json(&frames[0]).unwrap()
    }
}

#[test]
fn gemini_long_decode_keeps_continuation_token_and_maps_finish_reasons() {
    let token = "CiQ4ZmQ3-token";
    let source = gemini_upstream("CONTINUATION", Some(token));
    for stream in [false, true] {
        let frames = if stream {
            vec![
                json!({"responseId":"g_1","candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"partial"}]}}]}),
                json!({"responseId":"g_1","candidates":[{"index":0,"finishReason":"CONTINUATION","continuationToken":token}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":4}}),
            ]
        } else {
            vec![source.clone()]
        };
        let events = decode_gemini(stream, &frames);
        let native = render_gemini(ClientProtocol::Gemini, stream, events.clone());
        let native_packets = json_packets(stream, &native);
        let terminal = native_packets
            .iter()
            .rev()
            .find(|packet| packet["candidates"][0].get("finishReason").is_some())
            .unwrap();
        assert_eq!(terminal["candidates"][0]["finishReason"], "CONTINUATION");
        assert_eq!(terminal["candidates"][0]["continuationToken"], token);
        assert!(native_packets.iter().any(|packet| {
            packet["candidates"][0]["content"]["parts"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["text"] == "partial"))
        }));
        if stream {
            assert!(native_packets.iter().any(|packet| {
                packet["candidates"][0].get("finishReason").is_none()
                    && packet["candidates"][0].get("continuationToken").is_none()
                    && packet["candidates"][0]["content"]["parts"][0]["text"] == "partial"
            }));
        }
        let chat = render_gemini(ClientProtocol::Chat, stream, events);
        let finish = json_packets(stream, &chat)
            .into_iter()
            .rev()
            .find_map(|packet| {
                packet["choices"][0]["finish_reason"]
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap();
        assert_eq!(finish, "length");
        assert!(!chat.contains("continuationToken"));
        assert!(!chat.contains("CONTINUATION"));
    }

    for token in [None, Some("")] {
        let events = decode_gemini(false, &[gemini_upstream("CONTINUATION", token)]);
        let native = render_gemini(ClientProtocol::Gemini, false, events);
        let body: Value = serde_json::from_str(&native).unwrap();
        assert_eq!(body["candidates"][0]["finishReason"], "CONTINUATION");
        assert!(body["candidates"][0].get("continuationToken").is_none());
        assert!(!native.contains("continuationToken"));
    }

    let safety = decode_gemini(false, &[gemini_upstream("SAFETY", None)]);
    let chat = render_gemini(ClientProtocol::Chat, false, safety.clone());
    let chat_body: Value = serde_json::from_str(&chat).unwrap();
    assert_eq!(chat_body["choices"][0]["finish_reason"], "content_filter");
    let native = render_gemini(ClientProtocol::Gemini, false, safety);
    let native_body: Value = serde_json::from_str(&native).unwrap();
    assert_eq!(native_body["candidates"][0]["finishReason"], "SAFETY");
    assert!(
        native_body["candidates"][0]
            .get("continuationToken")
            .is_none()
    );

    let limited = decode_gemini(false, &[gemini_upstream("MAX_TOKENS", None)]);
    let chat = render_gemini(ClientProtocol::Chat, false, limited.clone());
    let chat_body: Value = serde_json::from_str(&chat).unwrap();
    assert_eq!(chat_body["choices"][0]["finish_reason"], "length");
    let native = render_gemini(ClientProtocol::Gemini, false, limited);
    let native_body: Value = serde_json::from_str(&native).unwrap();
    assert_eq!(native_body["candidates"][0]["finishReason"], "MAX_TOKENS");
    assert!(
        native_body["candidates"][0]
            .get("continuationToken")
            .is_none()
    );
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

#[test]
fn anthropic_merges_adjacent_assistant_tool_history_in_order() {
    let body = json!({
        "model": "alias",
        "messages": [
            {
                "role": "assistant",
                "content": "first",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "first_tool", "arguments": "{}"}
                }]
            },
            {
                "role": "assistant",
                "content": "second",
                "tool_calls": [{
                    "id": "call_2",
                    "type": "function",
                    "function": {"name": "second_tool", "arguments": "{}"}
                }]
            },
            {
                "role": "tool",
                "tool_call_id": "call_1",
                "content": "first result"
            },
            {
                "role": "tool",
                "tool_call_id": "call_2",
                "content": "second result"
            }
        ]
    });
    let body = match serde_json::to_vec(&body) {
        Ok(body) => body,
        Err(error) => panic!("test request serializes: {error}"),
    };
    let request = match Request::parse(ClientProtocol::Chat, &body, &Limits::default()) {
        Ok(request) => request,
        Err(error) => panic!("test request parses: {error}"),
    };
    let route = match Route::new(
        "anthropic".into(),
        UpstreamProtocol::Anthropic,
        "https://example.test/messages",
        "claude".into(),
        "test-only",
        1,
    ) {
        Ok(route) => route,
        Err(error) => panic!("test route is valid: {error}"),
    };
    let payload = match request.upstream(&route) {
        Ok(payload) => payload,
        Err(error) => panic!("test request converts: {error}"),
    };
    let messages = payload["messages"].as_array();
    assert_eq!(messages.map(Vec::len), Some(2));
    assert_eq!(
        messages
            .and_then(|messages| messages.first())
            .and_then(|message| message["role"].as_str()),
        Some("assistant")
    );
    let content = messages
        .and_then(|messages| messages.first())
        .and_then(|message| message["content"].as_array());
    assert_eq!(content.map(Vec::len), Some(4));
    assert_eq!(
        content
            .and_then(|content| content.first())
            .and_then(|item| item["text"].as_str()),
        Some("first")
    );
    assert_eq!(
        content
            .and_then(|content| content.get(1))
            .and_then(|item| item["id"].as_str()),
        Some("call_1")
    );
    assert_eq!(
        content
            .and_then(|content| content.get(2))
            .and_then(|item| item["text"].as_str()),
        Some("second")
    );
    assert_eq!(
        content
            .and_then(|content| content.get(3))
            .and_then(|item| item["id"].as_str()),
        Some("call_2")
    );
    let tool_results = messages
        .and_then(|messages| messages.get(1))
        .and_then(|message| message["content"].as_array());
    assert_eq!(
        messages
            .and_then(|messages| messages.get(1))
            .and_then(|message| message["role"].as_str()),
        Some("user")
    );
    assert_eq!(tool_results.map(Vec::len), Some(2));
    assert_eq!(
        tool_results
            .and_then(|items| items.first())
            .and_then(|item| item["tool_use_id"].as_str()),
        Some("call_1")
    );
    assert_eq!(
        tool_results
            .and_then(|items| items.get(1))
            .and_then(|item| item["tool_use_id"].as_str()),
        Some("call_2")
    );
}

#[test]
fn anthropic_merging_preserves_system_and_empty_role_boundaries() {
    let body = json!({"model":"alias","messages":[
        {"role":"system","content":"first instruction"},
        {"role":"developer","content":"second instruction"},
        {"role":"user","content":"first"},
        {"role":"user","content":"second"},
        {"role":"assistant","content":null},
        {"role":"user","content":"third"}
    ]});
    let request = Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&body).expect("system and role-boundary fixture serializes"),
        &Limits::default(),
    )
    .expect("system and role-boundary fixture parses");
    let payload = request
        .upstream_to(UpstreamProtocol::Anthropic, "claude")
        .expect("system and role-boundary fixture converts");
    assert_eq!(
        payload["system"],
        json!([
            {"type":"text","text":"first instruction"},
            {"type":"text","text":"second instruction"}
        ])
    );
    assert_eq!(
        payload["messages"],
        json!([
            {"role":"user","content":[{"type":"text","text":"first"},{"type":"text","text":"second"}]},
            {"role":"assistant","content":[]},
            {"role":"user","content":[{"type":"text","text":"third"}]}
        ])
    );
    for role in ["system", "developer"] {
        let body = json!({"model":"alias","messages":[
            {"role":"assistant","content":null},
            {"role":role,"content":"late instruction"}
        ]});
        let request = Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&body).expect("late system fixture serializes"),
            &Limits::default(),
        )
        .expect("late system fixture parses");
        assert!(matches!(
            request.upstream_to(UpstreamProtocol::Anthropic, "claude"),
            Err(RelayError::Unsupported(_))
        ));
    }
}

#[test]
fn anthropic_merging_preserves_signed_thinking_and_empty_same_role_messages() {
    let body = json!({"model":"alias","messages":[
        {"role":"assistant","content":null},
        {"role":"assistant","reasoning_content":"first thought","provider_metadata":{"protocol":"anthropic","signature":"first_signature"},"content":"first answer"},
        {"role":"assistant","content":[]},
        {"role":"assistant","reasoning_content":"second thought","provider_metadata":{"protocol":"anthropic","signature":"second_signature"},"content":"second answer"}
    ]});
    let request = Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&body).expect("signed thinking fixture serializes"),
        &Limits::default(),
    )
    .expect("signed thinking fixture parses");
    let payload = request
        .upstream_to(UpstreamProtocol::Anthropic, "claude")
        .expect("signed thinking fixture converts");
    assert_eq!(
        payload["messages"],
        json!([{"role":"assistant","content":[
            {"type":"thinking","thinking":"first thought","signature":"first_signature"},
            {"type":"text","text":"first answer"},
            {"type":"thinking","thinking":"second thought","signature":"second_signature"},
            {"type":"text","text":"second answer"}
        ]}])
    );
}
