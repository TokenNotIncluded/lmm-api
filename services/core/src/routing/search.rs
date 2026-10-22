//! Frozen supplier search charges. Counters are receipts, never request intent.
//! Search USD amounts enter the same exact expression as token charges before
//! its single final rounding. Supplier limits are captured before dispatch.
use num_bigint::BigUint;
use num_rational::Ratio;
use serde::{Deserialize, Serialize};

use super::{MAX_MULTIPLIER, RoutingError};
use crate::billing::trust_pricing::DiscountRatio;

pub const MAX_SEARCH_COUNT: u64 = 10_000;
const MAX_DENOMINATOR: u64 = 1_000_000_000_000_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    OpenAiResponsesV1,
    GeminiGroundingV1,
    AzureResponsesV1,
    XaiResponsesV1,
    DashscopeResponsesV1,
    DashscopeAnthropicV1,
    AnthropicMessagesV1,
    ZhipuChatV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchBillingUnit {
    Call,
    Query,
    GroundedRequest,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum SearchCurrency {
    #[serde(rename = "USD")]
    Usd,
}

/// Exact USD per one measured unit. This has no token or thousand-call scale.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchRate {
    pub numerator: u64,
    pub denominator: u64,
}

impl SearchRate {
    fn validate(self) -> Result<(), RoutingError> {
        if self.numerator > i64::MAX as u64 || !(1..=MAX_DENOMINATOR).contains(&self.denominator) {
            return Err(invalid());
        }
        Ok(())
    }

    fn amount(self, count: u64) -> Ratio<BigUint> {
        Ratio::new(
            BigUint::from(self.numerator) * BigUint::from(count),
            BigUint::from(self.denominator),
        )
    }
}

/// Alternate Gemini billing bases are an enum, so a single tariff cannot
/// charge both the query count and the grounded-request indicator.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchTariff {
    pub revision: u32,
    pub provider: SearchProvider,
    pub unit: SearchBillingUnit,
    pub currency: SearchCurrency,
    pub usd_per_unit: SearchRate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_posts_fetched_usd_per_unit: Option<SearchRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_users_fetched_usd_per_unit: Option<SearchRate>,
}

/// A cumulative receipt from one explicitly selected supplier contract.
/// Missing fields stay unknown, including on a final provider response.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchUsage {
    pub provider: SearchProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounded_requests: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_posts_fetched: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_users_fetched: Option<u64>,
}

impl SearchUsage {
    fn counters(self) -> [Option<u64>; 5] {
        [
            self.calls,
            self.queries,
            self.grounded_requests,
            self.x_posts_fetched,
            self.x_users_fetched,
        ]
    }

    pub fn valid(self) -> bool {
        let supported = match self.provider {
            SearchProvider::GeminiGroundingV1 => self.calls.unwrap_or(0) == 0,
            SearchProvider::ZhipuChatV1 => {
                self.calls.unwrap_or(0) == 0 && self.queries.unwrap_or(0) == 0
            }
            _ => self.queries.unwrap_or(0) == 0 && self.grounded_requests.unwrap_or(0) == 0,
        };
        supported
            && self
                .counters()
                .into_iter()
                .flatten()
                .all(|count| count <= MAX_SEARCH_COUNT)
            && self.grounded_requests.is_none_or(|count| count <= 1)
            && (self.provider == SearchProvider::XaiResponsesV1
                || (self.x_posts_fetched.unwrap_or(0) == 0
                    && self.x_users_fetched.unwrap_or(0) == 0))
            && !(self.grounded_requests == Some(0) && self.queries.is_some_and(|count| count > 0))
    }

    pub fn has_positive(self) -> bool {
        self.counters().into_iter().flatten().any(|count| count > 0)
    }

    pub fn for_unit(self, unit: SearchBillingUnit) -> Option<u64> {
        match unit {
            SearchBillingUnit::Call => self.calls,
            SearchBillingUnit::Query => self.queries,
            SearchBillingUnit::GroundedRequest => self.grounded_requests,
        }
    }

    pub fn follows(self, previous: Self) -> bool {
        self.valid()
            && previous.valid()
            && self.provider == previous.provider
            && self
                .counters()
                .into_iter()
                .zip(previous.counters())
                .all(|(now, old)| old.is_none_or(|old| now.is_some_and(|now| now >= old)))
    }
}

