//! Actual Core relay, PostgreSQL charging and ledger. Only the supplier is
//! synthetic; these tests do not establish compatibility with a live vendor.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    billing::{
        charging::{Charging, Price},
        transactional_ledger::TransactionalLedger,
    },
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::{self, *},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tower::ServiceExt;

#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
use model_fixture::{Provider, fixture};

fn tariff() -> TieredTariff {
    let rate = |numerator, denominator| DecimalRate {
        numerator,
        denominator,
    };
    TieredTariff {
        revision: 1,
        source_digest: "23".repeat(32),
        uses_cache_read: true,
        uses_cache_write: false,
        uses_cache_write_1h: false,
        uses_cache_write_12h: false,
        tiers: vec![
            TokenTier {
                up_to_input_tokens: Some(9),
                rates: TieredRates {
                    input: rate(5, 4),
                    output: rate(10, 1),
                    cache_read: rate(1, 8),
                    cache_write: rate(0, 1),
                    cache_write_1h: None,
                    cache_write_12h: None,
                },
            },
            TokenTier {
                up_to_input_tokens: None,
                rates: TieredRates {
                    input: rate(5, 2),
                    output: rate(15, 1),
                    cache_read: rate(1, 4),
                    cache_write: rate(0, 1),
                    cache_write_1h: None,
                    cache_write_12h: None,
                },
            },
        ],
    }
}

fn cache_12h_tariff() -> TieredTariff {
    let rate = |numerator, denominator| DecimalRate {
        numerator,
        denominator,
    };
    TieredTariff {
        revision: 4,
        source_digest: "45".repeat(32),
        uses_cache_read: true,
        uses_cache_write: true,
        uses_cache_write_1h: false,
        uses_cache_write_12h: true,
        tiers: vec![TokenTier {
            up_to_input_tokens: None,
            rates: TieredRates {
                input: rate(2, 1),
                output: rate(10, 1),
                cache_read: rate(1, 5),
                cache_write: rate(5, 2),
                cache_write_1h: None,
                cache_write_12h: Some(rate(5, 1)),
            },
        }],
    }
}

struct CacheProvider {
    endpoint: String,
    hits: Arc<std::sync::atomic::AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl CacheProvider {
    async fn start(with_ttl: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/custom/v1", listener.local_addr().unwrap());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_hits = hits.clone();
        let app = axum::Router::new().route(
            "/custom/v1/chat/completions",
            axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(request): axum::Json<Value>| {
                let hits = observed_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(headers["authorization"], "Bearer synthetic-upstream-key");
                    assert_eq!(request["model"], "provider-model");
                    let mut usage = json!({
                        "prompt_tokens":100,"completion_tokens":5,"total_tokens":105,
                        "prompt_tokens_details":{"cached_tokens":20,"cache_write_tokens":60}
                    });
                    if with_ttl {
                        usage["prompt_tokens_details"]["cache_write_tokens_details"] =
                            json!({"ttl_30m_tokens":20,"ttl_12h_tokens":40});
                    }
                    axum::Json(json!({
                        "id":"synthetic-cache-completion","object":"chat.completion",
                        "model":"provider-model","created":0,
                        "choices":[{"index":0,"message":{"role":"assistant","content":"cache answer"},"finish_reason":"stop"}],
                        "usage":usage
                    }))
                }
            }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            endpoint,
            hits,
            task,
        }
    }
}

impl Drop for CacheProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn gateway(
    pool: &PgPool,
    endpoint: &str,
    key_id: i64,
    version: u64,
    price: TieredTariff,
) -> ModelGateway {
    gateway_with_billing(
        pool,
        endpoint,
        key_id,
        version,
        Some(price),
        BillingMode::TieredDecimal,
        1_250_000,
    )
}

