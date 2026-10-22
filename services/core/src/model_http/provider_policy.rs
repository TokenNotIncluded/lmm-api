//! Reviewed, host-only external model context contracts. HTTP input cannot
//! enable a provider or supply a model limit. These are limits, never tokenizers.
use crate::{
    relay::{
        RelayError,
        honeypot_network::{literal_ip, validate_base_url},
    },
    routing::{Attempt, EmbeddingProvider, Protocol, SearchLimits, SearchProvider},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Host-reviewed limits for the exact model/endpoint below. A local stop after
/// receiving a response is not a supplier spending guarantee.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalSearchContract {
    pub provider: SearchProvider,
    pub limits: SearchLimits,
    pub contract_revision: String,
}

impl ExternalSearchContract {
    fn validate(&self, protocol: Protocol) -> Result<(), RelayError> {
        let expected = match self.provider {
            SearchProvider::OpenAiResponsesV1
            | SearchProvider::AzureResponsesV1
            | SearchProvider::XaiResponsesV1
            | SearchProvider::DashscopeResponsesV1 => Protocol::OpenAiResponses,
            SearchProvider::GeminiGroundingV1 => Protocol::Gemini,
            SearchProvider::DashscopeAnthropicV1 | SearchProvider::AnthropicMessagesV1 => {
                Protocol::AnthropicMessages
            }
            SearchProvider::ZhipuChatV1 => Protocol::OpenAiChat,
            SearchProvider::DeepSeekSearchV1 | SearchProvider::DeepSeekFetchV1 => {
                Protocol::DeepSeekTools
            }
        };
        self.limits
            .validate()
            .map_err(|_| RelayError::Invalid("invalid provider search limits"))?;
        if protocol != expected
            || self.contract_revision.is_empty()
            || self.contract_revision.len() > 512
            || self.contract_revision.chars().any(char::is_control)
        {
            return Err(RelayError::Invalid("invalid provider search contract"));
        }
        Ok(())
    }
}

