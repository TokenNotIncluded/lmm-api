//! Actual Core HTTP admission, PostgreSQL charging, checkpoints and ledger.
//! The bounded loopback suppliers below are synthetic protocol fixtures; they
//! are not live-provider acceptance or evidence for a production spending cap.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use lmm_core::{
    billing::{charging::Charging, transactional_ledger::TransactionalLedger},
    model_http::{
        ChargePolicy, ExternalModelContext, ExternalSearchContract, ModelGateway, ProviderPolicy,
        UpstreamCredential,
    },
    relay::Limits,
    routing::{self, *},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
use model_fixture::fixture;

const INPUT_LIMIT: u32 = 100;
const OUTPUT_LIMIT: u32 = 20;
const FULL_SEARCH_HOLD: i64 = 1640; // 100 input + 20*2 output + 3*500 search credits.

struct SearchSupplier {
    endpoint: String,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for SearchSupplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl SearchSupplier {
    async fn start(provider: SearchProvider, known: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/custom/v1", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let handler = move |uri: Uri, headers: HeaderMap, Json(request): Json<Value>| {
            let hits = observed.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let stream = if provider == SearchProvider::OpenAiResponsesV1 {
                    assert_eq!(headers["authorization"], "Bearer synthetic-upstream-key");
                    assert_eq!(request["model"], "provider-model");
                    assert_eq!(request["tools"][0]["type"], "web_search");
                    assert_eq!(request["max_tool_calls"], 3);
                    assert_eq!(request["max_output_tokens"], OUTPUT_LIMIT);
                    request["stream"] == true
                } else {
                    assert_eq!(headers["x-goog-api-key"], "synthetic-upstream-key");
                    assert_eq!(request["tools"][0]["googleSearch"], json!({}));
                    assert_eq!(request["generationConfig"]["maxOutputTokens"], OUTPUT_LIMIT);
                    uri.path().ends_with(":streamGenerateContent")
                };
                supplier_response(provider, stream, known)
            }
        };
        let app = Router::new()
            .route("/custom/v1/responses", post(handler.clone()))
            .route("/custom/v1/models/{action}", post(handler));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            hits,
            task,
        }
    }
}
fn sse(values: Vec<Value>) -> Response {
    let body: String = values
        .into_iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect();
    ([("content-type", "text/event-stream")], Body::from(body)).into_response()
}
fn supplier_response(provider: SearchProvider, stream: bool, known: bool) -> Response {
    if provider == SearchProvider::OpenAiResponsesV1 {
        let mut search = json!({"id":"search_1","type":"web_search_call","status":"completed"});
        if known {
            search["action"] = json!({"type":"search","query":"bounded synthetic source"});
        }
        let text = json!({"type":"output_text","text":"answer","annotations":[{"type":"url_citation","url":"https://example.test/source","title":"Source","start_index":0,"end_index":6}]});
        let response = json!({"id":"resp_search","status":"completed","output":[search,{"type":"message","role":"assistant","content":[text]}],"usage":{
            "input_tokens":10,"output_tokens":3,"input_tokens_details":{"cached_tokens":0},"cache_creation_input_tokens":0,"output_tokens_details":{"reasoning_tokens":0}
        }});
        if !stream {
            return Json(response).into_response();
        }
        sse(vec![
            json!({"type":"response.created","response":{"id":"resp_search","status":"in_progress"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"search_1","type":"web_search_call","status":"in_progress"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":search}),
            json!({"type":"response.output_item.done","output_index":0,"item":search}),
            json!({"type":"response.content_part.added","output_index":1,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"answer"}),
            json!({"type":"response.content_part.done","output_index":1,"content_index":0,"part":text}),
            json!({"type":"response.completed","response":response}),
        ])
    } else {
        let mut candidate = json!({"index":0,"content":{"role":"model","parts":[{"text":"answer"}]},"finishReason":"STOP"});
        if known {
            candidate["groundingMetadata"] = json!({
                "webSearchQueries":["alpha"," alpha ","beta"],
                "groundingChunks":[{"web":{"uri":"https://example.test/source","title":"Source"}}],
                "groundingSupports":[{"segment":{"text":"answer"},"groundingChunkIndices":[0]}]
            });
        }
        let usage = json!({"promptTokenCount":10,"candidatesTokenCount":3,"thoughtsTokenCount":0,"cachedContentTokenCount":0,"totalTokenCount":13});
        if !stream {
            return Json(json!({"responseId":"gemini_search","candidates":[candidate],"usageMetadata":usage})).into_response();
        }
        candidate.as_object_mut().unwrap().remove("finishReason");
        sse(vec![
            json!({"responseId":"gemini_search","candidates":[candidate]}),
            json!({"responseId":"gemini_search","candidates":[{"index":0,"finishReason":"STOP"}],"usageMetadata":usage}),
        ])
    }
}
fn protocol(provider: SearchProvider) -> Protocol {
    match provider {
        SearchProvider::OpenAiResponsesV1 => Protocol::OpenAiResponses,
        SearchProvider::GeminiGroundingV1 => Protocol::Gemini,
        _ => unreachable!("this integration fixture exercises the two native primary paths"),
    }
}
fn search_limits(unit: SearchBillingUnit) -> SearchLimits {
    match unit {
        SearchBillingUnit::Call => SearchLimits {
            calls: Some(3),
            ..SearchLimits::default()
        },
        SearchBillingUnit::Query => SearchLimits {
            queries: Some(3),
            ..SearchLimits::default()
        },
        SearchBillingUnit::GroundedRequest => SearchLimits {
            grounded_requests: Some(1),
            ..SearchLimits::default()
        },
    }
}
fn gateway(
    pool: &PgPool,
    supplier: &SearchSupplier,
    key_id: i64,
    provider: SearchProvider,
    unit: SearchBillingUnit,
    version: u64,
    fee_numerator: u64,
) -> ModelGateway {
    let protocol = protocol(provider);
    let mut config: RoutingConfig = serde_json::from_value(json!({
        "version":version,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":protocol,"credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"local-model","group":"allowed","enabled":true,"speeds":["standard"],
            "targets":[{"id":1,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
            "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
    })).unwrap();
    config.upstreams[0].endpoint = Endpoint::local_loopback(supplier.endpoint.clone()).unwrap();
    let tariff = SearchTariff {
        revision: 1,
        provider,
        unit,
        currency: SearchCurrency::Usd,
        usd_per_unit: SearchRate {
            numerator: fee_numerator,
            denominator: 1000,
        },
        x_posts_fetched_usd_per_unit: None,
        x_users_fetched_usd_per_unit: None,
    };
    let prices = PriceSnapshot::compile(PriceConfig {
        version,
        rules: vec![PriceRule {
            embedding: None,
            billing_mode: BillingMode::Tokens,
            tiered: None,
            search: Some(tariff),
            id: 1,
            model: "local-model".into(),
            group: "allowed".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: 2_000_000,
                output: 4_000_000,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm: MULTIPLIER_ONE,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        }],
    })
    .unwrap();
    let policy = ProviderPolicy::new(
        vec![ExternalModelContext {
            deepseek_parameters_contract: None,
            embedding: None,
            upstream_id: 1,
            upstream_model: "provider-model".into(),
            protocol,
            endpoint: supplier.endpoint.clone(),
            max_input_tokens: INPUT_LIMIT,
            max_output_tokens: OUTPUT_LIMIT,
            contract_revision: "synthetic-exact-token-bound-v1".into(),
            missing_cached_tokens_is_zero: false,
            missing_cache_creation_tokens_is_zero: true,
            search: Some(ExternalSearchContract {
                provider,
                limits: search_limits(unit),
                contract_revision: "synthetic-exact-search-bound-v1".into(),
            }),
        }],
        i64::from(INPUT_LIMIT),
    )
    .unwrap();
    ModelGateway::new_with_provider_policy(
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
            input_token_ceiling: i64::from(INPUT_LIMIT),
            lease_seconds: 30,
        },
        policy,
    )
    .unwrap()
    .with_local_streaming()
}
async fn begin(
    gateway: &ModelGateway,
    key: &str,
    id: &str,
    provider: SearchProvider,
    stream: bool,
) -> Response {
    let (path, value) = if provider == SearchProvider::OpenAiResponsesV1 {
        (
            "/v1/responses",
            json!({"model":"local-model","input":"find source","tools":[{"type":"web_search"}],"max_output_tokens":OUTPUT_LIMIT,"stream":stream}),
        )
    } else {
        (
            if stream {
                "/v1beta/models/local-model:streamGenerateContent?alt=sse"
            } else {
                "/v1beta/models/local-model:generateContent"
            },
            json!({"contents":[{"parts":[{"text":"find source"}]}],"tools":[{"googleSearch":{}}],"generationConfig":{"maxOutputTokens":OUTPUT_LIMIT}}),
        )
    };
    gateway
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {key}"))
                .header("idempotency-key", id)
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn invoke(
    gateway: &ModelGateway,
    key: &str,
    id: &str,
    provider: SearchProvider,
    stream: bool,
) -> (StatusCode, String, String) {
    let response = begin(gateway, key, id, provider, stream).await;
    let status = response.status();
    let operation_id = response
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        String::from_utf8(body.to_vec()).unwrap(),
        operation_id,
    )
}
async fn balanced(pool: &PgPool) {
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e").fetch_one(pool).await.unwrap();
    assert_eq!(unbalanced, 0);
}
fn charging(pool: &PgPool) -> Charging {
    Charging::new(pool.clone(), Arc::new(TransactionalLedger))
}

#[sqlx::test(migrations = false)]
async fn native_json_and_sse_search_settle_tokens_and_one_selected_search_basis(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10_000).await;
    let mut total = 0;
    for (provider, unit, expected, expected_hold) in [
        (
            SearchProvider::OpenAiResponsesV1,
            SearchBillingUnit::Call,
            516,
            FULL_SEARCH_HOLD,
        ),
        (
            SearchProvider::GeminiGroundingV1,
            SearchBillingUnit::Query,
            1016,
            FULL_SEARCH_HOLD,
        ),
        (
            SearchProvider::GeminiGroundingV1,
            SearchBillingUnit::GroundedRequest,
            516,
            640,
        ),
    ] {
        let supplier = SearchSupplier::start(provider, true).await;
        let service = gateway(&pool, &supplier, key_id, provider, unit, 1, 1);
        for stream in [false, true] {
            let id = format!("search-{provider:?}-{unit:?}-{stream}");
            let (status, body, operation_id) = invoke(&service, &key, &id, provider, stream).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(body.contains("answer"), "{body}");
            assert!(!body.contains("settlement_pending"), "{body}");
            assert!(!body.contains("invalid_upstream_response"), "{body}");
            if stream && provider == SearchProvider::OpenAiResponsesV1 {
                assert!(body.contains("response.completed"));
            }
            let receipt = charging(&pool)
                .resolve(&operation_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (receipt.reserved, receipt.settled, receipt.state.as_str()),
                (expected_hold, expected, "settled")
            );
            let search = receipt.usage.search.unwrap();
            assert_eq!(search.provider, provider);
            if provider == SearchProvider::OpenAiResponsesV1 {
                assert_eq!(search.calls, Some(1));
            } else {
                assert_eq!(
                    (search.queries, search.grounded_requests),
                    (Some(2), Some(1))
                );
            }
            assert_eq!(receipt.price.search.as_ref().unwrap().tariff.unit, unit);
            assert_eq!(receipt.price.cost(receipt.usage).unwrap(), expected);
            if stream {
                let terminal:Value=sqlx::query_scalar("SELECT provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed ORDER BY sequence DESC LIMIT 1").bind(&operation_id).fetch_one(&pool).await.unwrap();
                assert_eq!(terminal["search"], serde_json::to_value(search).unwrap());
            }
            total += expected;
        }
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 2);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (10_000 - total, 0)
    );
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn missing_search_evidence_retains_hold_across_price_changes_without_replay(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10_000).await;
    for (provider, unit) in [
        (SearchProvider::OpenAiResponsesV1, SearchBillingUnit::Call),
        (SearchProvider::GeminiGroundingV1, SearchBillingUnit::Query),
    ] {
        let supplier = SearchSupplier::start(provider, false).await;
        let service = gateway(&pool, &supplier, key_id, provider, unit, 1, 1);
        for stream in [false, true] {
            let id = format!("search-unknown-{provider:?}-{stream}");
            let (status, body, operation_id) = invoke(&service, &key, &id, provider, stream).await;
            assert_eq!(
                status,
                if stream {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                },
                "{body}"
            );
            if stream {
                assert!(!body.contains("response.completed"));
            }
            let original = charging(&pool)
                .resolve(&operation_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (original.reserved, original.settled, original.state.as_str()),
                (FULL_SEARCH_HOLD, 0, "reconcile")
            );
            let restarted = gateway(&pool, &supplier, key_id, provider, unit, 2, 99);
            assert_eq!(restarted.reconcile_completed_streams(20).await.unwrap(), 0);
            let repeated = invoke(&restarted, &key, &id, provider, stream).await;
            assert_eq!(repeated.0, StatusCode::CONFLICT, "{}", repeated.1);
            assert_eq!(repeated.2, operation_id);
            let restored = charging(&pool)
                .resolve(&operation_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(restored.price, original.price);
            assert_eq!((restored.reserved, restored.settled), (FULL_SEARCH_HOLD, 0));
        }
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 2);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (10_000 - 4 * FULL_SEARCH_HOLD, 4 * FULL_SEARCH_HOLD)
    );
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn duplicate_requests_and_discarded_client_terminal_ack_do_not_repeat_search_or_money(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10_000).await;
    let provider = SearchProvider::OpenAiResponsesV1;
    let supplier = SearchSupplier::start(provider, true).await;
    let service = gateway(
        &pool,
        &supplier,
        key_id,
        provider,
        SearchBillingUnit::Call,
        1,
        1,
    );
    let (one, two) = tokio::join!(
        invoke(&service, &key, "search-concurrent", provider, false),
        invoke(&service, &key, "search-concurrent", provider, false)
    );
    assert_eq!(one.2, two.2);
    assert!(
        (one.0 == StatusCode::OK && two.0 == StatusCode::CONFLICT)
            || (two.0 == StatusCode::OK && one.0 == StatusCode::CONFLICT)
    );
    let response = begin(&service, &key, "search-discard-ack", provider, false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let operation_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    // Non-stream success headers are released only after durable settlement.
    // Discard the unread terminal acknowledgement, then reconstruct the host.
    drop(response);
    let restarted = gateway(
        &pool,
        &supplier,
        key_id,
        provider,
        SearchBillingUnit::Call,
        2,
        99,
    );
    let repeated = invoke(&restarted, &key, "search-discard-ack", provider, false).await;
    assert_eq!(repeated.0, StatusCode::CONFLICT);
    assert_eq!(repeated.2, operation_id);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 2);
    let receipt = charging(&pool)
        .resolve(&operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((receipt.settled, receipt.state.as_str()), (516, "settled"));
    assert_eq!(
        receipt.price.search.unwrap().tariff.usd_per_unit.numerator,
        1
    );
    assert_eq!(
        (
            ledger.balance(account).await.unwrap().available_units,
            ledger.balance(account).await.unwrap().reserved_units
        ),
        (8968, 0)
    );
    let finals: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.usage_events WHERE event_id='final'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(finals, 2);
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn terminal_search_checkpoints_recover_after_final_write_failure_with_frozen_prices(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10_000).await;
    sqlx::raw_sql("CREATE FUNCTION public.reject_search_final() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id='final' THEN RAISE EXCEPTION 'synthetic search final commit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_search_final BEFORE INSERT ON core_charging.usage_events FOR EACH ROW EXECUTE FUNCTION public.reject_search_final()")
        .execute(&pool).await.unwrap();
    let mut pending = Vec::new();
    for (provider, unit, expected) in [
        (
            SearchProvider::OpenAiResponsesV1,
            SearchBillingUnit::Call,
            516,
        ),
        (
            SearchProvider::GeminiGroundingV1,
            SearchBillingUnit::Query,
            1016,
        ),
    ] {
        let supplier = SearchSupplier::start(provider, true).await;
        let service = gateway(&pool, &supplier, key_id, provider, unit, 1, 1);
        let id = format!("search-recover-{provider:?}");
        let (status, body, operation_id) = invoke(&service, &key, &id, provider, true).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("answer"), "{body}");
        assert!(!body.contains("response.completed"), "{body}");
        let original = charging(&pool)
            .resolve(&operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (original.reserved, original.settled, original.state.as_str()),
            (FULL_SEARCH_HOLD, 0, "reconcile")
        );
        let terminal:bool=sqlx::query_scalar("SELECT bool_or(terminal_observed) FROM core_model.output_checkpoints WHERE request_id=$1").bind(&operation_id).fetch_one(&pool).await.unwrap();
        assert!(terminal);
        pending.push((
            supplier,
            provider,
            unit,
            expected,
            id,
            operation_id,
            original.price,
        ));
    }
    sqlx::query("DROP TRIGGER reject_search_final ON core_charging.usage_events")
        .execute(&pool)
        .await
        .unwrap();
    for (supplier, provider, unit, expected, id, operation_id, price) in pending {
        let restarted = gateway(&pool, &supplier, key_id, provider, unit, 2, 99);
        let (one, two) = tokio::join!(
            restarted.reconcile_completed_streams(20),
            restarted.reconcile_completed_streams(20)
        );
        let _ = one.unwrap() + two.unwrap();
        let restored = charging(&pool)
            .resolve(&operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (restored.settled, restored.state.as_str()),
            (expected, "settled")
        );
        assert_eq!(restored.price, price);
        assert_eq!(restored.price.cost(restored.usage).unwrap(), expected);
        assert_eq!(restarted.reconcile_completed_streams(20).await.unwrap(), 0);
        let repeated = invoke(&restarted, &key, &id, provider, true).await;
        assert_eq!(repeated.0, StatusCode::CONFLICT);
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'").bind(&operation_id).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (8468, 0));
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn insufficient_combined_token_and_search_reserve_stops_before_supplier_bytes(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, FULL_SEARCH_HOLD - 1).await;
    let provider = SearchProvider::OpenAiResponsesV1;
    let supplier = SearchSupplier::start(provider, true).await;
    let service = gateway(
        &pool,
        &supplier,
        key_id,
        provider,
        SearchBillingUnit::Call,
        1,
        1,
    );
    let response = begin(&service, &key, "search-no-funds", provider, false).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("insufficient_funds"));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FULL_SEARCH_HOLD - 1, 0)
    );
    balanced(&pool).await;
}
