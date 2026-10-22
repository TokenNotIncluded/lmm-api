//! Frozen search receipts across finalization and recovery. No live provider is
//! contacted by these tests; PostgreSQL integration remains a separate gate.
use super::*;
use crate::billing::charging::{Price, ReservationBasis};
use crate::routing::{SearchProvider, SearchUsage};
use serde_json::json;

fn price() -> Price {
    serde_json::from_value(json!({
        "version":"synthetic-search-1", "model":"synthetic", "group":"allowed",
        "input_per_million":1, "cached_per_million":0, "output_per_million":0,
        "search":{
            "tariff":{
                "revision":1,"provider":"open_ai_responses_v1","unit":"call",
                "currency":"USD","usd_per_unit":{"numerator":1,"denominator":1000000}
            },
            "limits":{"calls":3},"supplier_contract_revision":"synthetic-finite-3",
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,"credits_per_usd":500000
        }
    }))
    .unwrap()
}

fn observed(calls: Option<u64>) -> crate::relay::Usage {
    crate::relay::Usage {
        input_tokens: Some(10),
        output_tokens: Some(0),
        cached_input_tokens: Some(0),
        search: Some(SearchUsage {
            provider: SearchProvider::OpenAiResponsesV1,
            calls,
            queries: None,
            grounded_requests: None,
            x_posts_fetched: None,
            x_users_fetched: None,
        }),
        ..Default::default()
    }
}

#[test]
fn search_finalization_preserves_exact_unit_and_rejects_missing_or_over_bound_receipts() {
    let frozen = price();
    let normalized = billing::complete_priced_usage(observed(Some(1)), &frozen).unwrap();
    assert_eq!(normalized.search.unwrap().calls, Some(1));
    // USD .000001 search = .5 credits, plus .00001 token credits: one ceil.
    assert_eq!(frozen.cost(normalized), Ok(1));
    assert_eq!(
        frozen.reservation_ceiling(ReservationBasis::Tokens {
            input: 10,
            output: 0
        }),
        Ok(2)
    );
    for mut invalid in [observed(None), observed(Some(4))] {
        assert!(billing::complete_priced_usage(invalid, &frozen).is_err());
        invalid.search = None;
        assert!(billing::complete_priced_usage(invalid, &frozen).is_err());
    }
    let zero = billing::complete_priced_usage(observed(Some(0)), &frozen).unwrap();
    assert_eq!(zero.search.unwrap().calls, Some(0));
    let mut wrong = observed(Some(1));
    wrong.search.as_mut().unwrap().provider = SearchProvider::AzureResponsesV1;
    assert!(billing::complete_priced_usage(wrong, &frozen).is_err());
    let mut unpriced = frozen.clone();
    unpriced.search = None;
    assert!(billing::complete_priced_usage(observed(Some(1)), &unpriced).is_err());
}

#[test]
fn search_frozen_price_and_receipt_survive_json_without_current_provider_policy() {
    let original = price();
    let restored: Price = serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    let evidence: crate::relay::Usage =
        serde_json::from_value(serde_json::to_value(observed(Some(2))).unwrap()).unwrap();
    let usage = billing::complete_priced_usage(evidence, &restored).unwrap();
    assert_eq!(restored, original);
    assert_eq!(restored.cost(usage), Ok(2));
    let mut changed = restored;
    changed
        .search
        .as_mut()
        .unwrap()
        .tariff
        .usd_per_unit
        .numerator = 10;
    assert_eq!(original.cost(usage), Ok(2));
    assert_eq!(changed.cost(usage), Ok(11));
}

#[test]
fn search_policy_does_not_grant_loopback_or_mismatched_endpoint_implicitly() {
    let policy = ProviderPolicy::new(
        vec![ExternalModelContext {
            search: Some(ExternalSearchContract {
                provider: SearchProvider::OpenAiResponsesV1,
                limits: crate::routing::SearchLimits {
                    calls: Some(3),
                    ..Default::default()
                },
                contract_revision: "synthetic-finite-3".into(),
            }),
            upstream_id: 1,
            upstream_model: "synthetic".into(),
            protocol: crate::routing::Protocol::OpenAiResponses,
            endpoint: "http://127.0.0.1:12345/v1/".into(),
            max_input_tokens: 100,
            max_output_tokens: 100,
            contract_revision: "synthetic-context".into(),
            missing_cached_tokens_is_zero: true,
            missing_cache_creation_tokens_is_zero: true,
        }],
        100,
    )
    .unwrap();
    assert!(
        policy
            .search_contract_for_binding(
                1,
                "synthetic",
                crate::routing::Protocol::OpenAiResponses,
                "http://127.0.0.1:12345/v1/"
            )
            .is_ok()
    );
    for (id, model, endpoint) in [
        (2, "synthetic", "http://127.0.0.1:12345/v1/"),
        (1, "other", "http://127.0.0.1:12345/v1/"),
        (1, "synthetic", "http://127.0.0.1:12346/v1/"),
    ] {
        assert!(
            policy
                .search_contract_for_binding(
                    id,
                    model,
                    crate::routing::Protocol::OpenAiResponses,
                    endpoint
                )
                .is_err()
        );
    }
    assert!(
        ProviderPolicy::default()
            .search_contract_for_binding(
                1,
                "synthetic",
                crate::routing::Protocol::OpenAiResponses,
                "http://127.0.0.1:12345/v1/"
            )
            .is_err()
    );
}
