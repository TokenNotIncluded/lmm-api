//! Exact native price import and priority changes. No writes or upstream fetches.
//! Source formats with different currency/token semantics are never guessed.
use super::{AdminError, Result};
use crate::routing::{
    BillingMode, DecimalRate, PriceConfig, PriceRule, PriceSnapshot, SearchRate, TokenRates,
    store::ReleaseDocument,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SOURCE_CONTRACT: &str = "core-native-price-source.v1";
pub(super) const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SELECTIONS: usize = 200;
const MAX_SAFE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriceSource {
    pub contract: String,
    pub currency: String,
    pub prices: PriceConfig,
}
impl PriceSource {
    pub fn from_release(document: &ReleaseDocument) -> Self {
        Self {
            contract: SOURCE_CONTRACT.to_owned(),
            currency: "USD".to_owned(),
            prices: document.pricing.clone(),
        }
    }
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > MAX_SOURCE_BYTES {
            return Err(AdminError::TooLarge);
        }
        let source: Self = serde_json::from_str(raw).map_err(|_| AdminError::InvalidSource)?;
        if source.contract != SOURCE_CONTRACT || source.currency != "USD" {
            return Err(AdminError::InvalidSource);
        }
        PriceSnapshot::compile(source.prices.clone()).map_err(|_| AdminError::InvalidSource)?;
        Ok(source)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Adjustment {
    Keep,
    Multiply,
    Divide,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriceSelection {
    pub model: String,
    pub adjustment: Adjustment,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Expected {
    pub routing: String,
    pub pricing: String,
    pub snapshot_sha256: String,
}
impl Expected {
    pub fn from_release(document: &ReleaseDocument) -> Result<Self> {
        Ok(Self {
            routing: document.routing.version.to_string(),
            pricing: document.pricing.version.to_string(),
            snapshot_sha256: digest(document)?,
        })
    }
    pub fn verify(&self, document: &ReleaseDocument) -> Result<()> {
        if self != &Self::from_release(document)? {
            return Err(AdminError::Conflict);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct FxSnapshot {
    pub key: String,
    pub revision: String,
    pub raw: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fraction {
    numerator: u64,
    denominator: u64,
}
impl Fraction {
    fn parse(raw: &str) -> Result<Self> {
        let value = raw.trim();
        if value.is_empty() || value.len() > 32 {
            return Err(AdminError::ExchangeRateUnavailable);
        }
        let mut parts = value.split('.');
        let whole = parts.next().unwrap();
        let decimals = parts.next().unwrap_or("");
        if parts.next().is_some()
            || whole.is_empty()
            || whole.len() > 7
            || decimals.len() > 12
            || !whole.bytes().all(|v| v.is_ascii_digit())
            || !decimals.bytes().all(|v| v.is_ascii_digit())
        {
            return Err(AdminError::ExchangeRateUnavailable);
        }
        let denominator = 10u64.pow(decimals.len() as u32);
        let whole: u64 = whole
            .parse()
            .map_err(|_| AdminError::ExchangeRateUnavailable)?;
        let fraction: u64 = if decimals.is_empty() {
            0
        } else {
            decimals
                .parse()
                .map_err(|_| AdminError::ExchangeRateUnavailable)?
        };
        let numerator = whole
            .checked_mul(denominator)
            .and_then(|n| n.checked_add(fraction))
            .ok_or(AdminError::ExchangeRateUnavailable)?;
        if numerator == 0 || numerator > 1_000_000 * denominator {
            return Err(AdminError::ExchangeRateUnavailable);
        }
        let divisor = gcd(numerator as u128, denominator as u128) as u64;
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
    fn scale(self, numerator: u64, denominator: u64) -> Result<(u64, u64)> {
        let numerator = u128::from(numerator) * u128::from(self.numerator);
        let denominator = u128::from(denominator) * u128::from(self.denominator);
        if denominator == 0 {
            return Err(AdminError::InvalidSource);
        }
        let divisor = gcd(numerator, denominator);
        let numerator =
            u64::try_from(numerator / divisor).map_err(|_| AdminError::Unrepresentable)?;
        let denominator =
            u64::try_from(denominator / divisor).map_err(|_| AdminError::Unrepresentable)?;
        if numerator > i64::MAX as u64 || denominator > 1_000_000_000_000_000_000 {
            return Err(AdminError::Unrepresentable);
        }
        Ok((numerator, denominator))
    }
    fn integer(self, value: u64) -> Result<u64> {
        let (numerator, denominator) = self.scale(value, 1)?;
        if denominator != 1 {
            return Err(AdminError::Unrepresentable);
        }
        Ok(numerator)
    }
    fn decimal(self, value: &mut DecimalRate) -> Result<()> {
        (value.numerator, value.denominator) = self.scale(value.numerator, value.denominator)?;
        Ok(())
    }
    fn search(self, value: &mut SearchRate) -> Result<()> {
        (value.numerator, value.denominator) = self.scale(value.numerator, value.denominator)?;
        Ok(())
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn factor(adjustment: Adjustment, fx: Option<&FxSnapshot>) -> Result<Fraction> {
    if adjustment == Adjustment::Keep {
        return Ok(Fraction {
            numerator: 1,
            denominator: 1,
        });
    }
    let fx = fx.ok_or(AdminError::ExchangeRateUnavailable)?;
    let mut fraction = Fraction::parse(&fx.raw)?;
    if adjustment == Adjustment::Divide {
        std::mem::swap(&mut fraction.numerator, &mut fraction.denominator);
    }
    Ok(fraction)
}
fn scale_rule(rule: &mut PriceRule, factor: Fraction) -> Result<()> {
    if rule.billing_mode == BillingMode::UpstreamCost {
        return Err(AdminError::UnsupportedPrice);
    }
    for rate in [
        &mut rule.rates.input,
        &mut rule.rates.output,
        &mut rule.rates.cache_read,
        &mut rule.rates.cache_write,
        &mut rule.rates.request,
    ] {
        *rate = factor.integer(*rate)?;
    }
    if let Some(tariff) = &mut rule.tiered {
        for tier in &mut tariff.tiers {
            for value in [
                &mut tier.rates.input,
                &mut tier.rates.output,
                &mut tier.rates.cache_read,
                &mut tier.rates.cache_write,
            ] {
                factor.decimal(value)?;
            }
            if let Some(value) = &mut tier.rates.cache_write_1h {
                factor.decimal(value)?;
            }
            if let Some(value) = &mut tier.rates.cache_write_12h {
                factor.decimal(value)?;
            }
        }
    }
    if let Some(embedding) = &mut rule.embedding {
        factor.decimal(&mut embedding.text_usd_per_million)?;
        factor.decimal(&mut embedding.image_usd_per_million)?;
    }
    if let Some(search) = &mut rule.search {
        factor.search(&mut search.usd_per_unit)?;
        if let Some(value) = &mut search.x_posts_fetched_usd_per_unit {
            factor.search(value)?;
        }
        if let Some(value) = &mut search.x_users_fetched_usd_per_unit {
            factor.search(value)?;
        }
    }
    Ok(())
}

/// Strip only monetary coefficients, provenance digest and the local rule ID.
/// A foreign source may not change provider counters, tier bounds, multipliers,
/// model/group/speed identity or the input/cache interpretation.
fn contract(rule: &PriceRule) -> Result<Value> {
    let mut rule = rule.clone();
    rule.id = 1;
    rule.rates = TokenRates {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        request: 0,
    };
    if let Some(tariff) = &mut rule.tiered {
        tariff.source_digest = "0".repeat(64);
        for tier in &mut tariff.tiers {
            let zero = DecimalRate {
                numerator: 0,
                denominator: 1,
            };
            tier.rates.input = zero;
            tier.rates.output = zero;
            tier.rates.cache_read = zero;
            tier.rates.cache_write = zero;
            if tier.rates.cache_write_1h.is_some() {
                tier.rates.cache_write_1h = Some(zero);
            }
            if tier.rates.cache_write_12h.is_some() {
                tier.rates.cache_write_12h = Some(zero);
            }
        }
    }
    if let Some(search) = &mut rule.search {
        let zero = SearchRate {
            numerator: 0,
            denominator: 1,
        };
        search.usd_per_unit = zero;
        if search.x_posts_fetched_usd_per_unit.is_some() {
            search.x_posts_fetched_usd_per_unit = Some(zero);
        }
        if search.x_users_fetched_usd_per_unit.is_some() {
            search.x_users_fetched_usd_per_unit = Some(zero);
        }
    }
    if let Some(embedding) = &mut rule.embedding {
        let zero = DecimalRate {
            numerator: 0,
            denominator: 1,
        };
        embedding.text_usd_per_million = zero;
        embedding.image_usd_per_million = zero;
    }
    serde_json::to_value(rule).map_err(|_| AdminError::Unavailable)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricePreviewRequest {
    pub expected: Expected,
    /// JSON is transported as text so browsers never round u64/fraction fields.
    pub source_json: String,
    pub selections: Vec<PriceSelection>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriorityChange {
    pub target_id: String,
    pub priority: u16,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriorityPreviewRequest {
    pub expected: Expected,
    pub route_id: String,
    pub changes: Vec<PriorityChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Plan {
    pub kind: String,
    pub before: ReleaseDocument,
    pub after: ReleaseDocument,
    pub source_sha256: String,
    pub fx: Option<FxSnapshot>,
    pub changes: Value,
}
fn next(value: u64) -> Result<u64> {
    value
        .checked_add(1)
        .filter(|value| *value <= MAX_SAFE)
        .ok_or(AdminError::Unavailable)
}
pub(super) fn prices(
    before: &ReleaseDocument,
    input: PricePreviewRequest,
    fx: Option<FxSnapshot>,
) -> Result<Plan> {
    input.expected.verify(before)?;
    if input.selections.is_empty() || input.selections.len() > MAX_SELECTIONS {
        return Err(AdminError::Invalid);
    }
    let source = PriceSource::parse(&input.source_json)?;
    let source_sha256 = hex(Sha256::digest(input.source_json.as_bytes()).as_slice());
    let mut selections = BTreeMap::new();
    for selection in input.selections {
        if selections
            .insert(selection.model, selection.adjustment)
            .is_some()
        {
            return Err(AdminError::Invalid);
        }
    }
    let mut source_rules = BTreeMap::new();
    for rule in source.prices.rules {
        source_rules.insert((rule.model.clone(), rule.group.clone(), rule.speed), rule);
    }
    let mut after = before.clone();
    let mut found = BTreeSet::new();
    let mut changes = Vec::new();
    for rule in &mut after.pricing.rules {
        let Some(adjustment) = selections.get(&rule.model) else {
            continue;
        };
        let source = source_rules
            .get(&(rule.model.clone(), rule.group.clone(), rule.speed))
            .ok_or(AdminError::InvalidSource)?;
        if contract(rule)? != contract(source)? {
            return Err(AdminError::UnsupportedPrice);
        }
        let original = rule.clone();
        let mut adjusted = source.clone();
        adjusted.id = original.id;
        scale_rule(&mut adjusted, factor(*adjustment, fx.as_ref())?)?;
        found.insert(rule.model.clone());
        changes.push(json!({"model":rule.model,"group":rule.group,"speed":rule.speed,"rule_id":rule.id.to_string(),"adjustment":adjustment,"before":price_view(&original),"source":price_view(source),"after":price_view(&adjusted)}));
        *rule = adjusted;
    }
    if found.len() != selections.len() || after.pricing == before.pricing {
        return Err(AdminError::Invalid);
    }
    after.routing.version = next(before.routing.version)?;
    after.pricing.version = next(before.pricing.version)?;
    after
        .clone()
        .into_router()
        .map_err(|_| AdminError::UnsupportedPrice)?;
    let uses_fx = selections
        .values()
        .any(|choice| *choice != Adjustment::Keep);
    Ok(Plan {
        kind: "prices".to_owned(),
        before: before.clone(),
        after,
        source_sha256,
        fx: if uses_fx { fx } else { None },
        changes: json!(changes),
    })
}
pub(super) fn priorities(before: &ReleaseDocument, input: PriorityPreviewRequest) -> Result<Plan> {
    input.expected.verify(before)?;
    if input.changes.is_empty() || input.changes.len() > 64 {
        return Err(AdminError::Invalid);
    }
    let id: u64 = input.route_id.parse().map_err(|_| AdminError::Invalid)?;
    let mut updates = BTreeMap::new();
    for change in input.changes {
        let target_id: u64 = change.target_id.parse().map_err(|_| AdminError::Invalid)?;
        if updates.insert(target_id, change.priority).is_some() {
            return Err(AdminError::Invalid);
        }
    }
    let mut after = before.clone();
    let route = after
        .routing
        .routes
        .iter_mut()
        .find(|route| route.id == id)
        .ok_or(AdminError::Invalid)?;
    let mut changes = Vec::new();
    for target in &mut route.targets {
        if let Some(priority) = updates.remove(&target.id) {
            if priority != target.priority {
                changes.push(json!({"route_id":id.to_string(),"target_id":target.id.to_string(),"upstream_id":target.upstream_id.to_string(),"before":target.priority,"after":priority}));
            }
            target.priority = priority;
        }
    }
    if !updates.is_empty() || changes.is_empty() {
        return Err(AdminError::Invalid);
    }
    after.routing.version = next(before.routing.version)?;
    after
        .clone()
        .into_router()
        .map_err(|_| AdminError::Invalid)?;
    Ok(Plan {
        kind: "priorities".to_owned(),
        before: before.clone(),
        after,
        source_sha256: digest(&changes)?,
        fx: None,
        changes: json!(changes),
    })
}

/// Validate a durable update against this process's host-controlled destinations.
/// Runtime publication cannot edit credentials, endpoints, groups or protocol maps.
pub(super) fn same_host_contract(base: &ReleaseDocument, next: &ReleaseDocument) -> Result<bool> {
    let mut a = base.routing.clone();
    let mut b = next.routing.clone();
    a.version = 1;
    b.version = 1;
    for routing in [&mut a, &mut b] {
        routing.groups.sort_by(|a, b| a.name.cmp(&b.name));
        routing.upstreams.sort_by_key(|v| v.id);
        routing.routes.sort_by_key(|v| v.id);
        routing
            .aliases
            .sort_by(|a, b| (&a.group, &a.alias).cmp(&(&b.group, &b.alias)));
        for route in &mut routing.routes {
            route.targets.sort_by_key(|v| v.id);
            for target in &mut route.targets {
                target.priority = 0;
            }
        }
    }
    if a != b || base.pricing.rules.len() != next.pricing.rules.len() {
        return Ok(false);
    }
    let mut next_rules = BTreeMap::new();
    for rule in &next.pricing.rules {
        next_rules.insert(rule.id, rule);
    }
    for rule in &base.pricing.rules {
        let Some(next) = next_rules.get(&rule.id) else {
            return Ok(false);
        };
        if contract(rule)? != contract(next)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn digest(value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| AdminError::Unavailable)?;
    Ok(hex(Sha256::digest(bytes).as_slice()))
}
fn fraction(numerator: u64, denominator: u64) -> Value {
    json!({"numerator":numerator.to_string(),"denominator":denominator.to_string()})
}
fn price_view(rule: &PriceRule) -> Value {
    let mut values = Vec::new();
    let mut add = |name: &str, unit: &str, bound: Option<u64>, numerator: u64, denominator: u64| {
        values.push(json!({"name":name,"unit":unit,"up_to_input_tokens":bound.map(|v|v.to_string()),"amount":fraction(numerator,denominator)}))
    };
    if rule.billing_mode == BillingMode::UpstreamCost {
        // Provider-reported monetary usage has no static token price.
    } else if let Some(embedding) = &rule.embedding {
        add(
            "embedding_text",
            "million_tokens",
            None,
            embedding.text_usd_per_million.numerator,
            embedding.text_usd_per_million.denominator,
        );
        add(
            "embedding_image",
            "million_tokens",
            None,
            embedding.image_usd_per_million.numerator,
            embedding.image_usd_per_million.denominator,
        );
    } else if let Some(tiered) = &rule.tiered {
        for tier in &tiered.tiers {
            for (name, value) in [
                ("input", tier.rates.input),
                ("output", tier.rates.output),
                ("cache_read", tier.rates.cache_read),
                ("cache_write", tier.rates.cache_write),
            ] {
                add(
                    name,
                    "million_tokens",
                    tier.up_to_input_tokens,
                    value.numerator,
                    value.denominator,
                );
            }
            if let Some(value) = tier.rates.cache_write_1h {
                add(
                    "cache_write_1h",
                    "million_tokens",
                    tier.up_to_input_tokens,
                    value.numerator,
                    value.denominator,
                );
            }
            if let Some(value) = tier.rates.cache_write_12h {
                add(
                    "cache_write_12h",
                    "million_tokens",
                    tier.up_to_input_tokens,
                    value.numerator,
                    value.denominator,
                );
            }
        }
    } else {
        for (name, value) in [
            ("input", rule.rates.input),
            ("output", rule.rates.output),
            ("cache_read", rule.rates.cache_read),
            ("cache_write", rule.rates.cache_write),
        ] {
            add(name, "million_tokens", None, value, 1_000_000);
        }
        add("request", "request", None, rule.rates.request, 1_000_000);
    }
    if let Some(search) = rule.search {
        let unit = match search.unit {
            crate::routing::SearchBillingUnit::Call => "search_call",
            crate::routing::SearchBillingUnit::Query => "search_query",
            crate::routing::SearchBillingUnit::GroundedRequest => "grounded_request",
        };
        add(
            "search",
            unit,
            None,
            search.usd_per_unit.numerator,
            search.usd_per_unit.denominator,
        );
        if let Some(value) = search.x_posts_fetched_usd_per_unit {
            add(
                "x_posts_fetched",
                "item",
                None,
                value.numerator,
                value.denominator,
            );
        }
        if let Some(value) = search.x_users_fetched_usd_per_unit {
            add(
                "x_users_fetched",
                "item",
                None,
                value.numerator,
                value.denominator,
            );
        }
    }
    json!({"model":rule.model,"group":rule.group,"speed":rule.speed,"rule_id":rule.id.to_string(),"billing_mode":rule.billing_mode,"group_multiplier_ppm":rule.group_multiplier_ppm,"speed_multiplier_ppm":rule.speed_multiplier_ppm,"currency":"USD","prices":values})
}
pub(super) fn catalog(
    document: &ReleaseDocument,
    fx: Option<FxSnapshot>,
    can_prices: bool,
    can_priorities: bool,
) -> Result<Value> {
    if document.pricing.rules.len() > 2000 || document.routing.routes.len() > 2000 {
        return Err(AdminError::TooLarge);
    }
    let prices: Vec<_> = document.pricing.rules.iter().map(price_view).collect();
    let routes: Vec<_> = document.routing.routes.iter().map(|route|json!({"route_id":route.id.to_string(),"model":route.model,"group":route.group,"targets":route.targets.iter().map(|target|json!({"target_id":target.id.to_string(),"upstream_id":target.upstream_id.to_string(),"model":target.upstream_model,"priority":target.priority,"weight":target.weight,"enabled":target.enabled})).collect::<Vec<_>>()})).collect();
    let usable_fx = fx
        .as_ref()
        .is_some_and(|fx| Fraction::parse(&fx.raw).is_ok());
    Ok(
        json!({"contract":"core.routing-admin.v1","expected":Expected::from_release(document)?,"prices":prices,"routes":routes,"exchange_rate":fx,"exchange_rate_available":usable_fx,"can_edit_prices":can_prices,"can_edit_priorities":can_priorities}),
    )
}
