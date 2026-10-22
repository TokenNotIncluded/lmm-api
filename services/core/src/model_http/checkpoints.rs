//! Append-only evidence through an EXECUTE-only PostgreSQL function. A retry
//! keeps every parameter identical, including sequence, after an unknown ACK.
use super::Bridge;
use crate::{
    billing::charging::{self, Outcome},
    relay::{OutputCheckpoint, RelayError, Reservation},
};

/// A discovery row carries evidence, not permission to change the payer or price.
type CompletedStream = (String, i64, serde_json::Value, i64);

impl super::ModelGateway {
    /// Scan one bounded recovery page. Only the trusted Rust host may call this.
    /// The returned pair is (confirmed settlements, next request ID). Continue
    /// with the returned ID even when the count is zero. None ends this sweep;
    /// start the next sweep with None so earlier/newly eligible IDs are revisited.
    ///
    /// Requires S11's EXECUTE-only completed_streams_after(integer,text) contract.
    /// An older database returns Unsupported; never silently fall back to a
    /// first-page query that can starve later requests. No upstream is contacted.
    pub async fn reconcile_completed_streams_page(
        &self,
        limit: i32,
        after_request_id: Option<&str>,
    ) -> Result<(usize, Option<String>), RelayError> {
        self.bridge
            .reconcile_completed_streams_page_internal(limit, after_request_id)
            .await
    }
}

fn validate_recovery_page(limit: i32, after: Option<&str>) -> Result<(), RelayError> {
    if !(1..=1000).contains(&limit)
        || after.is_some_and(|id| {
            id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        })
    {
        return Err(RelayError::Invalid("model recovery page"));
    }
    Ok(())
}

fn next_recovery_page(rows: &[CompletedStream], limit: usize) -> Option<String> {
    rows.last()
        .filter(|_| rows.len() == limit)
        .map(|row| row.0.clone())
}

/// Reject a known billable dimension before either partial or terminal output
/// becomes durable. A missing aggregate does not erase a known positive subset.
fn validate_cache_write_dimensions(
    usage: crate::relay::Usage,
    price: &charging::Price,
) -> Result<(), RelayError> {
    price
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    if let Some(observed) = usage.search {
        if !observed.valid()
            || price.search.as_ref().is_some_and(|search| {
                observed.provider != search.tariff.provider || !search.limits.contains(observed)
            })
            || (price.search.is_none() && observed.has_positive())
        {
            return Err(RelayError::SettlementPending);
        }
    }
    // This separate contract prices the supplier's measured total USD cost;
    // individual token categories do not determine its settlement amount.
    if price.upstream_cost_group_ppm.is_some() {
        return Ok(());
    }
    let revision = price.tiered.as_ref().map(|tariff| tariff.tariff.revision);
    let requires_creation = revision.is_some_and(|revision| revision != 1);
    let supports_12h = revision == Some(4);
    let anthropic_creation = usage.cache_creation_details.is_some_and(|details| {
        details
            .ephemeral_5m_input_tokens
            .is_some_and(|value| value != 0)
            || details
                .ephemeral_1h_input_tokens
                .is_some_and(|value| value != 0)
    });
    let short_creation = usage
        .openai_cache_write_details
        .is_some_and(|details| details.ttl_30m_tokens.is_some_and(|value| value != 0));
    let long_creation = usage
        .openai_cache_write_details
        .is_some_and(|details| details.ttl_12h_tokens.is_some_and(|value| value != 0));
    if (!requires_creation
        && (usage
            .cache_creation_input_tokens
            .is_some_and(|value| value != 0)
            || anthropic_creation
            || short_creation
            || long_creation))
        || (!supports_12h && long_creation)
        || (supports_12h && anthropic_creation)
    {
        return Err(RelayError::SettlementPending);
    }
    Ok(())
}

impl Bridge {
    pub(super) async fn reconcile_completed_streams(
        &self,
        limit: i32,
    ) -> Result<usize, RelayError> {
        validate_recovery_page(limit, None)?;
        let mut cursor_guard = self.recovery_cursor.lock().await;
        let after = cursor_guard.as_deref();
        let (recovered, next) = self
            .reconcile_completed_streams_page_internal(limit, after)
            .await?;
        *cursor_guard = next;
        Ok(recovered)
    }

