//! Server-owned image tariffs. No token usage is synthesized for image charges.
//! Historical rule evidence: main 3948a0a relaykit/dto/openai_image.go:135-170.
use crate::billing::{CREDITS_PER_USD, Credits};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ImageEndpoint {
    Generations,
    Edits,
}

/// Exact nonnegative fraction; no float or intermediate currency rounding.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExactRatio {
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageCountBasis {
    Requested,
    ReturnedPayloads,
}

/// Deployment configuration only. Do not deserialize this from model HTTP input.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImagePriceRule {
    pub upstream_id: u64,
    pub vendor: String,
    pub billing_group: String,
    pub upstream_model: String,
    pub semantic_model: String,
    pub endpoint: ImageEndpoint,
    pub size: String,
    pub quality: String,
    /// USD micro per image, including the selected size and quality.
    pub usd_micros_per_image: ExactRatio,
    /// Installation-owned group multiplier, never a caller supplied discount.
    pub group_multiplier: ExactRatio,
    pub max_n: u32,
    pub count_basis: ImageCountBasis,
    pub max_budget: Credits,
    /// Reviewed supplier tariff or deployment contract revision.
    pub source_revision: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagePriceConfig {
    pub revision: u64,
    pub rules: Vec<ImagePriceRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageQuoteRequest {
    pub upstream_id: u64,
    pub vendor: String,
    pub billing_group: String,
    pub upstream_model: String,
    pub semantic_model: String,
    pub endpoint: ImageEndpoint,
    pub size: String,
    pub quality: String,
    pub n: u32,
}

/// Immutable after quotation; not Deserialize, so clients cannot mint quotes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageQuote {
    binding: ImageQuoteRequest,
    revision: u64,
    source_revision: String,
    charge: Credits,
    max_budget: Credits,
    tariff: ImagePriceRule,
}
impl ImageQuote {
    /// Reservation is the full requested-count amount, independent of client disconnect.
    pub fn reservation_credits(&self) -> Credits {
        self.charge
    }
    /// Only Core-parsed completed payloads may supply this count. Zero completion
    /// is unknown, never evidence authorizing a refund after dispatch.
    pub fn settlement(&self, successful_images: u32) -> Result<Credits, ImagePriceError> {
        if successful_images > self.binding.n {
            return Err(ImagePriceError::InvalidCount);
        }
        let n = match self.tariff.count_basis {
            ImageCountBasis::Requested => self.binding.n,
            ImageCountBasis::ReturnedPayloads if successful_images > 0 => successful_images,
            ImageCountBasis::ReturnedPayloads => return Err(ImagePriceError::SettlementPending),
        };
        cost(&self.tariff, n)
    }
    pub fn persisted(&self) -> PersistedImagePrice {
        PersistedImagePrice {
            revision: self.revision,
            rule: self.tariff.clone(),
            request: self.binding.clone(),
            reservation_credits: self.charge,
        }
    }
    pub fn binding(&self) -> &ImageQuoteRequest {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn source_revision(&self) -> &str {
        &self.source_revision
    }
    pub fn charge(&self) -> Credits {
        self.charge
    }
    pub fn max_budget(&self) -> Credits {
        self.max_budget
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImagePriceError {
    InvalidConfig,
    SettlementPending,
    UnsupportedDimensions,
    InvalidCount,
    Overflow,
    BudgetExceeded,
}

/// Core-parsed image completion evidence; never supplier/Go spending authority.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(try_from = "ImageUsageWire")]
pub struct ImageUsage {
    pub output_images: u32,
    pub provider_receipt_sha256: [u8; 32],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageUsageWire {
    output_images: u32,
    provider_receipt_sha256: [u8; 32],
}
impl TryFrom<ImageUsageWire> for ImageUsage {
    type Error = &'static str;
    fn try_from(value: ImageUsageWire) -> Result<Self, Self::Error> {
        let result = Self {
            output_images: value.output_images,
            provider_receipt_sha256: value.provider_receipt_sha256,
        };
        result
            .validate()
            .map_err(|_| "invalid image usage evidence")?;
        Ok(result)
    }
}
impl ImageUsage {
    pub(crate) fn validate(&self) -> Result<(), ImagePriceError> {
        if !(1..=128).contains(&self.output_images) || self.provider_receipt_sha256 == [0; 32] {
            return Err(ImagePriceError::SettlementPending);
        }
        Ok(())
    }
    /// Image receipts cannot be extended with invented monotone token usage.
    pub(crate) fn follows(&self, previous: &Self) -> bool {
        self == previous && self.validate().is_ok()
    }
}

/// Private Core checkpoint payload; never accept this DTO from HTTP or Go RPC.
/// Restore validates a frozen tariff independently of current deployment pricing.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersistedImagePrice {
    revision: u64,
    rule: ImagePriceRule,
    request: ImageQuoteRequest,
    reservation_credits: Credits,
}
impl PersistedImagePrice {
    pub(crate) fn validate(&self) -> Result<(), ImagePriceError> {
        self.clone().restore().map(|_| ())
    }
    pub(crate) fn cost(&self, usage: &ImageUsage) -> Result<i64, ImagePriceError> {
        usage.validate()?;
        self.clone()
            .restore()?
            .settlement(usage.output_images)
            .map(i64::from)
    }
    pub(crate) fn model(&self) -> &str {
        &self.request.semantic_model
    }
    pub(crate) fn group(&self) -> &str {
        &self.request.billing_group
    }
    pub(crate) fn version(&self) -> u64 {
        self.revision
    }
    pub fn restore(self) -> Result<ImageQuote, ImagePriceError> {
        let config = ImagePriceConfig {
            revision: self.revision,
            rules: vec![self.rule],
        };
        let quote = config.quote(&self.request)?;
        if quote.binding != self.request || quote.charge != self.reservation_credits {
            return Err(ImagePriceError::InvalidConfig);
        }
        Ok(quote)
    }
}

impl ImageQuoteRequest {
    /// Same semantic defaults as historical JSON ImageRequest validation.
    /// Call before encoding the upstream request, so charged and sent dimensions agree.
    pub fn normalized(mut self) -> Result<Self, ImagePriceError> {
        if self.n == 0 {
            self.n = 1;
        }
        if self.n > 128 {
            return Err(ImagePriceError::InvalidCount);
        }
        if matches!(
            self.semantic_model.as_str(),
            "dall-e" | "dall-e-2" | "dall-e-3"
        ) {
            if self.size.is_empty() {
                self.size = "1024x1024".into();
            }
            let allowed = if self.semantic_model == "dall-e-3" {
                ["1024x1024", "1024x1792", "1792x1024"]
            } else {
                ["256x256", "512x512", "1024x1024"]
            };
            if !allowed.contains(&self.size.as_str()) {
                return Err(ImagePriceError::UnsupportedDimensions);
            }
        }
        if self.quality.is_empty() {
            if self.semantic_model == "dall-e-3" {
                self.quality = "standard".into();
            }
            if self.semantic_model == "gpt-image-1" {
                self.quality = "auto".into();
            }
        }
        Ok(self)
    }
}

impl ImagePriceConfig {
    pub fn validate(&self) -> Result<(), ImagePriceError> {
        if self.revision == 0 || self.rules.len() > 100_000 {
            return Err(ImagePriceError::InvalidConfig);
        }
        let mut seen = BTreeSet::new();
        for r in &self.rules {
            if r.upstream_id == 0
                || !label(&r.vendor, false)
                || !label(&r.billing_group, false)
                || !label(&r.upstream_model, false)
                || !label(&r.semantic_model, false)
                || !label(&r.size, true)
                || !label(&r.quality, true)
                || !label(&r.source_revision, false)
                || !(1..=128).contains(&r.max_n)
                || r.usd_micros_per_image.denominator == 0
                || r.group_multiplier.denominator == 0
                || !seen.insert((
                    r.upstream_id,
                    &r.vendor,
                    &r.billing_group,
                    &r.upstream_model,
                    &r.semantic_model,
                    r.endpoint,
                    &r.size,
                    &r.quality,
                ))
            {
                return Err(ImagePriceError::InvalidConfig);
            }
            let request = request_for(r, 1).normalized()?;
            if request.size != r.size || request.quality != r.quality {
                return Err(ImagePriceError::InvalidConfig);
            }
            // Every configured count must have a representable, fully reserved price.
            let charge = cost(r, r.max_n)?;
            if i64::from(charge) > i64::from(r.max_budget) {
                return Err(ImagePriceError::BudgetExceeded);
            }
        }
        Ok(())
    }

    pub fn quote(&self, request: &ImageQuoteRequest) -> Result<ImageQuote, ImagePriceError> {
        self.validate()?;
        let request = request.clone().normalized()?;
        let r = self
            .rules
            .iter()
            .find(|r| {
                let expected = request_for(r, request.n);
                expected == request
            })
            .ok_or(ImagePriceError::UnsupportedDimensions)?;
        if request.n > r.max_n {
            return Err(ImagePriceError::InvalidCount);
        }
        let charge = cost(r, request.n)?;
        if i64::from(charge) > i64::from(r.max_budget) {
            return Err(ImagePriceError::BudgetExceeded);
        }
        Ok(ImageQuote {
            binding: request,
            revision: self.revision,
            source_revision: r.source_revision.clone(),
            charge,
            max_budget: r.max_budget,
            tariff: r.clone(),
        })
    }
}
fn label(value: &str, empty: bool) -> bool {
    (empty || !value.is_empty()) && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn request_for(r: &ImagePriceRule, n: u32) -> ImageQuoteRequest {
    ImageQuoteRequest {
        upstream_id: r.upstream_id,
        vendor: r.vendor.clone(),
        billing_group: r.billing_group.clone(),
        upstream_model: r.upstream_model.clone(),
        semantic_model: r.semantic_model.clone(),
        endpoint: r.endpoint,
        size: r.size.clone(),
        quality: r.quality.clone(),
        n,
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let rem = a % b;
        a = b;
        b = rem;
    }
    a
}
fn cost(r: &ImagePriceRule, n: u32) -> Result<Credits, ImagePriceError> {
    let mut numerators = [
        u128::from(r.usd_micros_per_image.numerator),
        u128::from(r.group_multiplier.numerator),
        u128::from(n),
        CREDITS_PER_USD as u128,
    ];
    let mut denominators = [
        u128::from(r.usd_micros_per_image.denominator),
        u128::from(r.group_multiplier.denominator),
        1_000_000,
    ];
    if denominators.contains(&0) {
        return Err(ImagePriceError::InvalidConfig);
    }
    for a in &mut numerators {
        for b in &mut denominators {
            let divisor = gcd(*a, *b);
            *a /= divisor;
            *b /= divisor;
        }
    }
    let numerator = numerators
        .into_iter()
        .try_fold(1_u128, |a, b| a.checked_mul(b))
        .ok_or(ImagePriceError::Overflow)?;
    let denominator = denominators
        .into_iter()
        .try_fold(1_u128, |a, b| a.checked_mul(b))
        .ok_or(ImagePriceError::Overflow)?;
    let credits = numerator / denominator + u128::from(numerator % denominator != 0);
    let credits = i64::try_from(credits).map_err(|_| ImagePriceError::Overflow)?;
    Credits::try_from(credits).map_err(|_| ImagePriceError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ImagePriceConfig {
        ImagePriceConfig {
            revision: 7,
            rules: vec![ImagePriceRule {
                upstream_id: 42,
                vendor: "openai".into(),
                billing_group: "default".into(),
                upstream_model: "supplier-image".into(),
                semantic_model: "dall-e-3".into(),
                endpoint: ImageEndpoint::Generations,
                size: "1024x1024".into(),
                quality: "standard".into(),
                usd_micros_per_image: ExactRatio {
                    numerator: 40_000,
                    denominator: 1,
                },
                group_multiplier: ExactRatio {
                    numerator: 1,
                    denominator: 1,
                },
                max_n: 128,
                count_basis: ImageCountBasis::ReturnedPayloads,
                max_budget: Credits::try_from(3_000_000).unwrap(),
                source_revision: "main3948a0a-tariff-test".into(),
            }],
        }
    }
    #[test]
    fn exact_usd_and_snapshot() {
        let c = config();
        let q = c.quote(&request_for(&c.rules[0], 3)).unwrap();
        assert_eq!(i64::from(q.charge()), 60_000);
        assert_eq!(q.revision(), 7);
        assert_eq!(q.binding().n, 3);
    }
    #[test]
    fn rounds_only_after_count_and_group() {
        let mut c = config();
        c.rules[0].usd_micros_per_image = ExactRatio {
            numerator: 1,
            denominator: 3,
        };
        c.rules[0].group_multiplier = ExactRatio {
            numerator: 1,
            denominator: 2,
        };
        assert_eq!(
            i64::from(c.quote(&request_for(&c.rules[0], 12)).unwrap().charge()),
            1
        );
        assert_eq!(
            i64::from(c.quote(&request_for(&c.rules[0], 13)).unwrap().charge()),
            2
        );
    }
    #[test]
    fn identity_and_dimensions_are_all_bound() {
        let c = config();
        for field in 0..8 {
            let mut req = request_for(&c.rules[0], 1);
            match field {
                0 => req.upstream_id += 1,
                1 => req.vendor = "other".into(),
                2 => req.upstream_model = "other".into(),
                3 => req.semantic_model = "other".into(),
                4 => req.endpoint = ImageEndpoint::Edits,
                5 => req.size = "1024x1792".into(),
                6 => req.quality = "hd".into(),
                _ => req.billing_group = "other".into(),
            };
            assert_eq!(c.quote(&req), Err(ImagePriceError::UnsupportedDimensions));
        }
    }
    #[test]
    fn count_defaults_and_limit() {
        let c = config();
        assert_eq!(
            c.quote(&request_for(&c.rules[0], 0)).unwrap().binding().n,
            1
        );
        assert_eq!(
            c.quote(&request_for(&c.rules[0], 129)),
            Err(ImagePriceError::InvalidCount)
        );
    }
    #[test]
    fn defaults_are_canonical_and_not_price_wildcards() {
        let c = config();
        let mut req = request_for(&c.rules[0], 1);
        req.size.clear();
        req.quality.clear();
        assert_eq!(
            c.quote(&req).unwrap().binding(),
            &request_for(&c.rules[0], 1)
        );
        let mut c = c;
        c.rules[0].quality.clear();
        assert_eq!(c.validate(), Err(ImagePriceError::InvalidConfig));
    }
    #[test]
    fn ambiguous_and_invalid_tariffs_fail_closed() {
        let mut c = config();
        c.rules.push(c.rules[0].clone());
        assert_eq!(c.validate(), Err(ImagePriceError::InvalidConfig));
        c.rules.pop();
        c.rules[0].usd_micros_per_image.denominator = 0;
        assert_eq!(c.validate(), Err(ImagePriceError::InvalidConfig));
    }
    #[test]
    fn no_overflow_or_underreservation() {
        let mut c = config();
        c.rules[0].max_budget = Credits::try_from(1).unwrap();
        assert_eq!(c.validate(), Err(ImagePriceError::BudgetExceeded));
        c.rules[0].max_budget = Credits::try_from(i64::MAX).unwrap();
        c.rules[0].usd_micros_per_image.numerator = u64::MAX;
        assert_eq!(c.validate(), Err(ImagePriceError::Overflow));
    }
    #[test]
    fn free_is_explicit() {
        let mut c = config();
        c.rules[0].usd_micros_per_image.numerator = 0;
        c.rules[0].max_budget = Credits::try_from(0).unwrap();
        assert_eq!(
            i64::from(c.quote(&request_for(&c.rules[0], 128)).unwrap().charge()),
            0
        );
    }
    #[test]
    fn frozen_settlement_and_restore() {
        let mut c = config();
        let q = c.quote(&request_for(&c.rules[0], 3)).unwrap();
        c.rules[0].usd_micros_per_image.numerator *= 10;
        assert_eq!(i64::from(q.settlement(2).unwrap()), 40_000);
        assert_eq!(q.settlement(0), Err(ImagePriceError::SettlementPending));
        assert_eq!(q.settlement(4), Err(ImagePriceError::InvalidCount));
        let restored = q.persisted().restore().unwrap();
        assert_eq!(q, restored);
        let mut altered = q.persisted();
        altered.reservation_credits = Credits::try_from(1).unwrap();
        assert_eq!(altered.restore(), Err(ImagePriceError::InvalidConfig));
    }
    #[test]
    fn requested_count_contract_stays_fixed() {
        let mut c = config();
        c.rules[0].count_basis = ImageCountBasis::Requested;
        let q = c.quote(&request_for(&c.rules[0], 3)).unwrap();
        assert_eq!(i64::from(q.settlement(1).unwrap()), 60_000);
    }
    #[test]
    fn all_main_dalle_dimensions_can_be_explicitly_priced() {
        let mut c = config();
        c.rules.clear();
        // Main base price exists for DALL-E 3; DALL-E 2's base is operator config.
        // Here base is synthetic 40000 micro for both, testing dimensions only.
        for (model, size, quality, numerator, denominator) in [
            ("dall-e-2", "256x256", "", 2, 5),
            ("dall-e-2", "512x512", "", 9, 20),
            ("dall-e-2", "1024x1024", "", 1, 1),
            ("dall-e-3", "1024x1024", "standard", 1, 1),
            ("dall-e-3", "1024x1792", "standard", 2, 1),
            ("dall-e-3", "1792x1024", "standard", 2, 1),
            ("dall-e-3", "1024x1024", "hd", 2, 1),
            ("dall-e-3", "1024x1792", "hd", 3, 1),
            ("dall-e-3", "1792x1024", "hd", 3, 1),
        ] {
            let mut r = config().rules.remove(0);
            r.semantic_model = model.into();
            r.size = size.into();
            r.quality = quality.into();
            r.usd_micros_per_image = ExactRatio {
                numerator: 40_000 * numerator,
                denominator,
            };
            r.max_budget = Credits::try_from(10_000_000).unwrap();
            c.rules.push(r);
        }
        c.validate().unwrap();
        for (r, expected) in c
            .rules
            .iter()
            .zip([8000, 9000, 20000, 20000, 40000, 40000, 40000, 60000, 60000])
        {
            assert_eq!(
                i64::from(c.quote(&request_for(r, 1)).unwrap().charge()),
                expected
            );
        }
    }
    #[test]
    fn receipt_evidence_cannot_advance_or_erase() {
        let usage = ImageUsage {
            output_images: 2,
            provider_receipt_sha256: [1; 32],
        };
        assert!(usage.follows(&usage));
        let mut altered = usage.clone();
        altered.output_images = 3;
        assert!(!altered.follows(&usage));
        altered = usage.clone();
        altered.provider_receipt_sha256 = [2; 32];
        assert!(!altered.follows(&usage));
        altered.output_images = 0;
        assert!(altered.validate().is_err());
        altered = usage.clone();
        altered.provider_receipt_sha256 = [0; 32];
        assert!(altered.validate().is_err());
        let c = config();
        let q = c.quote(&request_for(&c.rules[0], 3)).unwrap();
        assert_eq!(q.persisted().cost(&usage).unwrap(), 40_000);
        assert_eq!(q.persisted().model(), "dall-e-3");
        assert_eq!(q.persisted().group(), "default");
        assert_eq!(q.persisted().version(), 7);
        q.persisted().validate().unwrap();
    }
    #[test]
    fn strict_receipt_and_checkpoint_roundtrip() {
        let valid = ImageUsage {
            output_images: 1,
            provider_receipt_sha256: [1; 32],
        };
        let mut wire = serde_json::to_value(valid).unwrap();
        assert_eq!(
            serde_json::from_value::<ImageUsage>(wire.clone()).unwrap(),
            valid
        );
        wire["output_images"] = serde_json::json!(0);
        assert!(serde_json::from_value::<ImageUsage>(wire.clone()).is_err());
        wire["output_images"] = serde_json::json!(1);
        wire["payer"] = serde_json::json!(1);
        assert!(serde_json::from_value::<ImageUsage>(wire).is_err());
        let c = config();
        let quote = c.quote(&request_for(&c.rules[0], 1)).unwrap();
        let wire = serde_json::to_value(quote.persisted()).unwrap();
        let restored: PersistedImagePrice = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(restored.restore().unwrap(), quote);
        let mut wire = wire;
        wire["client_price"] = serde_json::json!(0);
        assert!(serde_json::from_value::<PersistedImagePrice>(wire).is_err());
    }
}