/// Supplier-enforced or supplier-guaranteed bounds, not an after-the-fact
/// application spending limit. The selected endpoint/model binding owns them.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounded_requests: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_posts_fetched: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_users_fetched: Option<u64>,
}

impl SearchLimits {
    pub fn validate(self) -> Result<(), RoutingError> {
        if [
            self.calls,
            self.queries,
            self.grounded_requests,
            self.x_posts_fetched,
            self.x_users_fetched,
        ]
        .into_iter()
        .flatten()
        .any(|count| count > MAX_SEARCH_COUNT)
            || self.grounded_requests.is_some_and(|count| count > 1)
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn for_unit(self, unit: SearchBillingUnit) -> Option<u64> {
        match unit {
            SearchBillingUnit::Call => self.calls,
            SearchBillingUnit::Query => self.queries,
            SearchBillingUnit::GroundedRequest => self.grounded_requests,
        }
    }

    pub fn contains(self, usage: SearchUsage) -> bool {
        self.validate().is_ok()
            && usage.valid()
            && [
                self.calls,
                self.queries,
                self.grounded_requests,
                self.x_posts_fetched,
                self.x_users_fetched,
            ]
            .into_iter()
            .zip(usage.counters())
            .all(|(cap, measured)| {
                cap.is_none_or(|cap| measured.is_none_or(|measured| measured <= cap))
            })
    }

    fn usage(self, provider: SearchProvider) -> SearchUsage {
        SearchUsage {
            provider,
            calls: self.calls,
            queries: self.queries,
            grounded_requests: self.grounded_requests,
            x_posts_fetched: self.x_posts_fetched,
            x_users_fetched: self.x_users_fetched,
        }
    }
}

impl SearchTariff {
    pub fn validate(self) -> Result<(), RoutingError> {
        let unit_allowed = match self.provider {
            SearchProvider::GeminiGroundingV1 => matches!(
                self.unit,
                SearchBillingUnit::Query | SearchBillingUnit::GroundedRequest
            ),
            SearchProvider::ZhipuChatV1 => self.unit == SearchBillingUnit::GroundedRequest,
            _ => self.unit == SearchBillingUnit::Call,
        };
        if self.revision != 1
            || !unit_allowed
            || (self.provider != SearchProvider::XaiResponsesV1
                && (self.x_posts_fetched_usd_per_unit.is_some()
                    || self.x_users_fetched_usd_per_unit.is_some()))
        {
            return Err(invalid());
        }
        self.usd_per_unit.validate()?;
        for rate in [
            self.x_posts_fetched_usd_per_unit,
            self.x_users_fetched_usd_per_unit,
        ]
        .into_iter()
        .flatten()
        {
            rate.validate()?;
        }
        Ok(())
    }

    pub fn validate_usage(self, usage: SearchUsage) -> Result<(), RoutingError> {
        self.validate()?;
        if !usage.valid() || usage.provider != self.provider || usage.for_unit(self.unit).is_none()
        {
            return Err(RoutingError::InvalidRequest);
        }
        for (rate, count) in [
            (self.x_posts_fetched_usd_per_unit, usage.x_posts_fetched),
            (self.x_users_fetched_usd_per_unit, usage.x_users_fetched),
        ] {
            if (rate.is_some() && count.is_none())
                || (rate.is_none() && count.is_some_and(|count| count > 0))
            {
                return Err(RoutingError::InvalidRequest);
            }
        }
        Ok(())
    }

    pub fn validate_limits(self, limits: SearchLimits) -> Result<(), RoutingError> {
        self.validate()?;
        limits.validate()?;
        let permits_a_priced_unit = limits.for_unit(self.unit).is_some_and(|count| count > 0)
            || (self.x_posts_fetched_usd_per_unit.is_some()
                && limits.x_posts_fetched.is_some_and(|count| count > 0))
            || (self.x_users_fetched_usd_per_unit.is_some()
                && limits.x_users_fetched.is_some_and(|count| count > 0));
        if limits.for_unit(self.unit).is_none()
            || !permits_a_priced_unit
            || (self.x_posts_fetched_usd_per_unit.is_some() && limits.x_posts_fetched.is_none())
            || (self.x_users_fetched_usd_per_unit.is_some() && limits.x_users_fetched.is_none())
        {
            return Err(invalid());
        }
        self.validate_usage(limits.usage(self.provider))
    }

