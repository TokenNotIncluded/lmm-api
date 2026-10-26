//! Keep client object order only in the selected upstream tool-schema fields.
//!
//! Behavior follows QuantumNous/new-api `d0b840e75ff36d731b04ff981ee361819acd593f`.
//! Parsing and raw-field serialization reuse the already locked serde_json
//! 1.0.151 (`serde-rs/json` v1.0.151, MIT OR Apache-2.0). Global JSON maps remain
//! sorted, so billing and policy digests keep their existing representation.

use super::{RelayError, UpstreamProtocol};
use serde::{
    Deserialize, Serialize,
    de::{MapAccess, Visitor},
    ser::SerializeMap,
};
use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
enum Node {
    Scalar(Value),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Scalar(value) => value.serialize(serializer),
            Self::Array(items) => items.serialize(serializer),
            Self::Object(items) => {
                let mut map = serializer.serialize_map(Some(items.len()))?;
                for (key, value) in items {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RawSchema {
    Json(String),
    GeminiOpenApi(String),
}

pub(crate) fn ordered_json_matching(raw: &str, expected: &Value) -> Option<String> {
    let text = serde_json::to_string(&parse_json(raw)?).ok()?;
    (serde_json::from_str::<Value>(&text).ok().as_ref() == Some(expected)).then_some(text)
}

pub(crate) fn ordered_gemini_schema(raw: &str, expected: &Value) -> Option<String> {
    let node = convert_gemini(&parse_json(raw)?)?;
    let text = serde_json::to_string(&node).ok()?;
    (serde_json::from_str::<Value>(&text).ok().as_ref() == Some(expected)).then_some(text)
}

pub(crate) fn chat_schemas(body: &[u8]) -> Option<Vec<Option<RawSchema>>> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        tools: Vec<Tool>,
    }
    #[derive(Deserialize)]
    struct Tool {
        #[serde(default)]
        function: Option<Function>,
    }
    #[derive(Deserialize)]
    struct Function {
        #[serde(default)]
        parameters: Option<Box<RawValue>>,
    }
    let body: Body = serde_json::from_slice(body).ok()?;
    Some(
        body.tools
            .into_iter()
            .map(|tool| {
                tool.function.and_then(|function| {
                    function
                        .parameters
                        .map(|raw| RawSchema::Json(raw.get().to_owned()))
                })
            })
            .collect(),
    )
}

pub(crate) fn responses_schemas(body: &[u8]) -> Option<Vec<Option<RawSchema>>> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        tools: Vec<Tool>,
    }
    #[derive(Deserialize)]
    struct Tool {
        #[serde(default)]
        parameters: Option<Box<RawValue>>,
    }
    let body: Body = serde_json::from_slice(body).ok()?;
    Some(
        body.tools
            .into_iter()
            .map(|tool| {
                tool.parameters
                    .map(|raw| RawSchema::Json(raw.get().to_owned()))
            })
            .collect(),
    )
}

pub(crate) fn anthropic_schemas(body: &[u8]) -> Option<Vec<Option<RawSchema>>> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        tools: Vec<Tool>,
    }
    #[derive(Deserialize)]
    struct Tool {
        #[serde(default)]
        input_schema: Option<Box<RawValue>>,
    }
    let body: Body = serde_json::from_slice(body).ok()?;
    Some(
        body.tools
            .into_iter()
            .map(|tool| {
                tool.input_schema
                    .map(|raw| RawSchema::Json(raw.get().to_owned()))
            })
            .collect(),
    )
}

pub(crate) fn gemini_schemas(body: &[u8]) -> Option<Vec<Option<RawSchema>>> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        tools: Vec<Tool>,
    }
    #[derive(Deserialize)]
    struct Tool {
        #[serde(default, rename = "functionDeclarations")]
        function_declarations: Vec<Function>,
    }
    #[derive(Deserialize)]
    struct Function {
        #[serde(default)]
        parameters: Option<Box<RawValue>>,
        #[serde(default, rename = "parametersJsonSchema")]
        parameters_json_schema: Option<Box<RawValue>>,
    }
    let body: Body = serde_json::from_slice(body).ok()?;
    Some(
        body.tools
            .into_iter()
            .flat_map(|tool| tool.function_declarations)
            .map(
                |function| match (function.parameters, function.parameters_json_schema) {
                    (Some(raw), None) => Some(RawSchema::GeminiOpenApi(raw.get().to_owned())),
                    (None, Some(raw)) => Some(RawSchema::Json(raw.get().to_owned())),
                    (None, None) => None,
                    (Some(_), Some(_)) => None,
                },
            )
            .collect(),
    )
}

