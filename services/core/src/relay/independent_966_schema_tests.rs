use super::{ClientProtocol, Limits, RelayError, Request, Route, UpstreamProtocol};
use serde::{
    Deserialize,
    de::{IgnoredAny, MapAccess, Visitor},
};
use serde_json::{Value, json, value::RawValue};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Instant};

const ZA: &str =
    r#"{"type":"object","properties":{"zeta":{"type":"string"},"alpha":{"type":"string"}}}"#;
const AZ: &str =
    r#"{"type":"object","properties":{"alpha":{"type":"string"},"zeta":{"type":"string"}}}"#;

struct ObjectKeys(Vec<String>);
impl<'de> Deserialize<'de> for ObjectKeys {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct KeysVisitor;
        impl<'de> Visitor<'de> for KeysVisitor {
            type Value = ObjectKeys;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object whose key order is observable")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ObjectKeys, A::Error> {
                let mut keys = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    keys.push(key);
                    map.next_value::<IgnoredAny>()?;
                }
                Ok(ObjectKeys(keys))
            }
        }
        d.deserialize_map(KeysVisitor)
    }
}

fn raw_at(body: &[u8], path: &[&str]) -> String {
    let mut raw = std::str::from_utf8(body).unwrap().to_owned();
    for segment in path {
        if raw.trim_start().starts_with('[') {
            let values: Vec<Box<RawValue>> = serde_json::from_str(&raw).unwrap();
            raw = values[segment.parse::<usize>().unwrap()].get().to_owned();
        } else {
            let values: BTreeMap<String, Box<RawValue>> = serde_json::from_str(&raw).unwrap();
            raw = values[*segment].get().to_owned();
        }
    }
    raw
}

fn keys_at(body: &[u8], path: &[&str]) -> Vec<String> {
    serde_json::from_str::<ObjectKeys>(&raw_at(body, path))
        .unwrap()
        .0
}

fn route(protocol: UpstreamProtocol) -> Route {
    Route::new(
        "synthetic-966".into(),
        protocol,
        "https://provider.invalid/v1",
        "wire-model".into(),
        "synthetic-key",
        1,
    )
    .unwrap()
}

