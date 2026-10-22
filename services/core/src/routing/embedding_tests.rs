use super::*;
use serde_json::json;

fn tariff() -> PersistedEmbeddingPrice {
    PersistedEmbeddingPrice {
        tariff: EmbeddingTariff {
            revision: 1,
            provider: EmbeddingProvider::AliSeparateInputV1,
            currency: EmbeddingCurrency::Usd,
            text_usd_per_million: DecimalRate {
                numerator: 1,
                denominator: 3,
            },
            image_usd_per_million: DecimalRate {
                numerator: 1,
                denominator: 6,
            },
        },
        input_token_limit: 20,
        provider_contract_revision: "synthetic-native-partition-v1".into(),
        group_multiplier_ppm: 1_000_000,
        speed_multiplier_ppm: 1_000_000,
        credits_per_usd: 500_000,
    }
}
fn usage(text: u64, image: u64) -> EmbeddingUsage {
    EmbeddingUsage {
        provider: EmbeddingProvider::AliSeparateInputV1,
        text_tokens: Some(text),
        image_tokens: Some(image),
        total_input_tokens: Some(text + image),
    }
}

#[test]
fn modalities_sum_before_one_conversion_discount_and_ceiling() {
    let price = tariff();
    assert_eq!(price.cost(usage(1, 1)).unwrap(), 1); // 1/6 + 1/12 = 1/4 credit.
    assert_eq!(price.cost(usage(3, 6)).unwrap(), 1); // 1/2 + 1/2 exactly.
    assert_eq!(price.cost(usage(4, 6)).unwrap(), 2);
    let discount = Some(DiscountRatio::new(3, 5).unwrap());
    assert_eq!(price.cost_with_discount(usage(4, 6), discount).unwrap(), 1);
    assert_eq!(price.cost_with_discount(usage(0, 0), discount).unwrap(), 0);
    let mut free = price;
    free.tariff.text_usd_per_million.numerator = 0;
    free.tariff.image_usd_per_million.numerator = 0;
    assert_eq!(free.cost(usage(3, 9)).unwrap(), 0);
    let mut unknown = usage(0, 0);
    unknown.image_tokens = None;
    assert!(free.cost(unknown).is_err());
}

#[test]
fn shared_total_hold_bounds_every_partition_without_double_reservation() {
    let mut price = tariff();
    price.input_token_limit = 12;
    for (group, speed) in [
        (1_000_000, 1_000_000),
        (1_250_000, 2_000_000),
        (0, 1_000_000),
    ] {
        price.group_multiplier_ppm = group;
        price.speed_multiplier_ppm = speed;
        for discount in [None, Some(DiscountRatio::new(3, 5).unwrap())] {
            let hold = price.reservation_ceiling_with_discount(discount).unwrap();
            let mut maximum = 0;
            for total in 0..=12 {
                for text in 0..=total {
                    let cost = price
                        .cost_with_discount(usage(text, total - text), discount)
                        .unwrap();
                    assert!(cost <= hold);
                    maximum = maximum.max(cost);
                }
            }
            assert_eq!(hold, maximum);
        }
    }
    let price = tariff();
    assert!(price.cost(usage(21, 0)).is_err());
}

#[test]
fn incomplete_contradictory_decreasing_or_different_provider_evidence_is_rejected() {
    let previous = usage(3, 4);
    assert!(usage(4, 5).follows(previous));
    assert!(!usage(2, 6).follows(previous));
    let mut unknown = previous;
    unknown.text_tokens = None;
    assert!(unknown.valid());
    assert!(!unknown.complete());
    assert!(!unknown.follows(previous));
    assert!(tariff().cost(unknown).is_err());
    let mut inconsistent = previous;
    inconsistent.total_input_tokens = Some(8);
    assert!(!inconsistent.valid());
    let mut wrong = previous;
    wrong.provider = EmbeddingProvider::AliInclusiveInputV1;
    assert!(!wrong.follows(previous));
    assert!(tariff().cost(wrong).is_err());
    let mut overflow = previous;
    overflow.image_tokens = Some(u64::MAX);
    assert!(!overflow.valid());
}

#[test]
fn frozen_price_and_unknown_versus_zero_round_trip_strictly() {
    let original = tariff();
    let restored: PersistedEmbeddingPrice =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    assert_eq!(restored, original);
    let unknown: EmbeddingUsage =
        serde_json::from_value(json!({"provider":"ali_separate_input_v1"})).unwrap();
    assert!(unknown.valid());
    assert!(!unknown.complete());
    assert_eq!(
        serde_json::to_value(unknown).unwrap(),
        json!({"provider":"ali_separate_input_v1"})
    );
    assert!(usage(0, 0).complete());
    for value in [json!(-1), json!(1.5), json!("1")] {
        assert!(
            serde_json::from_value::<EmbeddingUsage>(
                json!({"provider":"ali_separate_input_v1","text_tokens":value})
            )
            .is_err()
        );
    }
    assert!(
        serde_json::from_str::<EmbeddingUsage>(
            r#"{"provider":"ali_separate_input_v1","text_tokens":1,"text_tokens":2}"#
        )
        .is_err()
    );
    let mut changed = original;
    changed.tariff.image_usd_per_million.numerator = 99;
    assert_ne!(restored.cost(usage(1, 3)), changed.cost(usage(1, 3)));
    changed.tariff.revision = 2;
    assert!(changed.validate().is_err());
}

#[test]
fn invalid_prices_and_final_credit_overflow_fail_without_partial_rounding() {
    let mut price = tariff();
    price.tariff.text_usd_per_million.denominator = 0;
    assert!(price.validate().is_err());
    price = tariff();
    price.input_token_limit = 0;
    assert!(price.validate().is_err());
    price = tariff();
    price.tariff.text_usd_per_million.numerator = i64::MAX as u64;
    price.tariff.text_usd_per_million.denominator = 1;
    assert_eq!(price.cost(usage(20, 0)), Err(RoutingError::PriceOverflow));
    price.group_multiplier_ppm = 0;
    assert_eq!(price.cost(usage(20, 0)).unwrap(), 0);
}