/// Serialize a verified payload, replacing only the protocol's actual schema
/// fields. A same-valued message/tool-result object can never consume an update.
pub(crate) fn encode_ordered_schemas(
    payload: &Value,
    protocol: UpstreamProtocol,
    schemas: &[String],
) -> Result<Vec<u8>, RelayError> {
    let canonical =
        || serde_json::to_vec(payload).map_err(|_| RelayError::Invalid("request encoding failed"));
    let Some(paths) = schema_paths(payload, protocol) else {
        return canonical();
    };
    if paths.len() != schemas.len() {
        return canonical();
    }
    let mut output = WireValue::Value(payload);
    for (path, schema) in paths.iter().zip(schemas) {
        let Some(actual) = value_at(payload, path) else {
            continue;
        };
        if serde_json::from_str::<Value>(schema).ok().as_ref() != Some(actual) {
            continue;
        }
        let Ok(raw) = RawValue::from_string(schema.clone()) else {
            continue;
        };
        replace_at(&mut output, path, raw);
    }
    let body =
        serde_json::to_vec(&output).map_err(|_| RelayError::Invalid("request encoding failed"))?;
    if serde_json::from_slice::<Value>(&body).ok().as_ref() == Some(payload) {
        Ok(body)
    } else {
        canonical()
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireValue<'a> {
    Value(&'a Value),
    Raw(Box<RawValue>),
    Object(BTreeMap<&'a str, WireValue<'a>>),
    Array(Vec<WireValue<'a>>),
}

#[derive(Clone, Copy)]
enum Member {
    Key(&'static str),
    Index(usize),
}

fn schema_paths(payload: &Value, protocol: UpstreamProtocol) -> Option<Vec<Vec<Member>>> {
    use Member::{Index, Key};
    let Some(tools) = payload.get("tools") else {
        return Some(Vec::new());
    };
    let mut paths = Vec::new();
    for (index, tool) in tools.as_array()?.iter().enumerate() {
        let mut path = vec![Key("tools"), Index(index)];
        match protocol {
            UpstreamProtocol::Chat => {
                path.extend([Key("function"), Key("parameters")]);
                paths.push(path);
            }
            UpstreamProtocol::Responses => {
                path.push(Key("parameters"));
                paths.push(path);
            }
            UpstreamProtocol::Anthropic => {
                path.push(Key("input_schema"));
                paths.push(path);
            }
            UpstreamProtocol::Gemini => {
                let declarations = tool.get("functionDeclarations")?.as_array()?;
                for declaration in 0..declarations.len() {
                    let mut path = path.clone();
                    path.extend([
                        Key("functionDeclarations"),
                        Index(declaration),
                        Key("parametersJsonSchema"),
                    ]);
                    paths.push(path);
                }
            }
        }
    }
    Some(paths)
}

fn value_at<'a>(mut value: &'a Value, path: &[Member]) -> Option<&'a Value> {
    for member in path {
        value = match member {
            Member::Key(key) => value.get(*key)?,
            Member::Index(index) => value.get(*index)?,
        };
    }
    Some(value)
}

fn replace_at<'a>(value: &mut WireValue<'a>, path: &[Member], raw: Box<RawValue>) {
    let Some((head, tail)) = path.split_first() else {
        *value = WireValue::Raw(raw);
        return;
    };
    if let WireValue::Value(original) = value {
        *value = match original {
            Value::Object(items) => WireValue::Object(
                items
                    .iter()
                    .map(|(key, value)| (key.as_str(), WireValue::Value(value)))
                    .collect(),
            ),
            Value::Array(items) => WireValue::Array(items.iter().map(WireValue::Value).collect()),
            _ => return,
        };
    }
    let next = match (value, head) {
        (WireValue::Object(items), Member::Key(key)) => items.get_mut(key),
        (WireValue::Array(items), Member::Index(index)) => items.get_mut(*index),
        _ => None,
    };
    if let Some(next) = next {
        replace_at(next, tail, raw);
    }
}

const GEMINI_KEYS: &[&str] = &[
    "type",
    "properties",
    "required",
    "description",
    "items",
    "enum",
    "format",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
];

fn convert_gemini(node: &Node) -> Option<Node> {
    let Node::Object(items) = node else {
        return None;
    };
    if items
        .iter()
        .any(|(key, _)| !GEMINI_KEYS.contains(&key.as_str()))
    {
        return None;
    }
    let mut out = Vec::with_capacity(items.len());
    for (key, value) in items {
        let converted = match key.as_str() {
            "type" => {
                let Node::Scalar(Value::String(kind)) = value else {
                    return None;
                };
                let mapped = match kind.as_str() {
                    "OBJECT" => "object",
                    "ARRAY" => "array",
                    "STRING" => "string",
                    "NUMBER" => "number",
                    "INTEGER" => "integer",
                    "BOOLEAN" => "boolean",
                    _ => return None,
                };
                Node::Scalar(Value::String(mapped.to_owned()))
            }
            "properties" => {
                let Node::Object(properties) = value else {
                    return None;
                };
                let mut converted = Vec::with_capacity(properties.len());
                for (name, schema) in properties {
                    converted.push((name.clone(), convert_gemini(schema)?));
                }
                Node::Object(converted)
            }
            "items" => convert_gemini(value)?,
            _ => value.clone(),
        };
        out.push((key.clone(), converted));
    }
    Some(Node::Object(out))
}

// RawValue and MapAccess delegate JSON syntax, UTF-8, numbers and escapes to
// serde_json. The bounded tree is only for local object order and Gemini type
// conversion; it is never a replacement for the main request validator.
struct Members<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MembersVisitor;
        impl<'de> Visitor<'de> for MembersVisitor {
            type Value = Members<'de>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut positions: BTreeMap<String, usize> = BTreeMap::new();
                let mut items: Vec<(String, &'de RawValue)> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let value = map.next_value::<&'de RawValue>()?;
                    if let Some(index) = positions.get(&key).copied() {
                        items[index].1 = value;
                    } else {
                        positions.insert(key.clone(), items.len());
                        items.push((key, value));
                    }
                }
                Ok(Members(items))
            }
        }
        deserializer.deserialize_map(MembersVisitor)
    }
}