/// Exact native embedding capability and supplier-enforced input contract.
/// Remote media is never downloaded by Core; enabling it requires the bound
/// to cover the supplier's remote-media validation and billing guarantees.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEmbeddingContract {
    pub provider: EmbeddingProvider,
    pub contract_revision: String,
    pub max_contents: u16,
    pub max_images: u16,
    pub max_videos: u16,
    pub max_text_bytes: u32,
    pub max_inline_image_bytes: u32,
    pub default_dimensions: u32,
    pub dimensions: Vec<u32>,
    /// A narrowed administrator allowlist does not imply that a supplier model
    /// has fixed dimensions. Adjustable models receive the actual chosen value.
    pub supports_dimension_parameter: bool,
    pub supports_fusion: bool,
    pub allow_remote_media: bool,
}
impl ExternalEmbeddingContract {
    fn validate(&self, protocol: Protocol) -> Result<(), RelayError> {
        let mut dimensions = self.dimensions.clone();
        dimensions.sort_unstable();
        dimensions.dedup();
        if protocol != Protocol::AliMultimodalEmbedding
            || self.contract_revision.is_empty()
            || self.contract_revision.len() > 512
            || self.contract_revision.chars().any(char::is_control)
            || !(1..=20).contains(&self.max_contents)
            || self.max_images > 8
            || self.max_videos > 1
            || !(1..=1_048_576).contains(&self.max_text_bytes)
            || self.max_inline_image_bytes > 5 * 1_048_576
            || !(1..=16).contains(&self.dimensions.len())
            || dimensions.len() != self.dimensions.len()
            || dimensions.iter().any(|v| !(1..=4096).contains(v))
            || !dimensions.contains(&self.default_dimensions)
            || (!self.supports_dimension_parameter && dimensions.len() != 1)
            || (self.max_videos != 0 && !self.allow_remote_media)
        {
            return Err(RelayError::Invalid("invalid native embedding contract"));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalModelContext {
    #[serde(default)]
    pub search: Option<ExternalSearchContract>,
    #[serde(default)]
    pub embedding: Option<ExternalEmbeddingContract>,
    /// Explicit opt-in for the reviewed DeepSeek request parameter contract.
    /// A model name or a protocol-compatible endpoint is never this authority.
    #[serde(default)]
    pub deepseek_parameters_contract: Option<String>,
    /// AWS authentication is a distinct host contract. This region is bound to
    /// the exact Mantle endpoint and never inferred from a model or client input.
    #[serde(default)]
    pub mantle_sigv4_region: Option<String>,
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
    /// Only an endpoint/model contract may establish that absent creation usage
    /// means zero. This is independent of the cache-read absence convention.
    #[serde(default)]
    pub missing_cache_creation_tokens_is_zero: bool,
}

#[derive(Clone, Default)]
pub struct ProviderPolicy {
    contexts: BTreeMap<(u64, String), ExternalModelContext>,
}
impl ProviderPolicy {
    pub(crate) fn missing_cache_creation_is_zero(&self, attempt: &Attempt) -> bool {
        self.contexts
            .get(&(attempt.upstream_id(), attempt.upstream_model().to_owned()))
            .is_some_and(|context| context.missing_cache_creation_tokens_is_zero)
    }

    pub fn new(
        contexts: Vec<ExternalModelContext>,
        reserved_input_tokens: i64,
    ) -> Result<Self, RelayError> {
        if !(1..=16_000_000).contains(&reserved_input_tokens) || contexts.len() > 100_000 {
            return Err(RelayError::Invalid("invalid provider context policy"));
        }
        let mut result = Self::default();
        for mut context in contexts {
            let endpoint = validate_base_url(&context.endpoint, true)?;
            if context.upstream_id == 0
                || context.upstream_model.is_empty()
                || context.upstream_model.len() > 256
                || context.upstream_model.chars().any(char::is_control)
                || (context.protocol != Protocol::DeepSeekTools && context.max_input_tokens == 0)
                || i64::from(context.max_input_tokens) > reserved_input_tokens
                || match context.protocol {
                    Protocol::DeepSeekTools => {
                        context.max_input_tokens != 0
                            || context.max_output_tokens != 0
                            || context.search.is_none()
                    }
                    Protocol::AliMultimodalEmbedding => {
                        context.max_output_tokens != 0 || context.embedding.is_none()
                    }
                    _ => !(1..=65_536).contains(&context.max_output_tokens),
                }
                || context.contract_revision.is_empty()
                || context.contract_revision.len() > 512
                || context.contract_revision.chars().any(char::is_control)
            {
                return Err(RelayError::Invalid("invalid provider context policy"));
            }
            if let Some(search) = &context.search {
                search.validate(context.protocol)?;
            }
            if let Some(embedding) = &context.embedding {
                embedding.validate(context.protocol)?;
                if context.search.is_some() {
                    return Err(RelayError::Invalid("mixed provider metering contract"));
                }
            }
            if let Some(revision) = &context.deepseek_parameters_contract
                && (revision.is_empty()
                    || revision.len() > 512
                    || revision.chars().any(char::is_control)
                    || !matches!(
                        context.protocol,
                        Protocol::OpenAiChat
                            | Protocol::OpenAiResponses
                            | Protocol::AnthropicMessages
                    ))
            {
                return Err(RelayError::Invalid("invalid DeepSeek parameter contract"));
            }
            if let Some(region) = &context.mantle_sigv4_region {
                if context.protocol != Protocol::AnthropicMessages
                    || context.deepseek_parameters_contract.is_some()
                    || context.search.is_some()
                    || context.embedding.is_some()
                {
                    return Err(RelayError::Invalid("mixed Mantle provider contract"));
                }
                crate::relay::validate_mantle_endpoint(endpoint.as_str(), region)?;
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

    /// Search is opt-in even for synthetic loopback routes. Bind its supplier
    /// contract before a hold or dispatch, then persist it with the price.
    pub(crate) fn search_contract(
        &self,
        attempt: &Attempt,
    ) -> Result<&ExternalSearchContract, RelayError> {
        self.search_contract_for_binding(
            attempt.upstream_id(),
            attempt.upstream_model(),
            attempt.protocol(),
            attempt.endpoint(),
        )
    }

    pub(crate) fn search_contract_for_binding(
        &self,
        upstream_id: u64,
        upstream_model: &str,
        protocol: Protocol,
        endpoint: &str,
    ) -> Result<&ExternalSearchContract, RelayError> {
        let endpoint = validate_base_url(endpoint, true)?;
        let context = self
            .contexts
            .get(&(upstream_id, upstream_model.to_owned()))
            .filter(|context| context.protocol == protocol && context.endpoint == endpoint.as_str())
            .ok_or(RelayError::Unsupported(
                "hosted search requires an exact supplier contract",
            ))?;
        context.search.as_ref().ok_or(RelayError::Unsupported(
            "hosted search requires reviewed supplier limits",
        ))
    }

    pub(crate) fn embedding_contract(
        &self,
        attempt: &Attempt,
    ) -> Result<(&ExternalEmbeddingContract, u32), RelayError> {
        self.embedding_contract_for_binding(
            attempt.upstream_id(),
            attempt.upstream_model(),
            attempt.protocol(),
            attempt.endpoint(),
        )
    }

    pub(crate) fn deepseek_parameters_contract(&self, attempt: &Attempt) -> Option<&str> {
        let endpoint = validate_base_url(attempt.endpoint(), true).ok()?;
        self.contexts
            .get(&(attempt.upstream_id(), attempt.upstream_model().to_owned()))
            .filter(|context| {
                context.protocol == attempt.protocol() && context.endpoint == endpoint.as_str()
            })
            .and_then(|context| context.deepseek_parameters_contract.as_deref())
    }

    pub(crate) fn mantle_region_for_binding(
        &self,
        upstream_id: u64,
        upstream_model: &str,
        protocol: Protocol,
        endpoint: &str,
    ) -> Option<&str> {
        let endpoint = validate_base_url(endpoint, true).ok()?;
        self.contexts
            .get(&(upstream_id, upstream_model.to_owned()))
            .filter(|context| context.protocol == protocol && context.endpoint == endpoint.as_str())
            .and_then(|context| context.mantle_sigv4_region.as_deref())
    }

    /// Freeze an authentication type for every credential reference, including
    /// disabled targets. A later priority/price publication cannot reinterpret
    /// an AWS document as a Bearer/API-key credential on another route.
    pub(crate) fn mantle_credential_references(
        &self,
        upstreams: &[crate::routing::UpstreamConfig],
        routes: &[crate::routing::ModelRoute],
    ) -> Result<BTreeSet<(u64, u64)>, RelayError> {
        let by_id: BTreeMap<_, _> = upstreams
            .iter()
            .map(|upstream| (upstream.id, upstream))
            .collect();
        let mut upstream_modes = BTreeMap::new();
        for target in routes.iter().flat_map(|route| &route.targets) {
            let upstream = by_id
                .get(&target.upstream_id)
                .ok_or(RelayError::RouteUnavailable)?;
            let is_mantle = self
                .mantle_region_for_binding(
                    upstream.id,
                    &target.upstream_model,
                    upstream.protocol,
                    upstream.endpoint.as_str(),
                )
                .is_some();
            if upstream_modes
                .insert(upstream.id, is_mantle)
                .is_some_and(|old| old != is_mantle)
            {
                return Err(RelayError::Invalid(
                    "upstream mixes credential authentication types",
                ));
            }
        }
        let mut modes = BTreeMap::new();
        for upstream in upstreams {
            let is_mantle = upstream_modes.get(&upstream.id).copied().unwrap_or(false);
            let reference = (upstream.credential.id, upstream.credential.revision);
            if modes
                .insert(reference, is_mantle)
                .is_some_and(|old| old != is_mantle)
            {
                return Err(RelayError::Invalid(
                    "credential reference mixes authentication types",
                ));
            }
        }
        Ok(modes
            .into_iter()
            .filter_map(|(reference, mantle)| mantle.then_some(reference))
            .collect())
    }

    pub(crate) fn embedding_contract_for_binding(
        &self,
        upstream_id: u64,
        model: &str,
        protocol: Protocol,
        endpoint: &str,
    ) -> Result<(&ExternalEmbeddingContract, u32), RelayError> {
        let endpoint = validate_base_url(endpoint, true)?;
        let context = self
            .contexts
            .get(&(upstream_id, model.to_owned()))
            .filter(|context| context.protocol == protocol && context.endpoint == endpoint.as_str())
            .ok_or(RelayError::Unsupported(
                "native embedding requires an exact supplier contract",
            ))?;
        let contract = context.embedding.as_ref().ok_or(RelayError::Unsupported(
            "native embedding requires reviewed supplier limits",
        ))?;
        Ok((contract, context.max_input_tokens))
    }

    /// Freeze the reviewed input bound for this exact selected attempt. The
    /// installation ceiling is only a maximum across models, not their price.
    pub(crate) fn reservation_input_bound(
        &self,
        attempt: &Attempt,
        reserved_input_tokens: i64,
        output_tokens: u32,
    ) -> Result<i64, RelayError> {
        self.validate_binding(
            attempt.upstream_id(),
            attempt.upstream_model(),
            attempt.protocol(),
            attempt.endpoint(),
            reserved_input_tokens,
            output_tokens,
        )?;
        let endpoint = validate_base_url(attempt.endpoint(), true)?;
        if literal_ip(&endpoint).is_some_and(|ip| ip.is_loopback()) {
            return Ok(reserved_input_tokens);
        }
        self.contexts
            .get(&(attempt.upstream_id(), attempt.upstream_model().to_owned()))
            .map(|context| i64::from(context.max_input_tokens))
            .ok_or(RelayError::RouteUnavailable)
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
        if !(1..=16_000_000).contains(&reserved_input_tokens) {
            return Err(RelayError::Invalid(
                "invalid provider input reservation bound",
            ));
        }
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
        if protocol == Protocol::SystemOne && context.max_input_tokens < 65_536 {
            return Err(RelayError::Unsupported(
                "System One requires the complete native input reservation bound",
            ));
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
    ) -> Result<i64, RelayError> {
        let bound = self.reservation_input_bound(attempt, reserved_input_tokens, 0)?;
        if long_context && bound <= 65_536 {
            return Err(RelayError::Unsupported(
                "System One long context requires an explicit larger provider reservation contract",
            ));
        }
        Ok(bound)
    }
}

impl ExternalModelContext {
    fn validate_usage(
        &self,
        usage: &crate::relay::Usage,
        upstream_cost: bool,
    ) -> Result<(), RelayError> {
        if let Some(observed) = usage.embedding {
            let contract = self
                .embedding
                .as_ref()
                .ok_or(RelayError::SettlementPending)?;
            if !observed.complete()
                || observed.provider != contract.provider
                || observed
                    .total_input_tokens
                    .is_none_or(|tokens| tokens > u64::from(self.max_input_tokens))
            {
                return Err(RelayError::SettlementPending);
            }
        }
        if let Some(observed) = usage.search {
            let search = self.search.as_ref().ok_or(RelayError::SettlementPending)?;
            if observed.provider != search.provider || !search.limits.contains(observed) {
                return Err(RelayError::SettlementPending);
            }
        }
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
    use crate::routing::{
        Endpoint, PriceConfig, PriceSnapshot, ReplayPermission, RouteRequest, Router, RoutingConfig,
    };
    use serde_json::json;
    use std::sync::Arc;

    fn attempt(protocol: Protocol, model: &str, endpoint: &str) -> Attempt {
        let mut config: RoutingConfig = serde_json::from_value(json!({
            "version":1,"groups":[{"name":"allowed","enabled":true}],
            "upstreams":[{"id":1,"endpoint":"https://api.example/v1","protocol":protocol,"credential":{"id":7,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"model","group":"allowed","enabled":true,"speeds":["standard"],
                "targets":[{"id":1,"upstream_id":1,"upstream_model":model,"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
                "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
        })).unwrap();
        config.upstreams[0].endpoint = if endpoint.starts_with("http://127.0.0.1") {
            Endpoint::local_loopback(endpoint.into()).unwrap()
        } else {
            Endpoint::try_from(endpoint.to_owned()).unwrap()
        };
        let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{
            "id":1,"model":"model","group":"allowed","speed":"standard",
            "rates":{"input":2000000,"output":0,"cache_read":0,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
        }]}))
        .unwrap();
        let router =
            Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap())).unwrap();
        router
            .route(RouteRequest {
                model: "model",
                group: "allowed",
                speed: None,
                protocol,
                replay: ReplayPermission::Denied,
            })
            .unwrap()
            .start()
            .unwrap()
    }

    #[test]
    fn selected_input_bound_uses_exact_model_contract_and_loopback_keeps_global() {
        let mut small = context();
        small.max_input_tokens = 65_536;
        small.max_output_tokens = 16;
        let mut large = small.clone();
        large.upstream_model = "large-model".into();
        large.max_input_tokens = 1_048_576;
        large.max_output_tokens = 65_536;
        let policy = ProviderPolicy::new(vec![small, large], 1_048_576).unwrap();
        let selected = attempt(
            Protocol::OpenAiChat,
            "bounded-model",
            "https://api.example/v1",
        );
        assert_eq!(
            policy
                .reservation_input_bound(&selected, 1_048_576, 16)
                .unwrap(),
            65_536
        );
        assert!(
            policy
                .reservation_input_bound(&selected, 1_048_576, 17)
                .is_err()
        );
        assert!(
            policy
                .reservation_input_bound(&selected, 65_535, 16)
                .is_err()
        );
        let large = attempt(
            Protocol::OpenAiChat,
            "large-model",
            "https://api.example/v1",
        );
        assert_eq!(
            policy
                .reservation_input_bound(&large, 1_048_576, 65_536)
                .unwrap(),
            1_048_576
        );
        for selected in [
            attempt(
                Protocol::OpenAiChat,
                "missing-model",
                "https://api.example/v1",
            ),
            attempt(
                Protocol::OpenAiResponses,
                "bounded-model",
                "https://api.example/v1",
            ),
            attempt(
                Protocol::OpenAiChat,
                "bounded-model",
                "https://other.example/v1",
            ),
        ] {
            assert!(
                policy
                    .reservation_input_bound(&selected, 1_048_576, 16)
                    .is_err()
            );
        }
        let local = attempt(
            Protocol::OpenAiChat,
            "bounded-model",
            "http://127.0.0.1:1234/v1",
        );
        assert_eq!(
            policy
                .reservation_input_bound(&local, 1_048_576, 16)
                .unwrap(),
            1_048_576
        );
        assert!(policy.reservation_input_bound(&local, 0, 16).is_err());
    }

    #[test]
    fn selected_systemone_bound_enforces_native_minimum_and_long_context_contract() {
        let selected = attempt(
            Protocol::SystemOne,
            "bounded-model",
            "https://api.example/v1",
        );
        for cap in [65_535, 65_536, 1_048_576] {
            let mut native = context();
            native.protocol = Protocol::SystemOne;
            native.max_input_tokens = cap;
            native.missing_cached_tokens_is_zero = true;
            let policy = ProviderPolicy::new(vec![native], 1_048_576).unwrap();
            let result = policy.validate_systemone_mode(&selected, 1_048_576, false);
            if cap < 65_536 {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), i64::from(cap));
            }
            assert_eq!(
                policy
                    .validate_systemone_mode(&selected, 1_048_576, true)
                    .is_ok(),
                cap > 65_536,
            );
        }
    }

    fn context() -> ExternalModelContext {
        ExternalModelContext {
            mantle_sigv4_region: None,
            deepseek_parameters_contract: None,
            embedding: None,
            search: None,
            missing_cache_creation_tokens_is_zero: false,
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
    fn exact_provider_contracts_normalize_only_the_reviewed_base_url() {
        let mut search = context();
        search.protocol = Protocol::OpenAiResponses;
        search.endpoint = "https://api.example".into();
        search.search = Some(ExternalSearchContract {
            provider: SearchProvider::OpenAiResponsesV1,
            limits: SearchLimits {
                calls: Some(1),
                ..Default::default()
            },
            contract_revision: "synthetic-search-v1".into(),
        });
        let policy = ProviderPolicy::new(vec![search], 128_000).unwrap();
        for endpoint in ["https://api.example", "https://api.example/"] {
            assert!(
                policy
                    .search_contract_for_binding(
                        1,
                        "bounded-model",
                        Protocol::OpenAiResponses,
                        endpoint
                    )
                    .is_ok()
            );
        }
        assert!(
            policy
                .search_contract_for_binding(
                    1,
                    "bounded-model",
                    Protocol::OpenAiResponses,
                    "https://api.example/v1"
                )
                .is_err()
        );
        let mut deepseek = context();
        deepseek.endpoint = "https://api.example".into();
        deepseek.deepseek_parameters_contract = Some("synthetic-deepseek-v1".into());
        let policy = ProviderPolicy::new(vec![deepseek], 128_000).unwrap();
        let selected = attempt(Protocol::OpenAiChat, "bounded-model", "https://api.example");
        assert_eq!(
            policy.deepseek_parameters_contract(&selected),
            Some("synthetic-deepseek-v1")
        );
        let other = attempt(
            Protocol::OpenAiChat,
            "bounded-model",
            "https://api.example/v1",
        );
        assert_eq!(policy.deepseek_parameters_contract(&other), None);
    }

    #[test]
    fn mantle_region_binds_host_protocol_and_excludes_other_vendor_contracts() {
        let mut contract = context();
        contract.protocol = Protocol::AnthropicMessages;
        contract.endpoint = "https://bedrock-mantle.us-east-1.api.aws".into();
        contract.mantle_sigv4_region = Some("us-east-1".into());
        let policy = ProviderPolicy::new(vec![contract.clone()], 128_000).unwrap();
        assert_eq!(
            policy.mantle_region_for_binding(
                1,
                "bounded-model",
                Protocol::AnthropicMessages,
                &contract.endpoint
            ),
            Some("us-east-1")
        );
        assert_eq!(
            policy.mantle_region_for_binding(
                1,
                "other-model",
                Protocol::AnthropicMessages,
                &contract.endpoint
            ),
            None
        );
        assert_eq!(
            policy.mantle_region_for_binding(
                1,
                "bounded-model",
                Protocol::OpenAiChat,
                &contract.endpoint
            ),
            None
        );
        assert_eq!(
            policy.mantle_region_for_binding(
                1,
                "bounded-model",
                Protocol::AnthropicMessages,
                "https://api.example/"
            ),
            None
        );
        for mutation in 0..4 {
            let mut bad = contract.clone();
            match mutation {
                0 => bad.mantle_sigv4_region = Some("eu-west-1".into()),
                1 => bad.protocol = Protocol::OpenAiChat,
                2 => bad.deepseek_parameters_contract = Some("reviewed-deepseek-v1".into()),
                _ => {
                    bad.search = Some(ExternalSearchContract {
                        provider: SearchProvider::AnthropicMessagesV1,
                        limits: SearchLimits {
                            calls: Some(1),
                            ..SearchLimits::default()
                        },
                        contract_revision: "reviewed-search-v1".into(),
                    })
                }
            }
            assert!(ProviderPolicy::new(vec![bad], 128_000).is_err());
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