    pub(super) async fn reconcile_completed_streams_page_internal(
        &self,
        limit: i32,
        after_request_id: Option<&str>,
    ) -> Result<(usize, Option<String>), RelayError> {
        validate_recovery_page(limit, after_request_id)?;
        let rows: Vec<CompletedStream> = sqlx::query_as(
            "SELECT request_id,revision,provider_usage,sequence \
             FROM core_model.completed_streams_after($1,$2)",
        )
        .bind(limit)
        .bind(after_request_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| {
            if error.as_database_error().and_then(|e| e.code()).as_deref() == Some("42883") {
                RelayError::Unsupported("paged model recovery requires the database contract")
            } else {
                RelayError::BillingUnavailable
            }
        })?;
        // Advance by scanned rows, not successful settlements. A full page of
        // malformed or concurrently resolved receipts must still yield a cursor.
        let next = next_recovery_page(&rows, limit as usize);
        let recovered = self.reconcile_stream_rows(rows).await?;
        Ok((recovered, next))
    }

    async fn reconcile_stream_rows(&self, rows: Vec<CompletedStream>) -> Result<usize, RelayError> {
        let mut recovered = 0;
        for (id, revision, value, sequence) in rows {
            let Some(original) = self
                .charging
                .resolve(&id)
                .await
                .map_err(super::billing::error)?
            else {
                continue;
            };
            if original.state != "reconcile" || original.revision != revision {
                continue;
            }
            let upstream_cost = original.price.upstream_cost_group_ppm.is_some();
            // A malformed historical receipt must not block the other rows in
            // this bounded batch. Never turn failed decoding into zero usage.
            let Some(usage) = recovery_priced_usage(value, sequence, &original.price) else {
                continue;
            };
            if upstream_cost && usage.upstream_cost_atto_usd.is_none() {
                continue;
            }
            let usage = if upstream_cost {
                usage
            } else {
                charging::Usage {
                    upstream_cost_atto_usd: None,
                    ..usage
                }
            };
            let evidence = format!("model-stream:{id}:{sequence}");
            let result = super::billing::retry(|| {
                self.charging
                    .reconcile(&id, revision, usage, Outcome::Completed, &evidence)
            })
            .await;
            let confirmed = match result {
                Ok(charge) => Some(charge),
                Err(
                    charging::Error::CommitUnknown
                    | charging::Error::Conflict
                    | charging::Error::WrongState,
                ) => {
                    // A lost ACK or another recovery worker is resolved by a
                    // read of the SAME operation. No model call or new hold.
                    self.charging
                        .resolve(&id)
                        .await
                        .map_err(super::billing::error)?
                }
                Err(charging::Error::Invalid | charging::Error::NeedReservation) => continue,
                Err(error) => return Err(super::billing::error(error)),
            };
            if confirmed.as_ref().is_some_and(|charge| {
                super::billing::confirmed_settlement(
                    charge,
                    &id,
                    &original.price,
                    usage,
                    Outcome::Completed,
                )
            }) {
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    pub(super) async fn persist_output(
        &self,
        reservation: &Reservation,
        checkpoint: &OutputCheckpoint,
    ) -> Result<(), RelayError> {
        let (worker, output_ceiling, selected) = {
            let active = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let operation = active.get(&reservation.id).ok_or(RelayError::Forbidden)?;
            (
                operation.worker,
                operation.output_token_ceiling,
                operation
                    .selected
                    .clone()
                    .ok_or(RelayError::RouteUnavailable)?,
            )
        };
        let upstream_cost = selected.price.upstream_cost_group_ppm.is_some();
        validate_cache_write_dimensions(checkpoint.usage, &selected.price)?;
        let normalized = if checkpoint.terminal_observed {
            Some(super::billing::complete_priced_usage(
                checkpoint.usage,
                &selected.price,
            )?)
        } else {
            None
        };
        // A persisted terminal is authority for automatic settlement after an
        // OS restart. Validate the original provider contract BEFORE writing it,
        // so recovery cannot turn a rejected receipt into a zero-cache charge.
        if checkpoint.terminal_observed {
            let mut policy_usage = checkpoint.usage;
            if let Some(usage) = normalized {
                policy_usage.input_tokens = Some(usage.input as u64);
            }
            self.provider_policy
                .validate_usage(&selected.attempt, &policy_usage, upstream_cost)?;
        }
        checkpoint
            .usage
            .validate()
            .map_err(|_| RelayError::SettlementPending)?;
        if normalized
            .map(|usage| usage.input as u64)
            .or(checkpoint.usage.input_tokens)
            .is_some_and(|v| v > selected.input_token_ceiling as u64)
            || checkpoint
                .usage
                .output_tokens
                .is_some_and(|v| v > u64::from(output_ceiling))
            || matches!((checkpoint.usage.cached_input_tokens,checkpoint.usage.input_tokens),(Some(cached),Some(input)) if cached>input)
            || matches!((checkpoint.usage.reasoning_output_tokens,checkpoint.usage.output_tokens),(Some(reasoning),Some(output)) if reasoning>output)
        {
            return Err(RelayError::SettlementPending);
        }
        let usage =
            serde_json::to_value(checkpoint.usage).map_err(|_| RelayError::BillingUnavailable)?;
        let sequence =
            i64::try_from(checkpoint.sequence).map_err(|_| RelayError::SettlementPending)?;
        let bytes =
            i64::try_from(checkpoint.packet_bytes).map_err(|_| RelayError::SettlementPending)?;
        for attempt in 0..3 {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|_| RelayError::BillingUnavailable)?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                .execute(&mut *tx)
                .await
                .map_err(|_| RelayError::BillingUnavailable)?;
            sqlx::query("SET LOCAL synchronous_commit=on")
                .execute(&mut *tx)
                .await
                .map_err(|_| RelayError::BillingUnavailable)?;
            let result = sqlx::query("SELECT core_model.checkpoint_output($1,$2,$3,$4,$5,$6,$7)")
                .bind(&reservation.id)
                .bind(worker.as_slice())
                .bind(sequence)
                .bind(checkpoint.packet_sha256.as_slice())
                .bind(bytes)
                .bind(&usage)
                .bind(checkpoint.terminal_observed)
                .execute(&mut *tx)
                .await;
            match result {
                Ok(_) => {
                    // If COMMIT succeeded but its reply was lost, the next
                    // exact function call confirms the already committed row.
                    if tx.commit().await.is_ok() {
                        return Ok(());
                    }
                }
                Err(error) => {
                    let retryable = matches!(
                        error.as_database_error().and_then(|e| e.code()).as_deref(),
                        Some("40001" | "40P01")
                    );
                    let _ = tx.rollback().await;
                    if !retryable {
                        return Err(RelayError::SettlementPending);
                    }
                }
            }
            if attempt < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt + 1))).await;
            }
        }
        Err(RelayError::SettlementPending)
    }
}

