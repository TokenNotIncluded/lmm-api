//! Cumulative legacy refund rounding. This does not import history or post a journal.
//!
//! A03 owns the immutable LegacyPaymentBasis rows. Until that table is installed
//! by S11 and filled offline, runtime dispatch of a historical refund stays
//! unavailable. Opening journals are not replayed as the original top-up.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyRefundError {
    Invalid,
}

/// Credit delta for one newly verified legacy refund.
///
/// `already_micros` includes the historical refund seed. The target is half-up
/// of the cumulative micros, then the already-applied half-up target is
/// subtracted. A zero delta is a real application: consume the proof and keep
/// the receipt, but do not post a zero journal.
/// @mark payments.legacy-basis role=authority
pub fn cumulative_half_up_credit_delta(
    already_micros: i128,
    this_micros: i128,
    numerator: i128,
    denominator: i128,
) -> Result<i64, LegacyRefundError> {
    if already_micros < 0 || this_micros <= 0 || numerator <= 0 || denominator <= 0 {
        return Err(LegacyRefundError::Invalid);
    }
    let cumulative = already_micros
        .checked_add(this_micros)
        .ok_or(LegacyRefundError::Invalid)?;
    let applied = half_up(already_micros, numerator, denominator)?;
    let target = half_up(cumulative, numerator, denominator)?;
    let delta = target
        .checked_sub(applied)
        .ok_or(LegacyRefundError::Invalid)?;
    if delta < 0 {
        return Err(LegacyRefundError::Invalid);
    }
    i64::try_from(delta).map_err(|_| LegacyRefundError::Invalid)
}

fn half_up(micros: i128, numerator: i128, denominator: i128) -> Result<i128, LegacyRefundError> {
    let product = micros
        .checked_mul(numerator)
        .ok_or(LegacyRefundError::Invalid)?;
    let biased = product
        .checked_add(denominator / 2)
        .ok_or(LegacyRefundError::Invalid)?;
    Ok(biased / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripe_cent_ratio_is_exact_and_positive() {
        let delta = cumulative_half_up_credit_delta(0, 100, 5000, 1).unwrap();
        assert_eq!(delta, 500_000);
    }

    #[test]
    fn cumulative_half_up_can_be_zero_without_a_journal() {
        // 1 micro rounds to 1 credit; the next micro does not add another credit.
        let first = cumulative_half_up_credit_delta(0, 1, 1, 2).unwrap();
        let second = cumulative_half_up_credit_delta(1, 1, 1, 2).unwrap();
        assert_eq!((first, second), (1, 0));
    }

    #[test]
    fn per_event_rounding_is_not_used() {
        // Two 1-micro events at 2/3 credit would each round to 1 if rounded alone.
        // Cumulative half-up of 2 micros is 1, so the second delta is 0.
        let first = cumulative_half_up_credit_delta(0, 1, 2, 3).unwrap();
        let second = cumulative_half_up_credit_delta(1, 1, 2, 3).unwrap();
        assert_eq!(
            first + second,
            cumulative_half_up_credit_delta(0, 2, 2, 3).unwrap()
        );
        assert_eq!((first, second), (1, 0));
    }

    #[test]
    fn seed_is_included_before_the_new_refund() {
        let delta = cumulative_half_up_credit_delta(5, 1, 1, 2).unwrap();
        assert_eq!(delta, 0);
        let later = cumulative_half_up_credit_delta(6, 1, 1, 2).unwrap();
        assert_eq!(later, 1);
    }

    #[test]
    fn nonpositive_amounts_are_rejected() {
        assert_eq!(
            cumulative_half_up_credit_delta(-1, 1, 1, 1),
            Err(LegacyRefundError::Invalid)
        );
        assert_eq!(
            cumulative_half_up_credit_delta(0, 0, 1, 1),
            Err(LegacyRefundError::Invalid)
        );
    }
}
