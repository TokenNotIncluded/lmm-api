//! Bounded, decimal token tariffs compiled by the installation owner.
//!
//! This is deliberately not an expression interpreter. Unsupported measured
//! dimensions must not be removed from a legacy expression to create a tariff.
//! Decimal coefficients are exact; settlement rounds once, half upwards, as
//! specified by revision 1. This does not reproduce intermediate float errors.
use num_bigint::BigUint;
use num_rational::Ratio;
use serde::{Deserialize, Serialize};

use super::{MAX_MULTIPLIER, RoutingError};

const MAX_TIERS: usize = 4;
const MAX_DENOMINATOR: u64 = 1_000_000_000_000_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecimalRate {
    /// USD per million tokens, with no intermediate conversion or rounding.
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TieredRates {
    pub input: DecimalRate,
    pub output: DecimalRate,
    pub cache_read: DecimalRate,
    pub cache_write: DecimalRate,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TokenTier {
    /// Inclusive total input bound. The last tier must have no upper bound.
    pub up_to_input_tokens: Option<u64>,
    pub rates: TieredRates,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TieredTariff {
    pub revision: u32,
    /// SHA-256 of the administrator's original expression, for provenance.
    pub source_digest: String,
    /// These flags preserve the legacy expression's used-variable semantics.
    /// In particular, `cc * 0` still excludes cache creation from prompt input.
    pub uses_cache_read: bool,
    pub uses_cache_write: bool,
    pub tiers: Vec<TokenTier>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputSemantics {
    Inclusive,
    AnthropicText,
}

/// Frozen in the charging request, including the selected upstream's usage
/// basis. Neither settlement nor restart recovery consults current prices.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedTieredTariff {
    pub tariff: TieredTariff,
    pub input_semantics: InputSemantics,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
    pub credits_per_usd: u64,
}

impl PersistedTieredTariff {
    pub fn validate(&self) -> Result<(), RoutingError> {
        self.tariff.compile()?;
        if self.group_multiplier_ppm > MAX_MULTIPLIER
            || !(1..=MAX_MULTIPLIER).contains(&self.speed_multiplier_ppm)
            || self.credits_per_usd != 500_000
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn cost(&self, usage: TieredUsage) -> Result<u64, RoutingError> {
        self.validate()?;
        self.tariff.compile()?.charge(
            usage,
            self.input_semantics,
            self.group_multiplier_ppm,
            self.speed_multiplier_ppm,
            self.credits_per_usd,
        )
    }
}

/// The same counters as the relay, including both cache subsets in input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TieredUsage {
    pub input: u64,
    pub cached: u64,
    pub created: u64,
    pub output: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledTariff {
    source_digest: [u8; 32],
    uses_cache_read: bool,
    uses_cache_write: bool,
    count: u8,
    tiers: [TokenTier; MAX_TIERS],
}

impl TieredTariff {
    pub fn compile(&self) -> Result<CompiledTariff, RoutingError> {
        // Revision 1 deliberately accepts only the measured text/cache-read
        // subset. Cache creation requires a separately reviewed usage contract.
        if self.revision != 1
            || self.uses_cache_write
            || !(1..=MAX_TIERS).contains(&self.tiers.len())
        {
            return Err(invalid());
        }
        let mut digest = [0_u8; 32];
        if self.source_digest.len() != 64 {
            return Err(invalid());
        }
        for (i, chunk) in self.source_digest.as_bytes().chunks_exact(2).enumerate() {
            let decode = |v: u8| match v {
                b'0'..=b'9' => Some(v - b'0'),
                b'a'..=b'f' => Some(v - b'a' + 10),
                _ => None,
            };
            digest[i] = decode(chunk[0]).ok_or_else(invalid)? * 16
                + decode(chunk[1]).ok_or_else(invalid)?;
        }
        let mut previous = 0;
        for (i, tier) in self.tiers.iter().enumerate() {
            match tier.up_to_input_tokens {
                Some(bound) if i + 1 < self.tiers.len() && bound > previous => {
                    previous = bound;
                }
                None if i + 1 == self.tiers.len() => {}
                _ => return Err(invalid()),
            }
            for rate in tier.rates.values() {
                if rate.denominator == 0 || rate.denominator > MAX_DENOMINATOR {
                    return Err(invalid());
                }
            }
            if (!self.uses_cache_read && tier.rates.cache_read.numerator != 0)
                || (!self.uses_cache_write && tier.rates.cache_write.numerator != 0)
            {
                return Err(invalid());
            }
        }
        let mut tiers = [self.tiers[0]; MAX_TIERS];
        tiers[..self.tiers.len()].copy_from_slice(&self.tiers);
        Ok(CompiledTariff {
            source_digest: digest,
            uses_cache_read: self.uses_cache_read,
            uses_cache_write: self.uses_cache_write,
            count: self.tiers.len() as u8,
            tiers,
        })
    }
}

impl TieredRates {
    fn values(self) -> [DecimalRate; 4] {
        [self.input, self.output, self.cache_read, self.cache_write]
    }
}

impl CompiledTariff {
    pub fn tariff(self) -> TieredTariff {
        TieredTariff {
            revision: 1,
            source_digest: self
                .source_digest
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect(),
            uses_cache_read: self.uses_cache_read,
            uses_cache_write: self.uses_cache_write,
            tiers: self.tiers[..usize::from(self.count)].to_vec(),
        }
    }

    pub fn uses_cache_write(self) -> bool {
        self.uses_cache_write
    }

    pub fn charge(
        self,
        usage: TieredUsage,
        semantics: InputSemantics,
        group_ppm: u32,
        speed_ppm: u32,
        credits_per_usd: u64,
    ) -> Result<u64, RoutingError> {
        let cached = usage
            .cached
            .checked_add(usage.created)
            .ok_or(RoutingError::InvalidRequest)?;
        if cached > usage.input || usage.created != 0 {
            return Err(RoutingError::InvalidRequest);
        }
        let tier = self.tiers[..usize::from(self.count)]
            .iter()
            .find(|t| {
                t.up_to_input_tokens
                    .is_none_or(|limit| usage.input <= limit)
            })
            .ok_or_else(invalid)?;
        let mut prompt = usage.input;
        if self.uses_cache_read || semantics == InputSemantics::AnthropicText {
            prompt -= usage.cached;
        }
        if self.uses_cache_write || semantics == InputSemantics::AnthropicText {
            prompt -= usage.created;
        }
        let raw = [
            (prompt, tier.rates.input),
            (usage.output, tier.rates.output),
            (usage.cached, tier.rates.cache_read),
            (usage.created, tier.rates.cache_write),
        ]
        .into_iter()
        .try_fold(Fraction::zero(), |sum, (tokens, rate)| {
            sum.add(Fraction::rate(tokens, rate)?)
        })?;
        convert(raw, group_ppm, speed_ppm, credits_per_usd, false)
    }

    /// Every tier and every cache split is considered. Tier rates need not be
    /// monotone; evaluating only the tariff at the maximum input is unsafe.
    pub fn reservation_ceiling(
        self,
        input: u64,
        output: u64,
        group_ppm: u32,
        speed_ppm: u32,
        credits_per_usd: u64,
    ) -> Result<u64, RoutingError> {
        let mut maximum = 0;
        let mut lower = 0;
        for tier in &self.tiers[..usize::from(self.count)] {
            if lower > input {
                break;
            }
            let bound = input.min(tier.up_to_input_tokens.unwrap_or(input));
            let mut input_rate = tier.rates.input;
            for rate in [tier.rates.cache_read, tier.rates.cache_write] {
                if Fraction::rate(1, rate)?.greater(Fraction::rate(1, input_rate)?)? {
                    input_rate = rate;
                }
            }
            let raw = Fraction::rate(bound, input_rate)?
                .add(Fraction::rate(output, tier.rates.output)?)?;
            maximum = maximum.max(convert(raw, group_ppm, speed_ppm, credits_per_usd, true)?);
            if let Some(upper) = tier.up_to_input_tokens {
                lower = upper.checked_add(1).ok_or(RoutingError::PriceOverflow)?;
            }
        }
        Ok(maximum)
    }
}

fn invalid() -> RoutingError {
    RoutingError::InvalidConfig("decimal token tariff")
}

/// At most four terms with bounded u64 numerators and denominators enter each
/// operation. Big-integer size is therefore bounded by this tariff contract,
/// rather than arbitrary input text, even when denominators are coprime.
#[derive(Clone)]
struct Fraction(Ratio<BigUint>);

impl Fraction {
    fn zero() -> Self {
        Self(Ratio::new(BigUint::from(0_u8), BigUint::from(1_u8)))
    }
    fn rate(tokens: u64, rate: DecimalRate) -> Result<Self, RoutingError> {
        Ok(Self(Ratio::new(
            BigUint::from(tokens) * BigUint::from(rate.numerator),
            BigUint::from(rate.denominator),
        )))
    }
    fn add(self, other: Self) -> Result<Self, RoutingError> {
        Ok(Self(self.0 + other.0))
    }
    fn multiply(self, numerator: u128, denominator: u128) -> Result<Self, RoutingError> {
        Ok(Self(
            self.0 * Ratio::new(BigUint::from(numerator), BigUint::from(denominator)),
        ))
    }
    fn greater(self, other: Self) -> Result<bool, RoutingError> {
        Ok(self.0 > other.0)
    }
}

fn convert(
    raw: Fraction,
    group: u32,
    speed: u32,
    credits: u64,
    ceiling: bool,
) -> Result<u64, RoutingError> {
    if group > MAX_MULTIPLIER || !(1..=MAX_MULTIPLIER).contains(&speed) || credits == 0 {
        return Err(invalid());
    }
    let amount = raw
        .multiply(u128::from(group), 1_000_000)?
        .multiply(u128::from(speed), 1_000_000)?
        .multiply(u128::from(credits), 1_000_000)?;
    let quotient = amount.0.to_integer();
    let remainder = amount.0.numer() % amount.0.denom();
    let round = if ceiling {
        remainder != BigUint::from(0_u8)
    } else {
        (&remainder << 1_usize) >= amount.0.denom().clone()
    };
    let result = quotient + BigUint::from(u8::from(round));
    // The legacy positive-price successful settlement has a one-credit floor.
    // A zero expression or free group remains zero; undispatched cancellation
    // has zero usage and therefore never acquires a minimum charge.
    let result = if amount.0.numer() != &BigUint::from(0_u8) {
        result.max(BigUint::from(1_u8))
    } else {
        result
    };
    let result = u64::try_from(result).map_err(|_| RoutingError::PriceOverflow)?;
    if result > i64::MAX as u64 {
        return Err(RoutingError::PriceOverflow);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(numerator: u64, denominator: u64) -> DecimalRate {
        DecimalRate {
            numerator,
            denominator,
        }
    }
    fn rates(input: DecimalRate, output: DecimalRate, cached: DecimalRate) -> TieredRates {
        TieredRates {
            input,
            output,
            cache_read: cached,
            cache_write: rate(0, 1),
        }
    }
    fn tariff(tiers: Vec<TokenTier>, uses_cache_read: bool) -> TieredTariff {
        TieredTariff {
            revision: 1,
            source_digest: "12".repeat(32),
            uses_cache_read,
            uses_cache_write: false,
            tiers,
        }
    }
    fn flat(
        input: DecimalRate,
        output: DecimalRate,
        cached: DecimalRate,
        uses_cache: bool,
    ) -> CompiledTariff {
        tariff(
            vec![TokenTier {
                up_to_input_tokens: None,
                rates: rates(input, output, cached),
            }],
            uses_cache,
        )
        .compile()
        .unwrap()
    }
    fn charge(tariff: CompiledTariff, usage: TieredUsage) -> u64 {
        tariff
            .charge(
                usage,
                InputSemantics::Inclusive,
                1_000_000,
                1_000_000,
                500_000,
            )
            .unwrap()
    }

    #[test]
    fn decimal_v1_sums_before_rounding_and_preserves_positive_minimum() {
        let price = flat(rate(1, 1), rate(3, 1), rate(1, 10), true);
        assert_eq!(
            charge(
                price,
                TieredUsage {
                    input: 100,
                    cached: 20,
                    output: 10,
                    ..TieredUsage::default()
                }
            ),
            56
        );
        assert_eq!(
            charge(
                price,
                TieredUsage {
                    input: 1,
                    ..TieredUsage::default()
                }
            ),
            1
        );
        assert_eq!(charge(price, TieredUsage::default()), 0);
        let zero = flat(rate(0, 1), rate(0, 1), rate(0, 1), false);
        assert_eq!(
            charge(
                zero,
                TieredUsage {
                    input: 100,
                    output: 50,
                    ..TieredUsage::default()
                }
            ),
            0
        );
        assert_eq!(
            price
                .charge(
                    TieredUsage {
                        input: 100,
                        ..TieredUsage::default()
                    },
                    InputSemantics::Inclusive,
                    0,
                    1_000_000,
                    500_000
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn unused_cache_variable_keeps_original_protocol_input_basis() {
        let price = flat(rate(1, 1), rate(0, 1), rate(0, 1), false);
        let usage = TieredUsage {
            input: 100,
            cached: 20,
            ..TieredUsage::default()
        };
        assert_eq!(charge(price, usage), 50);
        assert_eq!(
            price
                .charge(
                    usage,
                    InputSemantics::AnthropicText,
                    1_000_000,
                    1_000_000,
                    500_000
                )
                .unwrap(),
            40
        );
        let separate_zero = flat(rate(1, 1), rate(0, 1), rate(0, 1), true);
        assert_eq!(charge(separate_zero, usage), 40);
    }

    #[test]
    fn tier_condition_uses_inclusive_context_and_inclusive_upper_bound() {
        let price = tariff(
            vec![
                TokenTier {
                    up_to_input_tokens: Some(100),
                    rates: rates(rate(1, 1), rate(0, 1), rate(1, 10)),
                },
                TokenTier {
                    up_to_input_tokens: None,
                    rates: rates(rate(2, 1), rate(0, 1), rate(1, 5)),
                },
            ],
            true,
        )
        .compile()
        .unwrap();
        assert_eq!(
            charge(
                price,
                TieredUsage {
                    input: 100,
                    cached: 90,
                    ..TieredUsage::default()
                }
            ),
            10
        );
        assert_eq!(
            charge(
                price,
                TieredUsage {
                    input: 101,
                    cached: 90,
                    ..TieredUsage::default()
                }
            ),
            20
        );
    }

    #[test]
    fn rates_retain_long_decimals_and_non_power_of_ten_denominators() {
        let decimal = flat(
            rate(0, 1),
            rate(8_999_999_999_999_999, 10_000_000_000_000_000),
            rate(0, 1),
            false,
        );
        assert_eq!(
            charge(
                decimal,
                TieredUsage {
                    output: 10,
                    ..TieredUsage::default()
                }
            ),
            4
        );
        let division = flat(rate(500_000, 3_357_233), rate(0, 1), rate(0, 1), false);
        assert_eq!(
            charge(
                division,
                TieredUsage {
                    input: 1_000_000,
                    ..TieredUsage::default()
                }
            ),
            74_466
        );
    }

    #[test]
    fn reservation_considers_earlier_more_expensive_tiers_and_every_cache_split() {
        let price = tariff(
            vec![
                TokenTier {
                    up_to_input_tokens: Some(100),
                    rates: rates(rate(10, 1), rate(3, 1), rate(20, 1)),
                },
                TokenTier {
                    up_to_input_tokens: None,
                    rates: rates(rate(1, 1), rate(1, 1), rate(1, 10)),
                },
            ],
            true,
        )
        .compile()
        .unwrap();
        let hold = price
            .reservation_ceiling(150, 10, 1_000_000, 1_000_000, 500_000)
            .unwrap();
        assert_eq!(hold, 1015);
        for input in 0..=150 {
            for cached in 0..=input {
                for semantics in [InputSemantics::Inclusive, InputSemantics::AnthropicText] {
                    let amount = price
                        .charge(
                            TieredUsage {
                                input,
                                cached,
                                output: 10,
                                created: 0,
                            },
                            semantics,
                            1_000_000,
                            1_000_000,
                            500_000,
                        )
                        .unwrap();
                    assert!(
                        amount <= hold,
                        "reservation smaller than reachable settlement"
                    );
                }
            }
        }
    }

    #[test]
    fn unsupported_measurements_invalid_tiers_and_overflow_are_errors() {
        let price = flat(rate(1, 1), rate(1, 1), rate(1, 1), true);
        assert!(
            price
                .charge(
                    TieredUsage {
                        input: 5,
                        cached: 4,
                        created: 2,
                        output: 0
                    },
                    InputSemantics::Inclusive,
                    1_000_000,
                    1_000_000,
                    500_000
                )
                .is_err()
        );
        assert!(
            price
                .charge(
                    TieredUsage {
                        input: 5,
                        created: 1,
                        ..TieredUsage::default()
                    },
                    InputSemantics::Inclusive,
                    1_000_000,
                    1_000_000,
                    500_000
                )
                .is_err()
        );
        let mut config = price.tariff();
        config.uses_cache_write = true;
        assert!(config.compile().is_err());
        config.uses_cache_write = false;
        config.tiers[0].up_to_input_tokens = Some(10);
        assert!(config.compile().is_err());
        config.tiers[0].up_to_input_tokens = None;
        config.tiers[0].rates.input.denominator = 0;
        assert!(config.compile().is_err());
        let huge = flat(rate(u64::MAX, 1), rate(u64::MAX, 1), rate(0, 1), false);
        assert!(
            huge.charge(
                TieredUsage {
                    input: u64::MAX,
                    output: u64::MAX,
                    ..TieredUsage::default()
                },
                InputSemantics::Inclusive,
                1_000_000_000,
                1_000_000_000,
                500_000
            )
            .is_err()
        );
    }

    #[test]
    fn frozen_tariff_keeps_exact_rates_and_rejects_unknown_fields() {
        let compiled = flat(rate(1, 3), rate(7, 11), rate(0, 1), false);
        let frozen = PersistedTieredTariff {
            tariff: compiled.tariff(),
            input_semantics: InputSemantics::AnthropicText,
            group_multiplier_ppm: 1_250_000,
            speed_multiplier_ppm: 1_000_000,
            credits_per_usd: 500_000,
        };
        let bytes = serde_json::to_vec(&frozen).unwrap();
        let restored: PersistedTieredTariff = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(frozen, restored);
        assert_eq!(restored.tariff.compile().unwrap(), compiled);
        let mut value = serde_json::to_value(&frozen).unwrap();
        value["client_rate"] = serde_json::json!(0);
        assert!(serde_json::from_value::<PersistedTieredTariff>(value).is_err());
    }

    #[test]
    fn coprime_decimal_denominators_do_not_overflow_between_reservation_and_settlement() {
        let price = flat(
            rate(1, 1_000_000_000_000_000_000),
            rate(1, 999_999_999_999_999_999),
            rate(1, 999_999_999_999_999_997),
            true,
        );
        let hold = price
            .reservation_ceiling(2, 1, 1_000_000, 1_000_000, 500_000)
            .unwrap();
        let amount = charge(
            price,
            TieredUsage {
                input: 2,
                cached: 1,
                created: 0,
                output: 1,
            },
        );
        assert_eq!((hold, amount), (1, 1));
    }
}
