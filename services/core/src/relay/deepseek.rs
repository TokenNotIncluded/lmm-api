//! Bounded DeepSeek controls, selected by an exact administrator-owned contract.
//!
//! Protocol evidence: https://api-docs.deepseek.com/guides/thinking_mode/,
//! https://api-docs.deepseek.com/api/create-chat-completion/ and
//! https://api-docs.deepseek.com/guides/responses_api/ (reviewed 2026-10-22).
//! Comparison reference: QuantumNous/new-api, AGPL-3.0,
//! 6cf0d90b5ff2. This Rust adapter preserves this project's explicit bounds;
//! model suffixes and token budgets never select a supplier or an effort.
use super::{ClientProtocol, RelayError, Request, Role, UpstreamProtocol};
use serde_json::{Value, json};

pub(super) fn effort(value: &str) -> Option<&'static str> {
    match value {
        "none" => Some("none"),
        "minimal" | "low" => Some("low"),
        "medium" | "high" | "xhigh" => Some("high"),
        "max" | "ultra" => Some("max"),
        _ => None,
    }
}

pub(super) fn thinking(value: Option<&Value>) -> Result<Option<bool>, RelayError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or(RelayError::Invalid("invalid thinking control"))?;
    if object.len() != 1 || !object.contains_key("type") {
        return Err(RelayError::Unsupported(
            "thinking budgets or adaptive controls are not supported",
        ));
    }
    match value["type"].as_str() {
        Some("enabled") => Ok(Some(true)),
        Some("disabled") => Ok(Some(false)),
        _ => Err(RelayError::Unsupported("unsupported thinking control")),
    }
}

pub(super) fn reasoning_item(value: &Value) -> Result<String, RelayError> {
    let object = value
        .as_object()
        .ok_or(RelayError::Invalid("invalid reasoning item"))?;
    if object
        .keys()
        .any(|key| !["type", "id", "status", "content", "summary"].contains(&key.as_str()))
        || value["type"] != "reasoning"
        || value["id"].as_str().is_none_or(|id| {
            id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
        || value
            .get("status")
            .is_some_and(|status| !matches!(status.as_str(), Some("completed" | "incomplete")))
        || value
            .get("summary")
            .is_some_and(|summary| summary.as_array().is_none_or(|parts| !parts.is_empty()))
    {
        return Err(RelayError::Unsupported(
            "only bounded plaintext DeepSeek reasoning history is supported",
        ));
    }
    let parts = value["content"]
        .as_array()
        .ok_or(RelayError::Invalid("missing reasoning content"))?;
    if parts.is_empty() || parts.len() > 64 {
        return Err(RelayError::Limit("reasoning parts"));
    }
    let mut text = String::new();
    for part in parts {
        if part.as_object().is_none_or(|part| part.len() != 2) || part["type"] != "reasoning_text" {
            return Err(RelayError::Unsupported("unsupported reasoning content"));
        }
        text.push_str(
            part["text"]
                .as_str()
                .ok_or(RelayError::Invalid("invalid reasoning text"))?,
        );
    }
    Ok(text)
}

impl Request {
    pub fn validate_deepseek_parameters(
        &self,
        protocol: UpstreamProtocol,
        enabled: bool,
    ) -> Result<(), RelayError> {
        let requested = self.deepseek_thinking.is_some()
            || !self.response_reasoning_items.is_empty()
            || self.reasoning_effort.as_ref().is_some_and(|effort| {
                self.client == ClientProtocol::Anthropic
                    || !["low", "medium", "high"].contains(&effort.as_str())
            });
        if !enabled {
            return if requested {
                Err(RelayError::Unsupported(
                    "DeepSeek controls require an exact supplier contract",
                ))
            } else {
                Ok(())
            };
        }
        if !matches!(
            (self.client, protocol),
            (ClientProtocol::Chat, UpstreamProtocol::Chat)
                | (ClientProtocol::Responses, UpstreamProtocol::Responses)
                | (ClientProtocol::Anthropic, UpstreamProtocol::Anthropic)
        ) {
            return Err(RelayError::Unsupported(
                "DeepSeek controls require the same native protocol",
            ));
        }
        let effort = self
            .reasoning_effort
            .as_deref()
            .map(|value| {
                effort(value).ok_or(RelayError::Unsupported("unsupported DeepSeek effort"))
            })
            .transpose()?;
        if matches!(
            (self.deepseek_thinking, effort),
            (Some(true), Some("none")) | (Some(false), Some("low" | "high" | "max"))
        ) {
            return Err(RelayError::Invalid(
                "conflicting DeepSeek thinking controls",
            ));
        }
        if self.has_hosted_search()
            || self.prompt_cache_options.is_some()
            || self.parallel_tool_calls == Some(false)
            || self.tools.iter().any(|tool| tool.strict == Some(true))
            || self
                .messages
                .iter()
                .any(|message| message.role == Role::Developer)
        {
            return Err(RelayError::Unsupported(
                "request exceeds the bounded DeepSeek contract",
            ));
        }
        // DeepSeek documents model-dependent top_p behavior and ignores temperature
        // in thinking mode. Preserve requested semantics by refusing these combinations.
        let thinks = self.deepseek_thinking != Some(false) && effort != Some("none");
        if self.top_p.is_some() || (thinks && self.temperature.is_some()) {
            return Err(RelayError::Unsupported(
                "sampling controls cannot be preserved for this DeepSeek mode",
            ));
        }
        Ok(())
    }
}

pub(super) fn apply(
    request: &Request,
    protocol: UpstreamProtocol,
    value: &mut Value,
) -> Result<(), RelayError> {
    let effort = request
        .reasoning_effort
        .as_deref()
        .map(|value| effort(value).ok_or(RelayError::Unsupported("unsupported DeepSeek effort")))
        .transpose()?;
    let thinking = request
        .deepseek_thinking
        .or(effort.map(|effort| effort != "none"));
    let object = value
        .as_object_mut()
        .ok_or(RelayError::Invalid("request encoding failed"))?;
    object.remove("store");
    object.remove("parallel_tool_calls");
    match protocol {
        UpstreamProtocol::Chat => {
            object.remove("max_completion_tokens");
            object.insert("max_tokens".into(), json!(request.max_output_tokens));
            if let Some(thinking) = thinking {
                object.insert(
                    "thinking".into(),
                    json!({"type":if thinking {"enabled"} else {"disabled"}}),
                );
            }
            if let Some(effort) = effort {
                object.insert("reasoning_effort".into(), json!(effort));
            }
        }
        UpstreamProtocol::Responses => {
            if let Some(effort) = effort {
                object.insert("reasoning".into(), json!({"effort":effort}));
            }
        }
        UpstreamProtocol::Anthropic => {
            if let Some(thinking) = thinking {
                object.insert(
                    "thinking".into(),
                    json!({"type":if thinking {"enabled"} else {"disabled"}}),
                );
            }
            if let Some(effort) = effort.filter(|effort| *effort != "none") {
                object.insert("output_config".into(), json!({"effort":effort}));
            }
        }
        UpstreamProtocol::Gemini => {
            return Err(RelayError::Unsupported("DeepSeek Gemini conversion"));
        }
    }
    Ok(())
}
