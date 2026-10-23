//! Explicit multimodal embedding prices and measured input partitions.
//! The supplier contract selects the meaning of native counters; a model name
//! or a client's requested budget cannot establish either a price or a bound.
use num_bigint::BigUint;
use num_rational::Ratio;
use serde::{Deserialize, Serialize};

use super::{DecimalRate, MAX_MULTIPLIER, RoutingError};
use crate::billing::trust_pricing::DiscountRatio;

pub const MAX_EMBEDDING_INPUT_TOKENS: u64 = 16_000_000;
const MAX_DENOMINATOR: u64 = 1_000_000_000_000_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingProvider {
    /// Native input_tokens is text; image_tokens covers images and video.
    AliSeparateInputV1,
    /// Native input_tokens includes input_tokens_details.{text,image}_tokens.
    AliInclusiveInputV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum EmbeddingCurrency {
    #[serde(rename = "USD")]
    Usd,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingUsage {
    pub provider: EmbeddingProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<u64>,
    /// Images and video share this native supplier billing dimension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
}

impl EmbeddingUsage {
    pub fn valid(self) -> bool {
        if [self.text_tokens, self.image_tokens, self.total_input_tokens]
            .into_iter()
            .flatten()
            .any(|tokens| tokens > MAX_EMBEDDING_INPUT_TOKENS)
        {
            return false;
        }
        let known = self.text_tokens.unwrap_or(0) + self.image_tokens.unwrap_or(0);
        if known > MAX_EMBEDDING_INPUT_TOKENS {
            return false;
        }
        match self.total_input_tokens {
            Some(total) if self.text_tokens.is_some() && self.image_tokens.is_some() => {
                known == total
            }
            Some(total) => known <= total,
            None => true,
        }
    }

    pub fn complete(self) -> bool {
        self.valid()
            && self.text_tokens.is_some()
            && self.image_tokens.is_some()
            && self.total_input_tokens.is_some()
    }

    pub fn follows(self, previous: Self) -> bool {
        self.valid()
            && previous.valid()
            && self.provider == previous.provider
            && [self.text_tokens, self.image_tokens, self.total_input_tokens]
                .into_iter()
                .zip([
                    previous.text_tokens,
                    previous.image_tokens,
                    previous.total_input_tokens,
                ])
                .all(|(now, old)| old.is_none_or(|old| now.is_some_and(|now| now >= old)))
    }
}

/// Exact USD per million input tokens. The two rates are always explicit,
/// including an explicitly free dimension; missing evidence is never free.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingTariff {
    pub revision: u32,
    pub provider: EmbeddingProvider,
    pub currency: EmbeddingCurrency,
    pub text_usd_per_million: DecimalRate,
    pub image_usd_per_million: DecimalRate,
}

impl EmbeddingTariff {
    pub fn validate(self) -> Result<(), RoutingError> {
        if self.revision != 1
            || [self.text_usd_per_million, self.image_usd_per_million]
                .into_iter()
                .any(|rate| {
                    rate.numerator > i64::MAX as u64
                        || !(1..=MAX_DENOMINATOR).contains(&rate.denominator)
                })
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn validate_usage(self, usage: EmbeddingUsage) -> Result<(), RoutingError> {
        self.validate()?;
        if usage.provider != self.provider || !usage.complete() {
            return Err(RoutingError::InvalidRequest);
        }
        Ok(())
    }
}

/// Frozen before dispatch and sufficient for settlement after a restart. The
/// total input bound is shared by both dimensions, not two independent caps.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedEmbeddingPrice {
    pub tariff: EmbeddingTariff,
    pub input_token_limit: u64,
    pub provider_contract_revision: String,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
    pub credits_per_usd: u64,
}

impl PersistedEmbeddingPrice {
    pub fn validate(&self) -> Result<(), RoutingError> {
        self.tariff.validate()?;
        if !(1..=MAX_EMBEDDING_INPUT_TOKENS).contains(&self.input_token_limit)
            || self.provider_contract_revision.is_empty()
            || self.provider_contract_revision.len() > 512
            || self
                .provider_contract_revision
                .chars()
                .any(char::is_control)
            || self.group_multiplier_ppm > MAX_MULTIPLIER
            || !(1..=MAX_MULTIPLIER).contains(&self.speed_multiplier_ppm)
            || self.credits_per_usd != 500_000
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn validate_usage(&self, usage: EmbeddingUsage) -> Result<(), RoutingError> {
        self.validate()?;
        self.tariff.validate_usage(usage)?;
        if usage
            .total_input_tokens
            .is_none_or(|total| total > self.input_token_limit)
        {
            return Err(RoutingError::InvalidRequest);
        }
        Ok(())
    }

    pub fn cost(&self, usage: EmbeddingUsage) -> Result<i64, RoutingError> {
        self.cost_with_discount(usage, None)
    }

    pub fn cost_with_discount(
        &self,
        usage: EmbeddingUsage,
        discount: Option<DiscountRatio>,
    ) -> Result<i64, RoutingError> {
        self.validate_usage(usage)?;
        let amount = amount(
            self.tariff.text_usd_per_million,
            usage.text_tokens.ok_or(RoutingError::InvalidRequest)?,
        ) + amount(
            self.tariff.image_usd_per_million,
            usage.image_tokens.ok_or(RoutingError::InvalidRequest)?,
        );
        self.round_credits(amount, discount)
    }

    pub fn reservation_ceiling(&self) -> Result<i64, RoutingError> {
        self.reservation_ceiling_with_discount(None)
    }

    pub fn reservation_ceiling_with_discount(
        &self,
        discount: Option<DiscountRatio>,
    ) -> Result<i64, RoutingError> {
        self.validate()?;
        let text = amount(self.tariff.text_usd_per_million, self.input_token_limit);
        let image = amount(self.tariff.image_usd_per_million, self.input_token_limit);
        self.round_credits(text.max(image), discount)
    }

    fn round_credits(
        &self,
        amount: Ratio<BigUint>,
        discount: Option<DiscountRatio>,
    ) -> Result<i64, RoutingError> {
        let mut credits = amount
            * Ratio::new(
                BigUint::from(self.group_multiplier_ppm)
                    * BigUint::from(self.speed_multiplier_ppm)
                    * BigUint::from(self.credits_per_usd),
                BigUint::from(1_000_000_000_000_u64),
            );
        if let Some(discount) = discount {
            credits *= Ratio::new(
                BigUint::from(discount.numerator()),
                BigUint::from(discount.denominator()),
            );
        }
        let fractional = credits.numer() % credits.denom() != BigUint::from(0_u8);
        i64::try_from(credits.to_integer() + BigUint::from(u8::from(fractional)))
            .map_err(|_| RoutingError::PriceOverflow)
    }
}

fn amount(rate: DecimalRate, tokens: u64) -> Ratio<BigUint> {
    Ratio::new(
        BigUint::from(rate.numerator) * BigUint::from(tokens),
        BigUint::from(rate.denominator) * BigUint::from(1_000_000_u64),
    )
}

fn invalid() -> RoutingError {
    RoutingError::InvalidConfig("embedding price")
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
