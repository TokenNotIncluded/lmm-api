//! Exact, bounded display conversions. Never used by a writer or settlement.
//! Legacy semantics: 5999dc6563921a38df6f12f64b71656cf94c16c6,
//! apps/api-go/model/quota_money_projection.go (AGPL-3.0-or-later).
use serde::Serialize;

pub const MAX_RECORDS: usize = 10_000;
pub const MAX_BASES: usize = 64;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_BASIS: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MoneyProjection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_usd: Option<String>,
    pub quota_money_available: bool,
    pub quota_money_basis: &'static str,
}
impl MoneyProjection {
    pub fn unknown(basis: &'static str) -> Self {
        Self {
            quota_usd: None,
            quota_money_available: false,
            quota_money_basis: basis,
        }
    }
    pub(crate) fn scaled(value: &str, basis: &'static str) -> Option<Self> {
        // The SQL div/mod result is a nonnegative integer, never an exponent,
        // rounded by the database exactly once after combining all fractions.
        if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let padded = format!("{:0>19}", value);
        let (whole, fraction) = padded.split_at(padded.len() - 18);
        let fraction = fraction.trim_end_matches('0');
        let usd = if fraction.is_empty() {
            whole.to_owned()
        } else {
            format!("{whole}.{fraction}")
        };
        Some(Self {
            quota_usd: Some(usd),
            quota_money_available: true,
            quota_money_basis: basis,
        })
    }
}

/// Called by an offline trusted source adapter only. Client metadata is never
/// authoritative. Numeric integral decimals/exponents follow the pinned oracle.
pub fn captured_basis(metadata: &str) -> Option<i64> {
    if metadata.len() > MAX_METADATA_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(metadata).ok()?;
    let raw = value
        .as_object()?
        .get("pricing_credits_per_usd")?
        .as_number()?
        .to_string();
    if raw.len() > 64 || !raw.as_bytes().first()?.is_ascii_digit() {
        return None;
    }
    let (mantissa, exponent) =
        raw.split_once(['e', 'E'])
            .map_or(Some((raw.as_str(), 0)), |(m, e)| {
                let e: i32 = e.parse().ok()?;
                (-18..=18).contains(&e).then_some((m, e))
            })?;
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    // The oracle accepts integral decimals with many trailing zeroes within
    // its 64-byte input bound. Remove those zeroes before bounded arithmetic;
    // otherwise a valid anchor such as 1.000... could overflow u128 parsing.
    let fraction = fraction.trim_end_matches('0');
    let digits = format!("{whole}{fraction}");
    let mut number: u128 = digits.parse().ok()?;
    let shift = exponent.checked_sub(i32::try_from(fraction.len()).ok()?)?;
    if shift >= 0 {
        number = number.checked_mul(10u128.checked_pow(shift as u32)?)?;
    } else {
        let divisor = 10u128.checked_pow(shift.unsigned_abs())?;
        if !number.is_multiple_of(divisor) {
            return None;
        }
        number /= divisor;
    }
    (number > 0 && number <= MAX_BASIS as u128).then_some(number as i64)
}

pub fn log_money(quota: Option<i64>, log_type: i16, basis: Option<i64>) -> MoneyProjection {
    let Some(quota) = quota else {
        return MoneyProjection::unknown("invalid_stored_quota");
    };
    if log_type != 2 {
        return MoneyProjection::unknown("not_consumption");
    }
    let Some(basis) = basis.filter(|b| (1..=MAX_BASIS).contains(b)) else {
        return MoneyProjection::unknown("missing_credit_basis");
    };
    if quota < 0 {
        return MoneyProjection::unknown("missing_credit_basis");
    }
    let numerator = quota as u128 * 1_000_000_000_000_000_000;
    let denominator = basis as u128;
    let scaled = numerator / denominator + u128::from(numerator % denominator * 2 >= denominator);
    MoneyProjection::scaled(&scaled.to_string(), "captured_credits_per_usd").unwrap()
}

/// Bucket counters do not prove the composition or captured anchors of logs.
pub fn bucket_money() -> MoneyProjection {
    MoneyProjection::unknown("bucket_without_credit_basis")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_anchor_is_integral_bounded_and_not_a_string() {
        for raw in ["500000", "500000.0", "5e5", "500000000e-3"] {
            assert_eq!(
                captured_basis(&format!("{{\"pricing_credits_per_usd\":{raw}}}")),
                Some(500000)
            );
        }
        assert_eq!(
            captured_basis(&format!(
                "{{\"pricing_credits_per_usd\":1.{}}}",
                "0".repeat(62)
            )),
            Some(1)
        );
        for raw in [
            "\"500000\"",
            "null",
            "true",
            "0",
            "-1",
            "1.5",
            "1e19",
            "9007199254740992",
        ] {
            assert_eq!(
                captured_basis(&format!("{{\"pricing_credits_per_usd\":{raw}}}")),
                None
            );
        }
        assert_eq!(captured_basis("[]"), None);
    }
    #[test]
    fn missing_and_negative_are_unknown_while_zero_with_basis_is_proven() {
        assert!(!log_money(None, 2, Some(500000)).quota_money_available);
        assert!(!log_money(Some(-1), 2, Some(500000)).quota_money_available);
        assert!(!log_money(Some(1), 1, Some(500000)).quota_money_available);
        assert!(!log_money(Some(0), 2, None).quota_money_available);
        assert_eq!(
            log_money(Some(0), 2, Some(500000)).quota_usd.as_deref(),
            Some("0")
        );
        assert_eq!(
            log_money(Some(1), 2, Some(3)).quota_usd.as_deref(),
            Some("0.333333333333333333")
        );
        assert_eq!(
            log_money(Some(2), 2, Some(3)).quota_usd.as_deref(),
            Some("0.666666666666666667")
        );
        assert!(!bucket_money().quota_money_available);
    }
}
