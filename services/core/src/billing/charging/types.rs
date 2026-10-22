use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use std::{fmt, future::Future, pin::Pin};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Conflict,
    BudgetExceeded,
    NoFunds,
    NeedReservation,
    StaleWorker,
    WrongState,
    NotFound,
    Retry,
    Storage,
    Ledger,
    /// Query resolve() with the SAME request ID; never invent a new charge.
    CommitUnknown,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01" | "55P03") => Self::Retry,
            Some("23505") => Self::Conflict,
            _ => Self::Storage,
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Authoritative cumulative cache writes. A missing TTL is not a zero TTL.
/// The immutable usage event carries these counters across worker restarts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCreationUsage {
    pub total: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_minute: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_hour: Option<i64>,
    /// OpenAI's twelve-hour subset; absent historical evidence stays unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twelve_hour: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_mode: Option<crate::routing::CacheCreationMode>,
    /// Revision 3 keeps the original aggregate beside its normalized maximum.
    /// None is reserved for a frozen, explicitly reviewed absence-zero policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_total: Option<i64>,
}
impl CacheCreationUsage {
    pub(crate) fn valid(self) -> bool {
        if self.total < 0
            || [
                self.five_minute,
                self.one_hour,
                self.twelve_hour,
                self.reported_total,
            ]
            .into_iter()
            .flatten()
            .any(|v| v < 0)
        {
            return false;
        }
        if self.twelve_hour.is_some_and(|value| value > self.total)
            || (self.twelve_hour.is_some_and(|value| value != 0)
                && (self.cache_creation_mode.is_some()
                    || self.five_minute.is_some_and(|value| value != 0)
                    || self.one_hour.is_some_and(|value| value != 0)))
        {
            return false;
        }
        if self.cache_creation_mode == Some(crate::routing::CacheCreationMode::GenericRemainderV1) {
            return match self.reported_total {
                Some(reported) => crate::routing::CacheCreationPartition::from_normalized(
                    self.total as u64,
                    reported as u64,
                    self.five_minute.map(|v| v as u64),
                    self.one_hour.map(|v| v as u64),
                )
                .is_ok(),
                None => self.total == 0 && self.five_minute.is_none() && self.one_hour.is_none(),
            };
        }
        self.reported_total.is_none()
            && [self.five_minute, self.one_hour]
                .into_iter()
                .flatten()
                .all(|v| v <= self.total)
            && match (self.five_minute, self.one_hour) {
                (Some(five), Some(hour)) => five.checked_add(hour) == Some(self.total),
                _ => true,
            }
    }
    pub(crate) fn follows(self, previous: Self) -> bool {
        let follows = |now: Option<i64>, old: Option<i64>| {
            old.is_none_or(|old| now.is_some_and(|now| now >= old))
        };
        if self.cache_creation_mode != previous.cache_creation_mode {
            return false;
        }
        if !follows(self.twelve_hour, previous.twelve_hour)
            || !follows(
                self.twelve_hour
                    .and_then(|value| self.total.checked_sub(value)),
                previous
                    .twelve_hour
                    .and_then(|value| previous.total.checked_sub(value)),
            )
        {
            return false;
        }
        if self.cache_creation_mode == Some(crate::routing::CacheCreationMode::GenericRemainderV1) {
            return self.valid()
                && previous.valid()
                && self.total >= previous.total
                && follows(self.reported_total, previous.reported_total)
                && follows(self.five_minute, previous.five_minute)
                && follows(self.one_hour, previous.one_hour);
        }
        let buckets = |value: Self| match (value.five_minute, value.one_hour) {
            (Some(five), None) => (Some(five), value.total.checked_sub(five)),
            (None, Some(hour)) => (value.total.checked_sub(hour), Some(hour)),
            value => value,
        };
        let (five, hour) = buckets(self);
        let (old_five, old_hour) = buckets(previous);
        self.valid()
            && self.total >= previous.total
            && follows(self.five_minute, previous.five_minute)
            && follows(self.one_hour, previous.one_hour)
            && follows(five, old_five)
            && follows(hour, old_hour)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<crate::model_http::image_price::ImageUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_cost_atto_usd: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<CacheCreationUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<crate::routing::SearchUsage>,
    /// Includes cached input. Cached input is priced separately, not twice.
    pub input: i64,
    pub cached: i64,
    pub output: i64,
}
impl Usage {
    pub fn valid(self) -> bool {
        if let Some(image) = self.image {
            return image.validate().is_ok()
                && self.input == 0
                && self.cached == 0
                && self.output == 0
                && self.upstream_cost_atto_usd.is_none()
                && self.cache_creation.is_none()
                && self.search.is_none();
        }
        self.input >= 0
            && self.cached >= 0
            && self.cached <= self.input
            && self.output >= 0
            && self.search.is_none_or(|search| search.valid())
            && self.cache_creation.is_none_or(|created| {
                created.valid()
                    && self
                        .cached
                        .checked_add(created.total)
                        .is_some_and(|sum| sum <= self.input)
            })
    }
    pub fn follows(self, previous: Self) -> bool {
        self.valid()
            && match (self.image, previous.image) {
                (Some(now), Some(old)) => now.follows(&old),
                (None, Some(_)) => false,
                _ => true,
            }
            && self.input >= previous.input
            && self.cached >= previous.cached
            && self.output >= previous.output
            && match (previous.cache_creation, self.cache_creation) {
                (Some(old), Some(now)) => now.follows(old),
                (Some(_), None) => false,
                _ => true,
            }
            && match (previous.search, self.search) {
                (Some(old), Some(now)) => now.follows(old),
                (Some(_), None) => false,
                _ => true,
            }
            && match (previous.upstream_cost_atto_usd, self.upstream_cost_atto_usd) {
                (Some(old), Some(new)) => new >= old,
                (Some(_), None) => false,
                _ => true,
            }
    }
}

/// Supplied by the trusted Rust price/model resolver, never by client JSON.
/// Rates are integer core credits per million tokens (500,000 credits / USD).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    /// Only a verified Core actor's transaction can mint a new entitlement.
    /// Absence remains an explicitly legacy price, including on replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_trust: Option<crate::billing::trust_pricing::FrozenTrustPrice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<crate::model_http::image_price::PersistedImagePrice>,
    /// Set only by the trusted Rust router; no client or Go price authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_cost_group_ppm: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiered: Option<crate::routing::PersistedTieredTariff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<crate::routing::PersistedSearchPrice>,
    pub version: String,
    pub model: String,
    pub group: String,
    pub input_per_million: i64,
    pub cached_per_million: i64,
    pub output_per_million: i64,
}
impl Price {
    pub(crate) fn with_user_authority(
        mut self,
        authority: &crate::billing::trust_pricing::UserPriceAuthority,
    ) -> Result<Self> {
        if self.user_trust.is_some() {
            return Err(Error::Invalid);
        }
        self.user_trust = Some(authority.evidence().clone());
        self.validate()?;
        Ok(self)
    }
    pub(crate) fn base_price(&self) -> Self {
        let mut base = self.clone();
        base.user_trust = None;
        base
    }
    pub(crate) fn discount(&self) -> Option<crate::billing::trust_pricing::DiscountRatio> {
        self.user_trust.as_ref().map(|evidence| evidence.discount())
    }
    pub(crate) fn validate_actor(&self, user_id: i64) -> Result<()> {
        self.validate()?;
        if self
            .user_trust
            .as_ref()
            .is_some_and(|evidence| evidence.user_id() != user_id)
        {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(evidence) = &self.user_trust {
            evidence.validate().map_err(|_| Error::Invalid)?;
        }
        if !valid_id(&self.version)
            || !valid_price_model(&self.model)
            || !crate::routing::valid_group_name(&self.group)
            || self.input_per_million < 0
            || self.cached_per_million < 0
            || self.output_per_million < 0
        {
            return Err(Error::Invalid);
        }
        if let Some(image) = &self.image {
            image.validate().map_err(|_| Error::Invalid)?;
            if self.upstream_cost_group_ppm.is_some()
                || self.tiered.is_some()
                || self.input_per_million != 0
                || self.cached_per_million != 0
                || self.output_per_million != 0
                || image.model() != self.model
                || image.group() != self.group
                || self.version != format!("image-{}", image.version())
            {
                return Err(Error::Invalid);
            }
        }
        if let Some(ppm) = self.upstream_cost_group_ppm
            && (!crate::upstream_cost::supported_router(&self.model)
                || self.tiered.is_some()
                || !(1..=1_000_000_000).contains(&ppm)
                || self.input_per_million != 0
                || self.cached_per_million != 0
                || self.output_per_million != 0)
        {
            return Err(Error::Invalid);
        }
        if let Some(tariff) = &self.tiered {
            tariff.validate().map_err(|_| Error::Invalid)?;
            if self.input_per_million != 0
                || self.cached_per_million != 0
                || self.output_per_million != 0
            {
                return Err(Error::Invalid);
            }
        }
        if let Some(search) = &self.search {
            search.validate().map_err(|_| Error::Invalid)?;
            if self.image.is_some()
                || self.upstream_cost_group_ppm.is_some()
                || self.tiered.as_ref().is_some_and(|tariff| {
                    search.group_multiplier_ppm != tariff.group_multiplier_ppm
                        || search.speed_multiplier_ppm != tariff.speed_multiplier_ppm
                        || search.credits_per_usd != tariff.credits_per_usd
                })
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    pub fn cost(&self, usage: Usage) -> Result<i64> {
        self.validate()?;
        if !usage.valid() {
            return Err(Error::Invalid);
        }
        let expected_cache_mode = self.tiered.as_ref().and_then(|t| t.cache_creation_mode);
        if usage
            .cache_creation
            .is_some_and(|created| created.cache_creation_mode != expected_cache_mode)
        {
            return Err(Error::Invalid);
        }
        if let Some(price) = &self.image {
            return price
                .cost_with_discount(&usage.image.ok_or(Error::Invalid)?, self.discount())
                .map_err(|_| Error::Invalid);
        }
        if usage.image.is_some() {
            return Err(Error::Invalid);
        }
        let surcharge = self.search_surcharge(usage)?;
        if let Some(tariff) = &self.tiered {
            let created = self.cache_creation_for(tariff, usage)?;
            return tariff
                .tariff
                .compile()
                .and_then(|compiled| {
                    compiled.charge_with_surcharge(
                        crate::routing::TieredUsage {
                            input: usage.input as u64,
                            cached: usage.cached as u64,
                            created: created.map_or(0, |v| v.total as u64),
                            output: usage.output as u64,
                            created_5m: created.and_then(|v| v.five_minute.map(|v| v as u64)),
                            created_1h: created.and_then(|v| v.one_hour.map(|v| v as u64)),
                            created_12h: created.and_then(|v| v.twelve_hour.map(|v| v as u64)),
                        },
                        tariff.input_semantics,
                        tariff.group_multiplier_ppm,
                        tariff.speed_multiplier_ppm,
                        tariff.credits_per_usd,
                        self.discount(),
                        surcharge,
                    )
                })
                .and_then(|amount| {
                    i64::try_from(amount).map_err(|_| crate::routing::RoutingError::PriceOverflow)
                })
                .map_err(|_| Error::Invalid);
        }
        if let Some(ppm) = self.upstream_cost_group_ppm {
            return crate::upstream_cost::credits_with_discount(
                usage.upstream_cost_atto_usd.ok_or(Error::Invalid)?,
                ppm,
                self.discount(),
            )
            .ok_or(Error::Invalid);
        }
        if usage.cache_creation.is_some_and(|v| v.total != 0) {
            return Err(Error::Invalid);
        }
        let sum = [
            (usage.input - usage.cached, self.input_per_million),
            (usage.cached, self.cached_per_million),
            (usage.output, self.output_per_million),
        ]
        .into_iter()
        .try_fold(0_i128, |total, (count, rate)| {
            total.checked_add(i128::from(count) * i128::from(rate))
        })
        .ok_or(Error::Invalid)?;
        let numerator = u128::try_from(sum).map_err(|_| Error::Invalid)?;
        let amount = if let Some(search) = &self.search {
            surcharge
                .combine_credits(
                    numerator,
                    1_000_000,
                    search.group_multiplier_ppm,
                    search.speed_multiplier_ppm,
                    search.credits_per_usd,
                    self.discount(),
                )
                .map_err(|_| Error::Invalid)?
        } else {
            crate::upstream_cost::ceil_with_discount(numerator, 1_000_000, self.discount())
                .ok_or(Error::Invalid)?
        };
        i64::try_from(amount).map_err(|_| Error::Invalid)
    }

    /// The same frozen exact factor applies before the reservation's sole ceil.
    pub(crate) fn reservation_ceiling(&self, basis: ReservationBasis) -> Result<i64> {
        self.validate()?;
        let surcharge = self
            .search
            .as_ref()
            .map(|search| search.maximum_surcharge())
            .transpose()
            .map_err(|_| Error::Invalid)?
            .unwrap_or_else(crate::routing::SearchSurcharge::zero);
        let amount = match basis {
            ReservationBasis::Image => {
                return self
                    .image
                    .as_ref()
                    .ok_or(Error::Invalid)?
                    .reservation_with_discount(self.discount())
                    .map_err(|_| Error::Invalid);
            }
            ReservationBasis::UpstreamCost { atto_usd } => {
                return crate::upstream_cost::credits_with_discount(
                    atto_usd,
                    self.upstream_cost_group_ppm.ok_or(Error::Invalid)?,
                    self.discount(),
                )
                .ok_or(Error::Invalid);
            }
            ReservationBasis::Tokens { input, output } => {
                if self.image.is_some() || self.upstream_cost_group_ppm.is_some() {
                    return Err(Error::Invalid);
                }
                if let Some(tariff) = &self.tiered {
                    tariff
                        .tariff
                        .compile()
                        .map_err(|_| Error::Invalid)?
                        .reservation_ceiling_with_surcharge(
                            input,
                            output,
                            tariff.group_multiplier_ppm,
                            tariff.speed_multiplier_ppm,
                            tariff.credits_per_usd,
                            self.discount(),
                            surcharge,
                        )
                        .map_err(|_| Error::Invalid)?
                } else {
                    let numerator = u128::from(input)
                        .checked_mul(self.input_per_million.max(self.cached_per_million) as u128)
                        .and_then(|sum| {
                            sum.checked_add(u128::from(output) * self.output_per_million as u128)
                        })
                        .ok_or(Error::Invalid)?;
                    if let Some(search) = &self.search {
                        surcharge
                            .combine_credits(
                                numerator,
                                1_000_000,
                                search.group_multiplier_ppm,
                                search.speed_multiplier_ppm,
                                search.credits_per_usd,
                                self.discount(),
                            )
                            .map_err(|_| Error::Invalid)?
                    } else {
                        crate::upstream_cost::ceil_with_discount(
                            numerator,
                            1_000_000,
                            self.discount(),
                        )
                        .ok_or(Error::Invalid)?
                    }
                }
            }
        };
        i64::try_from(amount).map_err(|_| Error::Invalid)
    }

    fn search_surcharge(&self, usage: Usage) -> Result<crate::routing::SearchSurcharge> {
        match (&self.search, usage.search) {
            (Some(price), Some(search)) => price.surcharge(search).map_err(|_| Error::Invalid),
            (Some(_), None) if usage == Usage::default() => {
                Ok(crate::routing::SearchSurcharge::zero())
            }
            (None, None) => Ok(crate::routing::SearchSurcharge::zero()),
            (None, Some(search)) if !search.has_positive() => {
                Ok(crate::routing::SearchSurcharge::zero())
            }
            _ => Err(Error::Invalid),
        }
    }

    fn cache_creation_for(
        &self,
        tariff: &crate::routing::PersistedTieredTariff,
        usage: Usage,
    ) -> Result<Option<CacheCreationUsage>> {
        if tariff.tariff.revision == 1 {
            if usage.cache_creation.is_some() {
                return Err(Error::Invalid);
            }
            return Ok(None);
        }
        // Empty usage belongs to an undispatched release/initial checkpoint.
        // A successful nonempty revision-2 measurement must be explicit, even
        // when a provider reports exactly zero cache creation.
        match usage.cache_creation {
            Some(value)
                if value.cache_creation_mode == tariff.cache_creation_mode
                    && (tariff.cache_creation_mode.is_none()
                        || value.reported_total.is_some()
                        || (tariff.missing_cache_creation_is_zero
                            && value.total == 0
                            && value.five_minute.is_none()
                            && value.one_hour.is_none())) =>
            {
                Ok(Some(value))
            }
            Some(_) => Err(Error::Invalid),
            None if usage.input == 0 && usage.cached == 0 && usage.output == 0 => Ok(None),
            None => Err(Error::Invalid),
        }
    }
}

/// Captured by the Rust route/quote resolver, never accepted from HTTP or Go.
/// This remains separate from legacy Request's serialized idempotency contract.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum ReservationBasis {
    Tokens { input: u64, output: u64 },
    UpstreamCost { atto_usd: u128 },
    Image,
}

#[cfg(test)]
mod search_price_tests {
    use super::*;
    use crate::routing::{
        BillingMode, CreditRate, InputSemantics, PriceConfig, PriceRule, PriceSnapshot,
        SearchProvider, SearchUsage, Speed, TieredUsage, TokenRates,
    };
    use serde_json::json;

    fn scalar() -> Price {
        serde_json::from_value(json!({
            "version":"routing-search-1","model":"search-model","group":"default",
            "input_per_million":100000,"cached_per_million":300000,"output_per_million":200000,
            "search":{
                "tariff":{"revision":1,"provider":"open_ai_responses_v1","unit":"call","currency":"USD",
                    "usd_per_unit":{"numerator":1,"denominator":1000000}},
                "limits":{"calls":3},"supplier_contract_revision":"synthetic-bounded-search-v1",
                "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,"credits_per_usd":500000
            }
        })).unwrap()
    }

    fn measured(calls: u64) -> SearchUsage {
        SearchUsage {
            provider: SearchProvider::OpenAiResponsesV1,
            calls: Some(calls),
            queries: None,
            grounded_requests: None,
            x_posts_fetched: None,
            x_users_fetched: None,
        }
    }

    #[test]
    fn scalar_tokens_and_search_share_one_round_and_exact_quote() {
        let price = scalar();
        price.validate().unwrap();
        let usage = Usage {
            input: 1,
            search: Some(measured(1)),
            ..Usage::default()
        };
        // 0.1 token credit + 0.5 search credit rounds once to 1, not 1 + 1.
        assert_eq!(price.cost(usage), Ok(1));
        let prices = PriceSnapshot::compile(PriceConfig {
            version: 1,
            rules: vec![PriceRule {
                id: 1,
                model: price.model.clone(),
                group: price.group.clone(),
                speed: Speed::Standard,
                billing_mode: BillingMode::Tokens,
                tiered: None,
                search: Some(price.search.as_ref().unwrap().tariff),
                rates: TokenRates {
                    input: 200000,
                    cache_read: 600000,
                    output: 400000,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: 1_000_000,
                speed_multiplier_ppm: 1_000_000,
            }],
        })
        .unwrap();
        let quote = prices
            .quote(&price.model, &price.group, Speed::Standard)
            .unwrap()
            .with_credit_rate(CreditRate::new(1, 500000).unwrap());
        let hold = price
            .reservation_ceiling(ReservationBasis::Tokens {
                input: 4,
                output: 2,
            })
            .unwrap();
        assert_eq!(hold, 4); // ceil(4 * .3 + 2 * .2 + 3 * .5).
        for input in 0..=4 {
            for cached in 0..=input {
                for output in 0..=2 {
                    for calls in 0..=3 {
                        let usage = Usage {
                            input,
                            cached,
                            output,
                            search: Some(measured(calls)),
                            ..Usage::default()
                        };
                        let amount = price.cost(usage).unwrap();
                        let quoted = quote
                            .charge_with_search(
                                crate::routing::Usage {
                                    input: (input - cached) as u64,
                                    cache_read: cached as u64,
                                    output: output as u64,
                                    ..crate::routing::Usage::default()
                                },
                                usage.search,
                            )
                            .unwrap();
                        assert_eq!(quoted.0 as i64, amount);
                        assert!(amount <= hold);
                    }
                }
            }
        }
    }

    #[test]
    fn search_missing_unpriced_wrong_supplier_and_over_cap_cannot_settle() {
        let price = scalar();
        assert_eq!(
            price.cost(Usage {
                input: 1,
                ..Usage::default()
            }),
            Err(Error::Invalid)
        );
        assert_eq!(
            price.cost(Usage {
                input: 1,
                search: Some(SearchUsage {
                    calls: None,
                    ..measured(0)
                }),
                ..Usage::default()
            }),
            Err(Error::Invalid)
        );
        assert_eq!(
            price.cost(Usage {
                search: Some(measured(4)),
                ..Usage::default()
            }),
            Err(Error::Invalid)
        );
        assert_eq!(
            price.cost(Usage {
                search: Some(SearchUsage {
                    provider: SearchProvider::AnthropicMessagesV1,
                    ..measured(1)
                }),
                ..Usage::default()
            }),
            Err(Error::Invalid)
        );
        let mut unpriced = price.clone();
        unpriced.search = None;
        assert_eq!(
            unpriced.cost(Usage {
                search: Some(measured(1)),
                ..Usage::default()
            }),
            Err(Error::Invalid)
        );
        assert_eq!(price.cost(Usage::default()), Ok(0)); // Undispatched cancellation.
        assert_eq!(
            price.cost(Usage {
                search: Some(measured(0)),
                ..Usage::default()
            }),
            Ok(0)
        );
        let mut free = price.clone();
        free.input_per_million = 0;
        free.cached_per_million = 0;
        free.output_per_million = 0;
        free.search.as_mut().unwrap().group_multiplier_ppm = 0;
        assert_eq!(
            free.cost(Usage {
                input: 1,
                search: Some(measured(3)),
                ..Usage::default()
            }),
            Ok(0)
        );
        assert_eq!(
            free.reservation_ceiling(ReservationBasis::Tokens {
                input: 4,
                output: 2
            }),
            Ok(0)
        );
    }

    fn tiered() -> Price {
        let mut value = serde_json::to_value(scalar()).unwrap();
        value["input_per_million"] = json!(0);
        value["cached_per_million"] = json!(0);
        value["output_per_million"] = json!(0);
        value["search"]["tariff"]["usd_per_unit"] = json!({"numerator":3,"denominator":5000000});
        value["tiered"] = json!({
            "tariff":{"revision":1,"source_digest":"a7".repeat(32),"uses_cache_read":true,"uses_cache_write":false,
                "tiers":[
                    {"up_to_input_tokens":2,"rates":{"input":{"numerator":5,"denominator":2},"output":{"numerator":3,"denominator":2},"cache_read":{"numerator":1,"denominator":10},"cache_write":{"numerator":0,"denominator":1}}},
                    {"up_to_input_tokens":null,"rates":{"input":{"numerator":1,"denominator":5},"output":{"numerator":1,"denominator":5},"cache_read":{"numerator":1,"denominator":20},"cache_write":{"numerator":0,"denominator":1}}}
                ]},
            "input_semantics":"inclusive","group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,"credits_per_usd":500000
        });
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn tiered_search_keeps_half_up_settlement_and_bounds_every_tier() {
        let price = tiered();
        let frozen = price.tiered.as_ref().unwrap();
        let prices = PriceSnapshot::compile(PriceConfig {
            version: 1,
            rules: vec![PriceRule {
                id: 1,
                model: price.model.clone(),
                group: price.group.clone(),
                speed: Speed::Standard,
                billing_mode: BillingMode::TieredDecimal,
                tiered: Some(frozen.tariff.clone()),
                search: Some(price.search.as_ref().unwrap().tariff),
                rates: TokenRates {
                    input: 0,
                    cache_read: 0,
                    output: 0,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: 1_000_000,
                speed_multiplier_ppm: 1_000_000,
            }],
        })
        .unwrap();
        let quote = prices
            .quote(&price.model, &price.group, Speed::Standard)
            .unwrap()
            .with_credit_rate(CreditRate::new(1, 500000).unwrap());
        let hold = price
            .reservation_ceiling(ReservationBasis::Tokens {
                input: 4,
                output: 2,
            })
            .unwrap();
        assert_eq!(hold, 5); // Earlier tier: ceil(2 * 1.25 + 2 * .75 + 3 * .3).
        // Later tier gives .4 tokens + .3 search = .7, rounded together to 1.
        assert_eq!(
            price.cost(Usage {
                input: 4,
                search: Some(measured(1)),
                ..Usage::default()
            }),
            Ok(1)
        );
        for input in 0..=4 {
            for cached in 0..=input {
                for output in 0..=2 {
                    for calls in 0..=3 {
                        let usage = Usage {
                            input,
                            cached,
                            output,
                            search: Some(measured(calls)),
                            ..Usage::default()
                        };
                        let amount = price.cost(usage).unwrap();
                        let quoted = quote
                            .charge_tiered_usage_with_search(
                                TieredUsage {
                                    input: input as u64,
                                    cached: cached as u64,
                                    output: output as u64,
                                    ..TieredUsage::default()
                                },
                                InputSemantics::Inclusive,
                                usage.search,
                            )
                            .unwrap();
                        assert_eq!(quoted.0 as i64, amount);
                        assert!(amount <= hold);
                    }
                }
            }
        }
        let mut incompatible = price;
        incompatible.search.as_mut().unwrap().group_multiplier_ppm = 2_000_000;
        assert_eq!(incompatible.validate(), Err(Error::Invalid));
    }

    #[test]
    fn old_json_shape_is_unchanged_and_search_replay_freezes_price_and_limits() {
        let mut old = scalar();
        old.search = None;
        let old_json = serde_json::to_value(&old).unwrap();
        assert!(old_json.get("search").is_none());
        assert_eq!(serde_json::from_value::<Price>(old_json).unwrap(), old);
        let legacy_usage = r#"{"input":1,"cached":0,"output":0}"#;
        let decoded: Usage = serde_json::from_str(legacy_usage).unwrap();
        assert_eq!(serde_json::to_string(&decoded).unwrap(), legacy_usage);
        let original = scalar();
        let bytes = serde_json::to_vec(&original).unwrap();
        let restored: Price = serde_json::from_slice(&bytes).unwrap();
        let receipt = Usage {
            input: 1,
            search: Some(measured(1)),
            ..Usage::default()
        };
        assert_eq!(restored, original);
        assert_eq!(restored.cost(receipt), Ok(1));
        let mut current = original;
        current
            .search
            .as_mut()
            .unwrap()
            .tariff
            .usd_per_unit
            .numerator = 100;
        current.search.as_mut().unwrap().limits.calls = Some(1);
        assert_ne!(current.cost(receipt), restored.cost(receipt));
        assert_eq!(restored.search.as_ref().unwrap().limits.calls, Some(3));
        let missing = Usage {
            search: None,
            ..receipt
        };
        assert!(!missing.follows(receipt));
    }
}

#[cfg(test)]
mod trust_price_tests {
    use super::*;
    use serde_json::json;

    pub(super) fn base() -> Price {
        Price {
            search: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "v1".into(),
            model: "model".into(),
            group: "default".into(),
            input_per_million: 10_000,
            cached_per_million: 2_000,
            output_per_million: 20_000,
        }
    }
    fn frozen(mut price: Price, numerator: u64, denominator: u64) -> Price {
        let mut encoded = serde_json::to_value(&price).unwrap();
        encoded["user_trust"] = json!({
            "user_id":7,"policy_revision":1,"basis_revision":2,
            "effective_level":1,"level_source":"automatic",
            "discount":{"numerator":numerator,"denominator":denominator},
            "evaluated_at":100,"next_decay_at":200,"evidence_digest":vec![1u8;32]
        });
        price = serde_json::from_value(encoded).unwrap();
        price
    }
    #[test]
    fn legacy_price_preserves_original_json_and_absence_is_not_verified_one() {
        let price = base();
        let original = r#"{"version":"v1","model":"model","group":"default","input_per_million":10000,"cached_per_million":2000,"output_per_million":20000}"#;
        assert_eq!(serde_json::to_string(&price).unwrap(), original);
        let verified = frozen(price.clone(), 1, 1);
        assert_ne!(
            serde_json::to_value(&verified).unwrap(),
            serde_json::to_value(&price).unwrap()
        );
        assert_eq!(verified.base_price(), price);
        assert_eq!(verified.validate_actor(7), Ok(()));
        assert_eq!(verified.validate_actor(8), Err(Error::Forbidden));
    }
    #[test]
    fn scalar_settlement_and_hold_apply_factor_before_their_only_ceiling() {
        let price = base();
        let usage = Usage {
            input: 101,
            ..Usage::default()
        };
        assert_eq!(price.cost(usage), Ok(2));
        let frozen = frozen(price, 9, 10);
        assert_eq!(frozen.cost(usage), Ok(1));
        assert_eq!(
            frozen.reservation_ceiling(ReservationBasis::Tokens {
                input: 101,
                output: 0
            }),
            Ok(1)
        );
        // Exact frozen evidence, including its time, remains unchanged on reuse.
        let encoded = serde_json::to_vec(&frozen).unwrap();
        for _ in 0..3 {
            let restored: Price = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(restored, frozen);
            assert_eq!(restored.cost(usage), Ok(1));
        }
    }
    #[test]
    fn discounted_hold_bounds_all_scalar_cache_splits() {
        let price = frozen(base(), 97, 100);
        let hold = price
            .reservation_ceiling(ReservationBasis::Tokens {
                input: 1001,
                output: 101,
            })
            .unwrap();
        for cached in 0..=1001 {
            assert!(
                price
                    .cost(Usage {
                        input: 1001,
                        cached,
                        output: 101,
                        ..Usage::default()
                    })
                    .unwrap()
                    <= hold
            );
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Request {
    pub id: String,
    /// SHA-256 of the canonical full upstream request, including routing inputs.
    pub fingerprint: [u8; 32],
    pub price: Price,
    pub reserve: i64,
    pub lease_seconds: i64,
    pub maximum_seconds: i64,
}
impl Request {
    pub(super) fn validate(&self) -> Result<()> {
        self.price.validate()?;
        if !valid_id(&self.id)
            || self.fingerprint == [0; 32]
            || self.reserve < 0
            || !(1..=300).contains(&self.lease_seconds)
            || !(self.lease_seconds..=86_400).contains(&self.maximum_seconds)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Wallet,
    Subscription { id: i64, start: i64, end: i64 },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Charge {
    pub id: String,
    pub actor_user_id: i64,
    pub key_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_source: Option<crate::identity::grants::OAuthSource>,
    pub owner_account_id: i64,
    pub payer_account_id: i64,
    pub price: Price,
    pub source: Source,
    pub authorized_at: i64,
    pub lease_until: i64,
    pub hard_deadline: i64,
    pub state: String,
    pub reserved: i64,
    pub usage: Usage,
    pub settled: i64,
    pub refunded: i64,
    pub revision: i64,
    pub outcome: Option<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Cancelled,
    Failed,
}
impl Outcome {
    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Change {
    /// Absolute ceiling, not a delta. Replays must not reserve twice.
    Reserve {
        total: i64,
    },
    /// Capture actual cost AND release unused reservation atomically.
    Settle {
        amount: i64,
    },
    Release,
    Refund {
        amount: i64,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LedgerCommand {
    pub operation_id: String,
    pub request_id: String,
    pub actor_user_id: i64,
    pub key_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_source: Option<crate::identity::grants::OAuthSource>,
    pub owner_account_id: i64,
    pub payer_account_id: i64,
    pub price_version: String,
    pub source: Source,
    pub change: Change,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerError {
    Insufficient,
    Conflict,
    Retry,
    Unavailable,
}
impl From<LedgerError> for Error {
    fn from(error: LedgerError) -> Self {
        match error {
            LedgerError::Insufficient => Self::NoFunds,
            LedgerError::Conflict => Self::Conflict,
            LedgerError::Retry => Self::Retry,
            LedgerError::Unavailable => Self::Ledger,
        }
    }
}
pub type LedgerFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), LedgerError>> + Send + 'a>>;

/// Task 02 integration contract. ALL changes must use `connection`, which is
/// inside the charging transaction. No COMMIT, other pool, network side effect,
/// or success-before-durability. Identical operation IDs are idempotent; changed
/// payloads conflict. Wallet effects belong exclusively to the ledger. A
/// subscription command records entitlement-funded consumption, NOT wallet cash.
/// An adapter that cannot satisfy this contract MUST NOT implement this trait.
pub trait Ledger: Send + Sync {
    fn apply<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        command: LedgerCommand,
    ) -> LedgerFuture<'a>;
}

#[derive(Clone, Debug, Serialize)]
pub struct Budget {
    pub scope: String,
    pub account_id: Option<i64>,
    pub user_id: Option<i64>,
    pub key_id: Option<i64>,
    pub period: String,
    pub anchor: i64,
    pub seconds: i64,
    pub limit: i64,
}
impl Budget {
    pub(super) fn validate(&self) -> Result<()> {
        let target = match self.scope.as_str() {
            "account" => {
                self.account_id.is_some() && self.user_id.is_none() && self.key_id.is_none()
            }
            "member" => {
                self.account_id.is_some() && self.user_id.is_some() && self.key_id.is_none()
            }
            "self" => self.account_id.is_none() && self.user_id.is_some() && self.key_id.is_none(),
            "key" => self.account_id.is_none() && self.user_id.is_none() && self.key_id.is_some(),
            _ => false,
        };
        let cycle = match self.period.as_str() {
            "day" | "week" | "month" => self.anchor == 0 && self.seconds == 0,
            "lifetime" => self.scope == "key" && self.anchor == 0 && self.seconds == 0,
            "custom" => {
                (-62_135_596_800..=253_402_300_799).contains(&self.anchor)
                    && (1..=315_576_000).contains(&self.seconds)
            }
            _ => false,
        };
        if !target
            || !cycle
            || (self.limit < 0 && !(self.scope == "key" && self.period == "lifetime"))
            || [self.account_id, self.user_id, self.key_id]
                .into_iter()
                .flatten()
                .any(|id| id <= 0)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

fn valid_price_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/~".contains(&b))
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}

#[cfg(test)]
mod generic_remainder_tests {
    use super::*;
    use crate::routing::{
        CacheCreationMode, DecimalRate, InputSemantics, PersistedTieredTariff, TieredRates,
        TieredTariff, TokenTier,
    };

    fn rate(numerator: u64, denominator: u64) -> DecimalRate {
        DecimalRate {
            numerator,
            denominator,
        }
    }
    fn price() -> Price {
        Price {
            search: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: Some(PersistedTieredTariff {
                tariff: TieredTariff {
                    uses_cache_write_12h: false,
                    revision: 3,
                    source_digest: "ab".repeat(32),
                    uses_cache_read: true,
                    uses_cache_write: true,
                    uses_cache_write_1h: true,
                    tiers: vec![TokenTier {
                        up_to_input_tokens: None,
                        rates: TieredRates {
                            cache_write_12h: None,
                            input: rate(5, 1),
                            output: rate(25, 1),
                            cache_read: rate(1, 2),
                            cache_write: rate(25, 4),
                            cache_write_1h: Some(rate(10, 1)),
                        },
                    }],
                },
                input_semantics: InputSemantics::AnthropicText,
                group_multiplier_ppm: 1_000_000,
                speed_multiplier_ppm: 1_000_000,
                credits_per_usd: 500_000,
                missing_cache_creation_is_zero: false,
                cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
            }),
            version: "generic-v3".into(),
            model: "synthetic".into(),
            group: "allowed".into(),
            input_per_million: 0,
            cached_per_million: 0,
            output_per_million: 0,
        }
    }
    fn usage(reported: Option<i64>, total: i64, five: Option<i64>, hour: Option<i64>) -> Usage {
        Usage {
            input: 120 + total,
            cached: 20,
            cache_creation: Some(CacheCreationUsage {
                twelve_hour: None,
                total,
                five_minute: five,
                one_hour: hour,
                cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
                reported_total: reported,
            }),
            ..Usage::default()
        }
    }

    #[test]
    fn generic_price_preserves_raw_aggregate_and_prices_the_real_remainder() {
        let mut price = price();
        let measured = usage(Some(50), 50, Some(2), Some(3));
        assert_eq!(price.cost(measured), Ok(417));
        price.tiered.as_mut().unwrap().input_semantics = InputSemantics::Inclusive;
        assert_eq!(price.cost(measured), Ok(411));
        let below_split = usage(Some(20), 70, Some(30), Some(40));
        assert!(below_split.valid());
        assert_eq!(below_split.cache_creation.unwrap().reported_total, Some(20));
        assert!(!usage(Some(20), 80, Some(30), Some(40)).valid());
    }

    #[test]
    fn raw_aggregate_regression_cannot_hide_behind_the_same_effective_maximum() {
        let old = usage(Some(12), 12, Some(10), Some(2));
        let decreased = usage(Some(11), 12, Some(10), Some(2));
        assert!(old.valid() && decreased.valid());
        assert!(!decreased.follows(old));
        let reclassified = usage(Some(8), 8, None, Some(7));
        let first = usage(Some(7), 7, None, Some(5));
        assert!(reclassified.follows(first));
        let explicit_five = usage(Some(7), 7, Some(2), Some(5));
        assert!(!reclassified.follows(explicit_five));
    }

    #[test]
    fn generic_zero_requires_the_original_absence_contract_and_never_forges_a_receipt() {
        let mut price = price();
        let absent = usage(None, 0, None, None);
        assert!(absent.valid());
        assert_eq!(price.cost(absent), Err(Error::Invalid));
        price
            .tiered
            .as_mut()
            .unwrap()
            .missing_cache_creation_is_zero = true;
        assert_eq!(price.cost(absent), Ok(255));
        let actual_zero = usage(Some(0), 0, None, None);
        assert_eq!(price.cost(actual_zero), Ok(255));
        assert_ne!(
            serde_json::to_value(absent).unwrap(),
            serde_json::to_value(actual_zero).unwrap()
        );
        assert!(!usage(None, 0, Some(0), Some(0)).valid());
    }

    #[test]
    fn generic_evidence_is_not_accepted_by_an_old_or_unrelated_price() {
        let mut price = price();
        let measured = usage(Some(0), 0, None, None);
        price.tiered.as_mut().unwrap().tariff.revision = 2;
        price.tiered.as_mut().unwrap().cache_creation_mode = None;
        assert_eq!(price.cost(measured), Err(Error::Invalid));
        price.tiered = None;
        assert_eq!(price.cost(measured), Err(Error::Invalid));
        let price = self::price();
        let mut strict = measured;
        let created = strict.cache_creation.as_mut().unwrap();
        created.cache_creation_mode = None;
        created.reported_total = None;
        assert!(strict.valid());
        assert_eq!(price.cost(strict), Err(Error::Invalid));
        assert!(!measured.follows(strict));
    }

    #[test]
    fn old_cache_json_remains_byte_identical_while_new_mode_is_explicit() {
        let old = CacheCreationUsage {
            twelve_hour: None,
            total: 0,
            five_minute: Some(0),
            one_hour: Some(0),
            cache_creation_mode: None,
            reported_total: None,
        };
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"total":0,"five_minute":0,"one_hour":0}"#
        );
        let generic = usage(Some(0), 0, None, None).cache_creation.unwrap();
        let value = serde_json::to_value(generic).unwrap();
        assert_eq!(value["cache_creation_mode"], "generic_remainder_v1");
        assert_eq!(value["reported_total"], 0);
        assert!(value.get("five_minute").is_none());
        assert!(value.get("one_hour").is_none());
        assert_eq!(
            serde_json::from_value::<CacheCreationUsage>(value).unwrap(),
            generic
        );
    }
}

#[cfg(test)]
mod delegated_source_tests {
    use super::*;

    #[test]
    fn cache_usage_is_durable_optional_and_monotone_without_changing_old_json() {
        let old = Usage {
            input: 10,
            cached: 2,
            output: 3,
            ..Usage::default()
        };
        assert!(
            !CacheCreationUsage {
                twelve_hour: None,
                total: 8,
                five_minute: None,
                one_hour: Some(7),
                cache_creation_mode: None,
                reported_total: None,
            }
            .follows(CacheCreationUsage {
                twelve_hour: None,
                total: 7,
                five_minute: None,
                one_hour: Some(5),
                cache_creation_mode: None,
                reported_total: None,
            })
        );
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"input":10,"cached":2,"output":3}"#
        );
        assert_eq!(
            serde_json::from_str::<Usage>(r#"{"input":10,"cached":2,"output":3}"#).unwrap(),
            old
        );
        let measured = Usage {
            cache_creation: Some(CacheCreationUsage {
                twelve_hour: None,
                total: 5,
                five_minute: Some(2),
                one_hour: Some(3),
                cache_creation_mode: None,
                reported_total: None,
            }),
            ..old
        };
        assert!(measured.valid());
        assert!(measured.follows(old));
        assert!(!old.follows(measured));
        let missing_hour = Usage {
            cache_creation: Some(CacheCreationUsage {
                twelve_hour: None,
                total: 5,
                five_minute: Some(2),
                one_hour: None,
                cache_creation_mode: None,
                reported_total: None,
            }),
            ..old
        };
        assert!(!missing_hour.follows(measured));
        for cache in [
            CacheCreationUsage {
                twelve_hour: None,
                total: -1,
                five_minute: None,
                one_hour: None,
                cache_creation_mode: None,
                reported_total: None,
            },
            CacheCreationUsage {
                twelve_hour: None,
                total: 5,
                five_minute: Some(2),
                one_hour: Some(4),
                cache_creation_mode: None,
                reported_total: None,
            },
            CacheCreationUsage {
                twelve_hour: None,
                total: 9,
                five_minute: None,
                one_hour: None,
                cache_creation_mode: None,
                reported_total: None,
            },
        ] {
            assert!(
                !Usage {
                    cache_creation: Some(cache),
                    ..old
                }
                .valid()
            );
        }
        let zero = Usage {
            cache_creation: Some(CacheCreationUsage {
                twelve_hour: None,
                total: 0,
                five_minute: Some(0),
                one_hour: Some(0),
                cache_creation_mode: None,
                reported_total: None,
            }),
            ..old
        };
        assert_ne!(
            serde_json::to_value(zero).unwrap(),
            serde_json::to_value(old).unwrap()
        );
    }
    #[test]
    fn tilde_model_price_does_not_relax_other_identifiers_or_lengths() {
        let mut price = Price {
            search: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "routing-1-rule-1".into(),
            model: "~anthropic/claude-fable-latest".into(),
            group: "allowed".into(),
            input_per_million: 2_000_000,
            cached_per_million: 0,
            output_per_million: 0,
        };
        assert!(price.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&price).unwrap()["model"],
            "~anthropic/claude-fable-latest"
        );
        price.model = format!("~{}", "a".repeat(127));
        assert!(price.validate().is_ok());
        for bad in [
            format!("~{}", "a".repeat(128)),
            "~模型".into(),
            "~ model".into(),
            "~model\n".into(),
        ] {
            price.model = bad;
            assert!(price.validate().is_err());
        }
        price.model = "~model".into();
        price.version = "~version".into();
        assert!(price.validate().is_err());
        assert!(!valid_id("~operation"));
    }
    #[test]
    fn unicode_group_price_keeps_model_and_version_identifier_rules_strict() {
        let mut price = Price {
            search: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "routing-1-rule-1".into(),
            model: "model".into(),
            group: "国产[Kimi/Deepseek/GLM]".into(),
            input_per_million: 2_000_000,
            cached_per_million: 0,
            output_per_million: 0,
        };
        assert!(price.validate().is_ok());
        let encoded = serde_json::to_value(&price).unwrap();
        assert_eq!(encoded["group"], "国产[Kimi/Deepseek/GLM]");
        price.group = "官key".into();
        assert!(price.validate().is_ok());
        price.model = "中文-model".into();
        assert!(price.validate().is_err());
        price.model = "model".into();
        price.version = "中文-version".into();
        assert!(price.validate().is_err());
        price.version = "v1".into();
        for bad in [
            "",
            " group",
            "group ",
            "group,other",
            "group:other",
            "a\u{202e}b",
        ] {
            price.group = bad.into();
            assert!(price.validate().is_err());
        }
    }
    #[test]
    fn native_ledger_command_preserves_original_exact_json_bytes() {
        let command = LedgerCommand {
            operation_id: "a".repeat(64),
            request_id: "request-1".into(),
            actor_user_id: 7,
            key_id: Some(9),
            oauth_source: None,
            owner_account_id: 17,
            payer_account_id: 17,
            price_version: "v1".into(),
            source: Source::Wallet,
            change: Change::Reserve { total: 25 },
        };
        let original = format!(
            "{{\"operation_id\":\"{}\",\"request_id\":\"request-1\",\"actor_user_id\":7,\"key_id\":9,\"owner_account_id\":17,\"payer_account_id\":17,\"price_version\":\"v1\",\"source\":{{\"kind\":\"wallet\"}},\"change\":{{\"operation\":\"reserve\",\"total\":25}}}}",
            "a".repeat(64)
        );
        assert_eq!(serde_json::to_vec(&command).unwrap(), original.into_bytes());
    }
}
