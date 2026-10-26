//! Native Gemini text/function client adapter. Routing and settlement remain host-owned.
//! Wire reference: googleapis/python-genai ddd7d821497afe97a392e599ccffd5dd5011e44c
//! (Apache-2.0); independently implemented, no upstream source copied.
use super::{
    BlockKind, ClientProtocol, Event, FinishReason, Limits, RelayError, Request, UpstreamProtocol,
    Usage,
};
use serde::{
    Deserialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

// Value normally overwrites duplicate keys. Walk every object, including tool
// arguments and schemas, before any conversion can hide ambiguous input.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut out = Map::new();
                let mut seen = BTreeSet::new();
                while let Some(k) = a.next_key::<String>()? {
                    if !seen.insert(k.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                    out.insert(k, a.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(out)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element::<Unique>()? {
                    out.push(v.0);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
fn keys<'a>(v: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, RelayError> {
    let o = v
        .as_object()
        .ok_or(RelayError::Invalid("expected Gemini object"))?;
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(RelayError::Unsupported("unsupported Gemini field"));
    }
    Ok(o)
}
fn array(v: &Value) -> Result<&Vec<Value>, RelayError> {
    v.as_array()
        .ok_or(RelayError::Invalid("expected Gemini array"))
}
fn string(v: &Value) -> Result<&str, RelayError> {
    v.as_str()
        .ok_or(RelayError::Invalid("expected Gemini string"))
}
fn schema(v: &Value) -> Result<Value, RelayError> {
    keys(
        v,
        &[
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
        ],
    )?;
    let mut out = v.clone();
    if let Some(t) = v.get("type") {
        let t = match string(t)? {
            "OBJECT" => "object",
            "ARRAY" => "array",
            "STRING" => "string",
            "NUMBER" => "number",
            "INTEGER" => "integer",
            "BOOLEAN" => "boolean",
            _ => return Err(RelayError::Unsupported("unsupported Gemini schema type")),
        };
        out["type"] = json!(t);
    }
    if let Some(p) = v.get("properties") {
        let p = p
            .as_object()
            .ok_or(RelayError::Invalid("invalid Gemini properties"))?;
        let mut converted = Map::new();
        for (k, v) in p {
            converted.insert(k.clone(), schema(v)?);
        }
        out["properties"] = Value::Object(converted);
    }
    if let Some(i) = v.get("items") {
        out["items"] = schema(i)?;
    }
    Ok(out)
}

pub(crate) fn parse(
    model: &str,
    stream: bool,
    body: &[u8],
    limits: &Limits,
) -> Result<Request, RelayError> {
    limits.validate()?;
    if body.len() > limits.request_bytes {
        return Err(RelayError::Limit("request body"));
    }
    let v = serde_json::from_slice::<Unique>(body)
        .map_err(|_| RelayError::Invalid("invalid or ambiguous Gemini JSON"))?
        .0;
    keys(
        &v,
        &[
            "contents",
            "systemInstruction",
            "tools",
            "toolConfig",
            "generationConfig",
            "continuationToken",
        ],
    )?;
    let mut messages = Vec::new();
    if let Some(s) = v.get("systemInstruction") {
        keys(s, &["parts", "role"])?;
        if s.get("role").is_some_and(|r| r != "system") {
            return Err(RelayError::Unsupported(
                "unsupported system instruction role",
            ));
        }
        let mut content = Vec::new();
        for p in array(&s["parts"])? {
            keys(p, &["text"])?;
            content.push(json!({"type":"text","text":string(&p["text"])?}));
        }
        if content.is_empty() {
            return Err(RelayError::Invalid("empty system instruction"));
        }
        messages.push(json!({"role":"system","content":content}));
    }
    let mut calls: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for c in array(&v["contents"])? {
        keys(c, &["role", "parts"])?;
        let role = match c.get("role").map(string).transpose()?.unwrap_or("user") {
            "user" => "user",
            "model" => "assistant",
            _ => return Err(RelayError::Invalid("invalid Gemini role")),
        };
        let parts = array(&c["parts"])?;
        if parts.is_empty() {
            return Err(RelayError::Invalid("empty Gemini content"));
        }
        for p in parts {
            keys(
                p,
                &[
                    "text",
                    "thought",
                    "thoughtSignature",
                    "functionCall",
                    "functionResponse",
                ],
            )?;
            let count = ["text", "functionCall", "functionResponse"]
                .iter()
                .filter(|k| p.get(**k).is_some())
                .count();
            if count != 1 {
                return Err(RelayError::Invalid(
                    "Gemini part must have exactly one payload",
                ));
            }
            let thought = p
                .get("thought")
                .map(|t| {
                    t.as_bool()
                        .ok_or(RelayError::Invalid("invalid thought flag"))
                })
                .transpose()?
                .unwrap_or(false);
            let sig = p.get("thoughtSignature").map(string).transpose()?;
            if sig.is_some_and(str::is_empty) {
                return Err(RelayError::Invalid("empty thought signature"));
            }
            if let Some(text) = p.get("text") {
                let text = string(text)?;
                if thought {
                    if role != "assistant" || sig.is_none() {
                        return Err(RelayError::Unsupported(
                            "thought history requires model role and original signature",
                        ));
                    }
                    messages.push(json!({"role":role,"reasoning_content":text,"provider_metadata":{"protocol":"gemini","signature":sig},"content":null}));
                } else {
                    if sig.is_some() {
                        return Err(RelayError::Unsupported(
                            "signed ordinary text history cannot be represented",
                        ));
                    }
                    messages.push(json!({"role":role,"content":text}));
                }
            } else if let Some(f) = p.get("functionCall") {
                if role != "assistant" || thought {
                    return Err(RelayError::Invalid("function call requires model role"));
                }
                keys(f, &["id", "name", "args"])?;
                let name = string(&f["name"])?;
                let id = f
                    .get("id")
                    .map(string)
                    .transpose()?
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("gemini_call_{}", calls.len()));
                if calls.insert(id.clone(), (name.to_owned(), false)).is_some() {
                    return Err(RelayError::Invalid("duplicate function call ID"));
                }
                if !f["args"].is_object() {
                    return Err(RelayError::Invalid("function arguments must be object"));
                }
                let mut call = json!({"id":id,"type":"function","function":{"name":name,"arguments":f["args"].to_string()}});
                if let Some(sig) = sig {
                    call["provider_metadata"] = json!({"protocol":"gemini","signature":sig});
                }
                messages.push(json!({"role":role,"content":null,"tool_calls":[call]}));
            } else {
                if role != "user" || thought || sig.is_some() {
                    return Err(RelayError::Invalid("invalid function response part"));
                }
                let f = &p["functionResponse"];
                keys(f, &["id", "name", "response"])?;
                let name = string(&f["name"])?;
                if !f["response"].is_object() {
                    return Err(RelayError::Invalid("function response must be object"));
                }
                let id = if let Some(id) = f.get("id") {
                    string(id)?.to_owned()
                } else {
                    let matches: Vec<_> = calls
                        .iter()
                        .filter(|(_, (n, used))| n == name && !used)
                        .map(|(id, _)| id.clone())
                        .collect();
                    if matches.len() != 1 {
                        return Err(RelayError::Invalid("ambiguous or orphan function response"));
                    }
                    matches[0].clone()
                };
                let entry = calls
                    .get_mut(&id)
                    .ok_or(RelayError::Invalid("orphan function response"))?;
                if entry.0 != name || entry.1 {
                    return Err(RelayError::Invalid(
                        "mismatched or duplicate function response",
                    ));
                }
                entry.1 = true;
                messages.push(
                    json!({"role":"tool","tool_call_id":id,"content":f["response"].to_string()}),
                );
            }
        }
    }
    if messages.len() > 1024 {
        return Err(RelayError::Limit("message count"));
    }
    let mut canonical = json!({"model":model,"stream":stream,"messages":messages});
    if let Some(g) = v.get("generationConfig") {
        keys(
            g,
            &[
                "maxOutputTokens",
                "temperature",
                "topP",
                "stopSequences",
                "candidateCount",
            ],
        )?;
        for (native, chat) in [
            ("maxOutputTokens", "max_tokens"),
            ("temperature", "temperature"),
            ("topP", "top_p"),
            ("stopSequences", "stop"),
            ("candidateCount", "n"),
        ] {
            if let Some(value) = g.get(native) {
                if value.is_null() {
                    return Err(RelayError::Invalid("null generation option"));
                }
                canonical[chat] = value.clone();
            }
        }
    }
    let mut tools = Vec::new();
    if let Some(ts) = v.get("tools") {
        for t in array(ts)? {
            keys(t, &["functionDeclarations"])?;
            for f in array(&t["functionDeclarations"])? {
                keys(
                    f,
                    &["name", "description", "parameters", "parametersJsonSchema"],
                )?;
                if f.get("parameters").is_some() && f.get("parametersJsonSchema").is_some() {
                    return Err(RelayError::Invalid("conflicting function schemas"));
                }
                let mut function = json!({"name":string(&f["name"])?});
                if let Some(d) = f.get("description") {
                    function["description"] = json!(string(d)?);
                }
                if let Some(p) = f.get("parametersJsonSchema") {
                    function["parameters"] = p.clone();
                }
                if let Some(p) = f.get("parameters") {
                    function["parameters"] = schema(p)?;
                }
                tools.push(json!({"type":"function","function":function}));
            }
        }
    }
    canonical["tools"] = json!(tools);
    if let Some(t) = v.get("toolConfig") {
        keys(t, &["functionCallingConfig"])?;
        let f = &t["functionCallingConfig"];
        keys(f, &["mode", "allowedFunctionNames"])?;
        let mode = f.get("mode").map(string).transpose()?.unwrap_or("AUTO");
        canonical["tool_choice"] = match mode {
            "AUTO" => json!("auto"),
            "NONE" => json!("none"),
            "ANY" => json!("required"),
            _ => return Err(RelayError::Unsupported("unsupported function calling mode")),
        };
        if let Some(names) = f.get("allowedFunctionNames") {
            let names = array(names)?;
            if mode != "ANY" || names.len() != 1 {
                return Err(RelayError::Unsupported(
                    "only a single named function choice is supported",
                ));
            }
            canonical["tool_choice"] =
                json!({"type":"function","function":{"name":string(&names[0])?}});
        }
    }
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|_| RelayError::Invalid("Gemini conversion failed"))?;
    let mut request = Request::parse(ClientProtocol::Chat, &bytes, limits)?;
    request.continuation_token = continuation_token(v.get("continuationToken"), limits)?;
    Ok(request)
}
fn continuation_token(v: Option<&Value>, limits: &Limits) -> Result<Option<String>, RelayError> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let token = string(v)?;
    if token.is_empty() {
        return Ok(None);
    }
    if token.len() > limits.event_bytes || token.chars().any(char::is_control) {
        return Err(RelayError::Invalid("invalid Gemini continuation token"));
    }
    Ok(Some(token.to_owned()))
}

