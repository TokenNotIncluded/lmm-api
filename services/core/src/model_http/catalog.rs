//! Read-only metadata from the exact immutable release installed in ModelGateway.
//! API labels describe the gateway's bounded text/function adapter surface. They
//! do not grant invocation, promise current health, enable SSE, or lock a price.
use crate::routing::{
    BillingMode, ModelRoute, PayloadPolicy, PriceQuote, PriceSnapshot, Protocol, ReleaseVersion,
    Router, Speed, UpstreamConfig,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::Number;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const MAX_MODELS: usize = 4096;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogError {
    Unavailable,
}

/// Opaque server-injected proof. Only ModelGateway can create a serving surface.
/// No URL, credential material, raw release, or NativePrincipal is serialized.
pub struct CatalogSurface {
    trust_pricing_enabled: std::sync::atomic::AtomicBool,
    source: std::sync::OnceLock<std::sync::Weak<super::Bridge>>,
    state: SurfaceState,
}

enum SurfaceState {
    Disabled,
    Ready {
        router: Arc<Router>,
        version: ReleaseVersion,
        groups: BTreeMap<String, CatalogGroup>,
        models: Vec<CatalogModel>,
    },
}

#[derive(Clone, Serialize)]
pub struct CatalogV1 {
    pub schema_version: u32,
    pub resource: String,
    pub updated_at: u64,
    pub routing_version: u64,
    pub pricing_version: u64,
    pub groups: Vec<CatalogGroup>,
    pub models: Vec<CatalogModel>,
}

#[derive(Clone, Serialize)]
pub struct CatalogGroup {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub multiplier: Option<Number>,
}

#[derive(Clone, Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub group_id: String,
    pub group: String,
    pub upstream_model: String,
    pub name: String,
    pub apis: Vec<&'static str>,
    pub pricing: CatalogPricing,
    pub native_cost: Option<()>,
}

#[derive(Clone, Serialize)]
pub struct CatalogPricing {
    pub currency: &'static str,
    pub unit: &'static str,
    pub price_basis: &'static str,
    pub group_multiplier: Option<Number>,
    pub trust_multiplier: Option<Number>,
    pub input: Option<Number>,
    pub output: Option<Number>,
    pub cache_read: Option<Number>,
    pub cache_write: Option<Number>,
    pub request: Option<Number>,
    pub final_cost_depends_on_usage: bool,
    /// Observation time of this projection, not a fabricated activation timestamp.
    pub updated_at: u64,
}

