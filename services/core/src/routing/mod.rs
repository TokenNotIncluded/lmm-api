//! Version 1 routing decision contract for the Rust relay and control plane.
//!
//! This module performs no network forwarding, ledger writes, or Go calls.
//! See README.md for publication, retry, credential, and task 05/06 boundaries.

mod config;
mod engine;
mod group_name;
mod pricing;
pub mod store;
mod tiered;

pub use config::{
    CredentialRef, Endpoint, FailoverPolicy, ModelAlias, ModelGroup, ModelRoute, PayloadPolicy,
    Protocol, RetryCause, RouteTarget, RoutingConfig, Speed, UpstreamConfig,
};
pub use engine::{
    Attempt, DeliveryState, Failure, FailureKind, HealthStatus, PublicModel, ReleaseVersion,
    ReplayPermission, RoutePlan, RouteRequest, Router,
};
pub use group_name::valid_group_name;
pub use pricing::{
    BillingMode, CreditQuote, CreditRate, Credits, MAX_MULTIPLIER, MULTIPLIER_ONE, MicroUsd,
    PriceConfig, PriceQuote, PriceRule, PriceSnapshot, TokenRates, Usage,
};
// Relay code uses this re-export. The standalone routing test compiles this
// module by itself, so the name has no caller in that crate.
#[allow(unused_imports)]
pub(crate) use tiered::CacheCreationPartition;
pub use tiered::{
    CacheCreationMode, CompiledTariff, DecimalRate, InputSemantics, PersistedTieredTariff,
    TieredRates, TieredTariff, TieredUsage, TokenTier,
};

pub const CONTRACT_VERSION: u32 = 1;

/// Stable, sanitized errors. Never attach upstream response text, URLs, or tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutingError {
    InvalidConfig(&'static str),
    InvalidRequest,
    ConflictingSpeed,
    VersionConflict,
    VersionNotIncreasing,
    PriceVersionConflict,
    UnknownGroup,
    GroupDisabled,
    UnknownModel,
    RouteDisabled,
    SpeedDisabled,
    MissingPrice,
    NoEligibleUpstream,
    RetryDisallowed,
    UnsafeReplay,
    ResponseAlreadyStarted,
    AttemptsExhausted,
    InvalidTransition,
    PriceOverflow,
    LockPoisoned,
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "routing: {self:?}")
    }
}

impl std::error::Error for RoutingError {}

#[cfg(test)]
mod tests;