struct Block {
    kind: BlockKind,
    text: String,
    signature: Option<String>,
    ended: bool,
}
pub(crate) struct NativeEncoder {
    model: String,
    id: String,
    stream: bool,
    limits: Limits,
    blocks: Vec<Block>,
    usage: Usage,
    finish: Option<FinishReason>,
    continuation_token: Option<String>,
    started: bool,
    done: bool,
    emitted: bool,
    bytes: usize,
    wire_bytes: usize,
}
impl NativeEncoder {
    pub(crate) fn new(model: &str, request_id: &str, stream: bool, limits: &Limits) -> Self {
        Self {
            model: model.into(),
            id: request_id.into(),
            stream,
            limits: limits.clone(),
            blocks: Vec::new(),
            usage: Usage::default(),
            finish: None,
            continuation_token: None,
            started: false,
            done: false,
            emitted: false,
            bytes: 0,
            wire_bytes: 0,
        }
    }
    pub(crate) fn usage(&self) -> Usage {
        self.usage
    }
    pub(crate) fn finish_reason(&self) -> Option<FinishReason> {
        self.finish
    }
    fn account(&mut self, n: usize) -> Result<(), RelayError> {
        self.bytes = self
            .bytes
            .checked_add(n)
            .ok_or(RelayError::Limit("Gemini output"))?;
        if self.bytes > self.limits.output_bytes {
            return Err(RelayError::Limit("Gemini output"));
        }
        Ok(())
    }
    fn packet(&mut self, value: &Value) -> Result<Vec<u8>, RelayError> {
        let data = serde_json::to_vec(value)
            .map_err(|_| RelayError::Protocol("Gemini encoding failed"))?;
        self.wire_bytes = self
            .wire_bytes
            .checked_add(data.len().saturating_add(8))
            .ok_or(RelayError::Limit("encoded Gemini output"))?;
        if self.wire_bytes
            > self
                .limits
                .output_bytes
                .saturating_mul(8)
                .saturating_add(self.limits.max_blocks.saturating_mul(4096))
        {
            return Err(RelayError::Limit("encoded Gemini output"));
        }
        if self.stream {
            let mut packet = b"data: ".to_vec();
            packet.extend(data);
            packet.extend_from_slice(b"\n\n");
            Ok(packet)
        } else {
            Ok(data)
        }
    }
    fn part(b: &Block) -> Result<Value, RelayError> {
        let mut p = match &b.kind {
            BlockKind::Text => json!({"text":b.text}),
            BlockKind::Thinking => json!({"text":b.text,"thought":true}),
            BlockKind::Tool { id, name } => {
                let args = serde_json::from_str::<Unique>(&b.text)
                    .map_err(|_| RelayError::Protocol("incomplete function arguments"))?
                    .0;
                if !args.is_object() {
                    return Err(RelayError::Protocol("function arguments must be object"));
                }
                json!({"functionCall":{"id":id,"name":name,"args":args}})
            }
            BlockKind::Refusal => return Err(RelayError::Unsupported("Gemini refusal text")),
        };
        if let Some(s) = &b.signature {
            p["thoughtSignature"] = json!(s);
        }
        Ok(p)
    }
    pub(crate) fn consume(&mut self, event: &Event) -> Result<Vec<Vec<u8>>, RelayError> {
        if self.done {
            return Err(RelayError::Protocol("event after Gemini completion"));
        }
        let mut output_part = None;
        match event {
            Event::Start(_) if !self.started => self.started = true,
            Event::Start(_) => return Err(RelayError::Protocol("duplicate Gemini start")),
            Event::Block { index, kind } => {
                if !self.started || self.finish.is_some() || *index != self.blocks.len() {
                    return Err(RelayError::Protocol("invalid Gemini block order"));
                }
                if self.blocks.len() >= self.limits.max_blocks {
                    return Err(RelayError::Limit("Gemini blocks"));
                }
                if matches!(kind, BlockKind::Refusal) {
                    return Err(RelayError::Unsupported("Gemini has no refusal text part"));
                }
                if let BlockKind::Tool { id, name } = kind {
                    self.account(id.len().saturating_add(name.len()))?;
                }
                self.blocks.push(Block {
                    kind: kind.clone(),
                    text: String::new(),
                    signature: None,
                    ended: false,
                });
            }
            Event::Delta { index, text } => {
                self.account(text.len())?;
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown Gemini block"))?;
                if b.ended || self.finish.is_some() {
                    return Err(RelayError::Protocol("delta after Gemini block end"));
                }
                if matches!(b.kind, BlockKind::Tool { .. })
                    && b.text.len().saturating_add(text.len()) > self.limits.tool_argument_bytes
                {
                    return Err(RelayError::Limit("tool arguments"));
                }
                b.text.push_str(text);
                if self.stream && matches!(b.kind, BlockKind::Text) && !text.is_empty() {
                    output_part = Some(json!({"text":text}));
                }
            }
            Event::Signature {
                index,
                provider,
                value,
            } => {
                if *provider != UpstreamProtocol::Gemini {
                    return Err(RelayError::Unsupported(
                        "foreign signature cannot be emitted as Gemini",
                    ));
                }
                self.account(value.len())?;
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown signed Gemini block"))?;
                if b.ended || b.signature.is_some() || value.is_empty() {
                    return Err(RelayError::Protocol("invalid Gemini signature"));
                }
                if self.stream && matches!(b.kind, BlockKind::Text) {
                    return Err(RelayError::Unsupported(
                        "streaming ordinary text signatures cannot be preserved",
                    ));
                }
                b.signature = Some(value.clone());
            }
            Event::EndBlock(index) => {
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown Gemini block"))?;
                if b.ended {
                    return Err(RelayError::Protocol("duplicate Gemini block end"));
                }
                b.ended = true;
                // Tool arguments and thinking wait for the complete signed Part;
                // ordinary text is emitted immediately by the Delta branch.
                if self.stream && !matches!(b.kind, BlockKind::Text) {
                    output_part = Some(Self::part(b)?);
                }
            }
            Event::Usage(u) => {
                if !self.started {
                    return Err(RelayError::Protocol("usage before Gemini start"));
                }
                self.usage.merge(*u)?;
            }
            Event::StopSequence(sequence) => {
                if !self.started
                    || self.finish.is_some()
                    || sequence.is_empty()
                    || sequence.len() > 1024
                {
                    return Err(RelayError::Protocol("invalid stop sequence receipt"));
                }
                // Gemini reports STOP for a matched sequence; its native
                // response does not expose Anthropic's stop_sequence field.
                self.account(sequence.len())?;
            }
            Event::ContinuationToken(token) => {
                if !self.started
                    || token.is_empty()
                    || token.len() > self.limits.event_bytes
                    || token.chars().any(char::is_control)
                {
                    return Err(RelayError::Protocol("invalid Gemini continuation token"));
                }
                match &self.continuation_token {
                    Some(existing) if existing != token => {
                        return Err(RelayError::Protocol(
                            "conflicting Gemini continuation token",
                        ));
                    }
                    Some(_) => {}
                    None => {
                        self.account(token.len())?;
                        self.continuation_token = Some(token.clone());
                    }
                }
            }
            Event::Finish(r) => {
                if !self.started || self.finish.is_some() {
                    return Err(RelayError::Protocol("invalid Gemini finish"));
                }
                self.finish = Some(*r);
            }
            Event::Done => {
                if !self.started || self.finish.is_none() || self.blocks.iter().any(|b| !b.ended) {
                    return Err(RelayError::Truncated);
                }
                self.done = true;
            }
        }
        if let Some(part) = output_part {
            let value = json!({"responseId":self.id,"modelVersion":self.model,"candidates":[{"index":0,"content":{"role":"model","parts":[part]}}]});
            return Ok(vec![self.packet(&value)?]);
        }
        Ok(Vec::new())
    }
    pub(crate) fn terminal(&mut self) -> Result<Vec<Vec<u8>>, RelayError> {
        if !self.done || self.emitted {
            return Err(RelayError::Truncated);
        }
        let finish = self.finish.ok_or(RelayError::Truncated)?;
        if finish == FinishReason::ToolCalls
            && !self
                .blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Tool { .. }))
        {
            return Err(RelayError::Protocol("tool finish without tools"));
        }
        let mut parts = Vec::new();
        for b in &self.blocks {
            let part = Self::part(b)?;
            if !self.stream {
                parts.push(part);
            }
        }
        self.usage.validate()?;
        if self
            .usage
            .cache_creation_input_tokens
            .is_some_and(|v| v != 0)
            || self.usage.cache_creation_details.is_some_and(|details| {
                details.ephemeral_5m_input_tokens.is_some_and(|v| v != 0)
                    || details.ephemeral_1h_input_tokens.is_some_and(|v| v != 0)
            })
        {
            return Err(RelayError::Unsupported("Gemini cache write usage"));
        }
        let mut usage = Map::new();
        for (key, value) in [
            ("promptTokenCount", self.usage.input_tokens),
            ("cachedContentTokenCount", self.usage.cached_input_tokens),
            ("thoughtsTokenCount", self.usage.reasoning_output_tokens),
        ] {
            if let Some(n) = value {
                usage.insert(key.into(), json!(n));
            }
        }
        // Canonical output includes reasoning. Missing reasoning is unknown,
        // never evidence that candidatesTokenCount equals the output total.
        if let (Some(o), Some(r)) = (self.usage.output_tokens, self.usage.reasoning_output_tokens) {
            usage.insert(
                "candidatesTokenCount".into(),
                json!(
                    o.checked_sub(r)
                        .ok_or(RelayError::Protocol("invalid reasoning usage"))?
                ),
            );
        }
        if let (Some(i), Some(o)) = (self.usage.input_tokens, self.usage.output_tokens) {
            usage.insert(
                "totalTokenCount".into(),
                json!(
                    i.checked_add(o)
                        .ok_or(RelayError::Protocol("usage overflow"))?
                ),
            );
        }
        let finish_reason = match finish {
            FinishReason::Stop | FinishReason::ToolCalls => "STOP",
            FinishReason::Length => "MAX_TOKENS",
            FinishReason::Continuation => "CONTINUATION",
            FinishReason::ContentFilter => "SAFETY",
        };
        let mut candidate = json!({"index":0,"content":{"role":"model","parts":parts},"finishReason":finish_reason});
        if let Some(token) = &self.continuation_token {
            candidate["continuationToken"] = json!(token);
        }
        let mut response =
            json!({"responseId":self.id,"modelVersion":self.model,"candidates":[candidate]});
        if !usage.is_empty() {
            response["usageMetadata"] = Value::Object(usage);
        }
        let data = self.packet(&response)?;
        self.emitted = true;
        Ok(vec![data])
    }
    pub(crate) fn error_packet(&self, error: &RelayError) -> Result<Vec<u8>, RelayError> {
        let status = match error.status() {
            400 => "INVALID_ARGUMENT",
            401 => "UNAUTHENTICATED",
            402 | 403 => "PERMISSION_DENIED",
            409 => "ABORTED",
            413 | 429 => "RESOURCE_EXHAUSTED",
            499 => "CANCELLED",
            503 => "UNAVAILABLE",
            504 => "DEADLINE_EXCEEDED",
            _ => "INTERNAL",
        };
        let data = serde_json::to_vec(
            &json!({"error":{"code":error.status(),"message":error.to_string(),"status":status}}),
        )
        .map_err(|_| RelayError::Protocol("Gemini error encoding failed"))?;
        if self.stream {
            let mut packet = b"data: ".to_vec();
            packet.extend(data);
            packet.extend_from_slice(b"\n\n");
            Ok(packet)
        } else {
            Ok(data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gemini_native_rejects_nonzero_cache_creation_details_without_aggregate() {
        for details in [
            super::super::CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(1),
                ephemeral_1h_input_tokens: None,
            },
            super::super::CacheCreationDetails {
                ephemeral_5m_input_tokens: None,
                ephemeral_1h_input_tokens: Some(1),
            },
        ] {
            let mut encoder = NativeEncoder::new("m", "req", false, &Limits::default());
            for event in [
                Event::Start("up".into()),
                Event::Usage(Usage {
                    input_tokens: Some(2),
                    cache_creation_details: Some(details),
                    ..Usage::default()
                }),
                Event::Finish(FinishReason::Stop),
                Event::Done,
            ] {
                encoder.consume(&event).unwrap();
            }
            assert_eq!(
                encoder.terminal(),
                Err(RelayError::Unsupported("Gemini cache write usage"))
            );
        }
    }
    #[test]
    fn gemini_native_rejects_nested_duplicates_unknown_and_media() {
        for body in [
            r#"{"contents":[{"parts":[{"text":"a","text":"b"}]}]}"#,
            r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":"a"}}]}]}"#,
            r#"{"contents":[{"parts":[{"text":"a"}]}],"safetySettings":[]}"#,
        ] {
            assert!(parse("m", false, body.as_bytes(), &Limits::default()).is_err());
        }
    }
    #[test]
    fn gemini_native_preserves_signed_thought_calls_and_function_response() {
        let body = br#"{"contents":[{"role":"model","parts":[{"text":"plan","thought":true,"thoughtSignature":"sig"},{"functionCall":{"name":"weather","args":{"city":"Taipei"}},"thoughtSignature":"call-sig"}]},{"role":"user","parts":[{"functionResponse":{"name":"weather","response":{"ok":true}}}]}]}"#;
        let r = parse("m", true, body, &Limits::default()).unwrap();
        assert_eq!(r.messages.len(), 3);
        assert!(
            matches!(&r.messages[0].content[0],super::super::InputBlock::Thinking {signature:Some((UpstreamProtocol::Gemini,s)),..} if s=="sig")
        );
        assert!(
            matches!(&r.messages[1].content[0],super::super::InputBlock::ToolCall {signature:Some((UpstreamProtocol::Gemini,s)),..} if s=="call-sig")
        );
        assert!(r.upstream_to(UpstreamProtocol::Gemini, "m").is_ok());
        assert!(r.upstream_to(UpstreamProtocol::Chat, "m").is_err());
    }
    #[test]
    fn gemini_native_rejects_ambiguous_function_response_and_signed_text() {
        for body in [
            r#"{"contents":[{"role":"model","parts":[{"functionCall":{"name":"f","args":{}}},{"functionCall":{"name":"f","args":{}}}]},{"parts":[{"functionResponse":{"name":"f","response":{}}}]}]}"#,
            r#"{"contents":[{"role":"model","parts":[{"text":"a","thoughtSignature":"sig"}]}]}"#,
        ] {
            assert!(parse("m", false, body.as_bytes(), &Limits::default()).is_err());
        }
    }
    #[test]
    fn gemini_native_maps_tools_sampling_and_limits() {
        let body=br#"{"contents":[{"parts":[{"text":"a"}]}],"tools":[{"functionDeclarations":[{"name":"f","parameters":{"type":"OBJECT","properties":{"x":{"type":"STRING"}}}}]}],"toolConfig":{"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":["f"]}},"generationConfig":{"maxOutputTokens":8,"temperature":0.5,"topP":0.8,"stopSequences":["end"]}}"#;
        let r = parse("m", false, body, &Limits::default()).unwrap();
        assert_eq!(r.max_output_tokens, 8);
        assert_eq!(r.tools[0].parameters["properties"]["x"]["type"], "string");
        let limits = Limits {
            request_bytes: 10,
            ..Limits::default()
        };
        assert!(matches!(
            parse("m", false, body, &limits),
            Err(RelayError::Limit(_))
        ));
    }
    #[test]
    fn gemini_native_terminal_requires_finish_and_preserves_usage_unknowns() {
        let mut e = NativeEncoder::new("m", "id", false, &Limits::default());
        assert!(e.terminal().is_err());
        for event in [
            Event::Start("up".into()),
            Event::Block {
                index: 0,
                kind: BlockKind::Text,
            },
            Event::Delta {
                index: 0,
                text: "answer".into(),
            },
            Event::Signature {
                index: 0,
                provider: UpstreamProtocol::Gemini,
                value: "sig".into(),
            },
            Event::EndBlock(0),
            Event::Usage(Usage {
                input_tokens: Some(3),
                output_tokens: Some(7),
                ..Usage::default()
            }),
            Event::Finish(FinishReason::Stop),
            Event::Done,
        ] {
            assert!(e.consume(&event).unwrap().is_empty());
        }
        let packets = e.terminal().unwrap();
        let s = std::str::from_utf8(&packets[0]).unwrap();
        let v: Value = serde_json::from_str(s).unwrap();
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["thoughtSignature"],
            "sig"
        );
        assert_eq!(v["usageMetadata"]["totalTokenCount"], 10);
        assert!(v["usageMetadata"].get("candidatesTokenCount").is_none());
        assert!(v["usageMetadata"].get("cachedContentTokenCount").is_none());
        assert!(e.terminal().is_err());
    }
    #[test]
    fn gemini_native_rejects_foreign_signature_and_partial_tool_json() {
        let mut e = NativeEncoder::new("m", "id", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Tool {
                id: "call".into(),
                name: "f".into(),
            },
        })
        .unwrap();
        assert!(
            e.consume(&Event::Signature {
                index: 0,
                provider: UpstreamProtocol::Anthropic,
                value: "s".into()
            })
            .is_err()
        );
        for event in [
            Event::Delta {
                index: 0,
                text: "{".into(),
            },
            Event::EndBlock(0),
            Event::Finish(FinishReason::Length),
            Event::Done,
        ] {
            e.consume(&event).unwrap();
        }
        assert!(e.terminal().is_err());
    }
    #[test]
    fn gemini_native_streams_text_before_terminal_and_buffers_signed_tools() {
        let mut e = NativeEncoder::new("m", "id", true, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Text,
        })
        .unwrap();
        let text = e
            .consume(&Event::Delta {
                index: 0,
                text: "live".into(),
            })
            .unwrap();
        assert_eq!(text.len(), 1);
        assert!(std::str::from_utf8(&text[0]).unwrap().contains("live"));
        assert!(e.terminal().is_err());
        e.consume(&Event::EndBlock(0)).unwrap();
        e.consume(&Event::Block {
            index: 1,
            kind: BlockKind::Tool {
                id: "c".into(),
                name: "f".into(),
            },
        })
        .unwrap();
        assert!(
            e.consume(&Event::Delta {
                index: 1,
                text: "{}".into()
            })
            .unwrap()
            .is_empty()
        );
        e.consume(&Event::Signature {
            index: 1,
            provider: UpstreamProtocol::Gemini,
            value: "sig".into(),
        })
        .unwrap();
        let tool = e.consume(&Event::EndBlock(1)).unwrap();
        assert_eq!(tool.len(), 1);
        assert!(
            std::str::from_utf8(&tool[0])
                .unwrap()
                .contains("thoughtSignature")
        );
        e.consume(&Event::Finish(FinishReason::ToolCalls)).unwrap();
        e.consume(&Event::Done).unwrap();
        let end = e.terminal().unwrap();
        assert!(
            !std::str::from_utf8(&end[0])
                .unwrap()
                .contains("functionCall")
        );
    }
    #[test]
    fn gemini_native_error_envelope_is_native_sse() {
        let e = NativeEncoder::new("m", "id", true, &Limits::default());
        let bytes = e.error_packet(&RelayError::Unauthorized).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("data: "));
        assert!(text.ends_with("\n\n"));
        assert!(text.contains("UNAUTHENTICATED"));
    }
    #[test]
    fn gemini_request_forwards_continuation_token_and_omits_empty() {
        let with =
            br#"{"contents":[{"parts":[{"text":"go"}]}],"continuationToken":"CiQ4ZmQ3-token"}"#;
        let request = parse("m", true, with, &Limits::default()).unwrap();
        assert_eq!(
            request.continuation_token.as_deref(),
            Some("CiQ4ZmQ3-token")
        );
        let upstream = request.upstream_to(UpstreamProtocol::Gemini, "m").unwrap();
        assert_eq!(upstream["continuationToken"], "CiQ4ZmQ3-token");
        assert!(request.upstream_to(UpstreamProtocol::Chat, "m").is_err());
        for body in [
            br#"{"contents":[{"parts":[{"text":"go"}]}]}"#.as_slice(),
            br#"{"contents":[{"parts":[{"text":"go"}]}],"continuationToken":""}"#,
            br#"{"contents":[{"parts":[{"text":"go"}]}],"continuationToken":null}"#,
        ] {
            let request = parse("m", false, body, &Limits::default()).unwrap();
            assert!(request.continuation_token.is_none());
            let upstream = request.upstream_to(UpstreamProtocol::Gemini, "m").unwrap();
            assert!(upstream.get("continuationToken").is_none());
        }
        assert!(
            parse(
                "m",
                false,
                br#"{"contents":[{"parts":[{"text":"go"}]}],"continuationToken":1}"#,
                &Limits::default()
            )
            .is_err()
        );
    }
}