impl CatalogSurface {
    pub(super) fn attach_source(&self, bridge: std::sync::Weak<super::Bridge>) {
        let _ = self.source.set(bridge);
    }
    pub(super) fn enable_trust_pricing(&self) {
        self.trust_pricing_enabled
            .store(true, std::sync::atomic::Ordering::Release);
    }
    /// Called only after issuer ReadAuthority authentication, in its same
    /// SERIALIZABLE transaction. An OAuth reader remains a reader, not a Key.
    pub(crate) async fn project_for_user(
        &self,
        c: &mut sqlx::PgConnection,
        user_id: i64,
        groups: &[String],
        resource: &str,
        updated_at: u64,
    ) -> Result<CatalogV1, CatalogError> {
        let authority = if self
            .trust_pricing_enabled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            Some(
                crate::billing::trust_pricing::quote_in_transaction(c, user_id)
                    .await
                    .map_err(|_| CatalogError::Unavailable)?,
            )
        } else {
            None
        };
        let mut result = if let Some(source) = self.source.get() {
            let bridge = source.upgrade().ok_or(CatalogError::Unavailable)?;
            if let Some(admin) = bridge.routing_admin.get() {
                admin
                    .refresh_in(c)
                    .await
                    .map_err(|_| CatalogError::Unavailable)?;
            }
            let current = bridge
                .routing
                .version()
                .map_err(|_| CatalogError::Unavailable)?;
            if matches!(&self.state, SurfaceState::Ready { version, .. } if *version == current) {
                self.project(groups, resource, updated_at)?
            } else {
                Self::load(
                    bridge.routing.clone(),
                    &bridge.credentials,
                    bridge.total_timeout,
                    Some((&bridge.provider_policy, bridge.policy.input_token_ceiling)),
                )?
                .project(groups, resource, updated_at)?
            }
        } else {
            self.project(groups, resource, updated_at)?
        };
        if let Some(authority) = authority {
            let discount = authority.discount();
            let one: Number = "1".parse().map_err(|_| CatalogError::Unavailable)?;
            let display = super::pricing::scaled_number(Some(&one), discount)
                .map_err(|_| CatalogError::Unavailable)?
                .ok_or(CatalogError::Unavailable)?;
            for model in &mut result.models {
                model.pricing.trust_multiplier = Some(display.clone());
                model.pricing.price_basis = "effective_user_rates";
                for rate in [
                    &mut model.pricing.input,
                    &mut model.pricing.output,
                    &mut model.pricing.cache_read,
                    &mut model.pricing.cache_write,
                    &mut model.pricing.request,
                ] {
                    *rate = super::pricing::scaled_number(rate.as_ref(), discount)
                        .map_err(|_| CatalogError::Unavailable)?;
                }
            }
        }
        if serde_json::to_vec(&result)
            .map_err(|_| CatalogError::Unavailable)?
            .len()
            > MAX_CATALOG_BYTES
        {
            return Err(CatalogError::Unavailable);
        }
        Ok(result)
    }

    pub(super) fn from_gateway(
        router: Arc<Router>,
        credentials: &BTreeMap<(u64, u64), String>,
        total_timeout: Duration,
    ) -> Self {
        Self::load(router, credentials, total_timeout, None).unwrap_or(Self {
            trust_pricing_enabled: std::sync::atomic::AtomicBool::new(false),
            source: std::sync::OnceLock::new(),
            state: SurfaceState::Disabled,
        })
    }
    pub(super) fn from_gateway_with_provider_policy(
        router: Arc<Router>,
        credentials: &BTreeMap<(u64, u64), String>,
        total_timeout: Duration,
        policy: &super::ProviderPolicy,
        reserved_input_tokens: i64,
    ) -> Self {
        Self::load(
            router,
            credentials,
            total_timeout,
            Some((policy, reserved_input_tokens)),
        )
        .unwrap_or(Self {
            trust_pricing_enabled: std::sync::atomic::AtomicBool::new(false),
            source: std::sync::OnceLock::new(),
            state: SurfaceState::Disabled,
        })
    }

    fn load(
        router: Arc<Router>,
        credentials: &BTreeMap<(u64, u64), String>,
        total_timeout: Duration,
        provider_policy: Option<(&super::ProviderPolicy, i64)>,
    ) -> Result<Self, CatalogError> {
        // catalog_document takes one Arc snapshot, so config and prices cannot
        // come from different releases. It never consults the database pointer.
        let document = router
            .catalog_document()
            .map_err(|_| CatalogError::Unavailable)?;
        let version = ReleaseVersion {
            routing: document.routing.version,
            pricing: document.pricing.version,
        };
        let prices =
            PriceSnapshot::compile(document.pricing).map_err(|_| CatalogError::Unavailable)?;
        let upstreams: BTreeMap<_, _> = document
            .routing
            .upstreams
            .iter()
            .map(|u| (u.id, u))
            .collect();
        // Index once instead of scanning every route/alias for every group.
        let mut routes_by_group: BTreeMap<&str, Vec<&ModelRoute>> = BTreeMap::new();
        for route in &document.routing.routes {
            routes_by_group.entry(&route.group).or_default().push(route);
        }
        let mut aliases_by_route: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
        for alias in &document.routing.aliases {
            aliases_by_route
                .entry((&alias.group, &alias.model))
                .or_default()
                .push(&alias.alias);
        }
        let mut groups = BTreeMap::new();
        let mut models = Vec::new();
        for group in document.routing.groups.iter().filter(|g| g.enabled) {
            let group_id = URL_SAFE_NO_PAD.encode(group.name.as_bytes());
            let mut group_ppm = Vec::new();
            for route in routes_by_group
                .get(group.name.as_str())
                .into_iter()
                .flatten()
                .copied()
                .filter(|r| r.enabled && r.speeds.contains(&Speed::Standard))
            {
                let quote = prices
                    .quote(&route.model, &group.name, Speed::Standard)
                    .ok_or(CatalogError::Unavailable)?;
                // Reuse the actual gateway charging representation boundary;
                // unsupported dimensions are not advertised as usable APIs.
                if super::routing::credit_price(quote, &route.model, &group.name).is_err() {
                    continue;
                }
                let names = std::iter::once(route.model.as_str()).chain(
                    aliases_by_route
                        .get(&(group.name.as_str(), route.model.as_str()))
                        .into_iter()
                        .flatten()
                        .copied(),
                );
                for name in names {
                    let apis: Vec<_> = [
                        (Protocol::OpenAiChat, "openai-completions"),
                        (Protocol::OpenAiResponses, "openai-responses"),
                        (Protocol::SystemOne, "system-one"),
                    ]
                    .into_iter()
                    .filter_map(|(client, label)| {
                        api_supported(
                            route,
                            name,
                            client,
                            &upstreams,
                            credentials,
                            total_timeout,
                            quote,
                            provider_policy,
                        )
                        .then_some(label)
                    })
                    .collect();
                    if apis.is_empty() {
                        continue;
                    }
                    // The fixed client wire contract sends upstream_model as
                    // JSON model. api_supported proves it equals this request
                    // alias for every possible target; do not rewrite aliases.
                    models.push(CatalogModel {
                        id: format!("lmm:{group_id}:{}", URL_SAFE_NO_PAD.encode(name.as_bytes())),
                        group_id: group_id.clone(),
                        group: group.name.clone(),
                        upstream_model: name.to_owned(),
                        name: name.to_owned(),
                        apis,
                        pricing: pricing(quote)?,
                        native_cost: None,
                    });
                    group_ppm.push(quote.multipliers_ppm().0);
                    if models.len() > MAX_MODELS {
                        return Err(CatalogError::Unavailable);
                    }
                }
            }
            let multiplier = match group_ppm.first().copied() {
                Some(first) if group_ppm.iter().all(|ppm| *ppm == first) => {
                    Some(decimal(u128::from(first), 6)?)
                }
                _ => None,
            };
            groups.insert(
                group.name.clone(),
                CatalogGroup {
                    id: group_id.clone(),
                    name: group.name.clone(),
                    scope: format!("group:{group_id}"),
                    multiplier,
                },
            );
        }
        models.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
        if router.version().map_err(|_| CatalogError::Unavailable)? != version {
            return Err(CatalogError::Unavailable);
        }
        Ok(Self {
            trust_pricing_enabled: std::sync::atomic::AtomicBool::new(false),
            source: std::sync::OnceLock::new(),
            state: SurfaceState::Ready {
                router,
                version,
                groups,
                models,
            },
        })
    }

    /// groups must come from issuer ReadAuthority: token/family consent/current
    /// user policy/installation intersection, never a query or client nonce.
    /// Metadata alone grants no NativeKey or spending authority.
    pub fn project(
        &self,
        authorized_groups: &[String],
        resource: &str,
        updated_at: u64,
    ) -> Result<CatalogV1, CatalogError> {
        let SurfaceState::Ready {
            router,
            version,
            groups,
            models,
        } = &self.state
        else {
            return Err(CatalogError::Unavailable);
        };
        if resource.is_empty()
            || resource.len() > 2048
            || updated_at == 0
            || authorized_groups.is_empty()
            || router.version().map_err(|_| CatalogError::Unavailable)? != *version
        {
            return Err(CatalogError::Unavailable);
        }
        let allowed: std::collections::BTreeSet<_> = authorized_groups.iter().collect();
        let mut result = CatalogV1 {
            schema_version: 1,
            resource: resource.to_owned(),
            updated_at,
            routing_version: version.routing,
            pricing_version: version.pricing,
            groups: groups
                .iter()
                .filter(|(name, _)| allowed.contains(name))
                .map(|(_, g)| g.clone())
                .collect(),
            models: models
                .iter()
                .filter(|m| allowed.contains(&m.group))
                .cloned()
                .collect(),
        };
        for model in &mut result.models {
            model.pricing.updated_at = updated_at;
        }
        let used_groups: std::collections::BTreeSet<_> =
            result.models.iter().map(|m| m.group.as_str()).collect();
        result
            .groups
            .retain(|g| used_groups.contains(g.name.as_str()));
        if serde_json::to_vec(&result)
            .map_err(|_| CatalogError::Unavailable)?
            .len()
            > MAX_CATALOG_BYTES
            || router.version().map_err(|_| CatalogError::Unavailable)? != *version
        {
            return Err(CatalogError::Unavailable);
        }
        Ok(result)
    }
}