    pub(crate) fn surcharge(self, usage: SearchUsage) -> Result<SearchSurcharge, RoutingError> {
        self.validate_usage(usage)?;
        let mut amount = self.usd_per_unit.amount(
            usage
                .for_unit(self.unit)
                .ok_or(RoutingError::InvalidRequest)?,
        );
        for (rate, count) in [
            (self.x_posts_fetched_usd_per_unit, usage.x_posts_fetched),
            (self.x_users_fetched_usd_per_unit, usage.x_users_fetched),
        ] {
            if let Some(rate) = rate {
                amount += rate.amount(count.ok_or(RoutingError::InvalidRequest)?);
            }
        }
        Ok(SearchSurcharge(amount))
    }

    pub(crate) fn maximum_surcharge(
        self,
        limits: SearchLimits,
    ) -> Result<SearchSurcharge, RoutingError> {
        self.validate_limits(limits)?;
        self.surcharge(limits.usage(self.provider))
    }
}

/// Stored with the immutable charging price. Current configuration is never
/// used to reinterpret these units, limits, or conversion during recovery.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedSearchPrice {
    pub tariff: SearchTariff,
    pub limits: SearchLimits,
    pub supplier_contract_revision: String,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
    pub credits_per_usd: u64,
}

impl PersistedSearchPrice {
    pub fn validate(&self) -> Result<(), RoutingError> {
        self.tariff.validate_limits(self.limits)?;
        if self.supplier_contract_revision.is_empty()
            || self.supplier_contract_revision.len() > 512
            || self
                .supplier_contract_revision
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

    pub fn validate_usage(&self, usage: SearchUsage) -> Result<(), RoutingError> {
        self.validate()?;
        self.tariff.validate_usage(usage)?;
        if !self.limits.contains(usage) {
            return Err(RoutingError::InvalidRequest);
        }
        Ok(())
    }

    pub(crate) fn surcharge(&self, usage: SearchUsage) -> Result<SearchSurcharge, RoutingError> {
        self.validate_usage(usage)?;
        self.tariff.surcharge(usage)
    }

    pub(crate) fn maximum_surcharge(&self) -> Result<SearchSurcharge, RoutingError> {
        self.validate()?;
        self.tariff.maximum_surcharge(self.limits)
    }
}

/// Bounded exact USD sum, still unrounded and before any group/actor factors.
#[derive(Clone, Debug)]
pub(crate) struct SearchSurcharge(Ratio<BigUint>);

impl SearchSurcharge {
    pub(crate) fn zero() -> Self {
        Self(Ratio::from_integer(BigUint::from(0_u8)))
    }

    pub(crate) fn micro_usd(self) -> Ratio<BigUint> {
        self.0 * Ratio::from_integer(BigUint::from(1_000_000_u64))
    }