fn tool_path(protocol: UpstreamProtocol, index: usize) -> Vec<String> {
    let mut path: Vec<String> = match protocol {
        UpstreamProtocol::Chat => vec!["tools", "", "function", "parameters"],
        UpstreamProtocol::Responses => vec!["tools", "", "parameters"],
        UpstreamProtocol::Anthropic => vec!["tools", "", "input_schema"],
        UpstreamProtocol::Gemini => {
            return vec![
                "tools".into(),
                "0".into(),
                "functionDeclarations".into(),
                index.to_string(),
                "parametersJsonSchema".into(),
            ];
        }
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    path[1] = index.to_string();
    path
}

fn schema_keys(body: &[u8], protocol: UpstreamProtocol, index: usize) -> Vec<String> {
    let mut path = tool_path(protocol, index);
    path.push("properties".into());
    keys_at(body, &path.iter().map(String::as_str).collect::<Vec<_>>())
}

fn client_body(client: ClientProtocol, schemas: &[&str]) -> Vec<u8> {
    let tools = schemas
        .iter()
        .enumerate()
        .map(|(index, schema)| match client {
            ClientProtocol::Chat => format!(
                r#"{{"type":"function","function":{{"name":"tool_{index}","parameters":{schema}}}}}"#
            ),
            ClientProtocol::Responses => {
                format!(r#"{{"type":"function","name":"tool_{index}","parameters":{schema}}}"#)
            }
            ClientProtocol::Anthropic => {
                format!(r#"{{"name":"tool_{index}","input_schema":{schema}}}"#)
            }
            ClientProtocol::Gemini => {
                format!(r#"{{"name":"tool_{index}","parametersJsonSchema":{schema}}}"#)
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    match client {
        ClientProtocol::Chat => format!(
            r#"{{"model":"m","messages":[{{"role":"user","content":"hello"}}],"tools":[{tools}]}}"#
        ),
        ClientProtocol::Responses => {
            format!(r#"{{"model":"m","input":"hello","tools":[{tools}]}}"#)
        }
        ClientProtocol::Anthropic => format!(
            r#"{{"model":"m","max_tokens":16,"messages":[{{"role":"user","content":"hello"}}],"tools":[{tools}]}}"#
        ),
        ClientProtocol::Gemini => format!(
            r#"{{"contents":[{{"parts":[{{"text":"hello"}}]}}],"tools":[{{"functionDeclarations":[{tools}]}}]}}"#
        ),
    }
    .into_bytes()
}

fn parse(client: ClientProtocol, body: &[u8]) -> Request {
    match client {
        ClientProtocol::Gemini => {
            super::native_gemini::parse("m", false, body, &Limits::default()).unwrap()
        }
        _ => Request::parse(client, body, &Limits::default()).unwrap(),
    }
}

fn encoded(request: &Request, route: &Route) -> Vec<u8> {
    let body = request.upstream_body(route).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        request.upstream(route).unwrap(),
        "schema ordering must not alter the authorized payload's JSON value"
    );
    body
}

#[test]
fn independent_966_all_plain_protocol_directions_preserve_each_tool_schema_order() {
    for client in [
        ClientProtocol::Chat,
        ClientProtocol::Responses,
        ClientProtocol::Anthropic,
        ClientProtocol::Gemini,
    ] {
        let request = parse(client, &client_body(client, &[ZA, AZ]));
        for upstream in [
            UpstreamProtocol::Chat,
            UpstreamProtocol::Responses,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ] {
            let body = encoded(&request, &route(upstream));
            assert_eq!(schema_keys(&body, upstream, 0), ["zeta", "alpha"]);
            assert_eq!(schema_keys(&body, upstream, 1), ["alpha", "zeta"]);
        }
    }
}

#[test]
fn independent_966_anthropic_same_value_tool_input_cannot_steal_schema_order() {
    let body = format!(
        r#"{{"model":"m","max_tokens":16,"messages":[{{"role":"assistant","content":[{{"type":"tool_use","id":"call_1","name":"tool_0","input":{ZA}}}]}},{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"call_1","content":"done"}}]}}],"tools":[{{"name":"tool_0","input_schema":{ZA}}}]}}"#
    );
    let request = parse(ClientProtocol::Anthropic, body.as_bytes());
    let body = encoded(&request, &route(UpstreamProtocol::Anthropic));
    assert_eq!(
        keys_at(
            &body,
            &["messages", "0", "content", "0", "input", "properties"]
        ),
        ["alpha", "zeta"],
        "ordinary tool input keeps the existing canonical encoder behavior"
    );
    assert_eq!(
        schema_keys(&body, UpstreamProtocol::Anthropic, 0),
        ["zeta", "alpha"]
    );
}

#[test]
fn independent_966_gemini_same_value_call_args_cannot_steal_schema_order() {
    let body = format!(
        r#"{{"contents":[{{"role":"model","parts":[{{"functionCall":{{"id":"call_1","name":"tool_0","args":{ZA}}}}}]}},{{"role":"user","parts":[{{"functionResponse":{{"id":"call_1","name":"tool_0","response":{{"ok":true}}}}}}]}}],"tools":[{{"functionDeclarations":[{{"name":"tool_0","parametersJsonSchema":{ZA}}}]}}]}}"#
    );
    let request = parse(ClientProtocol::Gemini, body.as_bytes());
    let body = encoded(&request, &route(UpstreamProtocol::Gemini));
    assert_eq!(
        keys_at(
            &body,
            &[
                "contents",
                "0",
                "parts",
                "0",
                "functionCall",
                "args",
                "properties"
            ]
        ),
        ["alpha", "zeta"]
    );
    assert_eq!(
        schema_keys(&body, UpstreamProtocol::Gemini, 0),
        ["zeta", "alpha"]
    );
}

#[test]
fn independent_966_gemini_openapi_conversion_preserves_order_and_normalizes_types() {
    let body = br#"{"contents":[{"parts":[{"text":"hello"}]}],"tools":[{"functionDeclarations":[{"name":"tool_0","parameters":{"type":"OBJECT","properties":{"zeta":{"type":"STRING"},"alpha":{"type":"INTEGER"}}}}]}]}"#;
    let request = parse(ClientProtocol::Gemini, body);
    let body = encoded(&request, &route(UpstreamProtocol::Gemini));
    assert_eq!(
        schema_keys(&body, UpstreamProtocol::Gemini, 0),
        ["zeta", "alpha"]
    );
    let actual: Value = serde_json::from_str(&raw_at(
        &body,
        &[
            "tools",
            "0",
            "functionDeclarations",
            "0",
            "parametersJsonSchema",
        ],
    ))
    .unwrap();
    assert_eq!(actual["type"], "object");
    assert_eq!(actual["properties"]["alpha"]["type"], "integer");
}

#[test]
fn independent_966_deferred_parser_retains_order_and_route_authority() {
    let mut body = String::from_utf8(client_body(ClientProtocol::Chat, &[ZA])).unwrap();
    body.pop();
    body.push_str(r#","speed":"fast"}"#);
    let request =
        Request::parse_deferred_openai(ClientProtocol::Chat, body.as_bytes(), &Limits::default())
            .unwrap();
    let ordinary = route(UpstreamProtocol::Chat);
    assert!(
        request.upstream_body(&ordinary).is_err(),
        "raw body must not bypass route policy"
    );
    let allowed = route(UpstreamProtocol::Chat).with_payload_policy(
        crate::routing::PayloadPolicy::LegacyOpenai {
            allow_service_tier: false,
            allow_inference_geo: false,
            allow_speed: true,
            allow_safety_identifier: false,
            allow_include_obfuscation: false,
        },
    );
    let encoded = encoded(&request, &allowed);
    assert_eq!(
        schema_keys(&encoded, UpstreamProtocol::Chat, 0),
        ["zeta", "alpha"]
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&encoded).unwrap()["speed"],
        "fast"
    );
}

#[test]
fn independent_966_empty_and_mixed_schema_objects_do_not_touch_other_fields() {
    let request = parse(
        ClientProtocol::Chat,
        &client_body(ClientProtocol::Chat, &["{}", ZA, r#"{"type":"object"}"#]),
    );
    for upstream in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        let body = encoded(&request, &route(upstream));
        let p0 = tool_path(upstream, 0);
        assert_eq!(
            serde_json::from_str::<Value>(&raw_at(
                &body,
                &p0.iter().map(String::as_str).collect::<Vec<_>>()
            ))
            .unwrap(),
            json!({})
        );
        assert_eq!(schema_keys(&body, upstream, 1), ["zeta", "alpha"]);
    }
}

#[test]
fn independent_966_uncharged_strict_transport_keeps_order_without_route_authority() {
    let plain = parse(
        ClientProtocol::Chat,
        &client_body(ClientProtocol::Chat, &[ZA]),
    );
    let mut body = String::from_utf8(client_body(ClientProtocol::Chat, &[ZA])).unwrap();
    body.pop();
    body.push_str(r#","speed":"fast"}"#);
    let deferred =
        Request::parse_deferred_openai(ClientProtocol::Chat, body.as_bytes(), &Limits::default())
            .unwrap();
    for protocol in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        let output = plain.upstream_body_to(protocol, "wire-model").unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap(),
            plain.upstream_to(protocol, "wire-model").unwrap()
        );
        assert_eq!(schema_keys(&output, protocol, 0), ["zeta", "alpha"]);
        assert!(
            deferred.upstream_body_to(protocol, "wire-model").is_err(),
            "uncharged transport has no selected-route compatibility authority"
        );
    }
}

#[test]
fn independent_966_duplicate_schema_keys_keep_last_value_and_single_output_key() {
    let schema = r#"{"type":"object","properties":{"zeta":{"type":"number"},"alpha":{"type":"string"},"\u007aeta":{"type":"integer"}}}"#;
    let request = parse(
        ClientProtocol::Chat,
        &client_body(ClientProtocol::Chat, &[schema]),
    );
    let body = encoded(&request, &route(UpstreamProtocol::Chat));
    assert_eq!(
        schema_keys(&body, UpstreamProtocol::Chat, 0),
        ["zeta", "alpha"]
    );
    let actual: Value =
        serde_json::from_str(&raw_at(&body, &["tools", "0", "function", "parameters"])).unwrap();
    assert_eq!(actual["properties"]["zeta"]["type"], "integer");
}

#[test]
fn independent_966_large_numbers_unicode_and_nested_property_order_round_trip() {
    let schema = r#"{"type":"object","properties":{"zeta":{"type":"number","default":1234567890123456789012345678901234567890},"alpha":{"type":"object","description":"中文😀\ud83d\ude00","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}}}"#;
    let request = parse(
        ClientProtocol::Chat,
        &client_body(ClientProtocol::Chat, &[schema]),
    );
    let body = encoded(&request, &route(UpstreamProtocol::Chat));
    assert_eq!(
        schema_keys(&body, UpstreamProtocol::Chat, 0),
        ["zeta", "alpha"]
    );
    assert_eq!(
        keys_at(
            &body,
            &[
                "tools",
                "0",
                "function",
                "parameters",
                "properties",
                "alpha",
                "properties"
            ]
        ),
        ["reasoning", "answer"]
    );
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("1234567890123456789012345678901234567890"));
    let actual: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        actual["tools"][0]["function"]["parameters"]["properties"]["alpha"]["description"],
        "中文😀😀"
    );
}

#[test]
fn independent_966_capture_does_not_relax_ambiguous_controls_or_depth_limit() {
    let valid = String::from_utf8(client_body(ClientProtocol::Chat, &[ZA])).unwrap();
    let repeated = valid.replacen(r#""model":"m""#, r#""model":"m","MODEL":"other""#, 1);
    assert!(
        Request::parse(
            ClientProtocol::Chat,
            repeated.as_bytes(),
            &Limits::default()
        )
        .is_err()
    );
    let mut body = valid;
    body.pop();
    body.push_str(r#","stream_options":{"include_usage":true,"INCLUDE_USAGE":false}}"#);
    assert!(
        Request::parse_deferred_openai(ClientProtocol::Chat, body.as_bytes(), &Limits::default())
            .is_err()
    );
    let deep = format!(
        r#"{{"type":"object","properties":{{"x":{}0{}}}}}"#,
        "[".repeat(160),
        "]".repeat(160)
    );
    assert!(
        Request::parse(
            ClientProtocol::Chat,
            &client_body(ClientProtocol::Chat, &[&deep]),
            &Limits::default()
        )
        .is_err()
    );
}

#[test]
fn independent_966_input_byte_limit_remains_exact() {
    let body = client_body(ClientProtocol::Chat, &[ZA]);
    for offset in [0, 1] {
        let limits = Limits {
            request_bytes: body.len() + offset,
            ..Limits::default()
        };
        assert!(Request::parse(ClientProtocol::Chat, &body, &limits).is_ok());
    }
    let limits = Limits {
        request_bytes: body.len() - 1,
        ..Limits::default()
    };
    assert!(matches!(
        Request::parse(ClientProtocol::Chat, &body, &limits),
        Err(RelayError::Limit(_))
    ));
}

#[test]
fn independent_966_global_canonical_json_and_digest_stay_sorted() {
    let mut map = serde_json::Map::new();
    map.insert("z".into(), json!(1));
    map.insert("a".into(), json!(2));
    let actual = serde_json::to_vec(&Value::Object(map)).unwrap();
    let expected = br#"{"a":2,"z":1}"#;
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(&actual)),
        "c2985c5ba6f7d2a55e768f92490ca09388e95bc4cccb9fdf11b15f4d42f93e73"
    );
}

#[test]
fn independent_966_legal_wide_and_unicode_capacity_probe() {
    for n in [1024, 2048, 4096] {
        let properties = (0..n)
            .rev()
            .map(|i| format!(r#""p{i:05}":{{"type":"string"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let schema = format!(
            r#"{{"type":"object","description":"{}","properties":{{{properties}}}}}"#,
            "中".repeat(n)
        );
        let body = client_body(ClientProtocol::Chat, &[&schema]);
        assert!(body.len() < Limits::default().request_bytes);
        let started = Instant::now();
        let request = parse(ClientProtocol::Chat, &body);
        let output = encoded(&request, &route(UpstreamProtocol::Chat));
        let elapsed = started.elapsed();
        let keys = schema_keys(&output, UpstreamProtocol::Chat, 0);
        assert_eq!(keys.len(), n);
        assert_eq!(keys.first().unwrap(), &format!("p{:05}", n - 1));
        assert_eq!(keys.last().unwrap(), "p00000");
        eprintln!(
            "independent_966_capacity n={n} input_bytes={} output_bytes={} elapsed_ns={}",
            body.len(),
            output.len(),
            elapsed.as_nanos()
        );
    }
    // Functional capacity exercise only. The diagnostic timing is not a
    // release-mode, isolated-resource performance acceptance result.
}
