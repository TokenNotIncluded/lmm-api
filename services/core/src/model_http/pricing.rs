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
    pub token_price: Option<TokenPrice>,
    pub image_variants: Vec<ImageVariant>,
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
}
/// Decimal strings avoid integer precision loss in browser JSON parsers.
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct ExactFraction {
    pub numerator: String,
    pub denominator: String,
}
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct ImageVariant {
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
                let local = crate::relay::honeypot_network::literal_ip(
                    &reqwest::Url::parse(upstream.endpoint.as_str())
                        .map_err(|_| PricingError::Unavailable)?,
                )
                .is_some_and(|ip| ip.is_loopback());
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
                    token_price: Some(token_price(quote)?),
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
                        token_price: None,
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
        rate_basis: "effective_group_rate",
        group_multiplier: decimal(u128::from(g), 6)?,
        input: rate(r.input)?,
        output: rate(r.output)?,
        cache_read: rate(r.cache_read)?,
        cache_write: None,
        request: None,
        final_cost_depends_on_usage: true,
    })
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
            tiers: vec![
                TokenTier {
                    up_to_input_tokens: Some(9),
                    rates: TieredRates {
                        input: rate(5, 4),
                        output: rate(10, 1),
                        cache_read: rate(1, 8),
                        cache_write: rate(0, 1),
                    },
                },
                TokenTier {
                    up_to_input_tokens: None,
                    rates: TieredRates {
                        input: rate(5, 2),
                        output: rate(15, 1),
                        cache_read: rate(1, 4),
                        cache_write: rate(0, 1),
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
            pool,
            gateway.bridge.charging.clone(),
            ImagePriceConfig {
                revision: 23,
                rules: vec![rule, edit],
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
