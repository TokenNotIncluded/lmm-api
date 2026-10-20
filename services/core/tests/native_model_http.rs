//! Native client routes against counting TCP providers and real Core funds.
//! All credentials and data are synthetic; this never calls an external API.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use lmm_core::{
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::*,
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
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;
#[path = "common/model_fixture.rs"]
mod model_fixture;
use model_fixture::fixture;

#[derive(Clone)]
struct Supplier {
    protocol: Protocol,
    hits: Arc<AtomicUsize>,
    missing_usage: bool,
}
async fn supplier(
    State(state): State<Supplier>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    let key = if state.protocol == Protocol::AnthropicMessages {
        "x-api-key"
    } else {
        "x-goog-api-key"
    };
    assert_eq!(headers[key], "synthetic-upstream-key");
    assert!(!headers.contains_key("authorization"));
    if state.protocol == Protocol::AnthropicMessages {
        assert_eq!(request["model"], "provider-model");
        let mut usage = json!({"input_tokens":8,"cache_read_input_tokens":2,"cache_creation_input_tokens":0,"output_tokens":20});
        if state.missing_usage {
            usage = Value::Null;
        }
        if request["stream"] == true {
            let events = [
                json!({"type":"message_start","message":{"id":"synthetic-message","usage":{"input_tokens":8,"cache_read_input_tokens":2,"cache_creation_input_tokens":0,"output_tokens":0}}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"native answer"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":20}}),
                json!({"type":"message_stop"}),
            ];
            let body = events
                .iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect::<String>();
            return ([("content-type", "text/event-stream")], body).into_response();
        }
        return Json(json!({"id":"synthetic-message","type":"message","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","usage":usage})).into_response();
    }
    let usage = if state.missing_usage {
        Value::Null
    } else {
        json!({"promptTokenCount":10,"cachedContentTokenCount":2,"candidatesTokenCount":20,"thoughtsTokenCount":0,"totalTokenCount":30})
    };
    let value = json!({"candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"native answer"}]},"finishReason":"STOP"}],"usageMetadata":usage});
    // Gemini streaming is selected only by its configured endpoint.
    Json(value).into_response()
}
async fn gemini_stream(
    State(state): State<Supplier>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let response = supplier(State(state), headers, Json(request)).await;
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    (
        [("content-type", "text/event-stream")],
        format!("data: {}\n\n", String::from_utf8(body.to_vec()).unwrap()),
    )
        .into_response()
}
struct Server {
    endpoint: String,
    hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(protocol: Protocol, missing_usage: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/messages", post(supplier))
        .route("/v1/models/provider-model:generateContent", post(supplier))
        .route(
            "/v1/models/provider-model:streamGenerateContent",
            post(gemini_stream),
        )
        .with_state(Supplier {
            protocol,
            hits: hits.clone(),
            missing_usage,
        });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        endpoint,
        hits,
        task,
    }
}
fn gateway(pool: &PgPool, server: &Server, protocol: Protocol, key_id: i64) -> ModelGateway {
    let prices=PriceSnapshot::compile(serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"native-model","group":"allowed","speed":"standard","rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap()).unwrap();
    let routes = RoutingConfig {
        version: 1,
        groups: vec![ModelGroup {
            name: "allowed".into(),
            enabled: true,
        }],
        upstreams: vec![UpstreamConfig {
            id: 1,
            endpoint: Endpoint::local_loopback(server.endpoint.clone()).unwrap(),
            protocol,
            credential: CredentialRef { id: 7, revision: 1 },
            enabled: true,
        }],
        routes: vec![ModelRoute {
            id: 1,
            model: "native-model".into(),
            group: "allowed".into(),
            enabled: true,
            speeds: vec![Speed::Standard],
            targets: vec![RouteTarget {
                id: 1,
                upstream_id: 1,
                upstream_model: "provider-model".into(),
                enabled: true,
                priority: 0,
                weight: 1,
                speeds: vec![Speed::Standard],
                payload: PayloadPolicy::Adapt,
                timeout_ms: 5000,
            }],
            failover: FailoverPolicy::default(),
        }],
        aliases: vec![],
    };
    ModelGateway::new(
        pool.clone(),
        Arc::new(lmm_core::routing::Router::new(routes, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-upstream-key".into(),
        }],
        BTreeMap::from([(key_id, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: 20,
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 100,
            lease_seconds: 30,
        },
    )
    .unwrap()
    .with_local_streaming()
}
fn input(protocol: Protocol, stream: bool) -> (&'static str, Value, &'static str) {
    if protocol == Protocol::AnthropicMessages {
        (
            "/v1/messages",
            json!({"model":"native-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20,"stream":stream}),
            "x-api-key",
        )
    } else {
        (
            if stream {
                "/v1beta/models/native-model:streamGenerateContent?alt=sse"
            } else {
                "/v1beta/models/native-model:generateContent"
            },
            json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}],"generationConfig":{"maxOutputTokens":20}}),
            "x-goog-api-key",
        )
    }
}
#[sqlx::test(migrations = false)]
async fn native_anthropic_and_gemini_settle_real_funds_and_stream_checkpoints(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    for protocol in [Protocol::AnthropicMessages, Protocol::Gemini] {
        let server = server(protocol, false).await;
        let gateway = gateway(&pool, &server, protocol, key_id);
        for stream in [false, true] {
            let (path, body, header) = input(protocol, stream);
            let response = gateway
                .router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header, &key)
                        .header("content-type", "application/json")
                        .header("idempotency-key", format!("native-{protocol:?}-{stream}"))
                        .body(Body::from(serde_json::to_vec(&body).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let body = String::from_utf8(
                to_bytes(response.into_body(), 1_000_000)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert!(body.contains("native answer"), "{body}");
            if protocol == Protocol::AnthropicMessages {
                assert!(body.contains(if stream { "message_stop" } else { "end_turn" }));
            } else {
                assert!(body.contains("STOP"));
            }
            let charge:(String,i64,i64,i64)=sqlx::query_as("SELECT state,settled,input_tokens,output_tokens FROM core_charging.requests WHERE id=$1").bind(&id).fetch_one(&pool).await.unwrap();
            assert_eq!(charge, ("settled".into(), 49, 10, 20));
            let terminal:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed").bind(&id).fetch_one(&pool).await.unwrap();
            assert_eq!(terminal > 0, stream);
        }
        assert_eq!(server.hits.load(Ordering::SeqCst), 2);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(balance.available_units, 804);
    assert_eq!(balance.reserved_units, 0);
    let bad:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e").fetch_one(&pool).await.unwrap();
    assert_eq!(bad, 0);
}
#[sqlx::test(migrations = false)]
async fn native_missing_usage_holds_funds_and_ambiguous_credentials_never_dispatch(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let server = server(Protocol::AnthropicMessages, true).await;
    let gateway = gateway(&pool, &server, Protocol::AnthropicMessages, key_id);
    let (path, body, header) = input(Protocol::AnthropicMessages, false);
    let request = || {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header, &key)
            .header("content-type", "application/json")
    };
    let response = gateway
        .router()
        .oneshot(
            request()
                .header("authorization", format!("Bearer {key}"))
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    let response = gateway
        .router()
        .oneshot(
            request()
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let _ = to_bytes(response.into_body(), 100_000).await;
    let balance = ledger.balance(account).await.unwrap();
    assert!(balance.reserved_units > 0);
    assert_eq!(balance.available_units + balance.reserved_units, 1000);
    let states: Vec<String> = sqlx::query_scalar("SELECT state FROM core_charging.requests")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(states, vec!["reconcile"]);
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
}