fn api_supported(
    route: &ModelRoute,
    name: &str,
    client: Protocol,
    upstreams: &BTreeMap<u64, &UpstreamConfig>,
    credentials: &BTreeMap<(u64, u64), String>,
    total_timeout: Duration,
    quote: PriceQuote,
    provider_policy: Option<(&super::ProviderPolicy, i64)>,
) -> bool {
    if quote.embedding().is_some() {
        return false;
    }
    // Health/operational stops can recover without publishing new config. Check
    // every potentially selectable target, including lower priority targets.
    let targets: Vec<_> = route
        .targets
        .iter()
        .filter_map(|t| upstreams.get(&t.upstream_id).map(|u| (t, *u)))
        .filter(|(t, u)| {
            t.enabled
                && t.weight > 0
                && u.enabled
                && t.speeds.contains(&Speed::Standard)
                && (t.payload == PayloadPolicy::Adapt
                    || (u.protocol == client && t.upstream_model == name))
        })
        .collect();
    !targets.is_empty()
        && targets.into_iter().all(|(t, u)| {
            t.payload == PayloadPolicy::Adapt
                && u128::from(t.timeout_ms) >= total_timeout.as_millis()
                && t.upstream_model == name
                && (quote.billing_mode() != BillingMode::UpstreamCost
                    || t.upstream_model == route.model)
                && credentials
                    .get(&(u.credential.id, u.credential.revision))
                    .is_some_and(|secret| {
                        if client == Protocol::SystemOne {
                            u.protocol == Protocol::SystemOne
                                && t.payload == PayloadPolicy::Adapt
                                && provider_policy.is_some_and(|(policy, bound)| {
                                    bound >= 65_536
                                        && policy
                                            .validate_binding(
                                                u.id,
                                                &t.upstream_model,
                                                u.protocol,
                                                u.endpoint.as_str(),
                                                bound,
                                                0,
                                            )
                                            .is_ok()
                                })
                                && quote.is_input_only_scalar()
                                && reqwest::header::HeaderValue::from_str(&format!(
                                    "Bearer {secret}"
                                ))
                                .is_ok()
                                && super::systemone::endpoint(u.endpoint.as_str()).is_ok()
                        } else {
                            route_is_constructible(route, t, u, secret, quote)
                        }
                    })
        })
}

