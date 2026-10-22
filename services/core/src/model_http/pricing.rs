//! Host-created Web pricing projection. Never OAuth authority, routing input,
//! a supplier health probe, or a replacement for a request's frozen quote.
use super::ModelGateway;
use crate::routing::{BillingMode, PayloadPolicy, PriceQuote, PriceSnapshot, Protocol, Speed};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::Number;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
pub enum PricingError {
    Unavailable,
}
#[derive(Clone, Serialize)]
pub struct PricingModel {
    pub id: String,
    pub model: String,
    pub group: String,
    pub apis: Vec<&'static str>,
    pub billing_status: &'static str,
    /// Text-route restriction; independently available image APIs may coexist.
    pub unavailable_reason: Option<UnavailableReason>,
    pub token_price: Option<TokenPrice>,
    pub upstream_cost_price: Option<UpstreamCostPrice>,
    pub image_variants: Vec<ImageVariant>,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    UpstreamBudgetContractRequired,
    RoutingUnavailable,
}
/// A calculation rule, never a promise that the supplier can enforce a budget.
/// The final upstream total already includes its token, cache and tool charges.
#[derive(Clone, Serialize)]
pub struct UpstreamCostPrice {
    pub currency: &'static str,
    pub cost_source: &'static str,
    pub credits_per_usd: i64,
    pub rounding: &'static str,
    pub rate_basis: &'static str,
    pub multiplier: ExactFraction,
}
#[derive(Clone, Serialize)]
pub struct TokenPrice {
    pub currency: &'static str,
    pub unit: &'static str,
    pub rate_basis: &'static str,
    pub group_multiplier: Number,
    pub input: Option<Number>,
    pub output: Option<Number>,
    pub cache_read: Option<Number>,
    pub cache_write: Option<Number>,
    pub request: Option<Number>,
    pub final_cost_depends_on_usage: bool,
    /// Revision 4 prices remain exact and tiered; no headline scalar is inferred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_12h_tiers: Option<Vec<CacheWrite12hTier>>,
}
#[derive(Clone, Serialize)]
pub struct CacheWrite12hTier {
    pub up_to_input_tokens: Option<String>,
    pub input: ExactFraction,
    pub output: ExactFraction,
    pub cache_read: ExactFraction,
    pub cache_write: ExactFraction,
    pub cache_write_12h: ExactFraction,
}
impl CacheWrite12hTier {
    fn apply_discount(
        &mut self,
        discount: crate::billing::trust_pricing::DiscountRatio,
    ) -> Result<(), PricingError> {
        for rate in [
            &mut self.input,
            &mut self.output,
            &mut self.cache_read,
            &mut self.cache_write,
            &mut self.cache_write_12h,
        ] {
            *rate = discounted_fraction(rate, discount)?;
        }
        Ok(())
    }
}
/// Decimal strings avoid integer precision loss in browser JSON parsers.
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct ExactFraction {
    pub numerator: String,
    pub denominator: String,
}
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct ImageVariant {
    pub rate_basis: &'static str,
    pub endpoint: super::image_price::ImageEndpoint,
    pub size: String,
    pub quality: String,
    pub max_n: u32,
    pub count_basis: super::image_price::ImageCountBasis,
    pub usd_micros_per_image: ExactFraction,
    pub credits_per_image: ExactFraction,
    pub reservation_credits_for_one: i64,
    pub rounding: &'static str,
    pub pricing_version: u64,
}
#[derive(Clone)]
pub struct PricingSurface {
    trust: crate::billing::charging::Charging,
    router: Arc<crate::routing::Router>,
    routing_version: u64,
    pricing_version: u64,
    image_pricing_version: Option<u64>,
    models: Vec<PricingModel>,
}
#[derive(Clone, Serialize)]
pub struct PricingProjection {
    pub routing_version: u64,
    pub pricing_version: u64,
    pub image_pricing_version: Option<u64>,
    pub groups: Vec<String>,
    pub models: Vec<PricingModel>,
}
impl PricingSurface {
    pub(crate) fn trust_pricing_enabled(&self) -> bool {
        self.trust.trust_pricing_enabled()
    }

