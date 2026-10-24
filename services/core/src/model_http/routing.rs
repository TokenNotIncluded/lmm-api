use super::Bridge;
use crate::{
    billing::charging::Price,
    relay::{
        ClientProtocol, RelayError, RelayFuture, Request, RequestContext, Route, RouteProvider,
        UpstreamProtocol,
    },
    routing::{
        Attempt, CredentialRef, CreditQuote, CreditRate, PayloadPolicy, PriceQuote, Protocol,
        ReplayPermission, RouteRequest,
    },
};
use std::{collections::BTreeMap, sync::Arc};

/// Rust-host secret material. Deliberately has no Debug/Serialize implementation.
pub struct UpstreamCredential {
    pub reference: CredentialRef,
    pub secret: String,
}

#[derive(Clone)]
pub(super) struct Selected {
    pub attempt: Arc<Attempt>,
    pub price: Price,
    pub quote: CreditQuote,
    pub route_id: String,
    pub input_token_ceiling: i64,
}

impl Bridge {
    /// Complete admission before dispatch with the ledger's actual frozen price.
    pub(super) fn freeze_selected_price(
        &self,
        id: &str,
        charge: &crate::billing::charging::Charge,
    ) -> Result<(), RelayError> {
        charge
            .price
            .validate_actor(charge.actor_user_id)
            .map_err(super::billing::error)?;
        let mut active = self
            .active
            .lock()
            .map_err(|_| RelayError::BillingUnavailable)?;
        let selected = active
            .get_mut(id)
            .and_then(|op| op.selected.as_mut())
            .ok_or(RelayError::BillingUnavailable)?;
        if selected.price.base_price() != charge.price.base_price() {
            return Err(RelayError::Conflict);
        }
        selected.quote = selected.quote.with_discount(charge.price.discount());
        selected.price = charge.price.clone();
        Ok(())
    }
}

pub(super) fn credential_map(
    values: Vec<UpstreamCredential>,
) -> Result<BTreeMap<(u64, u64), String>, RelayError> {
    let mut result = BTreeMap::new();
    for value in values {
        if value.reference.id == 0
            || value.reference.revision == 0
            || value.secret.is_empty()
            || value.secret.len() > 4096
            || result
                .insert((value.reference.id, value.reference.revision), value.secret)
                .is_some()
        {
            return Err(RelayError::Invalid(
                "invalid upstream credential references",
            ));
        }
    }
    Ok(result)
}

/// Convert only prices that charging can represent EXACTLY. 1 USD is 500000
/// credits, hence one micro-USD is half a credit. Never round each token rate.
pub(super) fn credit_price(
    quote: PriceQuote,
    model: &str,
    group: &str,
) -> Result<Price, RelayError> {
    let rates = quote.rates();
    if let Some(tariff) = quote.tiered() {
        let (group_multiplier_ppm, speed_multiplier_ppm) = quote.multipliers_ppm();
        let price = Price {
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: Some(crate::routing::PersistedTieredTariff {
                tariff: tariff.tariff(),
                input_semantics: crate::routing::InputSemantics::Inclusive,
                group_multiplier_ppm,
                speed_multiplier_ppm,
                credits_per_usd: 500_000,
                missing_cache_creation_is_zero: false,
                cache_creation_mode: tariff.cache_creation_mode(),
            }),
            version: format!("routing-{}-rule-{}", quote.version(), quote.rule_id()),
            model: model.to_owned(),
            group: group.to_owned(),
            input_per_million: 0,
            cached_per_million: 0,
            output_per_million: 0,
        };
        price.validate().map_err(super::billing::error)?;
        return Ok(price);
    }
    if quote.billing_mode() == crate::routing::BillingMode::UpstreamCost {
        let (group_ppm, _) = quote.multipliers_ppm();
        let price = Price {
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: Some(group_ppm),
            tiered: None,
            version: format!("routing-{}-rule-{}", quote.version(), quote.rule_id()),
            model: model.to_owned(),
            group: group.to_owned(),
            input_per_million: 0,
            cached_per_million: 0,
            output_per_million: 0,
        };
        price.validate().map_err(super::billing::error)?;
        return Ok(price);
    }
    if rates.request != 0 || rates.cache_write != 0 {
        return Err(RelayError::Unsupported(
            "price dimensions require a compatible charging contract",
        ));
    }
    let (group_ppm, speed_ppm) = quote.multipliers_ppm();
    let convert = |rate: u64| -> Result<i64, RelayError> {
        let numerator = u128::from(rate)
            .checked_mul(u128::from(group_ppm))
            .and_then(|v| v.checked_mul(u128::from(speed_ppm)))
            .ok_or(RelayError::BillingUnavailable)?;
        const DENOMINATOR: u128 = 2_000_000_000_000;
        if numerator % DENOMINATOR != 0 {
            return Err(RelayError::Unsupported(
                "price cannot be represented exactly in integer credits",
            ));
        }
        i64::try_from(numerator / DENOMINATOR).map_err(|_| RelayError::BillingUnavailable)
    };
    let price = Price {
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
        version: format!("routing-{}-rule-{}", quote.version(), quote.rule_id()),
        model: model.to_owned(),
        group: group.to_owned(),
        input_per_million: convert(rates.input)?,
        cached_per_million: convert(rates.cache_read)?,
        output_per_million: convert(rates.output)?,
    };
    price.validate().map_err(super::billing::error)?;
    Ok(price)
}