fn route_is_constructible(
    route: &ModelRoute,
    target: &crate::routing::RouteTarget,
    upstream: &UpstreamConfig,
    secret: &str,
    quote: PriceQuote,
) -> bool {
    let protocol = match upstream.protocol {
        Protocol::OpenAiChat => crate::relay::UpstreamProtocol::Chat,
        Protocol::OpenAiResponses => crate::relay::UpstreamProtocol::Responses,
        _ => return false,
    };
    // Match operation_endpoint's no-I/O URL boundary before the real relay
    // constructor, including public-literal validation and explicit loopback.
    let Ok(mut endpoint) =
        crate::relay::honeypot_network::validate_base_url(upstream.endpoint.as_str(), true)
    else {
        return false;
    };
    {
        let Ok(mut segments) = endpoint.path_segments_mut() else {
            return false;
        };
        segments.pop_if_empty();
        match protocol {
            crate::relay::UpstreamProtocol::Chat => {
                segments.push("chat").push("completions");
            }
            crate::relay::UpstreamProtocol::Responses => {
                segments.push("responses");
            }
            _ => return false,
        }
    }
    // Use the real no-I/O route constructor, including sensitive HeaderValue
    // validation. A credential ref being loaded does not prove a usable header.
    crate::relay::Route::new(
        format!("r{}-t{}-u{}", route.id, target.id, upstream.id),
        protocol,
        endpoint.as_str(),
        target.upstream_model.clone(),
        secret,
        quote.version(),
    )
    .is_ok()
}

fn pricing(quote: PriceQuote) -> Result<CatalogPricing, CatalogError> {
    let (group_ppm, speed_ppm) = quote.multipliers_ppm();
    let known = quote.billing_mode() == BillingMode::Tokens;
    let rate = |value: u64| {
        if !known {
            return Ok(None);
        }
        let numerator = u128::from(value)
            .checked_mul(u128::from(group_ppm))
            .and_then(|v| v.checked_mul(u128::from(speed_ppm)))
            .ok_or(CatalogError::Unavailable)?;
        // micro-USD per million tokens, with both ppm multipliers applied.
        // arbitrary_precision is already enabled in the product dependency.
        decimal(numerator, 18).map(Some)
    };
    let rates = quote.rates();
    Ok(CatalogPricing {
        currency: "USD",
        unit: if known { "million_tokens" } else { "unknown" },
        price_basis: if known {
            "configured_base_rates"
        } else {
            "unknown"
        },
        group_multiplier: Some(decimal(u128::from(group_ppm), 6)?),
        trust_multiplier: None,
        input: rate(rates.input)?,
        output: rate(rates.output)?,
        cache_read: rate(rates.cache_read)?,
        cache_write: None,
        request: None,
        final_cost_depends_on_usage: true,
        updated_at: 0,
    })
}

