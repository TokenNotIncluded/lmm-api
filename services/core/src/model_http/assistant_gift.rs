//! Strict decoding of the only promotional model tools. No general tool runner.
//! This module consumes the actual bounded provider response inside Core. It
//! never deserializes a decision from an extension or a public request.
use crate::relay::RelayError;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GiftArguments {
    pub amount_credits: i64,
    #[serde(default)]
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArguments {}
pub(crate) enum GiftAction {
    Prepare(GiftArguments),
    Status,
}
/// Fields are private to the model transport. A backend consumer can read the
/// decision, but Go cannot construct this provenance or choose its operation.
pub(crate) struct GiftToolCall {
    id: String,
    response_sha256: [u8; 32],
    action: GiftAction,
}
impl GiftToolCall {
    pub(crate) fn action(&self) -> &GiftAction {
        &self.action
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn response_sha256(&self) -> [u8; 32] {
        self.response_sha256
    }
}
pub(super) fn tool_schemas(exclusive_cap: i64) -> Value {
    json!([
     {"type":"function","function":{"name":"prepare_new_user_gift","description":"Ask Core to check and immediately credit a new-user promotional gift. Core alone enforces verified identity, one-time eligibility, abuse controls and the exclusive amount cap. Zero means no gift.","strict":true,"parameters":{"type":"object","properties":{"amount_credits":{"type":"integer","minimum":0,"maximum":exclusive_cap.saturating_sub(1)},"reason":{"type":"string","maxLength":240}},"required":["amount_credits","reason"],"additionalProperties":false}}},
     {"type":"function","function":{"name":"get_new_user_gift_status","description":"Read this conversation owner's persisted Core gift status without creating or changing a gift.","strict":true,"parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false}}}
    ])
}
/// The caller enables the fixed schemas using a Core-owned grant policy. This
/// decoder is called only after reading the original bounded response bytes.
pub(super) fn parse(
    response: &Value,
    raw_response: &[u8],
) -> Result<Option<GiftToolCall>, RelayError> {
    let choices = response
        .get("choices")
        .and_then(Value::as_array)
        .filter(|v| v.len() == 1)
        .ok_or(RelayError::SettlementPending)?;
    let choice = &choices[0];
    let Some(calls) = choice
        .pointer("/message/tool_calls")
        .filter(|v| !v.is_null())
    else {
        return Ok(None);
    };
    let calls = calls.as_array().ok_or(RelayError::SettlementPending)?;
    if calls.is_empty() {
        return Ok(None);
    }
    if calls.len() != 1
        || choice.get("finish_reason").and_then(Value::as_str) != Some("tool_calls")
        || choice
            .pointer("/message/function_call")
            .is_some_and(|v| !v.is_null())
    {
        return Err(RelayError::SettlementPending);
    }
    let call = &calls[0];
    if call.get("type").and_then(Value::as_str) != Some("function") {
        return Err(RelayError::SettlementPending);
    }
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && !id.chars().any(|c| c.is_control() || c.is_whitespace())
        })
        .ok_or(RelayError::SettlementPending)?
        .to_owned();
    let args = call
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 4096)
        .ok_or(RelayError::SettlementPending)?;
    let action = match call.pointer("/function/name").and_then(Value::as_str) {
        Some("prepare_new_user_gift") => {
            let a: GiftArguments =
                serde_json::from_str(args).map_err(|_| RelayError::SettlementPending)?;
            if a.amount_credits < 0
                || a.amount_credits > 9_007_199_254_740_991
                || a.reason.chars().count() > 240
                || a.reason.chars().any(char::is_control)
            {
                return Err(RelayError::SettlementPending);
            }
            GiftAction::Prepare(a)
        }
        Some("get_new_user_gift_status") => {
            let _: EmptyArguments =
                serde_json::from_str(args).map_err(|_| RelayError::SettlementPending)?;
            GiftAction::Status
        }
        _ => return Err(RelayError::SettlementPending),
    };
    Ok(Some(GiftToolCall {
        id,
        response_sha256: Sha256::digest(raw_response).into(),
        action,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn response(name: &str, args: &str) -> Value {
        json!({"choices":[{"message":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":args}}]},"finish_reason":"tool_calls"}]})
    }
    #[test]
    fn only_fixed_typed_calls_are_decisions() {
        assert!(matches!(
            parse(
                &response(
                    "prepare_new_user_gift",
                    r#"{"amount_credits":7,"reason":"welcome"}"#
                ),
                b"provider"
            )
            .unwrap()
            .unwrap()
            .action(),
            GiftAction::Prepare(_)
        ));
        assert!(matches!(
            parse(&response("get_new_user_gift_status", "{}"), b"provider")
                .unwrap()
                .unwrap()
                .action(),
            GiftAction::Status
        ));
        for (name, args) in [
            ("arbitrary_tool", "{}"),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":-1,"reason":"x"}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1.5,"reason":"x"}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1,"reason":"x","payer":99}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1,"amount_credits":2,"reason":"x"}"#,
            ),
            ("get_new_user_gift_status", r#"{"user_id":9}"#),
        ] {
            assert!(parse(&response(name, args), b"provider").is_err());
        }
    }
    #[test]
    fn no_parallel_legacy_or_incomplete_tools() {
        let mut value = response("get_new_user_gift_status", "{}");
        value["choices"][0]["finish_reason"] = json!("length");
        assert!(parse(&value, b"x").is_err());
        let mut value = response("get_new_user_gift_status", "{}");
        let second = value["choices"][0]["message"]["tool_calls"][0].clone();
        value["choices"][0]["message"]["tool_calls"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(parse(&value, b"x").is_err());
        let mut value = response("get_new_user_gift_status", "{}");
        value["choices"][0]["message"]["function_call"] = json!({"name":"legacy"});
        assert!(parse(&value, b"x").is_err());
    }
}
