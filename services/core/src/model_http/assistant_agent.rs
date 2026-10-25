//! Agent rounds share the personal assistant's pinned transport, exact quote and
//! authoritative usage boundary. Only Core-installed definitions enter payloads.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct PreparedAgent {
    inner: PreparedAssistant,
    allowed_names: BTreeSet<String>,
}
impl PreparedAgent {
    pub(crate) fn price(&self) -> &crate::billing::charging::Price {
        &self.inner.price
    }
    pub(crate) fn reserve(&self) -> i64 {
        self.inner.reserve
    }
    pub(crate) fn context_bytes(&self) -> Result<Vec<u8>, RelayError> {
        serde_json::to_vec(&self.inner.payload).map_err(|_| RelayError::Invalid("agent payload"))
    }
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        self.inner.fingerprint
    }
}
pub(crate) struct AgentToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Vec<u8>,
}
pub(crate) enum AgentAnswer {
    Text(String),
    Tools(Vec<AgentToolCall>),
    Refused,
}
pub(crate) struct AgentObservation {
    pub answer: AgentAnswer,
    pub assistant_message: Value,
    pub response_json: Vec<u8>,
    pub response_sha256: [u8; 32],
    pub usage: crate::billing::charging::Usage,
}
impl ModelGateway {
    pub(crate) async fn prepare_agent(
        &self,
        model: &str,
        group: &str,
        max_output: u32,
        messages: Value,
        installed_definitions: &BTreeMap<String, Value>,
        allowed_names: BTreeSet<String>,
        transport: &crate::relay::provider_network::ProviderTransport,
        stored_price: &crate::billing::charging::Price,
        actor_user_id: i64,
    ) -> Result<PreparedAgent, RelayError> {
        if installed_definitions.len() > 72 || allowed_names.len() > 72 {
            return Err(RelayError::Invalid("agent tool count"));
        }
        let mut inner = self
            .prepare_assistant(
                model,
                group,
                max_output,
                messages,
                transport,
                None,
                stored_price,
                actor_user_id,
            )
            .await?;
        let mut tools = Vec::new();
        for (name, definition) in installed_definitions {
            if !allowed_names.contains(name)
                || definition.get("type").and_then(Value::as_str) != Some("function")
                || definition.pointer("/function/name").and_then(Value::as_str)
                    != Some(name.as_str())
                || !definition
                    .pointer("/function/parameters")
                    .is_some_and(Value::is_object)
            {
                return Err(RelayError::Invalid("agent installed definition"));
            }
            tools.push(definition.clone());
        }
        if !tools.is_empty() {
            inner.payload["tools"] = Value::Array(tools);
            inner.payload["parallel_tool_calls"] = json!(true);
        }
        let bytes =
            serde_json::to_vec(&inner.payload).map_err(|_| RelayError::Invalid("agent payload"))?;
        let local = inner
            .route
            .endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if bytes.len() > self.limits.request_bytes
            || (local && bytes.len() as i64 + 1024 > inner.input_ceiling)
        {
            return Err(RelayError::Limit("agent context budget"));
        }
        inner.fingerprint = Sha256::digest(bytes).into();
        let loaded_names = installed_definitions.keys().cloned().collect();
        Ok(PreparedAgent {
            inner,
            allowed_names: loaded_names,
        })
    }
    /// A single already-claimed model attempt, without retries or routing replay.
    pub(crate) async fn execute_agent(
        &self,
        prepared: PreparedAgent,
    ) -> Result<AgentObservation, RelayError> {
        let (bytes, value, usage) = tokio::time::timeout(
            self.limits.total_timeout,
            self.fetch_assistant_inner(&prepared.inner),
        )
        .await
        .map_err(|_| RelayError::SettlementPending)??;
        let (answer, assistant_message) = parse_answer(&value, &prepared.allowed_names)
            .unwrap_or((AgentAnswer::Refused, Value::Null));
        Ok(AgentObservation {
            answer,
            assistant_message,
            response_sha256: Sha256::digest(&bytes).into(),
            response_json: bytes,
            usage,
        })
    }
}
fn parse_answer(value: &Value, allowed: &BTreeSet<String>) -> Option<(AgentAnswer, Value)> {
    let choices = value.get("choices")?.as_array()?;
    if choices.len() != 1 {
        return None;
    }
    let message = choices[0].get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant")
        || message.get("function_call").is_some_and(|v| !v.is_null())
    {
        return None;
    }
    match choices[0].get("finish_reason")?.as_str()? {
        "stop" | "length" => {
            if message
                .get("tool_calls")
                .is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
            {
                return None;
            }
            let text = message.get("content")?.as_str()?;
            if text.is_empty() || text.len() > 32768 {
                return None;
            }
            Some((AgentAnswer::Text(text.to_owned()), message.clone()))
        }
        "tool_calls" => {
            if message
                .get("content")
                .is_some_and(|v| !v.is_null() && !v.as_str().is_some_and(|s| s.len() <= 32768))
            {
                return None;
            }
            let calls = message.get("tool_calls")?.as_array()?;
            if calls.is_empty() || calls.len() > 8 {
                return None;
            }
            let mut seen = BTreeSet::new();
            let mut parsed = Vec::new();
            let mut argument_bytes = 0usize;
            for call in calls {
                let id = call.get("id")?.as_str()?;
                if id.is_empty()
                    || id.len() > 128
                    || !id.as_bytes()[0].is_ascii_alphanumeric()
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                    || !seen.insert(id.to_owned())
                    || call.get("type")?.as_str()? != "function"
                {
                    return None;
                }
                let function = call.get("function")?;
                let name = function.get("name")?.as_str()?;
                let args = function.get("arguments")?.as_str()?;
                argument_bytes = argument_bytes.checked_add(args.len())?;
                if !allowed.contains(name)
                    || argument_bytes > 32768
                    || !serde_json::from_str::<Value>(args).ok()?.is_object()
                {
                    return None;
                }
                parsed.push(AgentToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments: args.as_bytes().to_vec(),
                });
            }
            Some((AgentAnswer::Tools(parsed), message.clone()))
        }
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_calls_preserve_exact_ids_arguments_and_order() {
        let allowed = BTreeSet::from(["discover_tools".into(), "get_account_access".into()]);
        let value = json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"discover_tools","arguments":"{\"names\":[\"get_account_access\"]}"}},{"id":"c2","type":"function","function":{"name":"get_account_access","arguments":"{}"}}]}}]});
        let Some((AgentAnswer::Tools(calls), _)) = parse_answer(&value, &allowed) else {
            panic!("complete tool calls expected")
        };
        assert_eq!(
            calls.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["c1", "c2"]
        );
        assert_eq!(calls[1].arguments, b"{}");
    }
    #[test]
    fn duplicate_unknown_legacy_and_truncated_calls_never_become_proposals() {
        let allowed = BTreeSet::from(["read".into()]);
        let call =
            json!({"id":"one","type":"function","function":{"name":"read","arguments":"{}"}});
        let mut v = json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[call.clone(),call]}}]});
        assert!(parse_answer(&v, &allowed).is_none());
        v["choices"][0]["message"]["tool_calls"]
            .as_array_mut()
            .unwrap()
            .pop();
        v["choices"][0]["message"]["tool_calls"][0]["function"]["name"] = json!("uninstalled");
        assert!(parse_answer(&v, &allowed).is_none());
        v["choices"][0]["message"]["tool_calls"][0]["function"]["name"] = json!("read");
        v["choices"][0]["message"]["function_call"] = json!({"name":"read"});
        assert!(parse_answer(&v, &allowed).is_none());
        v["choices"][0]["message"]["function_call"] = Value::Null;
        v["choices"][0]["finish_reason"] = json!("length");
        assert!(parse_answer(&v, &allowed).is_none());
    }
}