fn decimal(numerator: u128, scale: u32) -> Result<Number, CatalogError> {
    let denominator = 10_u128.pow(scale);
    let whole = numerator / denominator;
    let remainder = numerator % denominator;
    let text = if remainder == 0 {
        whole.to_string()
    } else {
        let fraction = format!("{remainder:0width$}", width = scale as usize);
        format!("{whole}.{}", fraction.trim_end_matches('0'))
    };
    text.parse().map_err(|_| CatalogError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{
        CredentialRef, Endpoint, FailoverPolicy, ModelAlias, ModelGroup, PriceConfig, PriceRule,
        RouteTarget, RoutingConfig, TokenRates,
    };

    fn fixture(model: &str) -> (RoutingConfig, PriceConfig) {
        (
            RoutingConfig {
                version: 1,
                groups: vec![ModelGroup {
                    name: "synthetic".into(),
                    enabled: true,
                }],
                upstreams: vec![UpstreamConfig {
                    id: 1,
                    endpoint: Endpoint::local_loopback("http://127.0.0.1:4444/v1".into()).unwrap(),
                    protocol: Protocol::OpenAiChat,
                    credential: CredentialRef { id: 1, revision: 1 },
                    enabled: true,
                }],
                routes: vec![ModelRoute {
                    id: 1,
                    model: model.into(),
                    group: "synthetic".into(),
                    enabled: true,
                    speeds: vec![Speed::Standard],
                    targets: vec![RouteTarget {
                        id: 1,
                        upstream_id: 1,
                        upstream_model: model.into(),
                        enabled: true,
                        priority: 0,
                        weight: 1,
                        speeds: vec![Speed::Standard],
                        payload: PayloadPolicy::Adapt,
                        timeout_ms: 5000,
                    }],
                    failover: FailoverPolicy::default(),
                }],
                aliases: vec![],
            },
            PriceConfig {
                version: 1,
                rules: vec![PriceRule {
                    embedding: None,
                    search: None,
                    billing_mode: BillingMode::Tokens,
                    tiered: None,
                    id: 1,
                    model: model.into(),
                    group: "synthetic".into(),
                    speed: Speed::Standard,
                    rates: TokenRates {
                        input: 2_000_000,
                        output: 4_000_000,
                        cache_read: 1_000_000,
                        cache_write: 0,
                        request: 0,
                    },
                    group_multiplier_ppm: 1_250_000,
                    speed_multiplier_ppm: 1_000_000,
                }],
            },
        )
    }

    fn surface(config: RoutingConfig, prices: PriceConfig) -> (Arc<Router>, CatalogSurface) {
        let router = Arc::new(
            Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap())).unwrap(),
        );
        let credentials = BTreeMap::from([((1, 1), "synthetic-only-not-a-provider-key".into())]);
        let surface =
            CatalogSurface::from_gateway(router.clone(), &credentials, Duration::from_secs(1));
        (router, surface)
    }

    fn view(surface: &CatalogSurface) -> CatalogV1 {
        surface
            .project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap()
    }

    #[sqlx::test]
    async fn native_and_oauth_price_reads_follow_durable_publication_in_the_same_read_transaction(
        pool: sqlx::PgPool,
    ) {
        crate::core_schema::init(&pool).await.unwrap();
        let serving_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        let (routing, pricing) = fixture("model");
        let router = Arc::new(
            Router::new(
                routing.clone(),
                Arc::new(PriceSnapshot::compile(pricing.clone()).unwrap()),
            )
            .unwrap(),
        );
        let gateway = super::super::ModelGateway::new(
            serving_pool.clone(),
            router.clone(),
            vec![super::super::UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: "synthetic-publication-credential".into(),
            }],
            BTreeMap::new(),
            crate::relay::Limits {
                total_timeout: Duration::from_secs(1),
                hook_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            super::super::ChargePolicy {
                input_token_ceiling: 65536,
                lease_seconds: 10,
            },
        )
        .unwrap();
        gateway.enable_routing_admin().unwrap();
        let public = super::super::pricing::PricingSurface::from_gateway(&gateway).unwrap();
        let oauth = gateway.oauth_catalog_surface();
        let store = crate::routing::PostgresRoutingStore::new(pool.clone());
        store
            .register_credential_reference(CredentialRef { id: 1, revision: 1 })
            .await
            .unwrap();
        store.stage_prices(pricing.clone()).await.unwrap();
        store.publish(routing.clone(), 1, None).await.unwrap();
        let mut read = serving_pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE READ ONLY")
            .execute(&mut *read)
            .await
            .unwrap();
        let before = public
            .project_current(&mut read, Some(&["synthetic".into()]), None)
            .await
            .unwrap();
        assert_eq!(
            before.models[0]
                .token_price
                .as_ref()
                .unwrap()
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "2.5"
        );
        let before_oauth = oauth
            .project_for_user(
                &mut read,
                1,
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .await
            .unwrap();
        assert_eq!(
            before_oauth.models[0]
                .pricing
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "2.5"
        );
        read.commit().await.unwrap();

        let mut next_routing = routing;
        next_routing.version = 2;
        let mut next_pricing = pricing;
        next_pricing.version = 2;
        next_pricing.rules[0].rates.input = 4_000_000;
        store.stage_prices(next_pricing).await.unwrap();
        store
            .publish(
                next_routing,
                2,
                Some(ReleaseVersion {
                    routing: 1,
                    pricing: 1,
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            router.version().unwrap(),
            ReleaseVersion {
                routing: 1,
                pricing: 1
            }
        );
        let mut read = serving_pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE READ ONLY")
            .execute(&mut *read)
            .await
            .unwrap();
        let after = tokio::time::timeout(
            Duration::from_secs(2),
            public.project_current(&mut read, Some(&["synthetic".into()]), None),
        )
        .await
        .expect("reuse the one available read connection")
        .unwrap();
        assert_eq!((after.routing_version, after.pricing_version), (2, 2));
        assert_eq!(
            after.models[0]
                .token_price
                .as_ref()
                .unwrap()
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "5"
        );
        let after_oauth = tokio::time::timeout(
            Duration::from_secs(2),
            oauth.project_for_user(
                &mut read,
                1,
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                124,
            ),
        )
        .await
        .expect("OAuth refresh must reuse the same read connection")
        .unwrap();
        assert_eq!(
            (after_oauth.routing_version, after_oauth.pricing_version),
            (2, 2)
        );
        assert_eq!(
            after_oauth.models[0]
                .pricing
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "5"
        );
        read.commit().await.unwrap();
        assert!(public.project(None, None).is_err());
        assert!(
            oauth
                .project(
                    &["synthetic".into()],
                    "https://synthetic.invalid/api/oauth2",
                    125
                )
                .is_err()
        );
        serving_pool.close().await;
    }

    #[test]
    fn configured_rates_are_exact_and_group_price_is_not_multiplied_twice() {
        let (r, p) = fixture("model");
        let (_, s) = surface(r, p);
        let v = serde_json::to_value(view(&s)).unwrap();
        assert_eq!(
            v["models"][0]["apis"],
            serde_json::json!(["openai-completions", "openai-responses"])
        );
        assert_eq!(v["models"][0]["pricing"]["input"].to_string(), "2.5");
        assert_eq!(v["models"][0]["pricing"]["output"].to_string(), "5");
        assert_eq!(v["models"][0]["pricing"]["cache_read"].to_string(), "1.25");
        assert!(v["models"][0]["pricing"]["trust_multiplier"].is_null());
        assert!(v["models"][0]["native_cost"].is_null());
        assert_eq!(v["groups"][0]["scope"], "group:c3ludGhldGlj");
    }

    #[test]
    fn one_supported_target_does_not_hide_an_unsupported_recoverable_target() {
        let (mut r, p) = fixture("model");
        let mut other = r.upstreams[0].clone();
        other.id = 2;
        other.protocol = Protocol::Gemini;
        r.upstreams.push(other);
        let mut target = r.routes[0].targets[0].clone();
        target.id = 2;
        target.upstream_id = 2;
        target.priority = 99;
        r.routes[0].targets.push(target);
        let router =
            Arc::new(Router::new(r, Arc::new(PriceSnapshot::compile(p).unwrap())).unwrap());
        router
            .set_health(
                router.version().unwrap(),
                2,
                crate::routing::HealthStatus::Unhealthy,
            )
            .unwrap();
        let credentials = BTreeMap::from([((1, 1), "synthetic-only-not-a-provider-key".into())]);
        let s = CatalogSurface::from_gateway(router, &credentials, Duration::from_secs(1));
        assert!(view(&s).models.is_empty());
    }

    #[test]
    fn public_alias_is_preserved_only_when_wire_model_is_unambiguous() {
        let (mut r, p) = fixture("canonical");
        r.routes[0].targets[0].upstream_model = "actual-alias".into();
        r.aliases.push(ModelAlias {
            group: "synthetic".into(),
            alias: "actual-alias".into(),
            model: "canonical".into(),
        });
        r.aliases.push(ModelAlias {
            group: "synthetic".into(),
            alias: "other-alias".into(),
            model: "canonical".into(),
        });
        let (_, s) = surface(r, p);
        let v = view(&s);
        assert_eq!(v.models.len(), 1);
        assert_eq!(v.models[0].upstream_model, "actual-alias");
        assert_eq!(v.models[0].name, "actual-alias");
        assert!(v.models[0].id.ends_with("YWN0dWFsLWFsaWFz"));
    }

    #[test]
    fn systemone_catalog_admits_only_safe_scalar_decimal_tariff() {
        let (mut config, mut prices) = fixture("jev-native");
        config.upstreams[0].protocol = Protocol::SystemOne;
        prices.rules[0].billing_mode = BillingMode::TieredDecimal;
        prices.rules[0].rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        let scalar: crate::routing::TieredTariff = serde_json::from_value(serde_json::json!({
            "revision":1,"source_digest":"af0d4b7d188d58f20f9eb13cef18353659ca208151824014795189b70dc0afd7",
            "uses_cache_read":false,"uses_cache_write":false,
            "tiers":[{"rates":{"input":{"numerator":21,"denominator":500},"output":{"numerator":0,"denominator":1},"cache_read":{"numerator":0,"denominator":1},"cache_write":{"numerator":0,"denominator":1}}}]
        })).unwrap();
        for accepted in [true, false] {
            let mut tariff = scalar.clone();
            if !accepted {
                tariff.tiers[0].rates.output.numerator = 1;
            }
            prices.rules[0].tiered = Some(tariff);
            let router = Arc::new(
                Router::new(
                    config.clone(),
                    Arc::new(PriceSnapshot::compile(prices.clone()).unwrap()),
                )
                .unwrap(),
            );
            let credentials = BTreeMap::from([((1, 1), "synthetic-systemone-only".into())]);
            let surface = CatalogSurface::from_gateway_with_provider_policy(
                router,
                &credentials,
                Duration::from_secs(1),
                &super::super::ProviderPolicy::default(),
                65536,
            );
            let result = view(&surface);
            if accepted {
                assert_eq!(result.models.len(), 1);
                assert_eq!(result.models[0].apis, ["system-one"]);
                let price = serde_json::to_value(&result.models[0].pricing).unwrap();
                for field in ["input", "output", "cache_read", "cache_write", "request"] {
                    assert!(price[field].is_null(), "{field}");
                }
            } else {
                assert!(result.models.is_empty());
            }
        }
    }
    #[test]
    fn upstream_cost_sentinel_is_unknown_and_never_a_free_price() {
        let (r, mut p) = fixture("openrouter/fusion");
        p.rules[0].billing_mode = BillingMode::UpstreamCost;
        p.rules[0].rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        let (_, s) = surface(r, p);
        let v = serde_json::to_value(view(&s)).unwrap();
        assert_eq!(v["models"].as_array().unwrap().len(), 1);
        let price = &v["models"][0]["pricing"];
        assert_eq!(price["unit"], "unknown");
        for field in ["input", "output", "cache_read", "cache_write", "request"] {
            assert!(price[field].is_null(), "{field}");
        }
    }

    #[test]
    fn default_speed_uses_standard_price_and_fast_only_has_no_default_row() {
        let (mut r, mut p) = fixture("model");
        r.routes[0].speeds.push(Speed::Fast);
        r.routes[0].targets[0].speeds.push(Speed::Fast);
        let mut fast = p.rules[0].clone();
        fast.id = 2;
        fast.speed = Speed::Fast;
        fast.speed_multiplier_ppm = 3_000_000;
        p.rules.push(fast);
        let (_, s) = surface(r.clone(), p.clone());
        assert_eq!(
            view(&s).models[0]
                .pricing
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "2.5"
        );
        r.routes[0].speeds = vec![Speed::Fast];
        r.routes[0].targets[0].speeds = vec![Speed::Fast];
        let (_, s) = surface(r, p);
        assert!(view(&s).models.is_empty());
    }

    #[test]
    fn changed_live_release_invalidates_old_surface_without_reading_db_active() {
        let (r, p) = fixture("model");
        let (router, s) = surface(r.clone(), p.clone());
        let old = router.version().unwrap();
        let mut next = r;
        next.version = 2;
        router
            .publish(old, next, Arc::new(PriceSnapshot::compile(p).unwrap()))
            .unwrap();
        assert!(matches!(
            s.project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            ),
            Err(CatalogError::Unavailable)
        ));
    }

    #[test]
    fn unknown_groups_cannot_select_another_groups_models_and_disabled_is_503() {
        let (r, p) = fixture("model");
        let (_, s) = surface(r, p);
        let v = s
            .project(
                &["client-forged-group".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap();
        assert!(v.models.is_empty());
        assert!(v.groups.is_empty());
        let disabled = CatalogSurface {
            trust_pricing_enabled: std::sync::atomic::AtomicBool::new(false),
            state: SurfaceState::Disabled,
        };
        assert!(matches!(
            disabled.project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            ),
            Err(CatalogError::Unavailable)
        ));
    }

    #[test]
    fn missing_credentials_timeout_passthrough_and_inexact_charge_are_not_apis() {
        let (base, prices) = fixture("model");
        for mode in 1..5 {
            let mut r = base.clone();
            let mut p = prices.clone();
            r.upstreams[0].endpoint = "https://synthetic.invalid/v1"
                .to_owned()
                .try_into()
                .unwrap();
            match mode {
                1 => r.upstreams[0].credential.revision = 2,
                2 => r.routes[0].targets[0].timeout_ms = 999,
                3 => r.routes[0].targets[0].payload = PayloadPolicy::Passthrough,
                4 => p.rules[0].rates.input = 1,
                _ => unreachable!(),
            }
            let (_, s) = surface(r, p);
            let catalog = view(&s);
            assert!(catalog.models.is_empty(), "mode {mode}");
            assert!(
                catalog.groups.is_empty(),
                "group without live model, mode {mode}"
            );
        }
    }
    #[test]
    fn configured_https_chat_and_responses_are_catalogued_without_provider_io() {
        // The former mode=0 exclusion was a fixture restriction, not the
        // gateway's production endpoint contract. Route::new is authoritative.
        for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses] {
            for endpoint in [
                "https://synthetic.invalid/v1",
                "https://[2606:4700:4700::1111]:8443/v1",
            ] {
                let (mut r, p) = fixture("model");
                r.upstreams[0].endpoint = endpoint.to_owned().try_into().unwrap();
                r.upstreams[0].protocol = protocol;
                let (_, s) = surface(r, p);
                let catalog = view(&s);
                assert_eq!(catalog.models.len(), 1);
                assert_eq!(catalog.groups.len(), 1);
                assert_eq!(
                    catalog.models[0].apis,
                    ["openai-completions", "openai-responses"]
                );
                assert_eq!(
                    catalog.models[0]
                        .pricing
                        .input
                        .as_ref()
                        .unwrap()
                        .to_string(),
                    "2.5"
                );
                let wire = serde_json::to_string(&catalog).unwrap();
                assert!(!wire.contains("synthetic.invalid/v1"));
                assert!(!wire.contains("synthetic-only-not-a-provider-key"));
            }
        }
    }
    #[test]
    fn https_recoverable_targets_must_all_pass_the_same_constructor_and_contract() {
        for mode in 0..4 {
            let (mut r, p) = fixture("model");
            r.upstreams[0].endpoint = "https://synthetic.invalid/v1"
                .to_owned()
                .try_into()
                .unwrap();
            let mut other = r.upstreams[0].clone();
            other.id = 2;
            other.credential.id = 2;
            r.upstreams.push(other);
            let mut target = r.routes[0].targets[0].clone();
            target.id = 2;
            target.upstream_id = 2;
            target.priority = 99;
            match mode {
                1 => target.timeout_ms = 999,
                2 => target.payload = PayloadPolicy::Passthrough,
                3 => target.upstream_model = "different-model".into(),
                _ => {}
            }
            r.routes[0].targets.push(target);
            let router =
                Arc::new(Router::new(r, Arc::new(PriceSnapshot::compile(p).unwrap())).unwrap());
            router
                .set_health(
                    router.version().unwrap(),
                    2,
                    crate::routing::HealthStatus::Unhealthy,
                )
                .unwrap();
            let credentials = BTreeMap::from([
                ((1, 1), "synthetic-only-first".into()),
                (
                    (2, 1),
                    if mode == 0 {
                        "synthetic\ninvalid-header".into()
                    } else {
                        "synthetic-only-second".into()
                    },
                ),
            ]);
            let surface =
                CatalogSurface::from_gateway(router, &credentials, Duration::from_secs(1));
            let catalog = view(&surface);
            if mode == 2 {
                // Chat's passthrough target is not selectable for Responses;
                // retain the first Adapt target's actual Responses surface.
                assert_eq!(catalog.models.len(), 1);
                assert_eq!(catalog.models[0].apis, ["openai-responses"]);
            } else {
                assert!(catalog.models.is_empty(), "mode {mode}");
            }
        }
    }
    #[test]
    fn https_catalog_fix_does_not_admit_insecure_urls_or_extra_protocols() {
        for endpoint in [
            "http://192.0.2.1/v1",
            "http://synthetic.invalid/v1",
            "ftp://synthetic.invalid/v1",
            "https://user:pass@synthetic.invalid/v1",
            "https://synthetic.invalid/v1?token=secret",
            "https://synthetic.invalid/v1#fragment",
        ] {
            assert!(
                crate::relay::Route::new(
                    "synthetic".into(),
                    crate::relay::UpstreamProtocol::Chat,
                    endpoint,
                    "model".into(),
                    "synthetic-only",
                    1
                )
                .is_err(),
                "{endpoint}"
            );
        }
        for endpoint in [
            "https://10.0.0.1/v1",
            "https://169.254.169.254/v1",
            "https://[2001:db8::1]:8443/v1",
            "https://[fe80::1]/v1",
        ] {
            let (mut r, p) = fixture("model");
            r.upstreams[0].endpoint = endpoint.to_owned().try_into().unwrap();
            let (_, surface) = surface(r, p);
            assert!(view(&surface).models.is_empty(), "{endpoint}");
        }
        for protocol in [Protocol::AnthropicMessages, Protocol::Gemini] {
            let (mut r, p) = fixture("model");
            r.upstreams[0].endpoint = "https://synthetic.invalid/v1"
                .to_owned()
                .try_into()
                .unwrap();
            r.upstreams[0].protocol = protocol;
            let (_, s) = surface(r, p);
            assert!(view(&s).models.is_empty());
        }
    }
    #[test]
    fn loaded_credential_reference_with_invalid_bearer_header_is_not_an_api() {
        let (r, p) = fixture("model");
        let router =
            Arc::new(Router::new(r, Arc::new(PriceSnapshot::compile(p).unwrap())).unwrap());
        let credentials = BTreeMap::from([((1, 1), "synthetic\ninvalid-header".into())]);
        let s = CatalogSurface::from_gateway(router, &credentials, Duration::from_secs(1));
        assert!(view(&s).models.is_empty());
    }
    #[test]
    fn valid_large_catalog_rejects_oversized_dto_instead_of_returning_partial_rows() {
        let (mut r, mut p) = fixture("model");
        for id in 2..=MAX_MODELS as u64 {
            let name = format!("model-{id:04}-{}", "x".repeat(117));
            let mut route = r.routes[0].clone();
            route.id = id;
            route.model = name.clone();
            route.targets[0].id = id;
            route.targets[0].upstream_model = name.clone();
            r.routes.push(route);
            let mut price = p.rules[0].clone();
            price.id = id;
            price.model = name;
            p.rules.push(price);
        }
        let (_, s) = surface(r, p);
        let SurfaceState::Ready { models, .. } = &s.state else {
            panic!("valid catalog within factory row budget must load");
        };
        assert_eq!(models.len(), MAX_MODELS);
        assert!(serde_json::to_vec(models).unwrap().len() > MAX_CATALOG_BYTES);
        assert!(matches!(
            s.project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            ),
            Err(CatalogError::Unavailable)
        ));
    }
}
