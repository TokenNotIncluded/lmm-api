//! Synthetic loopback receipts exercise the Rust gateway, PostgreSQL and real
//! ledger together. They do not enable or certify live OpenRouter forwarding.
use super::*;
use axum::extract::State;
use std::sync::atomic::AtomicUsize;

const MODEL: &str = "openrouter/fusion";
const COST: &str = "0.123456789";

async fn cost_provider(
    State(hits): State<Arc<AtomicUsize>>,
    Json(request): Json<Value>,
) -> Response {
    hits.fetch_add(1, Ordering::SeqCst);
    assert!(lmm_core::upstream_cost::supported_router(
        request["model"].as_str().unwrap()
    ));
    let prompt = request["messages"][0]["content"].as_str().unwrap();
    let cost = match prompt {
        "missing" | "early-zero" => None,
        "invalid" => Some("-1"),
        "zero" => Some("0"),
        "topup" => Some("2"),
        _ => Some(COST),
    };
    let extra = cost
        .map(|v| format!(",\"cost\":{v},\"cost_details\":{{\"upstream_inference_cost\":19}}"))
        .unwrap_or_default();
    let usage = format!(r#"{{"prompt_tokens":0,"completion_tokens":0{extra}}}"#);
    if request["stream"] == true {
        let body = format!(
            "data: {{\"id\":\"synthetic-router\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"router answer\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"synthetic-router\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: {{\"choices\":[],\"usage\":{usage}}}\n\ndata: {{\"choices\":[],\"usage\":{usage}}}\n\ndata: [DONE]\n\n"
        );
        let body = if prompt == "early-zero" {
            format!(
                "data: {}\n\n{body}",
                json!({"id":"synthetic-router","choices":[],"usage":{"cost":0}})
            )
        } else {
            body
        };
        return ([("content-type", "text/event-stream")], body).into_response();
    }
    let body = format!(
        r#"{{"id":"synthetic-router","object":"chat.completion","model":"chosen-model","choices":[{{"index":0,"message":{{"role":"assistant","content":"router answer"}},"finish_reason":"stop"}}],"usage":{usage}}}"#
    );
    ([("content-type", "application/json")], body).into_response()
}

async fn upstream() -> (String, JoinHandle<()>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/chat/completions", post(cost_provider))
        .with_state(hits.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (endpoint, task, hits)
}

fn cost_routes(endpoint: &str, model: &str) -> Arc<lmm_core::routing::Router> {
    let price = PriceSnapshot::compile(PriceConfig {
        version: 1,
        rules: vec![PriceRule {
            embedding: None,
            search: None,
            billing_mode: BillingMode::UpstreamCost,
            tiered: None,
            id: 1,
            model: model.into(),
            group: "allowed".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm: 1_250_000,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        }],
    })
    .unwrap();
    Arc::new(
        lmm_core::routing::Router::new(
            RoutingConfig {
                version: 1,
                groups: vec![ModelGroup {
                    name: "allowed".into(),
                    enabled: true,
                }],
                upstreams: vec![UpstreamConfig {
                    id: 1,
                    endpoint: Endpoint::local_loopback(endpoint.into()).unwrap(),
                    protocol: Protocol::OpenAiChat,
                    credential: CredentialRef { id: 7, revision: 1 },
                    enabled: true,
                }],
                routes: vec![ModelRoute {
                    id: 1,
                    model: model.into(),
                    group: "allowed".into(),
                    enabled: true,
                    speeds: vec![Speed::Standard],
                    targets: vec![RouteTarget {
                        id: 1,
                        upstream_id: 1,
                        upstream_model: model.into(),
                        enabled: true,
                        priority: 0,
                        weight: 1,
                        speeds: vec![Speed::Standard],
                        payload: PayloadPolicy::Adapt,
                        timeout_ms: 5_000,
                    }],
                    failover: FailoverPolicy {
                        max_attempts: 1,
                        retry_on: vec![],
                        allow_replay_after_send: false,
                    },
                }],
                aliases: vec![],
            },
            Arc::new(price),
        )
        .unwrap(),
    )
}

async fn request_cost(
    gateway: &ModelGateway,
    key: &str,
    model: &str,
    prompt: &str,
    stream: bool,
    id: &str,
) -> Response {
    gateway.router().oneshot(Request::builder().method("POST").uri("/v1/chat/completions").header("authorization", format!("Bearer {key}")).header("content-type", "application/json").header("idempotency-key", id)
        .body(Body::from(serde_json::to_vec(&json!({"model":model,"messages":[{"role":"user","content":prompt}],"max_tokens":20,"stream":stream,"stream_options":{"include_usage":true}})).unwrap())).unwrap()).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn upstream_cost_http_and_repeated_sse_bill_all_four_routers_once(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 3_000_000).await;
    let (endpoint, task, _hits) = upstream().await;
    for (index, model) in [
        "nvidia/switchyard",
        "openrouter/auto-beta",
        MODEL,
        "typesafe/jev-router",
    ]
    .into_iter()
    .enumerate()
    {
        let gateway = gateway_with_routes(&pool, cost_routes(&endpoint, model), key_id)
            .with_local_streaming();
        for stream in [false, true] {
            let response = request_cost(
                &gateway,
                &key,
                model,
                "hello",
                stream,
                &format!("router-{index}-{stream}"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let text = String::from_utf8(
                to_bytes(response.into_body(), 1_000_000)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert!(text.contains("router answer"));
            if stream {
                assert!(text.contains("[DONE]"), "{text}");
            }
        }
    }
    let amounts: Vec<i64> =
        sqlx::query_scalar("SELECT settled FROM core_charging.requests ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(amounts, vec![77_161; 8]);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (3_000_000 - 8 * 77_161, 0)
    );
    balanced(&pool).await;
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn upstream_cost_missing_invalid_and_zero_are_distinct_and_topup_is_authorized(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 5_000_000).await;
    let (endpoint, task, _hits) = upstream().await;
    let gateway = gateway_with_routes(&pool, cost_routes(&endpoint, MODEL), key_id);
    for (prompt, stream) in [
        ("missing", false),
        ("invalid", false),
        ("missing", true),
        ("invalid", true),
        ("early-zero", true),
    ] {
        let streaming = gateway.clone().with_local_streaming();
        let response = request_cost(
            &streaming,
            &key,
            MODEL,
            prompt,
            stream,
            &format!("{prompt}-{stream}"),
        )
        .await;
        if !stream {
            assert!(!response.status().is_success());
        }
        let text = String::from_utf8(
            to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        if stream {
            assert!(!text.contains("[DONE]"));
        }
    }
    assert_eq!(
        ledger.balance(account).await.unwrap().reserved_units,
        3_125_000
    );
    let response = request_cost(&gateway, &key, MODEL, "zero", false, "zero").await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert_eq!(
        ledger.balance(account).await.unwrap().reserved_units,
        3_125_000
    );
    let response = request_cost(&gateway, &key, MODEL, "topup", false, "topup").await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (625_000, 3_125_000)
    );
    balanced(&pool).await;
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn upstream_cost_completed_stream_recovers_exact_receipt_after_failed_final_commit(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 3_000_000).await;
    sqlx::raw_sql("CREATE FUNCTION public.reject_router_final() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id='final' THEN RAISE EXCEPTION 'synthetic final commit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_router_final BEFORE INSERT ON core_charging.usage_events FOR EACH ROW EXECUTE FUNCTION public.reject_router_final()")
        .execute(&pool).await.unwrap();
    let (endpoint, task, hits) = upstream().await;
    let running =
        gateway_with_routes(&pool, cost_routes(&endpoint, MODEL), key_id).with_local_streaming();
    let response = request_cost(&running, &key, MODEL, "hello", true, "router-recovery").await;
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let text = String::from_utf8(
        to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("router answer"));
    assert!(!text.contains("[DONE]"));
    assert_eq!(
        ledger.balance(account).await.unwrap().reserved_units,
        625_000
    );
    assert_unknown_hold(&pool, &request_id, 625_000).await;
    let terminal_usage: Vec<Value> = sqlx::query_scalar(
        "SELECT provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed ORDER BY sequence",
    )
    .bind(&request_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(!terminal_usage.is_empty());
    for usage in terminal_usage {
        assert_eq!(
            usage["upstream_cost_atto_usd"],
            json!(123_456_789_000_000_000_u128)
        );
    }
    let provider_usage: Value = sqlx::query_scalar(
        "SELECT detail->'usage' FROM core_charging.usage_events WHERE request_id=$1 AND event_id='usage:provider-final'",
    )
    .bind(&request_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        provider_usage,
        json!({"input":0,"cached":0,"output":0,"upstream_cost_atto_usd":123_456_789_000_000_000_u128})
    );
    sqlx::query("DROP TRIGGER reject_router_final ON core_charging.usage_events")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let expired = sqlx::query(
        "UPDATE core_charging.requests SET lease_until=0,revision=revision+1 WHERE state='streaming' AND id=$1",
    )
    .bind(&request_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(expired.rows_affected(), 0);
    tx.commit().await.unwrap();
    drop(running);
    let restarted =
        gateway_with_routes(&pool, cost_routes(&endpoint, MODEL), key_id).with_local_streaming();
    assert_eq!(restarted.recover_expired(10).await.unwrap(), 0);
    assert_unknown_hold(&pool, &request_id, 625_000).await;
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 1);
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 0);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (3_000_000 - 77_161, 0)
    );
    let final_usage: Value = sqlx::query_scalar(
        "SELECT detail->'usage' FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'",
    )
    .bind(&request_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(final_usage, provider_usage);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let duplicate = request_cost(&restarted, &key, MODEL, "hello", true, "router-recovery").await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    balanced(&pool).await;
    task.abort();
}