    /// The token fraction is already in credits, with group/speed conversion
    /// captured by its original price. Apply the actor discount to the total.
    pub(crate) fn combine_credits(
        self,
        token_numerator: u128,
        token_denominator: u128,
        group: u32,
        speed: u32,
        credits: u64,
        discount: Option<DiscountRatio>,
    ) -> Result<u64, RoutingError> {
        if token_denominator == 0
            || group > MAX_MULTIPLIER
            || !(1..=MAX_MULTIPLIER).contains(&speed)
            || credits == 0
        {
            return Err(invalid());
        }
        let mut amount = Ratio::new(
            BigUint::from(token_numerator),
            BigUint::from(token_denominator),
        ) + self.0
            * Ratio::new(
                BigUint::from(group) * BigUint::from(speed) * BigUint::from(credits),
                BigUint::from(1_000_000_000_000_u64),
            );
        if let Some(discount) = discount {
            amount *= Ratio::new(
                BigUint::from(discount.numerator()),
                BigUint::from(discount.denominator()),
            );
        }
        let quotient = amount.to_integer();
        let remainder = amount.numer() % amount.denom();
        let rounded = quotient + BigUint::from(u8::from(remainder != BigUint::from(0_u8)));
        let rounded = u64::try_from(rounded).map_err(|_| RoutingError::PriceOverflow)?;
        if rounded > i64::MAX as u64 {
            return Err(RoutingError::PriceOverflow);
        }
        Ok(rounded)
    }
}

fn invalid() -> RoutingError {
    RoutingError::InvalidConfig("supplier search tariff")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tariff(provider: SearchProvider, unit: SearchBillingUnit) -> SearchTariff {
        SearchTariff {
            revision: 1,
            provider,
            unit,
            currency: SearchCurrency::Usd,
            usd_per_unit: SearchRate {
                numerator: 1,
                denominator: 10_000_000,
            },
            x_posts_fetched_usd_per_unit: None,
            x_users_fetched_usd_per_unit: None,
        }
    }

    fn usage(provider: SearchProvider) -> SearchUsage {
        SearchUsage {
            provider,
            calls: None,
            queries: None,
            grounded_requests: None,
            x_posts_fetched: None,
            x_users_fetched: None,
        }
    }

    #[test]
    fn absent_zero_provider_units_and_caps_remain_distinct() {
        let price = tariff(SearchProvider::GeminiGroundingV1, SearchBillingUnit::Query);
        let unknown = usage(price.provider);
        assert!(price.validate_usage(unknown).is_err());
        assert!(
            price
                .validate_usage(SearchUsage {
                    queries: Some(0),
                    ..unknown
                })
                .is_ok()
        );
        assert!(
            price
                .validate_usage(SearchUsage {
                    grounded_requests: Some(1),
                    ..unknown
                })
                .is_err()
        );
        assert!(
            price
                .validate_usage(SearchUsage {
                    provider: SearchProvider::OpenAiResponsesV1,
                    queries: Some(1),
                    ..unknown
                })
                .is_err()
        );
        assert!(
            price
                .validate_limits(SearchLimits {
                    grounded_requests: Some(1),
                    ..SearchLimits::default()
                })
                .is_err()
        );
        assert!(
            price
                .validate_limits(SearchLimits {
                    queries: Some(3),
                    ..SearchLimits::default()
                })
                .is_ok()
        );
        let limits = SearchLimits {
            queries: Some(3),
            ..SearchLimits::default()
        };
        assert!(!limits.contains(SearchUsage {
            queries: Some(4),
            ..unknown
        }));
        assert!(
            !SearchUsage {
                grounded_requests: Some(2),
                ..unknown
            }
            .valid()
        );
    }

    #[test]
    fn xai_dimensions_are_independent_and_one_round_includes_tokens() {
        let mut price = tariff(SearchProvider::XaiResponsesV1, SearchBillingUnit::Call);
        price.x_posts_fetched_usd_per_unit = Some(SearchRate {
            numerator: 2,
            denominator: 10_000_000,
        });
        price.x_users_fetched_usd_per_unit = Some(SearchRate {
            numerator: 3,
            denominator: 10_000_000,
        });
        let measured = SearchUsage {
            calls: Some(2),
            x_posts_fetched: Some(3),
            x_users_fetched: Some(4),
            ..usage(price.provider)
        };
        // Search = (2 + 6 + 12)/10m USD = 1 credit. Tokens are 1/10 credit.
        assert_eq!(
            price
                .surcharge(measured)
                .unwrap()
                .combine_credits(1, 10, 1_000_000, 1_000_000, 500_000, None),
            Ok(2)
        );
        // Both terms are sub-credit and together remain below one credit.
        let one = SearchUsage {
            calls: Some(1),
            x_posts_fetched: Some(0),
            x_users_fetched: Some(0),
            ..measured
        };
        assert_eq!(
            price
                .surcharge(one)
                .unwrap()
                .combine_credits(1, 10, 1_000_000, 1_000_000, 500_000, None),
            Ok(1)
        );
        let mut missing_price = price;
        missing_price.x_posts_fetched_usd_per_unit = None;
        assert!(missing_price.surcharge(measured).is_err());
        assert!(
            price
                .surcharge(SearchUsage {
                    x_users_fetched: None,
                    ..measured
                })
                .is_err()
        );
        assert!(
            price
                .validate_limits(SearchLimits {
                    calls: Some(2),
                    x_posts_fetched: Some(3),
                    ..SearchLimits::default()
                })
                .is_err()
        );
        // An X-only contract can prohibit web calls while reserving its
        // separately priced posts/users. Every priced dimension stays explicit.
        assert!(
            price
                .validate_limits(SearchLimits {
                    calls: Some(0),
                    x_posts_fetched: Some(3),
                    x_users_fetched: Some(0),
                    ..SearchLimits::default()
                })
                .is_ok()
        );
        assert!(
            price
                .validate_limits(SearchLimits {
                    calls: Some(0),
                    x_posts_fetched: Some(0),
                    x_users_fetched: Some(0),
                    ..SearchLimits::default()
                })
                .is_err()
        );
    }

    #[test]
    fn gemini_query_and_grounded_request_are_alternative_prices() {
        let per_query = tariff(SearchProvider::GeminiGroundingV1, SearchBillingUnit::Query);
        let per_request = SearchTariff {
            unit: SearchBillingUnit::GroundedRequest,
            ..per_query
        };
        let measured = SearchUsage {
            queries: Some(30),
            grounded_requests: Some(1),
            ..usage(per_query.provider)
        };
        assert_eq!(
            per_query
                .surcharge(measured)
                .unwrap()
                .combine_credits(0, 1, 1_000_000, 1_000_000, 500_000, None),
            Ok(2)
        );
        assert_eq!(
            per_request
                .surcharge(measured)
                .unwrap()
                .combine_credits(0, 1, 1_000_000, 1_000_000, 500_000, None),
            Ok(1)
        );
        assert!(
            SearchTariff {
                unit: SearchBillingUnit::Call,
                ..per_query
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_value::<SearchTariff>(serde_json::json!({
                "revision":1,"provider":"gemini_grounding_v1","unit":"query","currency":"CNY",
                "usd_per_unit":{"numerator":1,"denominator":1}
            }))
            .is_err()
        );
    }

    #[test]
    fn cumulative_receipts_never_lose_a_known_dimension_or_switch_provider() {
        let old = SearchUsage {
            calls: Some(1),
            x_posts_fetched: Some(2),
            x_users_fetched: Some(3),
            ..usage(SearchProvider::XaiResponsesV1)
        };
        assert!(old.follows(old));
        assert!(
            SearchUsage {
                calls: Some(2),
                x_posts_fetched: Some(3),
                ..old
            }
            .follows(old)
        );
        for next in [
            SearchUsage { calls: None, ..old },
            SearchUsage {
                x_posts_fetched: Some(1),
                ..old
            },
            SearchUsage {
                provider: SearchProvider::OpenAiResponsesV1,
                ..old
            },
        ] {
            assert!(!next.follows(old));
        }
        let bytes = serde_json::to_vec(&old).unwrap();
        assert_eq!(serde_json::from_slice::<SearchUsage>(&bytes).unwrap(), old);
        let mut unknown = serde_json::to_value(old).unwrap();
        unknown["unreviewed_counter"] = serde_json::json!(1);
        assert!(serde_json::from_value::<SearchUsage>(unknown).is_err());
    }

    #[test]
    fn exact_surcharge_preserves_free_groups_discount_and_overflow_rejection() {
        let price = tariff(SearchProvider::OpenAiResponsesV1, SearchBillingUnit::Call);
        let measured = SearchUsage {
            calls: Some(10),
            ..usage(price.provider)
        };
        assert_eq!(
            price
                .surcharge(measured)
                .unwrap()
                .combine_credits(0, 1, 0, 1_000_000, 500_000, None),
            Ok(0)
        );
        assert_eq!(
            price.surcharge(measured).unwrap().combine_credits(
                1,
                2,
                1_000_000,
                1_000_000,
                500_000,
                Some(DiscountRatio::new(1, 2).unwrap())
            ),
            Ok(1)
        );
        let huge = SearchTariff {
            usd_per_unit: SearchRate {
                numerator: i64::MAX as u64,
                denominator: 1,
            },
            ..price
        };
        assert_eq!(
            huge.surcharge(measured)
                .unwrap()
                .combine_credits(0, 1, 1_000_000, 1_000_000, 500_000, None),
            Err(RoutingError::PriceOverflow)
        );
    }
}
