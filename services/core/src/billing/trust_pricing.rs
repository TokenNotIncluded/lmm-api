//! Core-owned dynamic price entitlements. No caller-supplied discount, balance
//! writes, network calls, startup DDL or compatibility-metadata authority.
//! Install/import explicitly; keep request-local evidence frozen at admission.
mod store;
mod types;
pub(crate) use store::{quote_in_transaction, quote_readonly_in_transaction};
pub(crate) use types::UserPriceAuthority;
pub use types::{DiscountRatio, FrozenTrustPrice, TrustPolicy, TrustTier};
pub const SCHEMA: &str = concat!(
    include_str!("../../schema/trust_pricing.sql"),
    "\n",
    include_str!("../../schema/image_trust_extension.sql")
);
#[cfg(test)]
mod payment_tests;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unavailable,
    Denied,
    Storage,
    Retry,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "trust pricing {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01" | "55P03") => Self::Retry,
            _ => Self::Storage,
        }
    }
}
type Result<T> = std::result::Result<T, Error>;

/// Count the existing Core measured final primitive, independent of price and
/// Completed/Failed/Cancelled. No estimate, HTTP status or caller flag enters.
pub(crate) fn counted_api_activity(usage: super::charging::Usage) -> Result<bool> {
    if !usage.valid() {
        return Err(Error::Invalid);
    }
    Ok(usage.input > 0
        || usage.cached > 0
        || usage.output > 0
        || usage.cache_creation.is_some_and(|v| v.total > 0)
        || usage.image.is_some_and(|v| v.output_images > 0)
        || usage.upstream_cost_atto_usd.is_some_and(|v| v > 0))
}
