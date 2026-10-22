use super::{Error, Result, Usage};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    event_id: String,
    usage: Usage,
}

/// Recover an actual cumulative snapshot, never a combination of unrelated
/// maxima. Event IDs and wall-clock timestamps do not order transactions.
pub(super) fn restore(base: Usage, evidence: Option<Value>) -> Result<Usage> {
    let Some(evidence) = evidence else {
        return Ok(base);
    };
    let events = evidence.as_array().ok_or(Error::Storage)?;
    // Old events predate typed cache writes/search. Their replay and decoding behavior
    // stays unchanged, including the distinction between absent and zero.
    if !events.iter().any(|event| {
        event
            .get("usage")
            .and_then(Value::as_object)
            .is_some_and(|usage| {
                usage.contains_key("cache_creation") || usage.contains_key("search")
            })
    }) {
        return Ok(base);
    }
    let mut events: Vec<Evidence> = serde_json::from_value(evidence).map_err(|_| Error::Storage)?;
    if events.iter().any(|event| {
        !(event.event_id == "final" || event.event_id.starts_with("usage:")) || !event.usage.valid()
    }) {
        return Err(Error::Storage);
    }
    // Every coordinate below is monotone under Usage::follows. Sorting these
    // coordinates and checking every adjacent pair proves one cumulative chain
    // without relying on insertion order or synthesizing a new usage receipt.
    events.sort_unstable_by_key(|event| {
        let usage = event.usage;
        (
            usage.input,
            usage.cached,
            usage.output,
            usage.upstream_cost_atto_usd,
            usage.cache_creation.map(|value| {
                (
                    value.total,
                    value.reported_total,
                    value.five_minute,
                    value.one_hour,
                    value.twelve_hour,
                )
            }),
            usage.search.map(|search| {
                (
                    search.provider,
                    search.calls,
                    search.queries,
                    search.grounded_requests,
                    search.x_posts_fetched,
                    search.x_users_fetched,
                )
            }),
            usage
                .image
                .map(|value| (value.output_images, value.provider_receipt_sha256)),
        )
    });
    if events
        .windows(2)
        .any(|pair| !pair[1].usage.follows(pair[0].usage))
    {
        return Err(Error::Storage);
    }
    let latest = events.last().ok_or(Error::Storage)?.usage;
    if latest.input != base.input
        || latest.cached != base.cached
        || latest.output != base.output
        || latest.image != base.image
        || latest.upstream_cost_atto_usd != base.upstream_cost_atto_usd
        || events
            .iter()
            .any(|event| event.event_id == "final" && event.usage != latest)
    {
        return Err(Error::Storage);
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::charging::CacheCreationUsage;
    use serde_json::json;

    fn usage(input: i64, total: i64, five_minute: Option<i64>, one_hour: Option<i64>) -> Usage {
        Usage {
            input,
            cache_creation: Some(CacheCreationUsage {
                twelve_hour: None,
                total,
                five_minute,
                one_hour,
                cache_creation_mode: None,
                reported_total: None,
            }),
            ..Usage::default()
        }
    }
    fn base(mut usage: Usage) -> Usage {
        usage.cache_creation = None;
        usage.search = None;
        usage
    }
    fn event(event_id: &str, usage: Usage) -> Value {
        json!({"event_id":event_id,"usage":usage})
    }

    fn generic(reported: i64) -> Usage {
        let mut value = usage(30, 10, Some(5), Some(5));
        let created = value.cache_creation.as_mut().unwrap();
        created.cache_creation_mode = Some(crate::routing::CacheCreationMode::GenericRemainderV1);
        created.reported_total = Some(reported);
        value
    }

    #[test]
    fn generic_replay_keeps_raw_aggregate_when_the_normalized_total_is_unchanged() {
        let first = generic(8);
        let latest = generic(9);
        assert_eq!(
            restore(
                base(latest),
                Some(json!(
                    [event("final", latest), event("usage:first", first),]
                ))
            ),
            Ok(latest)
        );
        assert_eq!(latest.cache_creation.unwrap().reported_total, Some(9));
    }

    #[test]
    fn generic_replay_cannot_relabel_strict_evidence_or_fabricate_an_effective_total() {
        let latest = generic(9);
        let strict = usage(30, 10, Some(5), Some(5));
        assert_eq!(
            restore(
                base(latest),
                Some(json!([
                    event("usage:strict", strict),
                    event("final", latest),
                ]))
            ),
            Err(Error::Storage)
        );
        let mut wrong = latest;
        wrong.cache_creation.as_mut().unwrap().total = 11;
        assert_eq!(
            restore(base(wrong), Some(json!([event("final", wrong)]))),
            Err(Error::Storage)
        );
    }

    #[test]
    fn absent_legacy_evidence_preserves_absence_and_old_decoding() {
        let old = Usage {
            input: 12,
            ..Usage::default()
        };
        assert_eq!(restore(old, None), Ok(old));
        assert_eq!(restore(old, Some(json!([]))), Ok(old));
        assert_eq!(
            restore(old, Some(json!([{"usage":{"old":"shape"}}]))),
            Ok(old)
        );
        assert_eq!(
            restore(old, Some(json!([event("usage:old", old)]))),
            Ok(old)
        );
    }

    #[test]
    fn explicit_zero_is_restored_without_rewriting_legacy_none() {
        let zero = usage(0, 0, Some(0), Some(0));
        let legacy = Usage::default();
        assert_eq!(
            restore(
                legacy,
                Some(json!([event("final", zero), event("usage:old", legacy)]))
            ),
            Ok(zero)
        );
        assert!(
            serde_json::to_value(legacy)
                .unwrap()
                .get("cache_creation")
                .is_none()
        );
        assert!(
            serde_json::to_value(zero)
                .unwrap()
                .get("cache_creation")
                .is_some()
        );
    }

    #[test]
    fn shuffled_cumulative_chain_restores_one_complete_receipt() {
        let first = usage(20, 5, Some(5), None);
        let last = usage(40, 12, Some(7), Some(5));
        let old = Usage {
            input: 10,
            ..Usage::default()
        };
        assert_eq!(
            restore(
                base(last),
                Some(json!([
                    event("final", last),
                    event("usage:z-first", first),
                    event("usage:a-old", old)
                ]))
            ),
            Ok(last)
        );
    }

    #[test]
    fn incomparable_counter_snapshots_cannot_be_combined() {
        let first = usage(20, 5, Some(4), Some(1));
        let second = usage(40, 8, Some(2), Some(6));
        assert_eq!(
            restore(
                base(second),
                Some(json!([event("usage:a", first), event("final", second)]))
            ),
            Err(Error::Storage)
        );
    }

    #[test]
    fn disappearance_after_counter_progress_is_rejected() {
        let first = usage(20, 5, Some(5), None);
        let missing = Usage {
            input: 40,
            ..Usage::default()
        };
        assert_eq!(
            restore(
                missing,
                Some(json!([event("usage:a", first), event("usage:b", missing)]))
            ),
            Err(Error::Storage)
        );
    }

    #[test]
    fn final_may_not_lose_cache_fields_at_identical_base_counters() {
        let first = usage(20, 0, Some(0), Some(0));
        let missing = base(first);
        assert_eq!(
            restore(
                missing,
                Some(json!([event("usage:a", first), event("final", missing)]))
            ),
            Err(Error::Storage)
        );
    }

    #[test]
    fn base_counter_or_cost_mismatch_is_rejected() {
        let mut final_usage = usage(20, 5, Some(4), Some(1));
        final_usage.upstream_cost_atto_usd = Some(123);
        let evidence = json!([event("final", final_usage)]);
        let mut different = base(final_usage);
        different.input += 1;
        assert_eq!(
            restore(different, Some(evidence.clone())),
            Err(Error::Storage)
        );
        different = base(final_usage);
        different.upstream_cost_atto_usd = None;
        assert_eq!(restore(different, Some(evidence)), Err(Error::Storage));
    }

    #[test]
    fn invalid_counts_and_unknown_fields_fail_closed() {
        let invalid = usage(10, 12, Some(12), None);
        assert_eq!(
            restore(base(invalid), Some(json!([event("final", invalid)]))),
            Err(Error::Storage)
        );
        let valid = usage(20, 5, Some(5), None);
        let mut evidence = json!([event("final", valid)]);
        evidence[0]["usage"]["unexpected"] = json!(true);
        assert_eq!(restore(base(valid), Some(evidence)), Err(Error::Storage));
    }

    #[test]
    fn image_and_typed_cache_cannot_share_a_receipt() {
        let mut invalid = usage(0, 0, Some(0), Some(0));
        invalid.image = Some(crate::model_http::image_price::ImageUsage {
            output_images: 1,
            provider_receipt_sha256: [1; 32],
        });
        assert_eq!(
            restore(base(invalid), Some(json!([event("final", invalid)]))),
            Err(Error::Storage)
        );
    }

    #[test]
    fn cache_12h_replay_restores_one_actual_snapshot_and_rejects_reclassification() {
        let mut first = usage(20, 6, None, None);
        first.cache_creation.as_mut().unwrap().twelve_hour = Some(1);
        let mut latest = usage(40, 10, None, None);
        latest.cache_creation.as_mut().unwrap().twelve_hour = Some(4);
        assert_eq!(
            restore(
                base(latest),
                Some(json!([
                    event("final", latest),
                    event("usage:first", first),
                    event("usage:duplicate", first)
                ]))
            ),
            Ok(latest)
        );
        let mut reclassified = latest;
        reclassified.cache_creation.as_mut().unwrap().twelve_hour = Some(8);
        assert_eq!(
            restore(
                base(reclassified),
                Some(json!([
                    event("usage:first", first),
                    event("final", reclassified)
                ]))
            ),
            Err(Error::Storage)
        );
        let mut missing = latest;
        missing.cache_creation.as_mut().unwrap().twelve_hour = None;
        assert_eq!(
            restore(
                base(missing),
                Some(json!([
                    event("usage:first", first),
                    event("final", missing)
                ]))
            ),
            Err(Error::Storage)
        );
    }

    #[test]
    fn search_replay_restores_actual_cumulative_dimensions_without_fabricating_a_maximum() {
        use crate::routing::{SearchProvider, SearchUsage};
        let first = Usage {
            input: 10,
            search: Some(SearchUsage {
                provider: SearchProvider::XaiResponsesV1,
                calls: Some(1),
                queries: None,
                grounded_requests: None,
                x_posts_fetched: Some(2),
                x_users_fetched: Some(1),
            }),
            ..Usage::default()
        };
        let latest = Usage {
            input: 20,
            search: Some(SearchUsage {
                calls: Some(2),
                x_posts_fetched: Some(4),
                ..first.search.unwrap()
            }),
            ..first
        };
        assert_eq!(
            restore(
                base(latest),
                Some(json!([
                    event("final", latest),
                    event("usage:first", first),
                    event("usage:repeated", first)
                ]))
            ),
            Ok(latest)
        );
        for search in [
            SearchUsage {
                x_posts_fetched: Some(1),
                ..latest.search.unwrap()
            },
            SearchUsage {
                x_users_fetched: None,
                ..latest.search.unwrap()
            },
            SearchUsage {
                provider: SearchProvider::OpenAiResponsesV1,
                x_posts_fetched: None,
                x_users_fetched: None,
                ..latest.search.unwrap()
            },
        ] {
            let invalid = Usage {
                search: Some(search),
                ..latest
            };
            assert_eq!(
                restore(
                    base(invalid),
                    Some(json!([
                        event("usage:first", first),
                        event("final", invalid)
                    ]))
                ),
                Err(Error::Storage)
            );
        }
        let missing = Usage {
            search: None,
            ..latest
        };
        assert_eq!(
            restore(
                base(missing),
                Some(json!([
                    event("usage:first", first),
                    event("final", missing)
                ]))
            ),
            Err(Error::Storage)
        );
    }
}