#[cfg(test)]
fn recovery_usage(
    value: serde_json::Value,
    sequence: i64,
    upstream_cost: bool,
) -> Option<charging::Usage> {
    if !(1..=65_536).contains(&sequence) {
        return None;
    }
    let observed = serde_json::from_value(value).ok()?;
    super::billing::complete_usage(observed, upstream_cost).ok()
}

fn recovery_priced_usage(
    value: serde_json::Value,
    sequence: i64,
    price: &charging::Price,
) -> Option<charging::Usage> {
    if !(1..=65_536).contains(&sequence) {
        return None;
    }
    super::billing::complete_priced_usage(serde_json::from_value(value).ok()?, price).ok()
}

#[cfg(test)]
mod a07_recovery_tests {
    use super::*;
    use serde_json::json;

    fn dimension_price(revision: Option<u32>) -> charging::Price {
        let mut value = json!({
            "version":"routing-1-rule-1","model":"model","group":"allowed",
            "input_per_million":0,"cached_per_million":0,"output_per_million":0
        });
        if let Some(revision) = revision {
            let mut tariff = json!({
                "revision":revision,"source_digest":"56".repeat(32),
                "uses_cache_read":true,"uses_cache_write":revision != 1,
                "tiers":[{"up_to_input_tokens":null,"rates":{
                    "input":{"numerator":2,"denominator":1},
                    "output":{"numerator":10,"denominator":1},
                    "cache_read":{"numerator":1,"denominator":5},
                    "cache_write":{"numerator":if revision == 1 {0} else {5},"denominator":2}
                }}]
            });
            if revision == 2 || revision == 3 {
                tariff["uses_cache_write_1h"] = json!(true);
                tariff["tiers"][0]["rates"]["cache_write_1h"] =
                    json!({"numerator":4,"denominator":1});
            }
            if revision == 4 {
                tariff["uses_cache_write_12h"] = json!(true);
                tariff["tiers"][0]["rates"]["cache_write_12h"] =
                    json!({"numerator":5,"denominator":1});
            }
            value["tiered"] = json!({
                "tariff":tariff,"input_semantics":"inclusive",
                "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,
                "credits_per_usd":500000
            });
            if revision == 3 {
                value["tiered"]["cache_creation_mode"] = json!("generic_remainder_v1");
            }
        }
        let price: charging::Price = serde_json::from_value(value).unwrap();
        price.validate().unwrap();
        price
    }

