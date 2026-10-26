//! Bounded Anthropic Messages client adapter. No media, hosted tools or cache controls.
use super::request::{InputBlock, Message, Request, Role};
use super::{
    BlockKind, ClientProtocol, Event, FinishReason, Limits, RelayError, UpstreamProtocol, Usage,
};
use serde::{
    Deserialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

// Value alone silently overwrites duplicate keys, including inside tool arguments.
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
                let mut m = Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if m.contains_key(&k) {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                    m.insert(k, a.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(m)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element::<Unique>()? {
                    v.push(x.0);
                }
                Ok(Unique(Value::Array(v)))
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
fn keys(v: &Value, allowed: &[&str]) -> Result<(), RelayError> {
    let m = v
        .as_object()
        .ok_or(RelayError::Invalid("expected object"))?;
    if m.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(RelayError::Unsupported("unsupported Anthropic field"));
    }
    Ok(())
}
fn string(v: &Value) -> Result<String, RelayError> {
    v.as_str()
        .map(str::to_owned)
        .ok_or(RelayError::Invalid("expected string"))
}
fn ident(v: &Value) -> Result<String, RelayError> {
    let s = string(v)?;
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(RelayError::Invalid("invalid tool identity"));
    }
    Ok(s)
}
fn blocks(v: &Value, role: Role, limits: &Limits) -> Result<Vec<InputBlock>, RelayError> {
    if let Some(s) = v.as_str() {
        return Ok(vec![InputBlock::Text(s.into())]);
    }
    let parts = v
        .as_array()
        .ok_or(RelayError::Invalid("invalid message content"))?;
    let mut out = Vec::new();
    for p in parts {
        out.push(match p["type"].as_str() {
            Some("text") => {
                keys(p, &["type", "text"])?;
                InputBlock::Text(string(&p["text"])?)
            }
            Some("thinking") if role == Role::Assistant => {
                keys(p, &["type", "thinking", "signature"])?;
                let s = string(&p["signature"])?;
                if s.is_empty() {
                    return Err(RelayError::Invalid("empty thinking signature"));
                }
                InputBlock::Thinking {
                    text: string(&p["thinking"])?,
                    signature: Some((UpstreamProtocol::Anthropic, s)),
                }
            }
            Some("tool_use") if role == Role::Assistant => {
                keys(p, &["type", "id", "name", "input"])?;
                if !p["input"].is_object() {
                    return Err(RelayError::Invalid("tool input must be object"));
                }
                let arguments = p["input"].to_string();
                if arguments.len() > limits.tool_argument_bytes {
                    return Err(RelayError::Limit("tool arguments"));
                }
                InputBlock::ToolCall {
                    id: ident(&p["id"])?,
                    name: ident(&p["name"])?,
                    arguments,
                    signature: None,
                }
            }
            Some("tool_result") if role == Role::User => {
                keys(p, &["type", "tool_use_id", "content"])?;
                let content = if p["content"].is_string() {
                    string(&p["content"])?
                } else {
                    let parts = blocks(&p["content"], Role::System, limits)?;
                    if parts.len() != 1 {
                        return Err(RelayError::Unsupported("multi-block tool result"));
                    }
                    match &parts[0] {
                        InputBlock::Text(s) => s.clone(),
                        _ => return Err(RelayError::Unsupported("non-text tool result")),
                    }
                };
                InputBlock::ToolResult {
                    id: ident(&p["tool_use_id"])?,
                    content,
                }
            }
            _ => {
                return Err(RelayError::Unsupported(
                    "unsupported Anthropic content block",
                ));
            }
        });
        if out.len() > limits.max_blocks {
            return Err(RelayError::Limit("input blocks"));
        }
    }
    if out.is_empty() {
        return Err(RelayError::Invalid("empty content"));
    }
    Ok(out)
}
pub(crate) fn parse(body: &[u8], limits: &Limits) -> Result<Request, RelayError> {
    limits.validate()?;
    if body.len() > limits.request_bytes {
        return Err(RelayError::Limit("request body"));
    }
    let v = serde_json::from_slice::<Unique>(body)
        .map_err(|_| RelayError::Invalid("invalid or ambiguous JSON"))?
        .0;
    keys(
        &v,
        &[
            "model",
            "messages",
            "system",
            "max_tokens",
            "stream",
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "stop_sequences",
        ],
    )?;
    if let Some(s) = v.get("stop_sequences") {
        let sequences = s
            .as_array()
            .ok_or(RelayError::Invalid("invalid stop sequences"))?;
        if sequences.is_empty()
            || sequences.len() > 4
            || sequences
                .iter()
                .any(|s| s.as_str().is_none_or(|s| s.is_empty() || s.len() > 1024))
        {
            return Err(RelayError::Invalid("invalid stop sequences"));
        }
    }
    let mut canonical = json!({"model":v["model"],"max_tokens":v["max_tokens"],"messages":[{"role":"user","content":"validation"}]});
    if let Some(sequences) = v.get("stop_sequences") {
        canonical["stop"] = sequences.clone();
    }
    for k in ["stream", "temperature", "top_p"] {
        if let Some(x) = v.get(k) {
            canonical[k] = x.clone();
        }
    }
    if v["max_tokens"].as_u64().is_none() {
        return Err(RelayError::Invalid("max_tokens is required"));
    }
    if v.get("temperature")
        .is_some_and(|x| x.as_f64().is_none_or(|x| x > 1.0))
    {
        return Err(RelayError::Invalid("invalid Anthropic temperature"));
    }
    if let Some(tools) = v.get("tools") {
        let mut dst = Vec::new();
        for t in tools
            .as_array()
            .ok_or(RelayError::Invalid("invalid tools"))?
        {
            keys(t, &["name", "description", "input_schema"])?;
            if !t["input_schema"].is_object() {
                return Err(RelayError::Invalid("tool schema must be object"));
            }
            let mut f = json!({"name":t["name"],"parameters":t["input_schema"]});
            if let Some(d) = t.get("description") {
                f["description"] = d.clone();
            }
            dst.push(json!({"type":"function","function":f}));
        }
        canonical["tools"] = json!(dst);
    }
    if let Some(t) = v.get("tool_choice") {
        keys(t, &["type", "name", "disable_parallel_tool_use"])?;
        canonical["tool_choice"] = match t["type"].as_str() {
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            Some("any") => json!("required"),
            Some("tool") => json!({"type":"function","function":{"name":t["name"]}}),
            _ => return Err(RelayError::Unsupported("invalid tool choice")),
        };
        if t["type"] != "tool" && t.get("name").is_some() {
            return Err(RelayError::Invalid("unexpected tool name"));
        }
        if let Some(p) = t.get("disable_parallel_tool_use") {
            canonical["parallel_tool_calls"] = json!(
                !p.as_bool()
                    .ok_or(RelayError::Invalid("invalid tool parallelism"))?
            );
        }
    }
    let mut request = Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&canonical).map_err(|_| RelayError::Invalid("request encoding"))?,
        limits,
    )?;
    let mut messages = Vec::new();
    if let Some(s) = v.get("system") {
        messages.push(Message {
            role: Role::System,
            content: blocks(s, Role::System, limits)?,
        });
    }
    for m in v["messages"]
        .as_array()
        .ok_or(RelayError::Invalid("messages required"))?
    {
        keys(m, &["role", "content"])?;
        let role = match m["role"].as_str() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => return Err(RelayError::Invalid("invalid message role")),
        };
        messages.push(Message {
            role,
            content: blocks(&m["content"], role, limits)?,
        });
    }
    if messages.is_empty()
        || messages.iter().all(|m| m.role == Role::System)
        || messages.len() > 1024
    {
        return Err(RelayError::Invalid("invalid message count"));
    }
    let (mut calls, mut results, mut count) = (BTreeSet::new(), BTreeSet::new(), 0usize);
    for m in &messages {
        for b in &m.content {
            count += 1;
            match b {
                InputBlock::ToolCall { id, .. } => {
                    if !calls.insert(id) {
                        return Err(RelayError::Invalid("duplicate call ID"));
                    }
                }
                InputBlock::ToolResult { id, .. } if !calls.contains(id) || !results.insert(id) => {
                    return Err(RelayError::Invalid("orphan or duplicate tool result"));
                }
                _ => {}
            }
        }
    }
    if count > limits.max_blocks {
        return Err(RelayError::Limit("input blocks"));
    }
    request.messages = messages;
    Ok(request)
}