/// Keep provider base prefixes and encode model identifiers as one segment.
/// Only Gemini's streaming switch can add a query, always server-derived.
fn operation_endpoint(
    base: &str,
    protocol: UpstreamProtocol,
    model: &str,
    stream: bool,
) -> Result<reqwest::Url, RelayError> {
    let mut endpoint = crate::relay::honeypot_network::validate_base_url(base, true)?;
    {
        let mut segments = endpoint
            .path_segments_mut()
            .map_err(|_| RelayError::RouteUnavailable)?;
        segments.pop_if_empty();
        match protocol {
            UpstreamProtocol::Chat => {
                segments.push("chat").push("completions");
            }
            UpstreamProtocol::Responses => {
                segments.push("responses");
            }
            UpstreamProtocol::Anthropic => {
                segments.push("messages");
            }
            UpstreamProtocol::Gemini => {
                segments.push("models").push(&format!(
                    "{model}:{}",
                    if stream {
                        "streamGenerateContent"
                    } else {
                        "generateContent"
                    }
                ));
            }
        }
    }
    if protocol == UpstreamProtocol::Gemini && stream {
        endpoint.set_query(Some("alt=sse"));
    }
    Ok(endpoint)
}

impl RouteProvider for Bridge {
    fn select<'a>(
        &'a self,
        context: &'a RequestContext,
        request: &'a Request,
    ) -> RelayFuture<'a, Route> {
        Box::pin(async move {
            let group = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?
                .get(&context.request_id)
                .ok_or(RelayError::Forbidden)?
                .group
                .clone();
            let protocol = match request.client {
                ClientProtocol::Chat => Protocol::OpenAiChat,
                ClientProtocol::Responses => Protocol::OpenAiResponses,
                ClientProtocol::Anthropic => Protocol::AnthropicMessages,
                ClientProtocol::Gemini => Protocol::Gemini,
            };
            let mut plan = self
                .routing
                .route(RouteRequest {
                    model: &request.model,
                    group: &group,
                    speed: None,
                    protocol,
                    replay: ReplayPermission::Denied,
                })
                .map_err(|_| RelayError::RouteUnavailable)?;
            // Capture the server's current, fixed ledger conversion once. The
            // persisted legacy Price is still required until A03 accepts the
            // complete quote; unsupported dimensions must keep failing closed.
            let quote = plan.quote().with_credit_rate(
                CreditRate::new(1, 500_000).map_err(|_| RelayError::BillingUnavailable)?,
            );
            let mut price = credit_price(quote.price(), plan.canonical_model(), &group)?;
            let attempt = plan.start().map_err(|_| RelayError::RouteUnavailable)?;
            if request.prompt_cache_options.is_some()
                && (attempt.protocol() != Protocol::OpenAiResponses
                    || !price
                        .tiered
                        .as_ref()
                        .is_some_and(|t| t.tariff.revision == 4))
            {
                return Err(RelayError::Unsupported(
                    "cache lifetime controls require a reviewed twelve-hour tariff",
                ));
            }
            if let Some(tariff) = &mut price.tiered {
                if tariff.tariff.revision == 4
                    && !matches!(
                        attempt.protocol(),
                        Protocol::OpenAiChat | Protocol::OpenAiResponses
                    )
                {
                    return Err(RelayError::Unsupported(
                        "twelve-hour cache pricing requires an OpenAI protocol route",
                    ));
                }
                tariff.input_semantics = if attempt.protocol() == Protocol::AnthropicMessages {
                    crate::routing::InputSemantics::AnthropicText
                } else {
                    crate::routing::InputSemantics::Inclusive
                };
                if tariff
                    .tariff
                    .compile()
                    .map_err(|_| RelayError::BillingUnavailable)?
                    .requires_cache_creation()
                {
                    tariff.missing_cache_creation_is_zero = self
                        .provider_policy
                        .missing_cache_creation_is_zero(&attempt);
                }
            }
            if !attempt.is_available()
                || attempt.payload_policy() != PayloadPolicy::Adapt
                || u128::from(attempt.timeout_ms()) < self.total_timeout.as_millis()
            {
                return Err(RelayError::RouteUnavailable);
            }
            let upstream = match attempt.protocol() {
                Protocol::OpenAiChat => UpstreamProtocol::Chat,
                Protocol::OpenAiResponses => UpstreamProtocol::Responses,
                Protocol::AnthropicMessages => UpstreamProtocol::Anthropic,
                Protocol::Gemini => UpstreamProtocol::Gemini,
                Protocol::SystemOne => {
                    return Err(RelayError::Unsupported(
                        "System One requires its native endpoint",
                    ));
                }
            };
            if request.client == ClientProtocol::Anthropic
                && upstream != UpstreamProtocol::Anthropic
            {
                return Err(RelayError::Unsupported(
                    "native Anthropic requires an upstream with disjoint cache usage receipts",
                ));
            }
            let reference = attempt.credential_ref();
            let secret = self
                .credentials
                .get(&(reference.id, reference.revision))
                .ok_or(RelayError::RouteUnavailable)?;
            let input_token_ceiling = self.provider_policy.reservation_input_bound(
                &attempt,
                self.policy.input_token_ceiling,
                request.max_output_tokens,
            )?;
            self.provider_policy
                .validate_billing_mode(&attempt, price.upstream_cost_group_ppm.is_some())?;
            if price.upstream_cost_group_ppm.is_some() && attempt.upstream_model() != price.model {
                return Err(RelayError::Unsupported(
                    "upstream-cost router cannot be mapped to another model",
                ));
            }
            let endpoint = operation_endpoint(
                attempt.endpoint(),
                upstream,
                attempt.upstream_model(),
                request.stream,
            )?;
            let route_id = format!(
                "r{}-t{}-u{}",
                attempt.route_id(),
                attempt.target_id(),
                attempt.upstream_id()
            );
            let attempt = Arc::new(attempt);
            let availability = attempt.clone();
            let route = Route::new(
                route_id.clone(),
                upstream,
                endpoint.as_str(),
                attempt.upstream_model().to_owned(),
                secret,
                attempt.quote().version(),
            )?
            .with_cache_creation_mode(price.tiered.as_ref().and_then(|t| t.cache_creation_mode))
            .with_availability_guard(Arc::new(move || availability.is_available()));
            let mut active = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let operation = active
                .get_mut(&context.request_id)
                .ok_or(RelayError::Forbidden)?;
            if operation.selected.is_some() {
                return Err(RelayError::Conflict);
            }
            operation.selected = Some(Selected {
                attempt,
                price,
                quote,
                route_id,
                input_token_ceiling,
            });
            Ok(route)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{
        MULTIPLIER_ONE, PriceConfig, PriceRule, PriceSnapshot, Speed, TokenRates,
    };

    #[test]
    fn provider_paths_preserve_prefix_and_never_take_client_queries() {
        for (protocol, tail) in [
            (UpstreamProtocol::Chat, "chat/completions"),
            (UpstreamProtocol::Responses, "responses"),
            (UpstreamProtocol::Anthropic, "messages"),
            (UpstreamProtocol::Gemini, "models/synthetic:generateContent"),
        ] {
            let endpoint = operation_endpoint(
                "https://api.example/prefix/v1/",
                protocol,
                "synthetic",
                false,
            )
            .unwrap();
            assert_eq!(
                endpoint.as_str(),
                format!("https://api.example/prefix/v1/{tail}")
            );
        }
        let stream = operation_endpoint(
            "https://api.example/v1beta",
            UpstreamProtocol::Gemini,
            "a/b?key=x",
            true,
        )
        .unwrap();
        assert_eq!(
            stream.path(),
            "/v1beta/models/a%2Fb%3Fkey=x:streamGenerateContent"
        );
        assert_eq!(stream.query(), Some("alt=sse"));
        assert!(
            operation_endpoint(
                "https://api.example/v1?key=x",
                UpstreamProtocol::Chat,
                "model",
                false
            )
            .is_err()
        );
        assert!(
            operation_endpoint(
                "https://10.0.0.1/v1",
                UpstreamProtocol::Chat,
                "model",
                false
            )
            .is_err()
        );
    }

    fn quote(input: u64, cache_write: u64, request: u64, multiplier: u32) -> PriceQuote {
        PriceSnapshot::compile(PriceConfig {
            version: 3,
            rules: vec![PriceRule {
                billing_mode: crate::routing::BillingMode::Tokens,
                tiered: None,
                id: 1,
                model: "test".into(),
                group: "default".into(),
                speed: Speed::Standard,
                rates: TokenRates {
                    input,
                    output: 4_000_000,
                    cache_read: 500_000,
                    cache_write,
                    request,
                },
                group_multiplier_ppm: multiplier,
                speed_multiplier_ppm: MULTIPLIER_ONE,
            }],
        })
        .unwrap()
        .quote("test", "default", Speed::Standard)
        .unwrap()
    }

    #[test]
    fn exact_integer_credits_and_multipliers_only() {
        let price =
            credit_price(quote(2_000_000, 0, 0, MULTIPLIER_ONE), "test", "default").unwrap();
        assert_eq!(
            (
                price.input_per_million,
                price.cached_per_million,
                price.output_per_million
            ),
            (1_000_000, 250_000, 2_000_000)
        );
        assert!(credit_price(quote(1, 0, 0, MULTIPLIER_ONE), "test", "default").is_err());
        assert!(credit_price(quote(2_000_000, 1, 0, MULTIPLIER_ONE), "test", "default").is_err());
        assert!(credit_price(quote(2_000_000, 0, 1, MULTIPLIER_ONE), "test", "default").is_err());
        assert!(credit_price(quote(2, 0, 0, 1), "test", "default").is_err());
        assert_eq!(
            credit_price(
                quote(2_000_000, 0, 0, MULTIPLIER_ONE * 2),
                "test",
                "default"
            )
            .unwrap()
            .input_per_million,
            2_000_000
        );
    }

    #[test]
    fn a07_exact_quote_matches_the_current_ledger_for_supported_prices() {
        for multiplier in [MULTIPLIER_ONE, MULTIPLIER_ONE * 2, MULTIPLIER_ONE * 3] {
            let price = quote(2_000_000, 0, 0, multiplier);
            let legacy = credit_price(price, "test", "default").unwrap();
            let frozen = price.with_credit_rate(CreditRate::new(1, 500_000).unwrap());
            for input in 0..=12_u64 {
                for cached in 0..=input {
                    for output in 0..=5_u64 {
                        let exact = frozen
                            .charge(
                                crate::routing::Usage::from_totals(input, output, cached, 0)
                                    .unwrap(),
                            )
                            .unwrap();
                        let amount = legacy
                            .cost(crate::billing::charging::Usage {
                                cache_creation: None,
                                image: None,
                                upstream_cost_atto_usd: None,
                                input: input as i64,
                                cached: cached as i64,
                                output: output as i64,
                            })
                            .unwrap();
                        assert_eq!(exact.0, amount as u64);
                        assert!(exact.0 <= frozen.reservation_ceiling(12, 5).unwrap().0);
                    }
                }
            }
        }
    }
}