    #[test]
    fn cache_dimensions_reject_known_unsupported_subsets_even_without_aggregate() {
        use crate::relay::{CacheCreationDetails, OpenAiCacheWriteDetails, Usage};
        let twelve = Usage {
            input_tokens: Some(12),
            cached_input_tokens: Some(0),
            output_tokens: Some(1),
            openai_cache_write_details: Some(OpenAiCacheWriteDetails {
                ttl_30m_tokens: None,
                ttl_12h_tokens: Some(3),
            }),
            ..Usage::default()
        };
        for aggregate in [None, Some(3)] {
            let observed = Usage {
                cache_creation_input_tokens: aggregate,
                ..twelve
            };
            observed.validate().unwrap();
            for revision in [None, Some(1), Some(2), Some(3)] {
                assert!(
                    matches!(
                        validate_cache_write_dimensions(observed, &dimension_price(revision)),
                        Err(RelayError::SettlementPending)
                    ),
                    "{revision:?}: {observed:?}"
                );
            }
            assert!(validate_cache_write_dimensions(observed, &dimension_price(Some(4))).is_ok());
        }
        let short = Usage {
            openai_cache_write_details: Some(OpenAiCacheWriteDetails {
                ttl_30m_tokens: Some(3),
                ttl_12h_tokens: Some(0),
            }),
            ..twelve
        };
        short.validate().unwrap();
        for revision in [None, Some(1)] {
            assert!(validate_cache_write_dimensions(short, &dimension_price(revision)).is_err());
        }
        // Supported partial dimensions may await their total; terminal
        // normalization separately requires a complete authoritative receipt.
        for revision in [2, 3, 4] {
            assert!(
                validate_cache_write_dimensions(short, &dimension_price(Some(revision))).is_ok()
            );
        }
        let anthropic = Usage {
            openai_cache_write_details: None,
            cache_creation_details: Some(CacheCreationDetails {
                ephemeral_5m_input_tokens: None,
                ephemeral_1h_input_tokens: Some(3),
            }),
            ..twelve
        };
        anthropic.validate().unwrap();
        assert!(validate_cache_write_dimensions(anthropic, &dimension_price(Some(4))).is_err());
        for revision in [None, Some(1), Some(2), Some(3), Some(4)] {
            assert!(
                validate_cache_write_dimensions(Usage::default(), &dimension_price(revision))
                    .is_ok()
            );
        }
    }

    #[test]
    fn upstream_cost_checkpoint_contract_keeps_total_cost_independent_of_cache_dimensions() {
        let mut price = dimension_price(None);
        price.model = "openrouter/auto-beta".into();
        price.upstream_cost_group_ppm = Some(1_000_000);
        price.validate().unwrap();
        let observed: crate::relay::Usage = serde_json::from_value(json!({
            "input_tokens":12,"output_tokens":1,"cached_input_tokens":0,
            "cache_creation_input_tokens":null,"reasoning_output_tokens":null,
            "openai_cache_write_details":{"ttl_12h_tokens":3},
            "upstream_cost_atto_usd":crate::upstream_cost::ATTO_USD
        }))
        .unwrap();
        observed.validate().unwrap();
        assert!(validate_cache_write_dimensions(observed, &price).is_ok());
        let usage =
            recovery_priced_usage(serde_json::to_value(observed).unwrap(), 1, &price).unwrap();
        assert_eq!(price.cost(usage).unwrap(), 500_000);
        assert!(usage.cache_creation.is_none());
    }