struct Block {
    kind: BlockKind,
    text: String,
    signature: String,
    ended: bool,
}
pub(crate) struct NativeEncoder {
    model: String,
    id: String,
    stream: bool,
    started: bool,
    done: bool,
    terminal_sent: bool,
    stop_sequence: Option<String>,
    wire_started: bool,
    pending: Vec<Vec<u8>>,
    blocks: Vec<Block>,
    usage: Usage,
    finish: Option<FinishReason>,
    bytes: usize,
    wire_bytes: usize,
    limits: Limits,
}
impl NativeEncoder {
    pub fn new(model: &str, request_id: &str, stream: bool, limits: &Limits) -> Self {
        Self {
            model: model.into(),
            id: format!("msg_{request_id}"),
            stream,
            started: false,
            done: false,
            terminal_sent: false,
            stop_sequence: None,
            wire_started: false,
            pending: Vec::new(),
            blocks: Vec::new(),
            usage: Usage::default(),
            finish: None,
            bytes: 0,
            wire_bytes: 0,
            limits: limits.clone(),
        }
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.finish
    }
    fn account(&mut self, n: usize) -> Result<(), RelayError> {
        self.bytes = self
            .bytes
            .checked_add(n)
            .ok_or(RelayError::Limit("output bytes"))?;
        if self.bytes > self.limits.output_bytes {
            return Err(RelayError::Limit("output bytes"));
        }
        Ok(())
    }
    fn packet(&mut self, kind: &str, mut value: Value) -> Result<Vec<u8>, RelayError> {
        value["type"] = json!(kind);
        let data = serde_json::to_vec(&value).map_err(|_| RelayError::Protocol("encoding"))?;
        if data.len() > self.limits.event_bytes {
            return Err(RelayError::Limit("encoded event"));
        }
        let p = format!(
            "event: {kind}\ndata: {}\n\n",
            String::from_utf8(data).map_err(|_| RelayError::Protocol("encoding"))?
        )
        .into_bytes();
        self.wire_bytes = self
            .wire_bytes
            .checked_add(p.len())
            .ok_or(RelayError::Limit("encoded output"))?;
        if self.wire_bytes > self.limits.output_bytes {
            return Err(RelayError::Limit("encoded output"));
        }
        Ok(p)
    }
    fn native_usage(&self) -> Result<Value, RelayError> {
        self.usage.validate()?;
        if self.usage.cache_creation_mode
            == Some(crate::routing::CacheCreationMode::GenericRemainderV1)
        {
            let input = self
                .usage
                .uncached_input_tokens
                .ok_or(RelayError::Protocol("missing uncached input usage"))?;
            let output = self
                .usage
                .output_tokens
                .ok_or(RelayError::Protocol("missing output usage"))?;
            let mut value = json!({"input_tokens":input,"output_tokens":output});
            if let Some(read) = self.usage.cached_input_tokens {
                value["cache_read_input_tokens"] = json!(read);
            }
            if let Some(write) = self.usage.cache_creation_input_tokens {
                value["cache_creation_input_tokens"] = json!(write);
            }
            if let Some(details) = self.usage.cache_creation_details {
                value["cache_creation"] = json!(details);
            }
            return Ok(value);
        }
        let input = self
            .usage
            .input_tokens
            .ok_or(RelayError::Protocol("missing input usage"))?;
        let read = self
            .usage
            .cached_input_tokens
            .ok_or(RelayError::Protocol("missing cache read usage"))?;
        let write = self
            .usage
            .cache_creation_input_tokens
            .ok_or(RelayError::Protocol("missing cache creation usage"))?;
        let effective_write = self
            .usage
            .effective_cache_creation_input_tokens()?
            .ok_or(RelayError::Protocol("missing cache creation usage"))?;
        let uncached = input
            .checked_sub(read)
            .and_then(|n| n.checked_sub(effective_write))
            .ok_or(RelayError::Protocol("invalid cache usage"))?;
        let mut value = json!({"input_tokens":uncached,"output_tokens":self.usage.output_tokens.ok_or(RelayError::Protocol("missing output usage"))?,"cache_read_input_tokens":read,"cache_creation_input_tokens":write});
        if let Some(details) = self.usage.cache_creation_details {
            value["cache_creation"] = json!(details);
        }
        Ok(value)
    }
    fn content(&self, i: usize, empty: bool) -> Result<Value, RelayError> {
        let b = &self.blocks[i];
        let s = if empty { "" } else { &b.text };
        Ok(match &b.kind {
            BlockKind::Text => json!({"type":"text","text":s}),
            BlockKind::Thinking => {
                json!({"type":"thinking","thinking":s,"signature":if empty{""}else{&b.signature}})
            }
            BlockKind::Tool { id, name } => {
                json!({"type":"tool_use","id":id,"name":name,"input":if empty{json!({})}else{serde_json::from_str::<Unique>(s).map_err(|_|RelayError::Protocol("invalid tool JSON"))?.0}})
            }
            BlockKind::Refusal => return Err(RelayError::Unsupported("native refusal block")),
        })
    }
    pub fn consume(&mut self, event: &Event) -> Result<Vec<Vec<u8>>, RelayError> {
        if self.done {
            return Err(RelayError::Protocol("event after done"));
        }
        let mut out = Vec::new();
        match event {
            Event::Start(_) => {
                if self.started {
                    return Err(RelayError::Protocol("duplicate start"));
                }
                self.started = true;
            }
            _ if !self.started => return Err(RelayError::Protocol("event before start")),
            Event::Block { index, kind } => {
                if self.finish.is_some()
                    || *index != self.blocks.len()
                    || self.blocks.len() >= self.limits.max_blocks
                    || self.blocks.iter().any(|b| !b.ended)
                {
                    return Err(RelayError::Protocol("invalid block order"));
                }
                if matches!(kind, BlockKind::Refusal) {
                    return Err(RelayError::Unsupported("native refusal block"));
                }
                if let BlockKind::Tool { id, name } = kind {
                    ident(&json!(id))?;
                    ident(&json!(name))?;
                    if self
                        .blocks
                        .iter()
                        .any(|b| matches!(&b.kind,BlockKind::Tool{id:old,..} if old==id))
                    {
                        return Err(RelayError::Protocol("duplicate tool ID"));
                    }
                    self.account(id.len() + name.len())?;
                }
                self.blocks.push(Block {
                    kind: kind.clone(),
                    text: String::new(),
                    signature: String::new(),
                    ended: false,
                });
                if self.stream {
                    out.push(self.packet(
                        "content_block_start",
                        json!({"index":index,"content_block":self.content(*index,true)?}),
                    )?);
                }
            }
            Event::Delta { index, text } => {
                self.account(text.len())?;
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown block"))?;
                if b.ended || self.finish.is_some() || !b.signature.is_empty() {
                    return Err(RelayError::Protocol("late delta"));
                }
                if matches!(b.kind, BlockKind::Tool { .. })
                    && b.text.len().saturating_add(text.len()) > self.limits.tool_argument_bytes
                {
                    return Err(RelayError::Limit("tool JSON"));
                }
                b.text.push_str(text);
                let d = match b.kind {
                    BlockKind::Text => json!({"type":"text_delta","text":text}),
                    BlockKind::Thinking => json!({"type":"thinking_delta","thinking":text}),
                    BlockKind::Tool { .. } => {
                        json!({"type":"input_json_delta","partial_json":text})
                    }
                    _ => return Err(RelayError::Unsupported("native content")),
                };
                if self.stream {
                    out.push(self.packet("content_block_delta", json!({"index":index,"delta":d}))?);
                }
            }
            Event::Signature {
                index,
                provider,
                value,
            } => {
                if *provider != UpstreamProtocol::Anthropic {
                    return Err(RelayError::Unsupported("foreign thinking signature"));
                }
                self.account(value.len())?;
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown signature block"))?;
                if b.ended || !matches!(b.kind, BlockKind::Thinking) || value.is_empty() {
                    return Err(RelayError::Protocol("invalid signature"));
                }
                b.signature.push_str(value);
                if self.stream {
                    out.push(self.packet(
                        "content_block_delta",
                        json!({"index":index,"delta":{"type":"signature_delta","signature":value}}),
                    )?);
                }
            }
            Event::EndBlock(i) => {
                let b = self
                    .blocks
                    .get_mut(*i)
                    .ok_or(RelayError::Protocol("unknown block"))?;
                if b.ended {
                    return Err(RelayError::Protocol("duplicate block end"));
                }
                if matches!(b.kind, BlockKind::Thinking) && b.signature.is_empty() {
                    return Err(RelayError::Unsupported("unsigned native thinking"));
                }
                if matches!(b.kind, BlockKind::Tool { .. })
                    && !serde_json::from_str::<Unique>(&b.text)
                        .map_err(|_| RelayError::Protocol("invalid tool JSON"))?
                        .0
                        .is_object()
                {
                    return Err(RelayError::Protocol("tool JSON must be object"));
                }
                b.ended = true;
                if self.stream {
                    out.push(self.packet("content_block_stop", json!({"index":i}))?);
                }
            }
            Event::Usage(u) => {
                self.usage.merge(*u)?;
                if self
                    .usage
                    .output_tokens
                    .is_some_and(|n| n > u64::from(self.limits.max_output_tokens))
                {
                    return Err(RelayError::Limit("output tokens"));
                }
            }
            Event::ContinuationToken(token) => {
                // Anthropic has no continuation field. The finish maps to max_tokens.
                if !self.started
                    || token.is_empty()
                    || token.len() > self.limits.event_bytes
                    || token.chars().any(char::is_control)
                {
                    return Err(RelayError::Protocol("invalid Gemini continuation token"));
                }
            }
            Event::StopSequence(sequence) => {
                if self.stop_sequence.is_some()
                    || self.finish.is_some()
                    || sequence.is_empty()
                    || sequence.len() > 1024
                {
                    return Err(RelayError::Protocol("invalid stop sequence receipt"));
                }
                self.account(sequence.len())?;
                self.stop_sequence = Some(sequence.clone());
            }
            Event::Finish(r) => {
                if self.stop_sequence.is_some() && *r != FinishReason::Stop {
                    return Err(RelayError::Protocol("stop sequence requires stop finish"));
                }
                if self.finish.is_some() || self.blocks.iter().any(|b| !b.ended) {
                    return Err(RelayError::Protocol("invalid finish"));
                }
                if *r == FinishReason::ContentFilter {
                    return Err(RelayError::Unsupported("native filter reason"));
                }
                if *r == FinishReason::ToolCalls
                    && !self
                        .blocks
                        .iter()
                        .any(|b| matches!(b.kind, BlockKind::Tool { .. }))
                {
                    return Err(RelayError::Protocol("tool finish mismatch"));
                }
                self.finish = Some(*r);
            }
            Event::Done => {
                if self.finish.is_none() {
                    return Err(RelayError::Truncated);
                }
                self.native_usage()?;
                self.done = true;
            }
        }
        if self.stream && !self.wire_started {
            if let Ok(usage) = self.native_usage() {
                let start = self.packet("message_start", json!({"message":{"id":self.id,"type":"message","role":"assistant","model":self.model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":usage}}))?;
                self.wire_started = true;
                self.pending.extend(out);
                out = vec![start];
                out.append(&mut self.pending);
            } else {
                self.pending.extend(out);
                out = Vec::new();
            }
        }
        Ok(out)
    }
    pub fn terminal(&mut self) -> Result<Vec<Vec<u8>>, RelayError> {
        if !self.done || self.terminal_sent {
            return Err(RelayError::Protocol("invalid terminal boundary"));
        }
        let reason = match self.finish {
            Some(FinishReason::Stop) if self.stop_sequence.is_some() => "stop_sequence",
            Some(FinishReason::Stop) => "end_turn",
            Some(FinishReason::Length | FinishReason::Continuation) => "max_tokens",
            Some(FinishReason::ToolCalls) => "tool_use",
            _ => return Err(RelayError::Protocol("missing finish")),
        };
        let usage = self.native_usage()?;
        let mut out = Vec::new();
        if self.stream {
            out.push(self.packet(
                "message_delta",
                json!({"delta":{"stop_reason":reason,"stop_sequence":self.stop_sequence},"usage":usage}),
            )?);
            out.push(self.packet("message_stop", json!({}))?);
        } else {
            let content = (0..self.blocks.len())
                .map(|i| self.content(i, false))
                .collect::<Result<Vec<_>, _>>()?;
            let v = json!({"id":self.id,"type":"message","role":"assistant","model":self.model,"content":content,"stop_reason":reason,"stop_sequence":self.stop_sequence,"usage":usage});
            let b = serde_json::to_vec(&v).map_err(|_| RelayError::Protocol("encoding"))?;
            if b.len() > self.limits.output_bytes {
                return Err(RelayError::Limit("encoded output"));
            }
            out.push(b);
        }
        self.terminal_sent = true;
        Ok(out)
    }
    pub fn error_packet(&mut self, error: &RelayError) -> Result<Vec<u8>, RelayError> {
        let kind = match error.status() {
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            413 => "request_too_large",
            429 => "rate_limit_error",
            400 => "invalid_request_error",
            _ => "api_error",
        };
        let v = json!({"type":"error","error":{"type":kind,"message":error.to_string()}});
        if self.stream {
            self.packet("error", v)
        } else {
            serde_json::to_vec(&v).map_err(|_| RelayError::Protocol("encoding"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(extra: Value) -> Vec<u8> {
        let mut v = json!({"model":"claude","max_tokens":128,"messages":[{"role":"user","content":"hello"}]});
        for (k, vv) in extra.as_object().unwrap() {
            v[k] = vv.clone();
        }
        serde_json::to_vec(&v).unwrap()
    }
    fn usage() -> Usage {
        Usage {
            input_tokens: Some(20),
            output_tokens: Some(4),
            cached_input_tokens: Some(5),
            cache_creation_input_tokens: Some(3),
            ..Usage::default()
        }
    }
    #[test]
    fn native_anthropic_cache_creation_details_preserve_only_present_dimensions() {
        for details in [
            Some(super::super::CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(1),
                ephemeral_1h_input_tokens: Some(2),
            }),
            Some(super::super::CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(0),
                ephemeral_1h_input_tokens: None,
            }),
            Some(super::super::CacheCreationDetails::default()),
            None,
        ] {
            for stream in [false, true] {
                let mut encoder = NativeEncoder::new("claude", "req", stream, &Limits::default());
                encoder.usage = Usage {
                    cache_creation_details: details,
                    ..usage()
                };
                let wire = encoder.native_usage().unwrap();
                assert_eq!(
                    wire.get("cache_creation").cloned(),
                    details.map(|v| json!(v))
                );
                assert_eq!(wire["input_tokens"], 12);
                assert_eq!(wire["cache_creation_input_tokens"], 3);
            }
        }
    }
    fn finish(e: &mut NativeEncoder, reason: FinishReason) {
        e.consume(&Event::Usage(usage())).unwrap();
        e.consume(&Event::Finish(reason)).unwrap();
        e.consume(&Event::Done).unwrap();
    }
    #[test]
    fn native_anthropic_parse_text_tools_and_signed_history() {
        let r=parse(&request(json!({"tools":[{"name":"lookup","input_schema":{"type":"object"}}],"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"thought","signature":"opaque"},{"type":"tool_use","id":"call_1","name":"lookup","input":{"q":"x"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"result"}]}]})),&Limits::default()).unwrap();
        assert_eq!(r.tools.len(), 1);
        assert!(
            matches!(&r.messages[0].content[0],InputBlock::Thinking{signature:Some((UpstreamProtocol::Anthropic,s)),..} if s=="opaque")
        );
        let wire = r
            .upstream_to(UpstreamProtocol::Anthropic, "claude")
            .unwrap();
        assert_eq!(wire["messages"][0]["content"][0]["signature"], "opaque");
        assert_eq!(wire["messages"][0]["content"][1]["input"]["q"], "x");
    }
    #[test]
    fn native_anthropic_rejects_nested_duplicates_and_unknown_fields() {
        let b=br#"{"model":"claude","max_tokens":10,"messages":[{"role":"user","content":"a","content":"b"}]}"#;
        assert!(parse(b, &Limits::default()).is_err());
        assert!(
            parse(
                &request(json!({"metadata":{"user_id":"x"}})),
                &Limits::default()
            )
            .is_err()
        );
        let b=br#"{"model":"claude","max_tokens":10,"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"f","input":{"q":1,"q":2}}]}]}"#;
        assert!(parse(b, &Limits::default()).is_err());
    }
    #[test]
    fn native_anthropic_rejects_media_servertools_and_lost_semantics() {
        for block in [
            json!({"type":"image","source":{"type":"base64","data":"a"}}),
            json!({"type":"server_tool_use","id":"a","name":"web_search","input":{}}),
            json!({"type":"text","text":"x","cache_control":{"type":"ephemeral"}}),
        ] {
            assert!(
                parse(
                    &request(json!({"messages":[{"role":"user","content":[block]}]})),
                    &Limits::default()
                )
                .is_err()
            );
        }
        let extra = json!({"thinking":{"type":"enabled","budget_tokens":1024}});
        assert!(parse(&request(extra), &Limits::default()).is_err());
        assert!(parse(&request(json!({"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"orphan","content":"x"}]}]})),&Limits::default()).is_err());
    }
    #[test]
    fn native_anthropic_nonstream_usage_cache_split_and_terminal_gate() {
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("upstream".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Text,
        })
        .unwrap();
        e.consume(&Event::Delta {
            index: 0,
            text: "hello".into(),
        })
        .unwrap();
        e.consume(&Event::EndBlock(0)).unwrap();
        assert!(e.terminal().is_err());
        finish(&mut e, FinishReason::Stop);
        let packets = e.terminal().unwrap();
        let v: Value = serde_json::from_slice(&packets[0]).unwrap();
        assert_eq!(v["content"][0]["text"], "hello");
        assert_eq!(v["usage"]["input_tokens"], 12);
        assert_eq!(v["usage"]["cache_read_input_tokens"], 5);
        assert_eq!(v["stop_reason"], "end_turn");
        assert!(e.terminal().is_err());
    }
    #[test]
    fn native_anthropic_sse_starts_with_observed_usage_and_settlement_gates_stop() {
        let mut e = NativeEncoder::new("claude", "req", true, &Limits::default());
        assert!(
            e.consume(&Event::Start("upstream".into()))
                .unwrap()
                .is_empty()
        );
        assert!(
            e.consume(&Event::Block {
                index: 0,
                kind: BlockKind::Text
            })
            .unwrap()
            .is_empty()
        );
        let packets = e.consume(&Event::Usage(usage())).unwrap();
        assert_eq!(packets.len(), 2);
        let first = String::from_utf8(packets[0].clone()).unwrap();
        assert!(first.starts_with("event: message_start\n"));
        assert!(first.contains("\"input_tokens\":12"));
        e.consume(&Event::Delta {
            index: 0,
            text: "hi".into(),
        })
        .unwrap();
        e.consume(&Event::EndBlock(0)).unwrap();
        finish(&mut e, FinishReason::Stop);
        let packets = e.terminal().unwrap();
        assert!(
            String::from_utf8(packets[0].clone())
                .unwrap()
                .starts_with("event: message_delta\n")
        );
        assert!(
            String::from_utf8(packets[1].clone())
                .unwrap()
                .starts_with("event: message_stop\n")
        );
    }
    #[test]
    fn native_anthropic_tool_and_signature_output_are_preserved() {
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Thinking,
        })
        .unwrap();
        e.consume(&Event::Delta {
            index: 0,
            text: "thought".into(),
        })
        .unwrap();
        e.consume(&Event::Signature {
            index: 0,
            provider: UpstreamProtocol::Anthropic,
            value: "sig".into(),
        })
        .unwrap();
        e.consume(&Event::EndBlock(0)).unwrap();
        e.consume(&Event::Block {
            index: 1,
            kind: BlockKind::Tool {
                id: "call_1".into(),
                name: "lookup".into(),
            },
        })
        .unwrap();
        e.consume(&Event::Delta {
            index: 1,
            text: "{\"q\":\"x\"}".into(),
        })
        .unwrap();
        e.consume(&Event::EndBlock(1)).unwrap();
        finish(&mut e, FinishReason::ToolCalls);
        let v: Value = serde_json::from_slice(&e.terminal().unwrap()[0]).unwrap();
        assert_eq!(v["content"][0]["signature"], "sig");
        assert_eq!(v["content"][1]["input"]["q"], "x");
        assert_eq!(v["stop_reason"], "tool_use");
    }
    #[test]
    fn native_anthropic_stop_sequence_roundtrip_and_invalid_inputs() {
        let parsed = parse(
            &request(json!({"stop_sequences":["END", "STOP"]})),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(parsed.stop, ["END", "STOP"]);
        let upstream = parsed
            .upstream_to(UpstreamProtocol::Anthropic, "claude")
            .unwrap();
        assert_eq!(upstream["stop_sequences"], json!(["END", "STOP"]));
        for invalid in [
            json!([]),
            json!([""]),
            json!(["a", "b", "c", "d", "e"]),
            json!([1]),
            json!("END"),
            json!(["x".repeat(1025)]),
        ] {
            assert!(
                parse(
                    &request(json!({"stop_sequences":invalid})),
                    &Limits::default()
                )
                .is_err()
            );
        }
        for stream in [false, true] {
            let mut e = NativeEncoder::new("claude", "req", stream, &Limits::default());
            e.consume(&Event::Start("up".into())).unwrap();
            e.consume(&Event::Usage(usage())).unwrap();
            e.consume(&Event::StopSequence("END".into())).unwrap();
            finish(&mut e, FinishReason::Stop);
            let packets = e.terminal().unwrap();
            let wire = String::from_utf8(packets[0].clone()).unwrap();
            assert!(wire.contains("\"stop_reason\":\"stop_sequence\""));
            assert!(wire.contains("\"stop_sequence\":\"END\""));
        }
    }
    #[test]
    fn native_anthropic_invalid_stop_sequence_receipts_fail() {
        for invalid in [String::new(), "x".repeat(1025)] {
            let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
            e.consume(&Event::Start("up".into())).unwrap();
            assert!(e.consume(&Event::StopSequence(invalid)).is_err());
        }
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::StopSequence("END".into())).unwrap();
        assert!(e.consume(&Event::StopSequence("END".into())).is_err());
        assert!(e.consume(&Event::Finish(FinishReason::Length)).is_err());
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Finish(FinishReason::Stop)).unwrap();
        assert!(e.consume(&Event::StopSequence("END".into())).is_err());
    }
    #[test]
    fn native_anthropic_output_bounds_and_invalid_tool_json_fail() {
        let limits = Limits {
            output_bytes: 200,
            ..Limits::default()
        };
        let mut e = NativeEncoder::new("claude", "req", false, &limits);
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Text,
        })
        .unwrap();
        assert!(
            e.consume(&Event::Delta {
                index: 0,
                text: "x".repeat(201)
            })
            .is_err()
        );
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Tool {
                id: "a".into(),
                name: "f".into(),
            },
        })
        .unwrap();
        e.consume(&Event::Delta {
            index: 0,
            text: "{\"x\":1,\"x\":2}".into(),
        })
        .unwrap();
        assert!(e.consume(&Event::EndBlock(0)).is_err());
    }
    #[test]
    fn native_anthropic_unknown_usage_unsigned_thinking_and_foreign_signatures_fail() {
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Finish(FinishReason::Stop)).unwrap();
        assert!(e.consume(&Event::Done).is_err());
        let mut e = NativeEncoder::new("claude", "req", false, &Limits::default());
        e.consume(&Event::Start("up".into())).unwrap();
        e.consume(&Event::Block {
            index: 0,
            kind: BlockKind::Thinking,
        })
        .unwrap();
        assert!(e.consume(&Event::EndBlock(0)).is_err());
        assert!(
            e.consume(&Event::Signature {
                index: 0,
                provider: UpstreamProtocol::Gemini,
                value: "sig".into()
            })
            .is_err()
        );
    }
}

#[cfg(test)]
mod generic_creation_tests {
    use super::*;
    use crate::routing::CacheCreationMode;

    #[test]
    fn native_generic_usage_subtracts_effective_creation_but_emits_reported_total() {
        for stream in [false, true] {
            let mut encoder = NativeEncoder::new("claude", "generic", stream, &Limits::default());
            encoder.usage = Usage {
                input_tokens: Some(37),
                uncached_input_tokens: Some(10),
                output_tokens: Some(1),
                cached_input_tokens: Some(20),
                cache_creation_input_tokens: Some(2),
                cache_creation_details: Some(super::super::CacheCreationDetails {
                    ephemeral_5m_input_tokens: Some(3),
                    ephemeral_1h_input_tokens: Some(4),
                }),
                cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
                ..Usage::default()
            };
            let wire = encoder.native_usage().unwrap();
            assert_eq!(wire["input_tokens"], 10);
            assert_eq!(wire["cache_creation_input_tokens"], 2);
            assert_eq!(wire["cache_creation"]["ephemeral_5m_input_tokens"], 3);
            assert_eq!(wire["cache_creation"]["ephemeral_1h_input_tokens"], 4);
            assert!(wire.get("cache_creation_mode").is_none());
            assert!(wire.get("uncached_input_tokens").is_none());
        }
    }

    #[test]
    fn native_generic_does_not_fill_unknown_ttls_or_missing_aggregate() {
        let mut encoder = NativeEncoder::new("claude", "generic", false, &Limits::default());
        encoder.usage = Usage {
            input_tokens: Some(40),
            uncached_input_tokens: Some(10),
            output_tokens: Some(1),
            cached_input_tokens: Some(20),
            cache_creation_input_tokens: Some(10),
            cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
            ..Usage::default()
        };
        let wire = encoder.native_usage().unwrap();
        assert_eq!(wire["input_tokens"], 10);
        assert!(wire.get("cache_creation").is_none());
        encoder.usage.cache_creation_input_tokens = None;
        encoder.usage.input_tokens = None;
        encoder.usage.cache_creation_details = Some(super::super::CacheCreationDetails {
            ephemeral_5m_input_tokens: Some(0),
            ephemeral_1h_input_tokens: Some(0),
        });
        let wire = encoder.native_usage().unwrap();
        assert_eq!(wire["input_tokens"], 10);
        assert!(wire.get("cache_creation_input_tokens").is_none());
        assert_eq!(wire["cache_creation"]["ephemeral_5m_input_tokens"], 0);
        assert_eq!(wire["cache_creation"]["ephemeral_1h_input_tokens"], 0);
        encoder.usage.cache_creation_mode = None;
        encoder.usage.uncached_input_tokens = None;
        encoder.usage.input_tokens = Some(40);
        assert!(encoder.native_usage().is_err());
    }

    #[test]
    fn native_generic_unknown_optional_counters_are_omitted_but_reported_zero_is_kept() {
        for stream in [false, true] {
            for counter in [None, Some(0)] {
                let mut encoder =
                    NativeEncoder::new("claude", "generic", stream, &Limits::default());
                encoder.usage = Usage {
                    uncached_input_tokens: Some(10),
                    output_tokens: Some(1),
                    cached_input_tokens: counter,
                    cache_creation_input_tokens: counter,
                    cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
                    ..Usage::default()
                };
                let wire = encoder.native_usage().unwrap();
                let mut expected = json!({"input_tokens":10,"output_tokens":1});
                if let Some(value) = counter {
                    expected["cache_read_input_tokens"] = json!(value);
                    expected["cache_creation_input_tokens"] = json!(value);
                }
                assert_eq!(wire, expected);
                assert_eq!(encoder.usage.input_tokens, None);
                assert_eq!(encoder.usage.cache_creation_input_tokens, counter);
            }
        }
    }

    #[test]
    fn native_generic_requires_real_input_and_output_not_a_derived_input_guess() {
        let mut encoder = NativeEncoder::new("claude", "generic", false, &Limits::default());
        encoder.usage = Usage {
            input_tokens: Some(40),
            output_tokens: Some(1),
            cached_input_tokens: Some(20),
            cache_creation_input_tokens: Some(10),
            cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
            ..Usage::default()
        };
        assert!(encoder.native_usage().is_err());
        encoder.usage.uncached_input_tokens = Some(10);
        encoder.usage.output_tokens = None;
        assert!(encoder.native_usage().is_err());
        encoder.usage.output_tokens = Some(1);
        assert!(encoder.native_usage().is_ok());
    }
}
