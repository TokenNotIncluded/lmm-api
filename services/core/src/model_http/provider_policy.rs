//! Reviewed, host-only external model context contracts. HTTP input cannot
//! enable a provider or supply a model limit. These are limits, never tokenizers.
use crate::{
    relay::{
        RelayError,
        honeypot_network::{literal_ip, validate_base_url},
    },
    routing::{Attempt, Protocol},
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalModelContext {
    pub upstream_id: u64,
    pub upstream_model: String,
    pub protocol: Protocol,
    pub endpoint: String,
    /// A reviewed supplier guarantee: every accepted request bills at most
    /// this many input tokens, including cached input and tool/message framing.
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    /// Supplier/model documentation or deployment contract revision, required.
    pub contract_revision: String,
    /// Missing cache reads are unknown unless the reviewed supplier contract
    /// explicitly defines absence as zero. Never infer this from vendor names.
    #[serde(default)]
    pub missing_cached_tokens_is_zero: bool,
}

#[derive(Clone, Default)]
pub struct ProviderPolicy {
    contexts: BTreeMap<(u64, String), ExternalModelContext>,
}
impl ProviderPolicy {
    pub fn new(
        contexts: Vec<ExternalModelContext>,
        reserved_input_tokens: i64,
    ) -> Result<Self, RelayError> {
        if !(1..=16_000_000).contains(&reserved_input_tokens) || contexts.len() > 100_000 {
            return Err(RelayError::Invalid("invalid provider context policy"));
        }
        let mut result = Self::default();
        for mut context in contexts {
            let endpoint = validate_base_url(&context.endpoint, false)?;
            if context.upstream_id == 0
                || context.upstream_model.is_empty()
                || context.upstream_model.len() > 256
                || context.upstream_model.chars().any(char::is_control)
                || context.max_input_tokens == 0
                || i64::from(context.max_input_tokens) > reserved_input_tokens
                || !(1..=65_536).contains(&context.max_output_tokens)
                || context.contract_revision.is_empty()
                || context.contract_revision.len() > 512
                || context.contract_revision.chars().any(char::is_control)
            {
                return Err(RelayError::Invalid("invalid provider context policy"));
            }
            context.endpoint = endpoint.to_string();
            if result
                .contexts
                .insert(
                    (context.upstream_id, context.upstream_model.clone()),
                    context,
                )
                .is_some()
            {
                return Err(RelayError::Invalid("duplicate provider model context"));
            }
        }
        Ok(result)
    }

    pub(crate) fn validate_attempt(
        &self,
        attempt: &Attempt,
        reserved_input_tokens: i64,
        output_tokens: u32,
    ) -> Result<(), RelayError> {
        self.validate_binding(
            attempt.upstream_id(),
            attempt.upstream_model(),
            attempt.protocol(),
            attempt.endpoint(),
            reserved_input_tokens,
            output_tokens,
        )
    }

    /// Local synthetic cost contracts retain their bounded test behavior.
    /// A supplier-reported final cost alone cannot enforce an external budget.
    pub(crate) fn validate_billing_mode(
        &self,
        attempt: &Attempt,
        upstream_cost: bool,
    ) -> Result<(), RelayError> {
        let endpoint = validate_base_url(attempt.endpoint(), true)?;
        if upstream_cost && !literal_ip(&endpoint).is_some_and(|ip| ip.is_loopback()) {
            return Err(RelayError::Unsupported(
                "external upstream-cost billing requires a supplier-enforced budget contract",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_usage(
        &self,
        attempt: &Attempt,
        usage: &crate::relay::Usage,
        upstream_cost: bool,
    ) -> Result<(), RelayError> {
        let endpoint = validate_base_url(attempt.endpoint(), true)?;
        if literal_ip(&endpoint).is_some_and(|ip| ip.is_loopback()) {
            return Ok(());
        }
        let context = self
            .contexts
            .get(&(attempt.upstream_id(), attempt.upstream_model().to_owned()))
            .ok_or(RelayError::SettlementPending)?;
        context.validate_usage(usage, upstream_cost)
    }

    pub(crate) fn validate_binding(
        &self,
        id: u64,
        model: &str,
        protocol: Protocol,
        endpoint: &str,
        reserved_input_tokens: i64,
        output_tokens: u32,
    ) -> Result<(), RelayError> {
        if protocol == Protocol::SystemOne && reserved_input_tokens < 65_536 {
            return Err(RelayError::Unsupported(
                "System One requires the complete native input reservation bound",
            ));
        }
        let endpoint = validate_base_url(endpoint, true)?;
        if literal_ip(&endpoint).is_some_and(|ip| ip.is_loopback()) {
            return Ok(());
        }
        let context = self
            .contexts
            .get(&(id, model.to_owned()))
            .ok_or(RelayError::Unsupported(
                "external provider requires an explicit model context contract",
            ))?;
        if context.endpoint != endpoint.as_str()
            || context.protocol != protocol
            || i64::from(context.max_input_tokens) > reserved_input_tokens
            || (protocol != Protocol::SystemOne && output_tokens > context.max_output_tokens)
        {
            return Err(RelayError::RouteUnavailable);
        }
        if protocol == Protocol::SystemOne && !context.missing_cached_tokens_is_zero {
            return Err(RelayError::Unsupported(
                "System One requires a reviewed no-discounted-cache usage contract",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_systemone_mode(
        &self,
        attempt: &Attempt,
        reserved_input_tokens: i64,
        long_context: bool,
    ) -> Result<(), RelayError> {
        self.validate_attempt(attempt, reserved_input_tokens, 0)?;
        if long_context {
            let bound = self
                .contexts
                .get(&(attempt.upstream_id(), attempt.upstream_model().to_owned()))
                .map_or(reserved_input_tokens, |context| {
                    i64::from(context.max_input_tokens)
                });
            if bound <= 65_536 {
                return Err(RelayError::Unsupported(
                    "System One long context requires an explicit larger provider reservation contract",
                ));
            }
        }
        Ok(())
    }
}

impl ExternalModelContext {
    fn validate_usage(
        &self,
        usage: &crate::relay::Usage,
        upstream_cost: bool,
    ) -> Result<(), RelayError> {
        if !upstream_cost
            && usage.cached_input_tokens.is_none()
            && !self.missing_cached_tokens_is_zero
        {
            return Err(RelayError::SettlementPending);
        }
        if usage
            .input_tokens
            .is_none_or(|tokens| tokens > u64::from(self.max_input_tokens))
            || usage.output_tokens.is_none()
            || (self.protocol != Protocol::SystemOne
                && usage
                    .output_tokens
                    .is_some_and(|tokens| tokens > u64::from(self.max_output_tokens)))
        {
            return Err(RelayError::SettlementPending);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> ExternalModelContext {
        ExternalModelContext {
            upstream_id: 1,
            upstream_model: "bounded-model".into(),
            protocol: Protocol::OpenAiChat,
            endpoint: "https://api.example/v1".into(),
            max_input_tokens: 128_000,
            max_output_tokens: 4096,
            contract_revision: "synthetic-contract-v1".into(),
            missing_cached_tokens_is_zero: false,
        }
    }
    #[test]
    fn external_context_is_explicit_and_binds_identity_protocol_and_origin() {
        let empty = ProviderPolicy::default();
        assert!(
            empty
                .validate_binding(
                    1,
                    "bounded-model",
                    Protocol::OpenAiChat,
                    "https://api.example/v1",
                    128_000,
                    1024
                )
                .is_err()
        );
        assert!(
            empty
                .validate_binding(
                    1,
                    "local",
                    Protocol::OpenAiChat,
                    "http://127.0.0.1:1234/v1",
                    128_000,
                    1024
                )
                .is_ok()
        );
        let policy = ProviderPolicy::new(vec![context()], 128_000).unwrap();
        assert!(
            policy
                .validate_binding(
                    1,
                    "bounded-model",
                    Protocol::OpenAiChat,
                    "https://api.example/v1",
                    128_000,
                    4096
                )
                .is_ok()
        );
        for (id, model, protocol, url, input, output) in [
            (
                2,
                "bounded-model",
                Protocol::OpenAiChat,
                "https://api.example/v1",
                128_000,
                4096,
            ),
            (
                1,
                "other",
                Protocol::OpenAiChat,
                "https://api.example/v1",
                128_000,
                4096,
            ),
            (
                1,
                "bounded-model",
                Protocol::OpenAiResponses,
                "https://api.example/v1",
                128_000,
                4096,
            ),
            (
                1,
                "bounded-model",
                Protocol::OpenAiChat,
                "https://api.example/other",
                128_000,
                4096,
            ),
            (
                1,
                "bounded-model",
                Protocol::OpenAiChat,
                "https://evil.example/v1",
                128_000,
                4096,
            ),
            (
                1,
                "bounded-model",
                Protocol::OpenAiChat,
                "https://api.example/v1",
                127_999,
                4096,
            ),
            (
                1,
                "bounded-model",
                Protocol::OpenAiChat,
                "https://api.example/v1",
                128_000,
                4097,
            ),
        ] {
            assert!(
                policy
                    .validate_binding(id, model, protocol, url, input, output)
                    .is_err()
            );
        }
    }
    #[test]
    fn unknown_cache_counters_and_excess_model_usage_retain_holds() {
        let mut contract = context();
        let mut usage = crate::relay::Usage {
            input_tokens: Some(12),
            output_tokens: Some(3),
            ..Default::default()
        };
        assert!(contract.validate_usage(&usage, false).is_err());
        usage.cached_input_tokens = Some(0);
        assert!(contract.validate_usage(&usage, false).is_ok());
        usage.input_tokens = Some(128_001);
        assert!(contract.validate_usage(&usage, false).is_err());
        usage.input_tokens = Some(12);
        usage.output_tokens = Some(4097);
        assert!(contract.validate_usage(&usage, false).is_err());
        usage.output_tokens = Some(3);
        usage.cached_input_tokens = None;
        contract.missing_cached_tokens_is_zero = true;
        assert!(contract.validate_usage(&usage, false).is_ok());
    }

    #[test]
    fn malformed_and_unfunded_context_contracts_fail_closed() {
        assert!(ProviderPolicy::new(vec![context()], 127_999).is_err());
        assert!(ProviderPolicy::new(vec![context(), context()], 128_000).is_err());
        for endpoint in [
            "http://api.example/v1",
            "https://10.0.0.1/v1",
            "https://127.0.0.1/v1",
            "https://api.example/v1?key=x",
        ] {
            let mut value = context();
            value.endpoint = endpoint.into();
            assert!(ProviderPolicy::new(vec![value], 128_000).is_err());
        }
        let mut value = context();
        value.contract_revision.clear();
        assert!(ProviderPolicy::new(vec![value], 128_000).is_err());
    }
}