fn gateway_with_billing(
    pool: &PgPool,
    endpoint: &str,
    key_id: i64,
    version: u64,
    price: Option<TieredTariff>,
    mode: BillingMode,
    group_multiplier_ppm: u32,
) -> ModelGateway {
    let mut config: RoutingConfig = serde_json::from_value(json!({
        "version":version,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"local-model","group":"allowed","enabled":true,"speeds":["standard"],
          "targets":[{"id":1,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
          "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
    })).unwrap();
    config.upstreams[0].endpoint = Endpoint::local_loopback(endpoint.into()).unwrap();
    let prices = PriceSnapshot::compile(PriceConfig {
        version,
        rules: vec![PriceRule {
            embedding: None,
            search: None,
            billing_mode: mode,
            tiered: price,
            id: 1,
            model: "local-model".into(),
            group: "allowed".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: if mode == BillingMode::Tokens {
                    2_000_000
                } else {
                    0
                },
                output: if mode == BillingMode::Tokens {
                    4_000_000
                } else {
                    0
                },
                cache_read: if mode == BillingMode::Tokens {
                    500_000
                } else {
                    0
                },
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        }],
    })
    .unwrap();
    ModelGateway::new(
        pool.clone(),
        Arc::new(routing::Router::new(config, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-upstream-key".into(),
        }],
        BTreeMap::from([(key_id, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 100,
            lease_seconds: 30,
        },
    )
    .unwrap()
}

async fn invoke(gateway: &ModelGateway, key: &str, id: &str) -> (StatusCode, Value, String) {
    let response = gateway.router().oneshot(Request::builder().method("POST").uri("/v1/chat/completions")
        .header("content-type","application/json").header("authorization",format!("Bearer {key}"))
        .header("idempotency-key",id)
        .body(Body::from(serde_json::to_vec(&json!({"model":"local-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20})).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    // Core namespaces an idempotency key to the authenticated principal. The
    // response header carries the actual durable charging operation ID.
    let operation_id = response
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap(), operation_id)
}

#[sqlx::test(migrations = false)]
async fn tiered_decimal_actual_key_call_freezes_quote_settles_once_and_survives_new_prices(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let provider = Provider::start(true).await;
    let first = gateway(&pool, &provider.endpoint, key_id, 1, tariff());
    let (status, body, operation_id) = invoke(&first, &key, "tiered-fixed-operation").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "local synthetic answer"
    );
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let receipt = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!(
        (receipt.reserved, receipt.settled, receipt.state.as_str()),
        (344, 200, "settled")
    );
    assert_eq!(receipt.price.tiered.as_ref().unwrap().tariff, tariff());
    assert_eq!(receipt.price.cost(receipt.usage).unwrap(), 200);
    let mut changed = tariff();
    changed.tiers[1].rates.output.numerator = 1000;
    let restarted = gateway(&pool, &provider.endpoint, key_id, 2, changed);
    let repeated = invoke(&restarted, &key, "tiered-fixed-operation").await;
    assert_eq!(repeated.0, StatusCode::CONFLICT);
    assert_eq!(repeated.2, operation_id);
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    let restored = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!(restored.price, receipt.price);
    assert_eq!(restored.price.cost(restored.usage).unwrap(), 200);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (800, 0));
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e").fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
}

#[sqlx::test(migrations = false)]
async fn tiered_decimal_missing_authoritative_usage_keeps_original_hold_and_never_replays(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let provider = Provider::start(false).await;
    let service = gateway(&pool, &provider.endpoint, key_id, 1, tariff());
    let (status, _, operation_id) = invoke(&service, &key, "tiered-unknown-usage").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let original = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!((original.reserved, original.settled), (344, 0));
    assert!(!matches!(original.state.as_str(), "released" | "settled"));
    let mut changed = tariff();
    changed.tiers[1].rates.output.numerator = 1;
    let restarted = gateway(&pool, &provider.endpoint, key_id, 2, changed);
    let repeated = invoke(&restarted, &key, "tiered-unknown-usage").await;
    assert_eq!(repeated.0, StatusCode::CONFLICT);
    assert_eq!(repeated.2, operation_id);
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    let after = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!(after.price, original.price);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (656, 344)
    );
}

#[sqlx::test(migrations = false)]
async fn openai_12h_actual_key_settlement_is_idempotent_and_preserves_frozen_evidence(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let provider = CacheProvider::start(true).await;
    let first = gateway(&pool, &provider.endpoint, key_id, 1, cache_12h_tariff());
    let (one, two) = tokio::join!(
        invoke(&first, &key, "cache-12h-concurrent"),
        invoke(&first, &key, "cache-12h-concurrent")
    );
    assert_eq!(one.2, two.2);
    let (success, repeated) = if one.0 == StatusCode::OK {
        (one, two)
    } else {
        (two, one)
    };
    assert_eq!(success.0, StatusCode::OK, "{}", success.1);
    assert_eq!(repeated.0, StatusCode::CONFLICT, "{}", repeated.1);
    assert_eq!(
        success.1["usage"]["prompt_tokens_details"]["cache_write_tokens_details"]["ttl_12h_tokens"],
        40
    );
    let operation_id = success.2;
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let receipt = charging.resolve(&operation_id).await.unwrap().unwrap();
    // Ceiling: ceil((100*5 + 20*10)*1.25/2) = 438.
    // Settlement: (20*2 + 20*.2 + 20*2.5 + 40*5 + 5*10)*1.25/2 = 215.
    assert_eq!(
        (receipt.reserved, receipt.settled, receipt.state.as_str()),
        (438, 215, "settled")
    );
    let creation = receipt.usage.cache_creation.unwrap();
    assert_eq!((creation.total, creation.twelve_hour), (60, Some(40)));
    assert_eq!(receipt.price.cost(receipt.usage).unwrap(), 215);
    assert_eq!(
        receipt.price.tiered.as_ref().unwrap().tariff,
        cache_12h_tariff()
    );
    let mut changed = cache_12h_tariff();
    changed.tiers[0]
        .rates
        .cache_write_12h
        .as_mut()
        .unwrap()
        .numerator = 1000;
    let restarted = gateway(&pool, &provider.endpoint, key_id, 2, changed);
    let after_restart = invoke(&restarted, &key, "cache-12h-concurrent").await;
    assert_eq!(after_restart.0, StatusCode::CONFLICT);
    assert_eq!(after_restart.2, operation_id);
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    let restored = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!(restored.price, receipt.price);
    assert_eq!(restored.usage, receipt.usage);
    assert_eq!(restored.price.cost(restored.usage).unwrap(), 215);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (785, 0));
    let unbalanced: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
}

#[sqlx::test(migrations = false)]
async fn openai_12h_missing_breakdown_retains_hold_after_price_change_without_replay(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let provider = CacheProvider::start(false).await;
    let service = gateway(&pool, &provider.endpoint, key_id, 1, cache_12h_tariff());
    let (status, _, operation_id) = invoke(&service, &key, "cache-12h-unknown").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let original = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!((original.reserved, original.settled), (438, 0));
    assert!(!matches!(original.state.as_str(), "released" | "settled"));
    let mut changed = cache_12h_tariff();
    changed.tiers[0]
        .rates
        .cache_write_12h
        .as_mut()
        .unwrap()
        .numerator = 1;
    let restarted = gateway(&pool, &provider.endpoint, key_id, 2, changed);
    let repeated = invoke(&restarted, &key, "cache-12h-unknown").await;
    assert_eq!(repeated.0, StatusCode::CONFLICT);
    assert_eq!(repeated.2, operation_id);
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    let restored = charging.resolve(&operation_id).await.unwrap().unwrap();
    assert_eq!(restored.price, original.price);
    assert_eq!((restored.reserved, restored.settled), (438, 0));
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (562, 438)
    );
}

#[test]
fn adding_optional_tariff_preserves_old_price_json_and_usage_bytes() {
    let price = r#"{"version":"routing-1-rule-1","model":"local-model","group":"allowed","input_per_million":1000000,"cached_per_million":250000,"output_per_million":2000000}"#;
    let decoded: Price = serde_json::from_str(price).unwrap();
    assert!(decoded.tiered.is_none());
    assert_eq!(serde_json::to_string(&decoded).unwrap(), price);
    let usage = r#"{"input":10,"cached":2,"output":20}"#;
    let decoded: lmm_core::billing::charging::Usage = serde_json::from_str(usage).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), usage);
}

#[sqlx::test(migrations = false)]
async fn explicit_free_token_and_tiered_groups_do_not_require_balance_or_expand_key_authority(
    pool: PgPool,
) {
    let (identity, ledger, session, key, key_id, account) = fixture(&pool, 0).await;
    let provider = Provider::start(true).await;
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let blocked = identity
        .issue_key(
            &session,
            lmm_core::accounts::Account {
                kind: lmm_core::accounts::AccountKind::Personal,
                id: 7,
            },
            lmm_core::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    for (index, mode) in [BillingMode::Tokens, BillingMode::TieredDecimal]
        .into_iter()
        .enumerate()
    {
        let service = gateway_with_billing(
            &pool,
            &provider.endpoint,
            key_id,
            1,
            (mode == BillingMode::TieredDecimal).then(tariff),
            mode,
            0,
        );
        let id = format!("explicit-free-{index}");
        let (status, body, operation_id) = invoke(&service, &key, &id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let receipt = charging.resolve(&operation_id).await.unwrap().unwrap();
        assert_eq!(
            (receipt.state.as_str(), receipt.reserved, receipt.settled),
            ("settled", 0, 0)
        );
        assert_eq!(receipt.price.cost(receipt.usage).unwrap(), 0);
        let repeated = invoke(&service, &key, &id).await;
        assert_eq!(repeated.0, StatusCode::CONFLICT);
        assert_eq!(repeated.2, operation_id);
        // This valid key lacks this host's routing-group grant. A zero price
        // must not supply that missing authority or contact the provider.
        let response=service.router().oneshot(Request::builder().method("POST").uri("/v1/chat/completions")
            .header("content-type","application/json").header("authorization",format!("Bearer {}",blocked.secret))
            .body(Body::from(serde_json::to_vec(&json!({"model":"local-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20})).unwrap())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let balance = ledger.balance(account).await.unwrap();
        assert_eq!((balance.available_units, balance.reserved_units), (0, 0));
        assert_eq!(provider.hits.load(Ordering::SeqCst), index + 1);
    }
}