    #[test]
    fn cache_v2_recovery_uses_the_original_frozen_usage_contract() {
        let mut price: charging::Price = serde_json::from_value(json!({
            "version":"routing-2-rule-1","model":"model","group":"allowed",
            "input_per_million":0,"cached_per_million":0,"output_per_million":0,
            "tiered":{
                "input_semantics":"anthropic_text","group_multiplier_ppm":1000000,
                "speed_multiplier_ppm":1000000,"credits_per_usd":500000,
                "tariff":{"revision":2,"source_digest":"34".repeat(32),
                    "uses_cache_read":true,"uses_cache_write":true,"uses_cache_write_1h":true,
                    "tiers":[{"up_to_input_tokens":null,"rates":{
                        "input":{"numerator":2,"denominator":1},
                        "output":{"numerator":10,"denominator":1},
                        "cache_read":{"numerator":1,"denominator":5},
                        "cache_write":{"numerator":5,"denominator":2},
                        "cache_write_1h":{"numerator":4,"denominator":1}
                    }}]}
            }
        }))
        .unwrap();
        price.validate().unwrap();
        let receipt = json!({"input_tokens":100,"output_tokens":5,"cached_input_tokens":20,
            "cache_creation_input_tokens":70,
            "cache_creation_details":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":40}});
        let usage = recovery_priced_usage(receipt.clone(), 1, &price).unwrap();
        assert_eq!(price.cost(usage).unwrap(), 155);
        assert!(recovery_usage(receipt, 1, false).is_none());
        let unknown = json!({"input_tokens":100,"output_tokens":5,"cached_input_tokens":20});
        assert!(recovery_priced_usage(unknown.clone(), 1, &price).is_none());
        price
            .tiered
            .as_mut()
            .unwrap()
            .missing_cache_creation_is_zero = true;
        let explicit_contract_zero = recovery_priced_usage(unknown, 1, &price).unwrap();
        assert_eq!(explicit_contract_zero.cache_creation.unwrap().total, 0);
        assert_eq!(
            explicit_contract_zero.cache_creation.unwrap().one_hour,
            Some(0)
        );
        let aggregate_only = json!({"input_tokens":100,"output_tokens":5,"cached_input_tokens":20,"cache_creation_input_tokens":70});
        assert!(recovery_priced_usage(aggregate_only, 1, &price).is_none());
    }

    #[tokio::test]
    async fn provider_terminal_contract_rejection_precedes_any_persisted_recovery_authority() {
        use crate::model_http::{
            ChargePolicy, ExternalModelContext, ModelGateway, ProviderPolicy, UpstreamCredential,
        };
        use crate::routing::{
            self, CredentialRef, CreditRate, PriceSnapshot, Protocol, ReplayPermission,
            RouteRequest,
        };
        use std::{collections::BTreeMap, sync::Arc, time::Duration};
        let config = serde_json::from_value(json!({
            "version": 1, "groups": [{"name":"allowed","enabled":true}],
            "upstreams": [{"id":1,"endpoint":"https://provider.example/v1","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
            "routes": [{"id":1,"model":"model","group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":1,"upstream_id":1,"upstream_model":"bounded-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}], "aliases": []
        })).unwrap();
        let price_config = serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"model","group":"allowed","speed":"standard","rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
        let router = Arc::new(
            routing::Router::new(
                config,
                Arc::new(PriceSnapshot::compile(price_config).unwrap()),
            )
            .unwrap(),
        );
        let mut plan = router
            .route(RouteRequest {
                model: "model",
                group: "allowed",
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .unwrap();
        let quote = plan
            .quote()
            .with_credit_rate(CreditRate::new(1, 500_000).unwrap());
        let price = super::super::routing::credit_price(quote.price(), "model", "allowed").unwrap();
        let selected = super::super::routing::Selected {
            attempt: Arc::new(plan.start().unwrap()),
            price,
            quote,
            route_id: "r1-t1-u1".into(),
            input_token_ceiling: 64,
        };
        let policy = ProviderPolicy::new(
            vec![ExternalModelContext {
                search: None,
                missing_cache_creation_tokens_is_zero: false,
                upstream_id: 1,
                upstream_model: "bounded-model".into(),
                protocol: Protocol::OpenAiChat,
                endpoint: "https://provider.example/v1".into(),
                max_input_tokens: 64,
                max_output_tokens: 8,
                contract_revision: "synthetic-v1".into(),
                missing_cached_tokens_is_zero: false,
            }],
            128,
        )
        .unwrap();
        // This pool is deliberately unconnected. Reaching SQL is a test failure:
        // invalid terminals must never enter the recoverable evidence relation.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(20))
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1:1/synthetic")
            .unwrap();
        let gateway = ModelGateway::new_with_provider_policy(
            pool,
            router,
            vec![UpstreamCredential {
                reference: CredentialRef { id: 7, revision: 1 },
                secret: "synthetic-only".into(),
            }],
            BTreeMap::from([(1, "allowed".into())]),
            crate::relay::Limits {
                total_timeout: Duration::from_secs(5),
                connect_retries: 0,
                ..Default::default()
            },
            ChargePolicy {
                input_token_ceiling: 128,
                lease_seconds: 90,
            },
            policy,
        )
        .unwrap();
        gateway.bridge.active.lock().unwrap().insert(
            "terminal-contract".into(),
            super::super::Operation {
                secret: "synthetic-only".into(),
                group: "allowed".into(),
                worker: [1; 32],
                fingerprint: [2; 32],
                selected: Some(selected),
                output_token_ceiling: 8,
                peer: None,
                oauth_issuer: None,
            },
        );
        for usage in [
            crate::relay::Usage {
                input_tokens: Some(12),
                output_tokens: Some(3),
                ..Default::default()
            },
            crate::relay::Usage {
                input_tokens: Some(65),
                cached_input_tokens: Some(0),
                output_tokens: Some(3),
                ..Default::default()
            },
        ] {
            let checkpoint = OutputCheckpoint {
                sequence: 1,
                packet_sha256: [3; 32],
                packet_bytes: 1,
                usage,
                terminal_observed: true,
            };
            let result = gateway
                .bridge
                .persist_output(
                    &Reservation {
                        id: "terminal-contract".into(),
                    },
                    &checkpoint,
                )
                .await;
            assert!(
                matches!(result, Err(RelayError::SettlementPending)),
                "invalid terminal reached persistence: {result:?}"
            );
        }
        for terminal_observed in [false, true] {
            for details in [
                crate::relay::OpenAiCacheWriteDetails {
                    ttl_30m_tokens: None,
                    ttl_12h_tokens: Some(3),
                },
                crate::relay::OpenAiCacheWriteDetails {
                    ttl_30m_tokens: Some(3),
                    ttl_12h_tokens: Some(0),
                },
            ] {
                let checkpoint = OutputCheckpoint {
                    sequence: 1,
                    packet_sha256: [4; 32],
                    packet_bytes: 1,
                    usage: crate::relay::Usage {
                        input_tokens: Some(12),
                        cached_input_tokens: Some(0),
                        output_tokens: Some(3),
                        openai_cache_write_details: Some(details),
                        ..Default::default()
                    },
                    terminal_observed,
                };
                checkpoint.usage.validate().unwrap();
                let result = gateway
                    .bridge
                    .persist_output(
                        &Reservation {
                            id: "terminal-contract".into(),
                        },
                        &checkpoint,
                    )
                    .await;
                assert!(
                    matches!(result, Err(RelayError::SettlementPending)),
                    "unsupported cache subset reached persistence: {terminal_observed}: {result:?}"
                );
            }
        }
    }

    #[test]
    fn recovery_page_rejects_unbounded_or_invalid_cursors() {
        for limit in [i32::MIN, -1, 0, 1001, i32::MAX] {
            assert!(validate_recovery_page(limit, None).is_err());
        }
        for cursor in ["", "has space", "line\nbreak", "查询", "quote'", "\0"] {
            assert!(validate_recovery_page(1, Some(cursor)).is_err());
        }
        assert!(validate_recovery_page(1, Some(&"a".repeat(129))).is_err());
        for limit in [1, 1000] {
            for cursor in [None, Some("a-_.:/Z09")] {
                assert!(validate_recovery_page(limit, cursor).is_ok());
            }
        }
        assert!(validate_recovery_page(1, Some(&"a".repeat(128))).is_ok());
    }

    #[test]
    fn full_invalid_page_still_advances_by_the_last_scanned_id() {
        let rows = vec![
            (
                "a".into(),
                1,
                json!({"input_tokens":-1,"output_tokens":2}),
                1,
            ),
            (
                "b".into(),
                1,
                json!({"input_tokens":1,"output_tokens":2,"cached_input_tokens":2}),
                1,
            ),
        ];
        assert!(
            rows.iter()
                .all(|row| recovery_usage(row.2.clone(), row.3, false).is_none())
        );
        assert_eq!(next_recovery_page(&rows, 2), Some("b".into()));
        assert_eq!(next_recovery_page(&rows[..1], 1), Some("a".into()));
    }

    #[test]
    fn short_or_empty_page_ends_the_sweep_not_the_recovery_service() {
        let rows = vec![(
            "last".into(),
            1,
            json!({"input_tokens":0,"output_tokens":0}),
            1,
        )];
        assert_eq!(next_recovery_page(&rows, 2), None);
        assert_eq!(next_recovery_page(&[], 1), None);
        // A page exactly at the limit needs a further (possibly empty) query.
        assert_eq!(next_recovery_page(&rows, 1), Some("last".into()));
    }

    #[test]
    fn malformed_receipt_does_not_hide_the_next_valid_receipt() {
        let receipts = [
            json!({"input_tokens":-1,"output_tokens":2}),
            json!({"input_tokens":4,"output_tokens":2,"cached_input_tokens":"invalid"}),
            json!({"input_tokens":4,"output_tokens":2,"cached_input_tokens":1}),
        ];
        let valid: Vec<_> = receipts
            .into_iter()
            .filter_map(|value| recovery_usage(value, 1, false))
            .collect();
        assert_eq!(
            valid,
            vec![charging::Usage {
                search: None,
                cache_creation: None,
                image: None,
                upstream_cost_atto_usd: None,
                input: 4,
                cached: 1,
                output: 2
            }]
        );
    }

    #[test]
    fn persisted_subsets_are_validated_again_before_reconciliation() {
        for value in [
            json!({"input_tokens":4,"output_tokens":2,"cached_input_tokens":5}),
            json!({"input_tokens":4,"output_tokens":2,"reasoning_output_tokens":3}),
            json!({"input_tokens":4,"output_tokens":2,"cache_creation_input_tokens":1}),
            json!({"input_tokens":4,"output_tokens":2,"future_counter":1}),
            json!({"input_tokens":u64::MAX,"output_tokens":2}),
        ] {
            assert!(recovery_usage(value, 1, false).is_none());
        }
    }

    #[test]
    fn unknown_totals_stay_held_but_explicit_zero_is_recoverable() {
        for value in [
            json!({}),
            json!({"input_tokens":0}),
            json!({"output_tokens":0}),
        ] {
            assert!(recovery_usage(value, 1, false).is_none());
        }
        assert_eq!(
            recovery_usage(json!({"input_tokens":0,"output_tokens":0}), 1, false),
            Some(charging::Usage::default())
        );
    }

    #[test]
    fn checkpoint_sequence_must_fit_the_persistent_contract() {
        let value = json!({"input_tokens":4,"output_tokens":2});
        for sequence in [-1, 0, 65_537, i64::MAX] {
            assert!(recovery_usage(value.clone(), sequence, false).is_none());
        }
        for sequence in [1, 65_536] {
            assert!(recovery_usage(value.clone(), sequence, false).is_some());
        }
    }

    #[test]
    fn upstream_cost_with_cache_creation_is_recovered_successfully() {
        let value = json!({
            "input_tokens": 4,
            "output_tokens": 2,
            "cache_creation_input_tokens": 1,
            "upstream_cost_atto_usd": 123_456_789_000_000_000_u128
        });
        assert_eq!(
            recovery_usage(value, 1, true),
            Some(charging::Usage {
                search: None,
                cache_creation: None,
                image: None,
                upstream_cost_atto_usd: Some(123_456_789_000_000_000),
                input: 4,
                cached: 0,
                output: 2,
            })
        );
    }

    #[test]
    fn token_pricing_rejects_cache_creation_in_recovery_even_with_reported_cost() {
        let value = json!({
            "input_tokens": 4,
            "output_tokens": 2,
            "cache_creation_input_tokens": 1,
            "upstream_cost_atto_usd": 123_456_789_000_000_000_u128
        });
        assert_eq!(recovery_usage(value, 1, false), None);
    }
}
