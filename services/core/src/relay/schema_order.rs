//! Keep JSON Schema object-key order when a tool schema is re-encoded.
//!
//! `serde_json::Map` is a `BTreeMap`, so a decode/encode cycle sorts keys.
//! Structured-output and tool callers that fill `properties` in listed order
//! then see `answer` before `reasoning`. This module reads the client's schema
//! bytes, writes them again with that order, and splices the result into the
//! upstream body. Global JSON encoding stays sorted: billing and policy
//! digests must not start following client key order.
//!
//! Adapted from QuantumNous/new-api `d0b840e7`, which keeps schema bytes
//! instead of round-tripping them through a map.

use super::RelayError;
use serde::Deserialize;
use serde_json::{Value, value::RawValue};

#[derive(Clone, Debug)]
enum Node {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

#[derive(Clone, Debug)]
pub(crate) enum RawSchema {
    Json(String),
    GeminiOpenApi(String),
}

pub(crate) fn ordered_json_matching(raw: &str, expected: &Value) -> Option<String> {
    let node = parse_json(raw)?;
    let text = emit(&node)?;
    (serde_json::from_str::<Value>(&text).ok().as_ref() == Some(expected)).then_some(text)
}

pub(crate) fn ordered_gemini_schema(raw: &str, expected: &Value) -> Option<String> {
    let node = convert_gemini(&parse_json(raw)?)?;
    let text = emit(&node)?;
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

/// Replace each tool schema in `payload`'s compact encoding with `schemas`,
/// which use the client's object-key order. The parsed JSON value stays equal
/// to `payload`. A splice that would change that value is discarded.
pub(crate) fn splice_ordered_schemas(
    payload: &Value,
    schemas: &[String],
) -> Result<Vec<u8>, RelayError> {
    let canonical =
        serde_json::to_vec(payload).map_err(|_| RelayError::Invalid("request encoding failed"))?;
    let mut body = canonical.clone();
    let mut from = 0usize;
    for schema in schemas {
        let Ok(parsed) = serde_json::from_str::<Value>(schema) else {
            continue;
        };
        let Ok(needle) = serde_json::to_vec(&parsed) else {
            continue;
        };
        let Some(relative) = find_subslice(&body[from..], &needle) else {
            continue;
        };
        let at = from + relative;
        if needle.as_slice() == schema.as_bytes() {
            from = at + needle.len();
            continue;
        }
        body.splice(at..at + needle.len(), schema.bytes());
        from = at + schema.len();
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(value) if value == *payload => Ok(body),
        _ => Ok(canonical),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
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
                let Node::String(kind) = value else {
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
                Node::String(mapped.to_owned())
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

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
    depth: usize,
}

fn parse_json(raw: &str) -> Option<Node> {
    let mut parser = Parser {
        bytes: raw.as_bytes(),
        index: 0,
        depth: 0,
    };
    let node = parser.value()?;
    parser.skip_ws();
    (parser.index == parser.bytes.len()).then_some(node)
}

impl<'a> Parser<'a> {
    fn value(&mut self) -> Option<Node> {
        self.skip_ws();
        if self.depth > 64 {
            return None;
        }
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Node::String),
            b't' => self.literal(b"true").map(|()| Node::Bool(true)),
            b'f' => self.literal(b"false").map(|()| Node::Bool(false)),
            b'n' => self.literal(b"null").map(|()| Node::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn object(&mut self) -> Option<Node> {
        self.bump(b'{')?;
        self.depth += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.index += 1;
            self.depth -= 1;
            return Some(Node::Object(items));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.bump(b':')?;
            let value = self.value()?;
            if let Some(existing) = items.iter_mut().find(|(name, _)| name == &key) {
                existing.1 = value;
            } else {
                items.push((key, value));
            }
            self.skip_ws();
            match self.peek()? {
                b',' => self.index += 1,
                b'}' => {
                    self.index += 1;
                    self.depth -= 1;
                    return Some(Node::Object(items));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self) -> Option<Node> {
        self.bump(b'[')?;
        self.depth += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.index += 1;
            self.depth -= 1;
            return Some(Node::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek()? {
                b',' => self.index += 1,
                b']' => {
                    self.index += 1;
                    self.depth -= 1;
                    return Some(Node::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        self.bump(b'"')?;
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.index += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.index += 1;
                    let escaped = self.peek()?;
                    self.index += 1;
                    match escaped {
                        b'"' | b'\\' | b'/' => out.push(escaped as char),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return None,
                    }
                }
                0x00..=0x1f => return None,
                byte if byte < 0x80 => {
                    out.push(byte as char);
                    self.index += 1;
                }
                _ => {
                    let rest = self.bytes.get(self.index..)?;
                    let text = std::str::from_utf8(rest).ok()?;
                    let ch = text.chars().next()?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
        None
    }

    fn unicode_escape(&mut self) -> Option<char> {
        let unit = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&unit) {
            self.bump(b'\\')?;
            self.bump(b'u')?;
            let low = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return None;
            }
            let code = 0x10000 + (((unit as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
            return char::from_u32(code);
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            return None;
        }
        char::from_u32(unit as u32)
    }

    fn hex4(&mut self) -> Option<u16> {
        let mut value = 0u16;
        for _ in 0..4 {
            let byte = self.peek()?;
            self.index += 1;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return None,
            };
            value = (value << 4) | u16::from(digit);
        }
        Some(value)
    }

    fn number(&mut self) -> Option<Node> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        match self.peek()? {
            b'0' => self.index += 1,
            b'1'..=b'9' => {
                self.index += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.index += 1;
                }
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            let mut digits = false;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                digits = true;
                self.index += 1;
            }
            if !digits {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            let mut digits = false;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                digits = true;
                self.index += 1;
            }
            if !digits {
                return None;
            }
        }
        let literal = std::str::from_utf8(&self.bytes[start..self.index]).ok()?;
        Some(Node::Number(literal.to_owned()))
    }

    fn literal(&mut self, expected: &[u8]) -> Option<()> {
        if self.bytes.get(self.index..)?.starts_with(expected) {
            self.index += expected.len();
            Some(())
        } else {
            None
        }
    }

    fn bump(&mut self, expected: u8) -> Option<()> {
        if self.peek() == Some(expected) {
            self.index += 1;
            Some(())
        } else {
            None
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }
}

fn emit(node: &Node) -> Option<String> {
    let mut out = String::new();
    write_node(&mut out, node)?;
    Some(out)
}

fn write_node(out: &mut String, node: &Node) -> Option<()> {
    match node {
        Node::Null => out.push_str("null"),
        Node::Bool(true) => out.push_str("true"),
        Node::Bool(false) => out.push_str("false"),
        Node::Number(literal) => out.push_str(literal),
        Node::String(text) => out.push_str(&serde_json::to_string(text).ok()?),
        Node::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_node(out, item)?;
            }
            out.push(']');
        }
        Node::Object(items) => {
            out.push('{');
            for (index, (key, value)) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).ok()?);
                out.push(':');
                write_node(out, value)?;
            }
            out.push('}');
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::{
        ClientProtocol, Limits, UpstreamProtocol, native_anthropic, native_gemini, request,
    };
    use super::*;
    use serde_json::json;

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
                .upstream_body(UpstreamProtocol::Chat, "upstream-model")
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
                .upstream_body(UpstreamProtocol::Responses, "upstream-model")
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
                .upstream_body(UpstreamProtocol::Anthropic, "upstream-model")
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
            .upstream_body(UpstreamProtocol::Gemini, "gemini-test")
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
                .upstream_body(UpstreamProtocol::Gemini, "gemini-test")
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
}