fn parse_json(raw: &str) -> Option<Node> {
    parse_node(serde_json::from_str::<&RawValue>(raw).ok()?, 0)
}

fn parse_node(raw: &RawValue, depth: usize) -> Option<Node> {
    if depth > 64 {
        return None;
    }
    match raw.get().as_bytes().first()? {
        b'{' => {
            let members: Members<'_> = serde_json::from_str(raw.get()).ok()?;
            let mut items = Vec::with_capacity(members.0.len());
            for (key, value) in members.0 {
                items.push((key, parse_node(value, depth + 1)?));
            }
            Some(Node::Object(items))
        }
        b'[' => {
            let values: Vec<&RawValue> = serde_json::from_str(raw.get()).ok()?;
            let mut items = Vec::with_capacity(values.len());
            for value in values {
                items.push(parse_node(value, depth + 1)?);
            }
            Some(Node::Array(items))
        }
        _ => Some(Node::Scalar(serde_json::from_str(raw.get()).ok()?)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        ClientProtocol, Limits, Route, UpstreamProtocol, native_anthropic, native_gemini, request,
    };
    use super::*;
    use serde_json::json;

    fn route(protocol: UpstreamProtocol) -> Route {
        Route::new(
            "test-route".into(),
            protocol,
            "https://example.test/model",
            "upstream-model".into(),
            "synthetic-key",
            1,
        )
        .unwrap()
    }

    const SCHEMA: &str = r#"{"type":"object","properties":{"reasoning":{"type":"string","description":"why"},"answer":{"type":"string"}},"required":["reasoning","answer"]}"#;

    #[test]
    fn schema_properties_keep_insertion_order_across_a_marshal_round_trip() {
        let ordered =
            ordered_json_matching(SCHEMA, &serde_json::from_str::<Value>(SCHEMA).unwrap())
                .expect("schema matches its parsed value");
        assert!(
            ordered.contains(
                r#""properties":{"reasoning":{"type":"string","description":"why"},"answer":{"type":"string"}}"#
            ),
            "{ordered}"
        );
        let again =
            ordered_json_matching(&ordered, &serde_json::from_str::<Value>(&ordered).unwrap())
                .unwrap();
        assert_eq!(again, ordered);
        let sorted =
            serde_json::to_string(&serde_json::from_str::<Value>(SCHEMA).unwrap()).unwrap();
        assert!(sorted.find("\"answer\"").unwrap() < sorted.find("\"reasoning\"").unwrap());
        assert_ne!(ordered, sorted);
    }

    #[test]
    fn chat_responses_and_anthropic_upstream_bodies_keep_property_order() {
        let chat = br#"{"model":"m","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"answer","description":"d","parameters":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}},"required":["reasoning","answer"]}}}]}"#;
        let request =
            request::Request::parse(ClientProtocol::Chat, chat, &Limits::default()).unwrap();
        let body = String::from_utf8(
            request
                .upstream_body(&route(UpstreamProtocol::Chat))
                .unwrap(),
        )
        .unwrap();
        assert!(
            body.contains(
                r#""parameters":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}},"required":["reasoning","answer"]}"#
            ),
            "{body}"
        );
        let responses = br#"{"model":"m","input":"hi","tools":[{"type":"function","name":"answer","parameters":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}}]}"#;
        let request =
            request::Request::parse(ClientProtocol::Responses, responses, &Limits::default())
                .unwrap();
        let body = String::from_utf8(
            request
                .upstream_body(&route(UpstreamProtocol::Responses))
                .unwrap(),
        )
        .unwrap();
        assert!(
            body.contains(
                r#""parameters":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}"#
            ),
            "{body}"
        );
        let anthropic = br#"{"model":"m","max_tokens":16,"messages":[{"role":"user","content":"hi"}],"tools":[{"name":"answer","input_schema":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}}]}"#;
        let request = native_anthropic::parse(anthropic, &Limits::default()).unwrap();
        let body = String::from_utf8(
            request
                .upstream_body(&route(UpstreamProtocol::Anthropic))
                .unwrap(),
        )
        .unwrap();
        assert!(
            body.contains(
                r#""input_schema":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}"#
            ),
            "{body}"
        );
    }

    #[test]
    fn gemini_openapi_parameters_keep_property_order_while_types_are_lowercased() {
        let body = br#"{"contents":[{"parts":[{"text":"hi"}]}],"tools":[{"functionDeclarations":[{"name":"answer","parameters":{"type":"OBJECT","properties":{"reasoning":{"type":"STRING"},"answer":{"type":"STRING"}}}}]}]}"#;
        let request = native_gemini::parse("gemini-test", false, body, &Limits::default()).unwrap();
        let encoded = request
            .upstream_body(&route(UpstreamProtocol::Gemini))
            .unwrap();
        let text = String::from_utf8(encoded).unwrap();
        assert!(
            text.contains(
                r#""parametersJsonSchema":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}"#
            ),
            "{text}"
        );
        let value: Value = serde_json::from_slice(body).unwrap();
        assert_eq!(
            value["tools"][0]["functionDeclarations"][0]["parameters"]["type"],
            json!("OBJECT")
        );
    }

    #[test]
    fn gemini_json_schema_parameters_keep_client_key_order() {
        let body = br#"{"contents":[{"parts":[{"text":"hi"}]}],"tools":[{"functionDeclarations":[{"name":"answer","parametersJsonSchema":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}}]}]}"#;
        let request = native_gemini::parse("gemini-test", false, body, &Limits::default()).unwrap();
        let text = String::from_utf8(
            request
                .upstream_body(&route(UpstreamProtocol::Gemini))
                .unwrap(),
        )
        .unwrap();
        assert!(
            text.contains(
                r#""parametersJsonSchema":{"type":"object","properties":{"reasoning":{"type":"string"},"answer":{"type":"string"}}}"#
            ),
            "{text}"
        );
    }

    #[test]
    fn replacing_schema_order_cannot_rewrite_a_same_value_tool_input() {
        let raw = r#"{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}}}"#;
        let body = format!(
            r#"{{"model":"m","max_tokens":16,"messages":[{{"role":"assistant","content":[{{"type":"tool_use","id":"call_1","name":"echo","input":{raw}}}]}},{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"call_1","content":"done"}}]}}],"tools":[{{"name":"echo","input_schema":{raw}}}]}}"#
        );
        let request = request::Request::parse(
            ClientProtocol::Anthropic,
            body.as_bytes(),
            &Limits::default(),
        )
        .unwrap();
        let route = route(UpstreamProtocol::Anthropic);
        let payload = request.upstream(&route).unwrap();
        let canonical =
            serde_json::to_string(&payload["messages"][0]["content"][0]["input"]).unwrap();
        let bytes = request.upstream_body(&route).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), payload);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(&format!("\"input_schema\":{raw}")), "{text}");
        assert!(text.contains(&format!("\"input\":{canonical}")), "{text}");
    }

    #[test]
    fn equal_tool_schemas_keep_their_own_order_by_index() {
        let first =
            r#"{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}}}"#;
        let second =
            r#"{"properties":{"a":{"type":"string"},"z":{"type":"string"}},"type":"object"}"#;
        let body = format!(
            r#"{{"model":"m","messages":[{{"role":"user","content":"hi"}}],"tools":[{{"type":"function","function":{{"name":"one","parameters":{first}}}}},{{"type":"function","function":{{"name":"two","parameters":{second}}}}}]}}"#
        );
        let request =
            request::Request::parse(ClientProtocol::Chat, body.as_bytes(), &Limits::default())
                .unwrap();
        let route = route(UpstreamProtocol::Chat);
        let bytes = request.upstream_body(&route).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            request.upstream(&route).unwrap()
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(&format!("\"parameters\":{first}")), "{text}");
        assert!(text.contains(&format!("\"parameters\":{second}")), "{text}");
    }

    #[test]
    fn deferred_capture_and_body_encoding_keep_the_selected_route_policy() {
        let body = br#"{"model":"m","service_tier":"priority","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"answer","parameters":{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}}}}}]}"#;
        let request =
            request::Request::parse_deferred_openai(ClientProtocol::Chat, body, &Limits::default())
                .unwrap();
        assert!(
            request
                .upstream_body(&route(UpstreamProtocol::Chat))
                .is_err()
        );
        for allowed in [false, true] {
            let route = route(UpstreamProtocol::Chat).with_payload_policy(
                crate::routing::PayloadPolicy::LegacyOpenai {
                    allow_service_tier: allowed,
                    allow_inference_geo: false,
                    allow_speed: false,
                    allow_safety_identifier: false,
                    allow_include_obfuscation: false,
                },
            );
            let bytes = request.upstream_body(&route).unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value, request.upstream(&route).unwrap());
            assert_eq!(value.get("service_tier").is_some(), allowed);
            let text = String::from_utf8(bytes).unwrap();
            assert!(
                text.contains(r#""properties":{"z":{"type":"string"},"a":{"type":"string"}}"#),
                "{text}"
            );
        }
    }

    #[test]
    fn duplicate_escaped_keys_use_last_value_at_first_position() {
        let raw = r#"{"z":1,"a":{"description":"中文😄"},"\u007a":1234567890123456789012345678901234567890123}"#;
        let expected: Value = serde_json::from_str(raw).unwrap();
        let text = ordered_json_matching(raw, &expected).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
        assert!(text.find("\"z\"").unwrap() < text.find("\"a\"").unwrap());
        assert_eq!(text.matches("\"z\"").count(), 1);
    }

    #[test]
    fn strict_transport_encoding_preserves_schema_and_rejects_deferred_fields() {
        let strict = br#"{"model":"m","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"answer","parameters":{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}}}}}]}"#;
        let request =
            request::Request::parse(ClientProtocol::Chat, strict, &Limits::default()).unwrap();
        for protocol in [
            UpstreamProtocol::Chat,
            UpstreamProtocol::Responses,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ] {
            let bytes = request
                .upstream_body_to(protocol, "upstream-model")
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                request.upstream_to(protocol, "upstream-model").unwrap()
            );
            let text = String::from_utf8(bytes).unwrap();
            assert!(
                text.contains(r#""properties":{"z":{"type":"string"},"a":{"type":"string"}}"#),
                "{text}"
            );
        }
        let deferred = br#"{"model":"m","service_tier":"priority","messages":[{"role":"user","content":"hi"}]}"#;
        let request = request::Request::parse_deferred_openai(
            ClientProtocol::Chat,
            deferred,
            &Limits::default(),
        )
        .unwrap();
        assert!(matches!(
            request.upstream_body_to(UpstreamProtocol::Chat, "upstream-model"),
            Err(RelayError::Unsupported(_))
        ));
    }
}