    pub fn from_gateway(gateway: &ModelGateway) -> Result<Self, PricingError> {
        let router = gateway.bridge.routing.clone();
        let doc = router
            .catalog_document()
            .map_err(|_| PricingError::Unavailable)?;
        let routing_version = doc.routing.version;
        let pricing_version = doc.pricing.version;
        let prices = PriceSnapshot::compile(doc.pricing).map_err(|_| PricingError::Unavailable)?;
        let upstreams: BTreeMap<_, _> = doc.routing.upstreams.iter().map(|u| (u.id, u)).collect();
        let enabled: BTreeSet<_> = doc
            .routing
            .groups
            .iter()
            .filter(|g| g.enabled)
            .map(|g| g.name.as_str())
            .collect();
        let mut models = Vec::new();
        for route in doc.routing.routes.iter().filter(|r| {
            r.enabled && enabled.contains(r.group.as_str()) && r.speeds.contains(&Speed::Standard)
        }) {
            let quote = prices
                .quote(&route.model, &route.group, Speed::Standard)
                .ok_or(PricingError::Unavailable)?;
            let price_ok = super::routing::credit_price(quote, &route.model, &route.group).is_ok();
            let mut apis = BTreeSet::new();
            let mut upstream_budget_contract_required = false;
            for target in route.targets.iter().filter(|t| {
                t.enabled
                    && t.weight > 0
                    && t.speeds.contains(&Speed::Standard)
                    && t.payload == PayloadPolicy::Adapt
            }) {
                let Some(upstream) = upstreams.get(&target.upstream_id).filter(|u| u.enabled)
                else {
                    continue;
                };
                if quote.tiered().is_some_and(|tariff| tariff.revision() == 4)
                    && !matches!(
                        upstream.protocol,
                        Protocol::OpenAiChat | Protocol::OpenAiResponses
                    )
                {
                    continue;
                }
                let local = crate::relay::honeypot_network::literal_ip(
                    &reqwest::Url::parse(upstream.endpoint.as_str())
                        .map_err(|_| PricingError::Unavailable)?,
                )
                .is_some_and(|ip| ip.is_loopback());
                if quote.billing_mode() == BillingMode::UpstreamCost && !local {
                    upstream_budget_contract_required = true;
                }
                let Some(secret) = gateway
                    .bridge
                    .credentials
                    .get(&(upstream.credential.id, upstream.credential.revision))
                else {
                    continue;
                };
                if u128::from(target.timeout_ms) < gateway.bridge.total_timeout.as_millis()
                    || secret.is_empty()
                {
                    continue;
                }
                if gateway
                    .bridge
                    .provider_policy
                    .validate_binding(
                        upstream.id,
                        &target.upstream_model,
                        upstream.protocol,
                        upstream.endpoint.as_str(),
                        gateway.bridge.policy.input_token_ceiling,
                        1,
                    )
                    .is_err()
                {
                    continue;
                }
                if quote.billing_mode() == BillingMode::UpstreamCost
                    && (!local || target.upstream_model != route.model)
                {
                    continue;
                }
                let header = match upstream.protocol {
                    Protocol::Gemini => secret.clone(),
                    _ => format!("Bearer {secret}"),
                };
                if reqwest::header::HeaderValue::from_str(&header).is_err() {
                    continue;
                }
                if upstream.protocol == Protocol::SystemOne && !quote.is_input_only_scalar() {
                    continue;
                }
                apis.insert(match upstream.protocol {
                    Protocol::OpenAiChat => "openai-completions",
                    Protocol::OpenAiResponses => "openai-responses",
                    Protocol::AnthropicMessages => "anthropic-messages",
                    Protocol::Gemini => "google-generative-ai",
                    Protocol::SystemOne => "system-one",
                });
            }
            if !price_ok {
                apis.clear();
            }
            let status = if !price_ok || apis.is_empty() {
                "unavailable"
            } else if matches!(
                quote.billing_mode(),
                BillingMode::UpstreamCost | BillingMode::TieredDecimal
            ) {
                "usage_dependent"
            } else {
                "configured"
            };
            let unavailable_reason =
                (status == "unavailable").then_some(if upstream_budget_contract_required {
                    UnavailableReason::UpstreamBudgetContractRequired
                } else {
                    UnavailableReason::RoutingUnavailable
                });
            let (token_price, upstream_cost_price) =
                if quote.billing_mode() == BillingMode::UpstreamCost {
                    (None, Some(upstream_cost_price(quote)))
                } else {
                    (Some(token_price(quote)?), None)
                };
            for name in std::iter::once(route.model.as_str()).chain(
                doc.routing
                    .aliases
                    .iter()
                    .filter(|a| a.group == route.group && a.model == route.model)
                    .map(|a| a.alias.as_str()),
            ) {
                models.push(PricingModel {
                    id: model_id(&route.group, name),
                    model: name.into(),
                    group: route.group.clone(),
                    apis: apis.iter().copied().collect(),
                    billing_status: status,
                    unavailable_reason,
                    token_price: token_price.clone(),
                    upstream_cost_price: upstream_cost_price.clone(),
                    image_variants: Vec::new(),
                });
            }
        }
        let image_pricing_version = if let Some(images) = gateway.image_service() {
            let (version, variants) = images.pricing_projection()?;
            for (group, model, variant) in variants {
                if !enabled.contains(group.as_str()) {
                    continue;
                }
                if let Some(existing) = models
                    .iter_mut()
                    .find(|m| m.group == group && m.model == model)
                {
                    add_image(existing, variant);
                } else {
                    let mut entry = PricingModel {
                        id: model_id(&group, &model),
                        model,
                        group,
                        apis: Vec::new(),
                        billing_status: "configured",
                        unavailable_reason: None,
                        token_price: None,
                        upstream_cost_price: None,
                        image_variants: Vec::new(),
                    };
                    add_image(&mut entry, variant);
                    models.push(entry);
                }
            }
            Some(version)
        } else {
            None
        };
        if models.len() > 4096 {
            return Err(PricingError::Unavailable);
        }
        models.sort_by(|a, b| (&a.group, &a.model).cmp(&(&b.group, &b.model)));
        let result = Self {
            trust: gateway.bridge.charging.clone(),
            router,
            routing_version,
            pricing_version,
            image_pricing_version,
            models,
        };
        result.check_version()?;
        Ok(result)
    }
    fn check_version(&self) -> Result<(), PricingError> {
        let v = self
            .router
            .version()
            .map_err(|_| PricingError::Unavailable)?;
        if v.routing != self.routing_version || v.pricing != self.pricing_version {
            Err(PricingError::Unavailable)
        } else {
            Ok(())
        }
    }
    /// Filtering facts must be derived by the server's current read transaction.
    pub(crate) fn project(
        &self,
        groups: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<PricingProjection, PricingError> {
        self.check_version()?;
        let selected: Vec<_> = self
            .models
            .iter()
            .filter(|m| {
                groups.is_none_or(|g| g.contains(&m.group))
                    && models.is_none_or(|names| names.contains(&m.model))
            })
            .cloned()
            .collect();
        let result = PricingProjection {
            routing_version: self.routing_version,
            pricing_version: self.pricing_version,
            image_pricing_version: self.image_pricing_version,
            groups: selected
                .iter()
                .map(|m| m.group.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            models: selected,
        };
        if serde_json::to_vec(&result)
            .map_err(|_| PricingError::Unavailable)?
            .len()
            > 2 * 1024 * 1024
        {
            return Err(PricingError::Unavailable);
        }
        self.check_version()?;
        Ok(result)
    }
}
fn add_image(model: &mut PricingModel, variant: ImageVariant) {
    if model.billing_status == "unavailable" {
        model.billing_status = "configured";
    }
    let api = match variant.endpoint {
        super::image_price::ImageEndpoint::Generations => "openai-images-generations",
        super::image_price::ImageEndpoint::Edits => "openai-images-edits",
    };
    if !model.apis.contains(&api) {
        model.apis.push(api)
    }
    if !model.image_variants.contains(&variant) {
        model.image_variants.push(variant)
    }
}
fn model_id(group: &str, model: &str) -> String {
    format!(
        "lmm:{}:{}",
        URL_SAFE_NO_PAD.encode(group.as_bytes()),
        URL_SAFE_NO_PAD.encode(model.as_bytes())
    )
}
fn decimal(value: u128, scale: u32) -> Result<Number, PricingError> {
    let divisor = 10_u128.pow(scale);
    let whole = value / divisor;
    let tail = value % divisor;
    let text = if tail == 0 {
        whole.to_string()
    } else {
        format!(
            "{whole}.{}",
            format!("{tail:0width$}", width = scale as usize).trim_end_matches('0')
        )
    };
    text.parse().map_err(|_| PricingError::Unavailable)
}
fn token_price(quote: PriceQuote) -> Result<TokenPrice, PricingError> {
    let (g, s) = quote.multipliers_ppm();
    let known = quote.billing_mode() == BillingMode::Tokens && quote.tiered().is_none();
    let rate = |v: u64| {
        if known {
            decimal(u128::from(v) * u128::from(g) * u128::from(s), 18).map(Some)
        } else {
            Ok(None)
        }
    };
    let r = quote.rates();
    Ok(TokenPrice {
        currency: "USD",
        unit: "million_tokens",
        rate_basis: "base_group_rate",
        group_multiplier: decimal(u128::from(g), 6)?,
        input: rate(r.input)?,
        output: rate(r.output)?,
        cache_read: rate(r.cache_read)?,
        cache_write: None,
        request: None,
        final_cost_depends_on_usage: true,
        cache_write_12h_tiers: cache_write_12h_tiers(quote)?,
    })
}

fn cache_write_12h_tiers(
    quote: PriceQuote,
) -> Result<Option<Vec<CacheWrite12hTier>>, PricingError> {
    use num_bigint::BigUint;
    use num_rational::Ratio;
    let Some(tariff) = quote.tiered().filter(|tariff| tariff.revision() == 4) else {
        return Ok(None);
    };
    let (group, speed) = quote.multipliers_ppm();
    let rate = |value: crate::routing::DecimalRate| -> Result<ExactFraction, PricingError> {
        if value.denominator == 0 {
            return Err(PricingError::Unavailable);
        }
        let fraction = Ratio::new(
            BigUint::from(value.numerator) * BigUint::from(group) * BigUint::from(speed),
            BigUint::from(value.denominator) * BigUint::from(1_000_000_000_000_u64),
        );
        Ok(ExactFraction {
            numerator: fraction.numer().to_string(),
            denominator: fraction.denom().to_string(),
        })
    };
    tariff
        .tariff()
        .tiers
        .into_iter()
        .map(|tier| {
            Ok(CacheWrite12hTier {
                up_to_input_tokens: tier.up_to_input_tokens.map(|bound| bound.to_string()),
                input: rate(tier.rates.input)?,
                output: rate(tier.rates.output)?,
                cache_read: rate(tier.rates.cache_read)?,
                cache_write: rate(tier.rates.cache_write)?,
                cache_write_12h: rate(
                    tier.rates
                        .cache_write_12h
                        .ok_or(PricingError::Unavailable)?,
                )?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn upstream_cost_price(quote: PriceQuote) -> UpstreamCostPrice {
    UpstreamCostPrice {
        currency: "USD",
        cost_source: "usage.cost",
        credits_per_usd: crate::billing::CREDITS_PER_USD,
        rounding: "ceil_total_credits",
        rate_basis: "base_group_rate",
        multiplier: ExactFraction {
            numerator: quote.multipliers_ppm().0.to_string(),
            denominator: "1000000".into(),
        },
    }
}

pub(crate) fn image_variant(
    rule: &super::image_price::ImagePriceRule,
    quote: &super::image_price::ImageQuote,
) -> Result<ImageVariant, PricingError> {
    let n = u128::from(rule.usd_micros_per_image.numerator)
        .checked_mul(u128::from(rule.group_multiplier.numerator))
        .ok_or(PricingError::Unavailable)?;
    let d = u128::from(rule.usd_micros_per_image.denominator)
        .checked_mul(u128::from(rule.group_multiplier.denominator))
        .ok_or(PricingError::Unavailable)?;
    let credits_n = n
        .checked_mul(crate::billing::CREDITS_PER_USD as u128)
        .ok_or(PricingError::Unavailable)?;
    let credits_d = d.checked_mul(1_000_000).ok_or(PricingError::Unavailable)?;
    Ok(ImageVariant {
        rate_basis: "base_group_rate",
        endpoint: rule.endpoint,
        size: rule.size.clone(),
        quality: rule.quality.clone(),
        max_n: rule.max_n,
        count_basis: rule.count_basis,
        usd_micros_per_image: ExactFraction {
            numerator: n.to_string(),
            denominator: d.to_string(),
        },
        credits_per_image: ExactFraction {
            numerator: credits_n.to_string(),
            denominator: credits_d.to_string(),
        },
        reservation_credits_for_one: quote.reservation_credits().into(),
        rounding: "ceil_total_credits",
        pricing_version: quote.revision(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::routing::*;
    use std::time::Duration;
    pub(crate) fn gateway(pool: sqlx::PgPool) -> ModelGateway {
        let mut upstreams = Vec::new();
        let mut routes = Vec::new();
        let mut rules = Vec::new();
        for (index, (name, protocol)) in [
            ("model", Protocol::OpenAiChat),
            ("responses", Protocol::OpenAiResponses),
            ("claude", Protocol::AnthropicMessages),
            ("gemini", Protocol::Gemini),
            ("jev", Protocol::SystemOne),
        ]
        .into_iter()
        .enumerate()
        {
            let id = index as u64 + 1;
            upstreams.push(UpstreamConfig {
                id,
                endpoint: Endpoint::local_loopback("http://127.0.0.1:9/v1".into()).unwrap(),
                protocol,
                credential: CredentialRef { id: 1, revision: 1 },
                enabled: true,
            });
            routes.push(ModelRoute {
                id,
                model: name.into(),
                group: "allowed".into(),
                enabled: true,
                speeds: vec![Speed::Standard],
                targets: vec![RouteTarget {
                    id,
                    upstream_id: id,
                    upstream_model: format!("provider-{name}"),
                    enabled: true,
                    priority: 0,
                    weight: 1,
                    speeds: vec![Speed::Standard],
                    payload: PayloadPolicy::Adapt,
                    timeout_ms: 5000,
                }],
                failover: FailoverPolicy::default(),
            });
            rules.push(PriceRule {
                billing_mode: BillingMode::Tokens,
                tiered: None,
                id,
                model: name.into(),
                group: "allowed".into(),
                speed: Speed::Standard,
                rates: TokenRates {
                    input: 2_000_000,
                    output: if protocol == Protocol::SystemOne {
                        0
                    } else {
                        4_000_000
                    },
                    cache_read: 0,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: 1_250_000,
                speed_multiplier_ppm: 1_000_000,
            });
        }
        let mut private = routes[0].clone();
        private.id = 99;
        private.group = "private".into();
        private.targets[0].id = 99;
        routes.push(private);
        let mut rate = rules[0].clone();
        rate.id = 99;
        rate.group = "private".into();
        rate.group_multiplier_ppm = 2_000_000;
        rules.push(rate);
        let routing = RoutingConfig {
            version: 9,
            groups: vec![
                ModelGroup {
                    name: "allowed".into(),
                    enabled: true,
                },
                ModelGroup {
                    name: "private".into(),
                    enabled: true,
                },
            ],
            upstreams,
            routes,
            aliases: vec![ModelAlias {
                alias: "friendly".into(),
                model: "model".into(),
                group: "allowed".into(),
            }],
        };
        let router = Arc::new(
            Router::new(
                routing,
                Arc::new(PriceSnapshot::compile(PriceConfig { version: 11, rules }).unwrap()),
            )
            .unwrap(),
        );
        let limits = crate::relay::Limits {
            total_timeout: Duration::from_secs(1),
            hook_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        ModelGateway::new(
            pool,
            router,
            vec![super::super::UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: "synthetic-pricing-private-provider-credential".into(),
            }],
            BTreeMap::new(),
            limits,
            super::super::ChargePolicy {
                input_token_ceiling: 65536,
                lease_seconds: 10,
            },
        )
        .unwrap()
    }
    pub(crate) fn upstream_cost_gateway(pool: sqlx::PgPool, external: bool) -> ModelGateway {
        let initial = gateway(pool.clone());
        let mut doc = initial.bridge.routing.catalog_document().unwrap();
        let source_route = doc.routing.routes[0].clone();
        let source_price = doc.pricing.rules[0].clone();
        doc.routing.upstreams.truncate(1);
        if external {
            doc.routing.upstreams[0].endpoint =
                Endpoint::try_from("https://private-cost-provider.invalid/v1".to_owned()).unwrap();
        }
        doc.routing.routes.clear();
        doc.pricing.rules.clear();
        let names = [
            "nvidia/switchyard",
            "openrouter/auto-beta",
            "openrouter/fusion",
            "typesafe/jev-router",
        ];
        for (index, name) in names.into_iter().chain(["openrouter/fusion"]).enumerate() {
            let id = index as u64 + 1;
            let group = if index == 4 { "private" } else { "allowed" };
            let mut route = source_route.clone();
            route.id = id;
            route.model = name.into();
            route.group = group.into();
            route.targets[0].id = id;
            route.targets[0].upstream_model = name.into();
            doc.routing.routes.push(route);
            let mut price = source_price.clone();
            price.id = id;
            price.model = name.into();
            price.group = group.into();
            price.billing_mode = BillingMode::UpstreamCost;
            price.rates = TokenRates {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            };
            price.group_multiplier_ppm = if group == "private" {
                2_000_000
            } else {
                1_250_000
            };
            doc.pricing.rules.push(price);
        }
        doc.routing.aliases = vec![ModelAlias {
            alias: "friendly-router".into(),
            model: "openrouter/fusion".into(),
            group: "allowed".into(),
        }];
        let contexts = if external {
            names
                .into_iter()
                .map(|name| super::super::ExternalModelContext {
                    upstream_id: 1,
                    upstream_model: name.into(),
                    protocol: Protocol::OpenAiChat,
                    endpoint: doc.routing.upstreams[0].endpoint.as_str().into(),
                    max_input_tokens: 65536,
                    max_output_tokens: 1000,
                    contract_revision: "synthetic-context-without-supplier-budget".into(),
                    missing_cached_tokens_is_zero: true,
                    missing_cache_creation_tokens_is_zero: false,
                })
                .collect()
        } else {
            Vec::new()
        };
        let policy = super::super::ProviderPolicy::new(contexts, 65536).unwrap();
        let router = Arc::new(
            Router::new(
                doc.routing,
                Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
            )
            .unwrap(),
        );
        ModelGateway::new_with_provider_policy(
            pool,
            router,
            vec![super::super::UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: "synthetic-upstream-cost-private-credential".into(),
            }],
            BTreeMap::new(),
            initial.limits.clone(),
            initial.bridge.policy,
            policy,
        )
        .unwrap()
    }
    #[tokio::test]
    async fn pricing_upstream_cost_descriptor_keeps_external_budget_gate_and_exact_group_basis() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        for external in [false, true] {
            let gateway = upstream_cost_gateway(pool.clone(), external);
            let surface = PricingSurface::from_gateway(&gateway).unwrap();
            let public = surface.project(None, None).unwrap();
            let wire = serde_json::to_value(&public).unwrap();
            assert_eq!(public.models.len(), 6);
            for model in &public.models {
                assert!(model.token_price.is_none());
                let price = model.upstream_cost_price.as_ref().unwrap();
                assert_eq!(price.currency, "USD");
                assert_eq!(price.cost_source, "usage.cost");
                assert_eq!(price.credits_per_usd, 500_000);
                assert_eq!(price.rounding, "ceil_total_credits");
                assert_eq!(price.rate_basis, "base_group_rate");
                assert_eq!(price.multiplier.denominator, "1000000");
                assert_eq!(
                    price.multiplier.numerator,
                    if model.group == "private" {
                        "2000000"
                    } else {
                        "1250000"
                    }
                );
                if external {
                    assert_eq!(model.billing_status, "unavailable");
                    assert_eq!(
                        model.unavailable_reason,
                        Some(UnavailableReason::UpstreamBudgetContractRequired)
                    );
                    assert!(model.apis.is_empty());
                } else {
                    assert_eq!(model.billing_status, "usage_dependent");
                    assert!(model.unavailable_reason.is_none());
                    assert_eq!(model.apis, ["openai-completions"]);
                }
            }
            let encoded = wire.to_string();
            for private in [
                "private-credential",
                "127.0.0.1",
                "private-cost-provider",
                "synthetic-context",
            ] {
                assert!(!encoded.contains(private));
            }
            let allowed = surface.project(Some(&["allowed".into()]), None).unwrap();
            assert_eq!(allowed.models.len(), 5);
            assert!(allowed.models.iter().all(|m| m.group == "allowed"));
            let alias = surface
                .project(Some(&["allowed".into()]), Some(&["friendly-router".into()]))
                .unwrap();
            assert_eq!(alias.models.len(), 1);
            assert_eq!(alias.models[0].model, "friendly-router");
            assert_eq!(
                serde_json::to_value(&alias.models[0].upstream_cost_price).unwrap(),
                wire["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["model"] == "openrouter/fusion" && m["group"] == "allowed")
                    .unwrap()["upstream_cost_price"]
            );
        }
    }
    #[tokio::test]
    async fn pricing_safe_projection_retains_effective_group_rates_and_native_protocols() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let gateway = gateway(pool);
        let surface = PricingSurface::from_gateway(&gateway).unwrap();
        let public = surface.project(None, None).unwrap();
        let wire = serde_json::to_value(&public).unwrap();
        let allowed = public
            .models
            .iter()
            .find(|m| m.model == "model" && m.group == "allowed")
            .unwrap();
        assert_eq!(
            allowed
                .token_price
                .as_ref()
                .unwrap()
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "2.5"
        );
        let private = public
            .models
            .iter()
            .find(|m| m.model == "model" && m.group == "private")
            .unwrap();
        assert_eq!(
            private
                .token_price
                .as_ref()
                .unwrap()
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "4"
        );
        assert!(
            public
                .models
                .iter()
                .any(|m| m.apis.contains(&"anthropic-messages"))
        );
        assert!(
            public
                .models
                .iter()
                .any(|m| m.apis.contains(&"google-generative-ai"))
        );
        assert_eq!(
            public
                .models
                .iter()
                .find(|m| m.model == "jev")
                .unwrap()
                .apis,
            ["system-one"]
        );
        assert!(
            wire.to_string()
                .find("synthetic-pricing-private-provider-credential")
                .is_none()
        );
        assert!(!wire.to_string().contains("127.0.0.1"));
        assert!(!wire.to_string().contains("provider-model"));
        let own = surface
            .project(Some(&["allowed".into()]), Some(&["friendly".into()]))
            .unwrap();
        assert_eq!(own.models.len(), 1);
        assert_eq!(own.models[0].model, "friendly");
        assert!(surface.project(Some(&[]), None).unwrap().models.is_empty());
    }
    #[tokio::test]
    async fn pricing_tiered_decimal_projection_never_displays_placeholder_zero_as_free() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let initial = gateway(pool.clone());
        let mut doc = initial.bridge.routing.catalog_document().unwrap();
        let rate = |numerator, denominator| DecimalRate {
            numerator,
            denominator,
        };
        // Same actual validated tariff as tests/tiered_token_billing.rs.
        let rule = doc.pricing.rules.iter_mut().find(|r| r.id == 1).unwrap();
        rule.billing_mode = BillingMode::TieredDecimal;
        rule.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        rule.tiered = Some(TieredTariff {
            revision: 1,
            source_digest: "23".repeat(32),
            uses_cache_read: true,
            uses_cache_write: false,
            uses_cache_write_1h: false,
            uses_cache_write_12h: false,
            tiers: vec![
                TokenTier {
                    up_to_input_tokens: Some(9),
                    rates: TieredRates {
                        input: rate(5, 4),
                        output: rate(10, 1),
                        cache_read: rate(1, 8),
                        cache_write: rate(0, 1),
                        cache_write_1h: None,
                        cache_write_12h: None,
                    },
                },
                TokenTier {
                    up_to_input_tokens: None,
                    rates: TieredRates {
                        input: rate(5, 2),
                        output: rate(15, 1),
                        cache_read: rate(1, 4),
                        cache_write: rate(0, 1),
                        cache_write_1h: None,
                        cache_write_12h: None,
                    },
                },
            ],
        });
        let router = Arc::new(
            Router::new(
                doc.routing,
                Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
            )
            .unwrap(),
        );
        let gateway = ModelGateway::new(
            pool,
            router,
            vec![super::super::UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: "synthetic-tiered-pricing-credential".into(),
            }],
            BTreeMap::new(),
            initial.limits.clone(),
            initial.bridge.policy,
        )
        .unwrap();
        let public = PricingSurface::from_gateway(&gateway)
            .unwrap()
            .project(None, None)
            .unwrap();
        for model in public
            .models
            .iter()
            .filter(|m| m.group == "allowed" && matches!(m.model.as_str(), "model" | "friendly"))
        {
            assert_eq!(model.billing_status, "usage_dependent");
            assert_eq!(model.apis, ["openai-completions"]);
            let price = model.token_price.as_ref().unwrap();
            assert!(price.input.is_none());
            assert!(price.output.is_none());
            assert!(price.cache_read.is_none());
            assert!(price.cache_write.is_none());
            assert!(price.request.is_none());
            assert!(price.final_cost_depends_on_usage);
        }
        assert_eq!(
            public
                .models
                .iter()
                .filter(|m| {
                    m.group == "allowed" && matches!(m.model.as_str(), "model" | "friendly")
                })
                .count(),
            2
        );
    }
    #[tokio::test]
    async fn pricing_systemone_scalar_decimal_is_visible_but_not_a_free_placeholder() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let initial = gateway(pool.clone());
        let mut doc = initial.bridge.routing.catalog_document().unwrap();
        let rule = doc
            .pricing
            .rules
            .iter_mut()
            .find(|r| r.model == "jev")
            .unwrap();
        rule.billing_mode = BillingMode::TieredDecimal;
        rule.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        rule.tiered = Some(serde_json::from_value(serde_json::json!({
            "revision":1,"source_digest":"af0d4b7d188d58f20f9eb13cef18353659ca208151824014795189b70dc0afd7",
            "uses_cache_read":false,"uses_cache_write":false,
            "tiers":[{"rates":{"input":{"numerator":21,"denominator":500},"output":{"numerator":0,"denominator":1},"cache_read":{"numerator":0,"denominator":1},"cache_write":{"numerator":0,"denominator":1}}}]
        })).unwrap());
        for accepted in [true, false] {
            let mut prices = doc.pricing.clone();
            if !accepted {
                prices
                    .rules
                    .iter_mut()
                    .find(|r| r.model == "jev")
                    .unwrap()
                    .tiered
                    .as_mut()
                    .unwrap()
                    .tiers[0]
                    .rates
                    .output
                    .numerator = 1;
            }
            let router = Arc::new(
                Router::new(
                    doc.routing.clone(),
                    Arc::new(PriceSnapshot::compile(prices).unwrap()),
                )
                .unwrap(),
            );
            let gateway = ModelGateway::new(
                pool.clone(),
                router,
                vec![super::super::UpstreamCredential {
                    reference: CredentialRef { id: 1, revision: 1 },
                    secret: "synthetic-decimal-projection".into(),
                }],
                BTreeMap::new(),
                initial.limits.clone(),
                initial.bridge.policy,
            )
            .unwrap();
            let projection = PricingSurface::from_gateway(&gateway)
                .unwrap()
                .project(None, None)
                .unwrap();
            let jev = projection.models.iter().find(|m| m.model == "jev").unwrap();
            if accepted {
                assert_eq!(jev.billing_status, "usage_dependent");
                assert_eq!(jev.apis, ["system-one"]);
            } else {
                assert_eq!(jev.billing_status, "unavailable");
                assert!(jev.apis.is_empty());
            }
            let rates = jev.token_price.as_ref().unwrap();
            assert!(
                rates.input.is_none()
                    && rates.output.is_none()
                    && rates.cache_read.is_none()
                    && rates.cache_write.is_none()
                    && rates.request.is_none()
            );
        }
    }
    #[tokio::test]
    async fn pricing_external_context_and_release_freshness_are_real_boundaries() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let initial = gateway(pool.clone());
        let mut doc = initial.bridge.routing.catalog_document().unwrap();
        for upstream in &mut doc.routing.upstreams {
            upstream.endpoint =
                Endpoint::try_from("https://private-provider.invalid/v1".to_owned()).unwrap();
        }
        let router = Arc::new(
            Router::new(
                doc.routing.clone(),
                Arc::new(PriceSnapshot::compile(doc.pricing.clone()).unwrap()),
            )
            .unwrap(),
        );
        let credentials = || {
            vec![super::super::UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: "synthetic-private-price-credential".into(),
            }]
        };
        let without = ModelGateway::new(
            pool.clone(),
            router.clone(),
            credentials(),
            BTreeMap::new(),
            initial.limits.clone(),
            initial.bridge.policy,
        )
        .unwrap();
        assert!(
            PricingSurface::from_gateway(&without)
                .unwrap()
                .project(None, None)
                .unwrap()
                .models
                .iter()
                .all(|m| m.billing_status == "unavailable" && m.apis.is_empty())
        );
        let contexts = doc
            .routing
            .upstreams
            .iter()
            .map(|upstream| {
                let target = doc
                    .routing
                    .routes
                    .iter()
                    .flat_map(|r| r.targets.iter())
                    .find(|t| t.upstream_id == upstream.id)
                    .unwrap();
                super::super::ExternalModelContext {
                    missing_cache_creation_tokens_is_zero: false,
                    upstream_id: upstream.id,
                    upstream_model: target.upstream_model.clone(),
                    protocol: upstream.protocol,
                    endpoint: upstream.endpoint.as_str().into(),
                    max_input_tokens: 65536,
                    max_output_tokens: 1000,
                    contract_revision: "synthetic-only".into(),
                    missing_cached_tokens_is_zero: true,
                }
            })
            .collect();
        let policy = super::super::ProviderPolicy::new(contexts, 65536).unwrap();
        let configured = ModelGateway::new_with_provider_policy(
            pool,
            router.clone(),
            credentials(),
            BTreeMap::new(),
            initial.limits.clone(),
            initial.bridge.policy,
            policy,
        )
        .unwrap();
        let surface = PricingSurface::from_gateway(&configured).unwrap();
        let projected = surface.project(None, None).unwrap();
        assert!(
            projected
                .models
                .iter()
                .all(|m| m.billing_status == "configured")
        );
        let wire = serde_json::to_string(&projected).unwrap();
        assert!(!wire.contains("private-provider"));
        assert!(!wire.contains("private-price-credential"));
        doc.routing.version += 1;
        doc.pricing.version += 1;
        router
            .publish(
                crate::routing::ReleaseVersion {
                    routing: 9,
                    pricing: 11,
                },
                doc.routing,
                Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
            )
            .unwrap();
        assert!(surface.project(None, None).is_err());
    }

    #[tokio::test]
    async fn pricing_image_surface_uses_actual_unique_provider_tariffs() {
        use super::super::{
            image_execution::{ImageService, ProviderSpec},
            image_price::*,
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let gateway = gateway(pool.clone());
        let rule = ImagePriceRule {
            upstream_id: 11,
            vendor: "private-vendor".into(),
            billing_group: "allowed".into(),
            upstream_model: "private-paint-provider".into(),
            semantic_model: "dall-e-2".into(),
            endpoint: ImageEndpoint::Generations,
            size: "256x256".into(),
            quality: "standard".into(),
            usd_micros_per_image: ExactRatio {
                numerator: 3,
                denominator: 2,
            },
            group_multiplier: ExactRatio {
                numerator: 5,
                denominator: 4,
            },
            max_n: 4,
            count_basis: ImageCountBasis::Requested,
            max_budget: crate::billing::Credits::try_from(100).unwrap(),
            source_revision: "private-tariff".into(),
        };
        let mut edit = rule.clone();
        edit.endpoint = ImageEndpoint::Edits;
        let provider = ProviderSpec {
            upstream_id: 11,
            vendor: rule.vendor.clone(),
            upstream_model: rule.upstream_model.clone(),
            base_url: "http://127.0.0.1:9/v1".into(),
            secret: "synthetic-image-price-credential".into(),
            local_http: true,
        };
        let images = ImageService::new(
            pool.clone(),
            gateway.bridge.charging.clone(),
            ImagePriceConfig {
                revision: 23,
                rules: vec![rule.clone(), edit],
            },
            vec![provider],
            &gateway.limits,
        )
        .unwrap();
        let gateway = gateway.with_image_service(images);
        let surface = PricingSurface::from_gateway(&gateway).unwrap();
        let view = surface.project(Some(&["allowed".into()]), None).unwrap();
        let image = view.models.iter().find(|m| m.model == "dall-e-2").unwrap();
        assert!(image.token_price.is_none());
        assert_eq!(image.image_variants.len(), 2);
        assert!(image.apis.contains(&"openai-images-edits"));
        assert!(image.apis.contains(&"openai-images-generations"));
        assert_eq!(view.image_pricing_version, Some(23));
        let wire = serde_json::to_string(image).unwrap();
        assert!(!wire.contains("private-paint"));
        assert!(!wire.contains("private-vendor"));
        assert!(!wire.contains("synthetic-image-price"));

        // An independently available image API does not authorize the text
        // router or erase the reason its supplier billing is still blocked.
        let gateway = upstream_cost_gateway(pool.clone(), true);
        let mut shared_rule = rule;
        shared_rule.semantic_model = "openrouter/fusion".into();
        let provider = ProviderSpec {
            upstream_id: shared_rule.upstream_id,
            vendor: shared_rule.vendor.clone(),
            upstream_model: shared_rule.upstream_model.clone(),
            base_url: "http://127.0.0.1:9/v1".into(),
            secret: "synthetic-mixed-price-credential".into(),
            local_http: true,
        };
        let images = ImageService::new(
            pool,
            gateway.bridge.charging.clone(),
            ImagePriceConfig {
                revision: 24,
                rules: vec![shared_rule],
            },
            vec![provider],
            &gateway.limits,
        )
        .unwrap();
        let gateway = gateway.with_image_service(images);
        let mixed = PricingSurface::from_gateway(&gateway)
            .unwrap()
            .project(
                Some(&["allowed".into()]),
                Some(&["openrouter/fusion".into()]),
            )
            .unwrap();
        assert_eq!(mixed.models.len(), 1);
        let row = &mixed.models[0];
        assert_eq!(row.billing_status, "configured");
        assert_eq!(
            row.unavailable_reason,
            Some(UnavailableReason::UpstreamBudgetContractRequired)
        );
        assert_eq!(row.apis, ["openai-images-generations"]);
        assert_eq!(row.image_variants.len(), 1);
        assert!(row.token_price.is_none());
        assert!(row.upstream_cost_price.is_some());
    }

    #[test]
    fn pricing_image_rational_is_effective_not_a_token_price() {
        use super::super::image_price::*;
        let rule = ImagePriceRule {
            upstream_id: 1,
            vendor: "private-vendor".into(),
            billing_group: "allowed".into(),
            upstream_model: "provider-private".into(),
            semantic_model: "dall-e-2".into(),
            endpoint: ImageEndpoint::Generations,
            size: "256x256".into(),
            quality: "standard".into(),
            usd_micros_per_image: ExactRatio {
                numerator: 3,
                denominator: 2,
            },
            group_multiplier: ExactRatio {
                numerator: 5,
                denominator: 4,
            },
            max_n: 4,
            count_basis: ImageCountBasis::Requested,
            max_budget: crate::billing::Credits::try_from(100).unwrap(),
            source_revision: "private-tariff".into(),
        };
        let config = ImagePriceConfig {
            revision: 23,
            rules: vec![rule.clone()],
        };
        let quote = config
            .quote(&ImageQuoteRequest {
                upstream_id: 1,
                vendor: rule.vendor.clone(),
                billing_group: rule.billing_group.clone(),
                upstream_model: rule.upstream_model.clone(),
                semantic_model: rule.semantic_model.clone(),
                endpoint: rule.endpoint,
                size: rule.size.clone(),
                quality: rule.quality.clone(),
                n: 1,
            })
            .unwrap();
        let projected = image_variant(&rule, &quote).unwrap();
        assert_eq!(projected.reservation_credits_for_one, 1);
        assert_eq!(projected.usd_micros_per_image.numerator, "15");
        assert_eq!(projected.usd_micros_per_image.denominator, "8");
        let wire = serde_json::to_string(&projected).unwrap();
        assert!(!wire.contains("private"));
        assert_eq!(projected.pricing_version, 23);
    }
}

/// Display only: exact decimal when representable, otherwise unknown, not zero.
/// Funds continue to use the independently frozen rational quote.
pub(crate) fn scaled_number(
    value: Option<&Number>,
    discount: crate::billing::trust_pricing::DiscountRatio,
) -> Result<Option<Number>, PricingError> {
    use num_bigint::BigUint;
    use num_rational::Ratio;
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = value.to_string();
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().map_err(|_| PricingError::Unavailable)?),
        None => (raw.as_str(), 0),
    };
    let pieces: Vec<_> = mantissa.split('.').collect();
    if raw.len() > 1024
        || exponent.unsigned_abs() > 1024
        || pieces.len() > 2
        || !pieces
            .iter()
            .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(PricingError::Unavailable);
    }
    let digits = pieces.concat();
    let scale = i32::try_from(pieces.get(1).map_or(0, |s| s.len()))
        .map_err(|_| PricingError::Unavailable)?
        - exponent;
    let mut n: BigUint = digits.parse().map_err(|_| PricingError::Unavailable)?;
    let ten = BigUint::from(10_u8);
    let mut d = BigUint::from(discount.denominator());
    if scale >= 0 {
        d *= ten.pow(scale as u32);
    } else {
        n *= ten.pow(scale.unsigned_abs());
    }
    let r = Ratio::new(n * BigUint::from(discount.numerator()), d);
    decimal_ratio(&r)
}
fn decimal_ratio(
    r: &num_rational::Ratio<num_bigint::BigUint>,
) -> Result<Option<Number>, PricingError> {
    use num_bigint::BigUint;
    let zero = BigUint::from(0_u8);
    let one = BigUint::from(1_u8);
    let mut d = r.denom().clone();
    let mut powers = [0_u32; 2];
    for (i, prime) in [2_u8, 5].into_iter().enumerate() {
        let p = BigUint::from(prime);
        while &d % &p == zero {
            d /= &p;
            powers[i] += 1;
        }
    }
    if d != one {
        return Ok(None);
    }
    let scale = powers[0].max(powers[1]);
    if scale > 128 {
        return Err(PricingError::Unavailable);
    }
    let ten = BigUint::from(10_u8).pow(scale);
    let n = r.numer() * (&ten / r.denom());
    let whole = &n / &ten;
    let tail = &n % &ten;
    let text = if tail == zero {
        whole.to_string()
    } else {
        format!(
            "{whole}.{}",
            format!("{tail:0width$}", width = scale as usize).trim_end_matches('0')
        )
    };
    text.parse()
        .map(Some)
        .map_err(|_| PricingError::Unavailable)
}
fn discounted_fraction(
    value: &ExactFraction,
    discount: crate::billing::trust_pricing::DiscountRatio,
) -> Result<ExactFraction, PricingError> {
    use num_bigint::BigUint;
    use num_rational::Ratio;
    let n: BigUint = value
        .numerator
        .parse()
        .map_err(|_| PricingError::Unavailable)?;
    let d: BigUint = value
        .denominator
        .parse()
        .map_err(|_| PricingError::Unavailable)?;
    if d == BigUint::from(0_u8) {
        return Err(PricingError::Unavailable);
    }
    let r = Ratio::new(
        n * BigUint::from(discount.numerator()),
        d * BigUint::from(discount.denominator()),
    );
    Ok(ExactFraction {
        numerator: r.numer().to_string(),
        denominator: r.denom().to_string(),
    })
}
impl PricingProjection {
    pub(crate) fn apply_user_authority(
        &mut self,
        authority: &crate::billing::trust_pricing::UserPriceAuthority,
    ) -> Result<(), PricingError> {
        let discount = authority.discount();
        for row in &mut self.models {
            if let Some(upstream) = &mut row.upstream_cost_price {
                upstream.rate_basis = "effective_user_rate";
                upstream.multiplier = discounted_fraction(&upstream.multiplier, discount)?;
            }
            if let Some(token) = &mut row.token_price {
                token.rate_basis = "effective_user_rate";
                for v in [
                    &mut token.input,
                    &mut token.output,
                    &mut token.cache_read,
                    &mut token.cache_write,
                    &mut token.request,
                ] {
                    *v = scaled_number(v.as_ref(), discount)?;
                }
                if let Some(tiers) = &mut token.cache_write_12h_tiers {
                    for tier in tiers {
                        tier.apply_discount(discount)?;
                    }
                }
            }
            for image in &mut row.image_variants {
                image.rate_basis = "effective_user_rate";
                image.usd_micros_per_image =
                    discounted_fraction(&image.usd_micros_per_image, discount)?;
                image.credits_per_image = discounted_fraction(&image.credits_per_image, discount)?;
                use num_bigint::BigUint;
                let n: BigUint = image
                    .credits_per_image
                    .numerator
                    .parse()
                    .map_err(|_| PricingError::Unavailable)?;
                let d: BigUint = image
                    .credits_per_image
                    .denominator
                    .parse()
                    .map_err(|_| PricingError::Unavailable)?;
                image.reservation_credits_for_one = ((&n + &d - BigUint::from(1_u8)) / &d)
                    .to_string()
                    .parse()
                    .map_err(|_| PricingError::Unavailable)?;
            }
        }
        Ok(())
    }
    pub(crate) fn legacy_fixture_basis(&mut self) {
        for row in &mut self.models {
            if let Some(token) = &mut row.token_price {
                token.rate_basis = "effective_group_rate";
            }
            if let Some(upstream) = &mut row.upstream_cost_price {
                upstream.rate_basis = "effective_group_rate";
            }
        }
    }
}

#[cfg(test)]
mod user_rate_tests {
    use super::*;
    use crate::billing::trust_pricing::DiscountRatio;
    #[test]
    fn account_rates_scale_once_without_decimal_or_reservation_rounding() {
        let d = DiscountRatio::new(97, 100).unwrap();
        for (base, expected) in [
            ("0.3", "0.291"),
            ("1e-7", "0.000000097"),
            ("2E2", "194"),
            ("0", "0"),
            ("0.00000001", "0.0000000097"),
        ] {
            let number: Number = base.parse().unwrap();
            let got = scaled_number(Some(&number), d).unwrap().unwrap();
            assert_eq!(got.to_string(), expected);
        }
        assert!(scaled_number(None, d).unwrap().is_none());
        assert!(
            scaled_number(
                Some(&"1".parse().unwrap()),
                DiscountRatio::new(1, 3).unwrap()
            )
            .unwrap()
            .is_none()
        );
        let fraction = discounted_fraction(
            &ExactFraction {
                numerator: "101".into(),
                denominator: "100".into(),
            },
            d,
        )
        .unwrap();
        assert_eq!(fraction.numerator, "9797");
        assert_eq!(fraction.denominator, "10000");
        // ceil(1.01) * 0.97 is not the actual quote; exact discounted cost < 1.
        let n: num_bigint::BigUint = fraction.numerator.parse().unwrap();
        let den: num_bigint::BigUint = fraction.denominator.parse().unwrap();
        assert_eq!(
            ((&n + &den - num_bigint::BigUint::from(1_u8)) / den).to_string(),
            "1"
        );
    }
}

#[cfg(test)]
mod cache_write_12h_projection_tests {
    use super::*;
    use crate::routing::*;

    #[test]
    fn cache_write_12h_projection_keeps_tiers_and_exact_effective_rates() {
        let rate = |numerator, denominator| DecimalRate {
            numerator,
            denominator,
        };
        let prices = PriceSnapshot::compile(PriceConfig {
            version: 7,
            rules: vec![PriceRule {
                id: 1,
                model: "cache-model".into(),
                group: "default".into(),
                speed: Speed::Standard,
                billing_mode: BillingMode::TieredDecimal,
                rates: TokenRates {
                    input: 0,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: 1_250_000,
                speed_multiplier_ppm: 2_000_000,
                tiered: Some(TieredTariff {
                    revision: 4,
                    source_digest: "ab".repeat(32),
                    uses_cache_read: true,
                    uses_cache_write: true,
                    uses_cache_write_1h: false,
                    uses_cache_write_12h: true,
                    tiers: [Some(272_000), None]
                        .into_iter()
                        .enumerate()
                        .map(|(index, bound)| {
                            let scale = index as u64 + 1;
                            TokenTier {
                                up_to_input_tokens: bound,
                                rates: TieredRates {
                                    input: rate(scale, 1),
                                    output: rate(5 * scale, 1),
                                    cache_read: rate(0, 1),
                                    cache_write: rate(5 * scale, 4),
                                    cache_write_1h: None,
                                    cache_write_12h: Some(rate(5 * scale, 2)),
                                },
                            }
                        })
                        .collect(),
                }),
            }],
        })
        .unwrap();
        let projected = token_price(
            prices
                .quote("cache-model", "default", Speed::Standard)
                .unwrap(),
        )
        .unwrap();
        assert!(projected.input.is_none());
        assert!(projected.output.is_none());
        let mut tiers = projected.cache_write_12h_tiers.unwrap();
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].up_to_input_tokens.as_deref(), Some("272000"));
        assert!(tiers[1].up_to_input_tokens.is_none());
        assert_eq!(tiers[0].cache_write.numerator, "25");
        assert_eq!(tiers[0].cache_write.denominator, "8");
        assert_eq!(tiers[0].cache_write_12h.numerator, "25");
        assert_eq!(tiers[0].cache_write_12h.denominator, "4");
        assert_eq!(tiers[1].cache_write_12h.numerator, "25");
        assert_eq!(tiers[1].cache_write_12h.denominator, "2");
        tiers[0]
            .apply_discount(crate::billing::trust_pricing::DiscountRatio::new(97, 100).unwrap())
            .unwrap();
        assert_eq!(tiers[0].cache_write_12h.numerator, "97");
        assert_eq!(tiers[0].cache_write_12h.denominator, "16");
        assert_eq!(tiers[0].cache_read.numerator, "0");
        assert_eq!(tiers[0].cache_read.denominator, "1");
    }
}
