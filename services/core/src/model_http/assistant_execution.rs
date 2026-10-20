//! Host-only bounded personal text execution using the real gateway router.
//! There is no deserializable URL, price, payer or upstream credential input.
#[path = "assistant_agent.rs"]
mod agent;
pub(crate) use agent::{AgentAnswer, AgentObservation, PreparedAgent};

use super::*;
use crate::routing::{CreditRate, Protocol, ReplayPermission, RouteRequest};
use serde_json::Value;
use tokio_stream::StreamExt;

pub(crate) struct PreparedAssistant {
    pub price: crate::billing::charging::Price,
    pub reserve: i64,
    pub fingerprint: [u8; 32],
    route: relay::Route,
    payload: Value,
    input_ceiling: i64,
    output_ceiling: u32,
    connection: crate::relay::provider_network::ProviderConnection,
    attempt: Arc<crate::routing::Attempt>,
    gift_tools: bool,
}
pub(crate) enum AssistantAnswer {
    Text(String),
    Gift(super::assistant_gift::GiftToolCall),
}
pub(crate) struct AssistantObservation {
    pub answer: AssistantAnswer,
    pub usage: crate::billing::charging::Usage,
}
impl ModelGateway {
    pub(crate) fn assistant_parallelism(&self) -> usize {
        self.limits.max_in_flight.min(16)
    }
    pub(crate) fn assistant_quote(
        &self,
        model: &str,
        group: &str,
    ) -> Result<crate::billing::charging::Price, RelayError> {
        let plan = self
            .bridge
            .routing
            .route(RouteRequest {
                model,
                group,
                speed: None,
                protocol: crate::routing::Protocol::OpenAiChat,
                replay: crate::routing::ReplayPermission::Denied,
            })
            .map_err(|_| RelayError::RouteUnavailable)?;
        routing::credit_price(plan.quote(), plan.canonical_model(), group)
    }
    pub(crate) fn validate_assistant_policy(
        &self,
        model: &str,
        group: &str,
        max_output: u32,
    ) -> Result<(), RelayError> {
        let mut plan = self
            .bridge
            .routing
            .route(RouteRequest {
                model,
                group,
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .map_err(|_| RelayError::RouteUnavailable)?;
        let price = routing::credit_price(plan.quote(), plan.canonical_model(), group)?;
        if price.upstream_cost_group_ppm.is_some() {
            return Err(RelayError::Unsupported("assistant requires token quote"));
        }
        let attempt = plan.start().map_err(|_| RelayError::RouteUnavailable)?;
        if attempt.protocol() != Protocol::OpenAiChat
            || attempt.payload_policy() != crate::routing::PayloadPolicy::Adapt
            || u128::from(attempt.timeout_ms()) < self.limits.total_timeout.as_millis()
        {
            return Err(RelayError::RouteUnavailable);
        }
        self.bridge.provider_policy.validate_attempt(
            &attempt,
            self.bridge.policy.input_token_ceiling,
            max_output,
        )?;
        let reference = attempt.credential_ref();
        if !self
            .bridge
            .credentials
            .contains_key(&(reference.id, reference.revision))
        {
            return Err(RelayError::RouteUnavailable);
        }
        Ok(())
    }
    pub(crate) async fn prepare_assistant(
        &self,
        model: &str,
        group: &str,
        max_output: u32,
        messages: Value,
        transport: &crate::relay::provider_network::ProviderTransport,
        gift_cap: Option<i64>,
    ) -> Result<PreparedAssistant, RelayError> {
        if max_output == 0 || max_output > 4096 {
            return Err(RelayError::Invalid("assistant output limit"));
        }
        let mut plan = self
            .bridge
            .routing
            .route(RouteRequest {
                model,
                group,
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .map_err(|_| RelayError::RouteUnavailable)?;
        let quote = plan.quote().with_credit_rate(
            CreditRate::new(1, 500_000).map_err(|_| RelayError::BillingUnavailable)?,
        );
        let price = routing::credit_price(quote.price(), plan.canonical_model(), group)?;
        if price.upstream_cost_group_ppm.is_some() {
            return Err(RelayError::Unsupported("assistant requires token quote"));
        }
        let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
        if attempt.protocol() != Protocol::OpenAiChat
            || attempt.payload_policy() != crate::routing::PayloadPolicy::Adapt
            || !attempt.is_available()
            || u128::from(attempt.timeout_ms()) < self.limits.total_timeout.as_millis()
        {
            return Err(RelayError::RouteUnavailable);
        }
        let mut endpoint =
            reqwest::Url::parse(attempt.endpoint()).map_err(|_| RelayError::RouteUnavailable)?;
        let local = endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        self.bridge.provider_policy.validate_attempt(
            &attempt,
            self.bridge.policy.input_token_ceiling,
            max_output,
        )?;
        endpoint
            .path_segments_mut()
            .map_err(|_| RelayError::RouteUnavailable)?
            .pop_if_empty()
            .push("chat")
            .push("completions");
        let reference = attempt.credential_ref();
        let secret = self
            .bridge
            .credentials
            .get(&(reference.id, reference.revision))
            .ok_or(RelayError::RouteUnavailable)?;
        let availability = attempt.clone();
        let route = relay::Route::new(
            format!(
                "r{}-t{}-u{}",
                attempt.route_id(),
                attempt.target_id(),
                attempt.upstream_id()
            ),
            relay::UpstreamProtocol::Chat,
            endpoint.as_str(),
            attempt.upstream_model().to_owned(),
            secret,
            attempt.quote().version(),
        )?
        .with_availability_guard(Arc::new(move || availability.is_available()));
        let mut payload =
            json!({"model":route.model,"messages":messages,"max_tokens":max_output,"stream":false});
        let gift_tools = gift_cap.is_some();
        if let Some(cap) = gift_cap {
            payload["tools"] = super::assistant_gift::tool_schemas(cap);
            payload["parallel_tool_calls"] = json!(false);
            payload["messages"].as_array_mut().ok_or(RelayError::Invalid("assistant messages"))?.insert(0,json!({"role":"system","content":"You are the site's onboarding assistant. Only discuss onboarding, account safety and new-user gifts. Use only the fixed Core gift tools. Never claim a gift was credited without a Core receipt. Never request secrets. Zero means no gift; Core enforces eligibility, caps and spending. Ask at least two distinct substantive onboarding questions before proposing a positive gift; never request personal secrets."}));
        }
        let bytes =
            serde_json::to_vec(&payload).map_err(|_| RelayError::Invalid("assistant payload"))?;
        // Synthetic local contract: all serialized UTF8 bytes plus framing are
        // bounded before reserving; never apply this to an external tokenizer.
        if bytes.len() > self.limits.request_bytes
            || (local && bytes.len() as i64 + 1024 > self.bridge.policy.input_token_ceiling)
        {
            return Err(RelayError::Limit("assistant context budget"));
        }
        let reserve = i64::try_from(
            quote
                .reservation_ceiling(
                    self.bridge.policy.input_token_ceiling as u64,
                    u64::from(max_output),
                )
                .map_err(|_| RelayError::BillingUnavailable)?
                .0,
        )
        .map_err(|_| RelayError::BillingUnavailable)?;
        let connection = tokio::time::timeout(
            self.limits.total_timeout,
            transport.prepare(&route.endpoint, &self.limits),
        )
        .await
        .map_err(|_| RelayError::RouteUnavailable)??;
        Ok(PreparedAssistant {
            price,
            reserve,
            fingerprint: Sha256::digest(bytes).into(),
            route,
            payload,
            input_ceiling: self.bridge.policy.input_token_ceiling,
            output_ceiling: max_output,
            connection,
            attempt,
            gift_tools,
        })
    }
    /// Called only after the original Core worker claim has committed. No retry.
    pub(crate) async fn execute_assistant(
        &self,
        p: PreparedAssistant,
    ) -> Result<AssistantObservation, RelayError> {
        tokio::time::timeout(self.limits.total_timeout, self.execute_assistant_inner(p))
            .await
            .map_err(|_| RelayError::SettlementPending)?
    }
    async fn execute_assistant_inner(
        &self,
        p: PreparedAssistant,
    ) -> Result<AssistantObservation, RelayError> {
        let (bytes, value, usage) = self.fetch_assistant_inner(&p).await?;
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(RelayError::SettlementPending)?;
        let gift = if p.gift_tools {
            super::assistant_gift::parse(&value, &bytes)?
        } else {
            None
        };
        let answer = if let Some(call) = gift {
            AssistantAnswer::Gift(call)
        } else {
            if choices.len() != 1
                || !matches!(
                    choices[0].get("finish_reason").and_then(Value::as_str),
                    Some("stop" | "length")
                )
            {
                return Err(RelayError::SettlementPending);
            }
            let tool_calls = choices[0].pointer("/message/tool_calls");
            if tool_calls.is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
                || choices[0]
                    .pointer("/message/function_call")
                    .is_some_and(|v| !v.is_null())
            {
                return Err(RelayError::SettlementPending);
            }
            let answer = choices[0]
                .pointer("/message/content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 32768)
                .ok_or(RelayError::SettlementPending)?
                .to_owned();
            AssistantAnswer::Text(answer)
        };
        Ok(AssistantObservation { answer, usage })
    }
    async fn fetch_assistant_inner(
        &self,
        p: &PreparedAssistant,
    ) -> Result<(Vec<u8>, Value, crate::billing::charging::Usage), RelayError> {
        if !p.route.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        let response = p
            .connection
            .client()
            .post(p.route.endpoint.clone())
            .header(header::AUTHORIZATION, p.route.credential.clone())
            .json(&p.payload)
            .send()
            .await
            .map_err(|_| RelayError::SettlementPending)?;
        if !p.connection.matches_peer(response.remote_addr()) || !response.status().is_success() {
            return Err(RelayError::SettlementPending);
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| RelayError::SettlementPending)?;
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|n| n > 131072)
            {
                return Err(RelayError::Limit("assistant provider output"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| RelayError::SettlementPending)?;
        let input = value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0 && *n <= p.input_ceiling)
            .ok_or(RelayError::SettlementPending)?;
        let output = value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0 && *n <= i64::from(p.output_ceiling))
            .ok_or(RelayError::SettlementPending)?;
        let cached_observed = match value.pointer("/usage/prompt_tokens_details/cached_tokens") {
            None => None,
            Some(v) => Some(
                v.as_i64()
                    .filter(|n| *n >= 0 && *n <= input)
                    .ok_or(RelayError::SettlementPending)?,
            ),
        };
        let observed = relay::Usage {
            input_tokens: Some(input as u64),
            output_tokens: Some(output as u64),
            cached_input_tokens: cached_observed.map(|n| n as u64),
            ..relay::Usage::default()
        };
        self.bridge
            .provider_policy
            .validate_usage(&p.attempt, &observed, false)?;
        let cached = cached_observed.unwrap_or(0);
        Ok((
            bytes,
            value,
            crate::billing::charging::Usage {
                image: None,
                input,
                output,
                cached,
                upstream_cost_atto_usd: None,
            },
        ))
    }
}
