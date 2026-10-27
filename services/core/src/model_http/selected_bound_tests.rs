//! Real PostgreSQL identity, selected routing, charging and ledger boundaries.
//! HTTPS contract tests stop before DNS or provider HTTP. Compatibility tests
//! use numeric-loopback HTTP with captured bodies and synthetic protocol replies.
//! Supplied usage is test evidence, not a live vendor receipt.
use super::{
    ChargePolicy, ExternalModelContext, ModelGateway, Operation, ProviderPolicy, UpstreamCredential,
};
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{self, Charge},
        ledger::{Action, Ledger, Request as LedgerRequest},
    },
    funding::FundingPolicy,
    identity::IdentityStore,
    relay::{
        self, Billing, ClientProtocol, FinishReason, Limits, OutputCheckpoint, RelayError,
        RequestContext, Route, RouteProvider,
    },
    routing::{
        self, BillingMode, CredentialRef, Endpoint, MULTIPLIER_ONE, PriceConfig, PriceRule,
        PriceSnapshot, Protocol, RoutingConfig, Speed, TokenRates,
    },
};
use axum::http::{HeaderMap, HeaderValue, header};
use rand::{RngCore, rngs::OsRng};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const SMALL_INPUT: i64 = 65_536;
const GLOBAL_INPUT: i64 = 1_048_576;
const SMALL_OUTPUT: u32 = 128;
const GLOBAL_OUTPUT: u32 = 4096;
const EXTERNAL_BASE: &str = "https://selected-bound.example/v1";

struct CompatibilityProvider {
    endpoint: String,
    bodies: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for CompatibilityProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl CompatibilityProvider {
    async fn start() -> Self {
        use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
        async fn reply(
            State(bodies): State<Arc<std::sync::Mutex<Vec<serde_json::Value>>>>,
            Json(body): Json<serde_json::Value>,
        ) -> axum::response::Response {
            bodies.lock().unwrap().push(body.clone());
            let chat = body.get("messages").is_some();
            let full = if chat {
                json!({"id":"synthetic-chat","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"synthetic answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":0}}})
            } else {
                json!({"id":"synthetic-resp","object":"response","status":"completed","output":[{"id":"item1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"synthetic answer","annotations":[]}]}],"usage":{"input_tokens":2,"output_tokens":3,"input_tokens_details":{"cached_tokens":0}}})
            };
            if body["stream"] != true {
                return Json(full).into_response();
            }
            let events = if chat {
                vec![
                    json!({"id":"synthetic-chat","choices":[{"index":0,"delta":{"role":"assistant","content":"synthetic answer"},"finish_reason":null}]}),
                    json!({"id":"synthetic-chat","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":full["usage"]}),
                ]
            } else {
                vec![
                    json!({"type":"response.created","response":{"id":"synthetic-resp","status":"in_progress","output":[]}}),
                    json!({"type":"response.output_item.added","output_index":0,"item":{"id":"item1","type":"message","role":"assistant","content":[]}}),
                    json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
                    json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"synthetic answer"}),
                    json!({"type":"response.content_part.done","output_index":0,"content_index":0}),
                    json!({"type":"response.output_item.done","output_index":0,"item":full["output"][0]}),
                    json!({"type":"response.completed","response":full}),
                ]
            };
            let mut wire = String::new();
            for event in events {
                if !chat {
                    wire.push_str(&format!("event: {}\n", event["type"].as_str().unwrap()));
                }
                wire.push_str(&format!("data: {event}\n\n"));
            }
            if chat {
                wire.push_str("data: [DONE]\n\n");
            }
            ([("content-type", "text/event-stream")], wire).into_response()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v1/chat/completions", post(reply))
            .route("/v1/responses", post(reply))
            .with_state(bodies.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            endpoint,
            bodies,
            task,
        }
    }
}

fn compatibility_gateway(f: &Fixture, base: &str, protocol: Protocol) -> ModelGateway {
    let original = gateway(f, base, vec![]);
    let mut document = original.bridge.routing.catalog_document().unwrap();
    document.routing.upstreams[0].protocol = protocol;
    let policies: Vec<routing::PayloadPolicy> = vec![
        serde_json::from_value(json!({"legacy_openai":{}})).unwrap(),
        serde_json::from_value(json!({"legacy_openai":{
            "allow_service_tier":true,"allow_inference_geo":true,"allow_speed":true,
            "allow_safety_identifier":true,"allow_include_obfuscation":true
        }}))
        .unwrap(),
        routing::PayloadPolicy::Adapt,
        routing::PayloadPolicy::Passthrough,
    ];
    let route = document.routing.routes[0].clone();
    let price = document.pricing.rules[0].clone();
    document.routing.routes.clear();
    document.pricing.rules.clear();
    for (index, policy) in policies.into_iter().enumerate() {
        let id = index as u64 + 1;
        let name = format!("compat-{index}");
        let mut route = route.clone();
        route.id = id;
        route.model = name.clone();
        route.targets[0].id = id;
        route.targets[0].upstream_model = name.clone();
        route.targets[0].payload = policy;
        document.routing.routes.push(route);
        let mut price = price.clone();
        price.id = id;
        price.model = name;
        document.pricing.rules.push(price);
    }
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(
            routing::Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-compatibility-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        original.limits.clone(),
        original.bridge.policy,
        ProviderPolicy::new(vec![], GLOBAL_INPUT).unwrap(),
    )
    .unwrap()
    .with_local_streaming()
}

fn compatibility_body(chat: bool, stream: bool, model: &str) -> serde_json::Value {
    let mut body = if chat {
        json!({"model":model,"messages":[{"role":"user","content":"hello"}],"max_completion_tokens":16,"stream":stream})
    } else {
        json!({"model":model,"input":"hello","max_output_tokens":16,"stream":stream})
    };
    body["service_tier"] = json!("auto");
    body["inference_geo"] = json!("synthetic-region");
    body["speed"] = json!("synthetic-speed");
    body["safety_identifier"] = json!("synthetic-user");
    if stream {
        body["stream_options"] = json!({"include_obfuscation":false});
    }
    body
}

async fn compatibility_call(
    g: &ModelGateway,
    f: &Fixture,
    chat: bool,
    id: &str,
    body: String,
) -> (u16, String, String) {
    use tower::ServiceExt;
    let response = g
        .router()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(if chat {
                    "/v1/chat/completions"
                } else {
                    "/v1/responses"
                })
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", f.key))
                .header("x-request-id", id)
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let request_id = response
        .headers()
        .get("x-request-id")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap();
    // Body closure precedes the host's completion receiver removing the operation.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if !g.bridge.active.lock().unwrap().contains_key(&request_id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    (
        status,
        String::from_utf8(bytes.to_vec()).unwrap(),
        request_id,
    )
}

#[sqlx::test(migrations = false)]
async fn legacy_openai_gateway_forwards_selected_policy_and_settles(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let provider = CompatibilityProvider::start().await;
    let mut completed = 0;
    for chat in [true, false] {
        let protocol = if chat {
            Protocol::OpenAiChat
        } else {
            Protocol::OpenAiResponses
        };
        let g = compatibility_gateway(&f, &provider.endpoint, protocol);
        for stream in [false, true] {
            for allowed in [false, true] {
                let model = if allowed { "compat-1" } else { "compat-0" };
                let id = format!("compat-success-{chat}-{stream}-{allowed}");
                let body = compatibility_body(chat, stream, model);
                let (status, response, request_id) =
                    compatibility_call(&g, &f, chat, &id, body.to_string()).await;
                assert_eq!(status, 200, "{response}");
                assert!(!request_id.is_empty());
                assert!(response.contains("synthetic answer"), "{response}");
                if stream {
                    assert!(response.contains(if chat { "[DONE]" } else { "response.completed" }));
                }
                let mut expected = if chat {
                    json!({"model":model,"messages":[{"role":"user","content":"hello"}],"max_completion_tokens":16,"stream":stream,"store":false})
                } else {
                    json!({"model":model,"input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"max_output_tokens":16,"stream":stream,"store":false})
                };
                if chat && stream {
                    expected["stream_options"] = json!({"include_usage":true});
                }
                if allowed {
                    for name in [
                        "service_tier",
                        "inference_geo",
                        "speed",
                        "safety_identifier",
                    ] {
                        expected[name] = body[name].clone();
                    }
                    if stream {
                        if expected.get("stream_options").is_none() {
                            expected["stream_options"] = json!({});
                        }
                        expected["stream_options"]["include_obfuscation"] = json!(false);
                    }
                }
                assert_eq!(provider.bodies.lock().unwrap()[completed], expected);
                let charge: (String, i64, i64, i64) = sqlx::query_as("SELECT state,settled,input_tokens,output_tokens FROM core_charging.requests WHERE id=$1")
                    .bind(&request_id).fetch_one(&f.pool).await.unwrap();
                assert_eq!(charge, ("settled".into(), 2, 2, 3));
                assert!(checkpoint_rows(&f.pool, &request_id).await > 0);
                completed += 1;
                assert_balance(&f, 10_000_000 - 2 * completed as i64, 0).await;
            }
        }
    }
    assert_eq!(completed, 8);
    assert_eq!(provider.bodies.lock().unwrap().len(), completed);
    assert_eq!(requests(&f.pool).await, completed as i64);
}

#[sqlx::test(migrations = false)]
async fn legacy_openai_gateway_rejects_before_reserve_or_provider(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let provider = CompatibilityProvider::start().await;
    for chat in [true, false] {
        let protocol = if chat {
            Protocol::OpenAiChat
        } else {
            Protocol::OpenAiResponses
        };
        let g = compatibility_gateway(&f, &provider.endpoint, protocol);
        let mut cases = Vec::new();
        for name in [
            "service_tier",
            "inference_geo",
            "speed",
            "safety_identifier",
        ] {
            let mut body = compatibility_body(chat, true, "compat-1");
            body[name] = json!({"invalid":true});
            cases.push((body.to_string(), 400));
        }
        let mut body = compatibility_body(chat, true, "compat-1");
        body["stream_options"]["include_obfuscation"] = json!("false");
        cases.push((body.to_string(), 400));
        let mut body = compatibility_body(chat, false, "compat-1");
        body["stream_options"] = json!({"include_obfuscation":true});
        cases.push((body.to_string(), 400));
        for model in ["compat-0", "compat-2", "compat-3"] {
            let body = compatibility_body(chat, true, model).to_string();
            if model != "compat-0" {
                // The gateway has always refused Passthrough before encoding.
                cases.push((body.clone(), if model == "compat-3" { 503 } else { 400 }));
            }
            cases.push((
                body.replace(
                    "\"include_obfuscation\":false",
                    "\"include_obfuscation\":false,\"INCLUDE_OBFUSCATION\":true",
                ),
                400,
            ));
            cases.push((
                body.replace(
                    "\"service_tier\":\"auto\"",
                    "\"service_tier\":\"auto\",\"service_tier\":\"auto\"",
                ),
                400,
            ));
            let mut body = compatibility_body(chat, true, model);
            body["payload_policy"] = json!({"legacy_openai":{}});
            cases.push((body.to_string(), 400));
        }
        for (index, (body, expected_status)) in cases.into_iter().enumerate() {
            let id = format!("compat-reject-{chat}-{index}");
            let (status, response, _) = compatibility_call(&g, &f, chat, &id, body).await;
            assert_eq!(status, expected_status, "{id}: {response}");
            assert_eq!(requests(&f.pool).await, 0, "{id}");
            assert!(provider.bodies.lock().unwrap().is_empty(), "{id}");
            assert_balance(&f, 10_000_000, 0).await;
        }
    }
}

struct Fixture {
    pool: PgPool,
    ledger: Ledger,
    session: String,
    key: String,
    key_id: i64,
    account: i64,
}

async fn fixture(pool: PgPool, amount: i64) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::check(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let principal = identity.authorize(&session.secret).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-selected-bound-pg')")
        .execute(&pool)
        .await
        .unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger
        .open_wallet(principal.owner_account_id)
        .await
        .unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "selected-bound-pg".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in disposable selected-bound PostgreSQL".into(),
            action: Action::Credit {
                account_id: principal.owner_account_id,
                amount_units: Credits::try_from(amount).unwrap(),
                payment_reference: "synthetic:selected-bound".into(),
            },
        })
        .await
        .unwrap();
    Fixture {
        pool,
        ledger,
        session: session.secret,
        key: key.secret,
        key_id: key.id,
        account: principal.owner_account_id,
    }
}

fn contexts(small_input: i64) -> Vec<ExternalModelContext> {
    [
        ("small-provider", small_input, SMALL_OUTPUT),
        ("large-provider", GLOBAL_INPUT, GLOBAL_OUTPUT),
    ]
    .into_iter()
    .map(|(model, input, output)| ExternalModelContext {
        reservation: Default::default(),
        missing_cache_creation_tokens_is_zero: false,
        upstream_id: 1,
        upstream_model: model.into(),
        protocol: Protocol::OpenAiChat,
        endpoint: EXTERNAL_BASE.into(),
        max_input_tokens: u32::try_from(input).unwrap(),
        max_output_tokens: output,
        contract_revision: "synthetic-selected-bound-v1".into(),
        missing_cached_tokens_is_zero: false,
    })
    .collect()
}

fn gateway(f: &Fixture, base: &str, contexts: Vec<ExternalModelContext>) -> ModelGateway {
    gateway_with_output_rate(f, base, contexts, 0)
}

fn gateway_with_output_rate(
    f: &Fixture,
    base: &str,
    contexts: Vec<ExternalModelContext>,
    output_rate: u64,
) -> ModelGateway {
    let mut config: RoutingConfig = serde_json::from_value(json!({
        "version":1,
        "groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":EXTERNAL_BASE,"protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[
            {"id":1,"model":"small-model","group":"allowed","enabled":true,"speeds":["standard"],
             "targets":[{"id":1,"upstream_id":1,"upstream_model":"small-provider","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
             "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}},
            {"id":2,"model":"large-model","group":"allowed","enabled":true,"speeds":["standard"],
             "targets":[{"id":2,"upstream_id":1,"upstream_model":"large-provider","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
             "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}
        ],
        "aliases":[]
    }))
    .unwrap();
    config.upstreams[0].endpoint = if base.starts_with("https:") {
        base.to_owned().try_into().unwrap()
    } else {
        Endpoint::local_loopback(base.to_owned()).unwrap()
    };
    let prices = PriceSnapshot::compile(PriceConfig {
        version: 1,
        rules: ["small-model", "large-model"]
            .into_iter()
            .enumerate()
            .map(|(index, model)| PriceRule {
                billing_mode: BillingMode::Tokens,
                tiered: None,
                id: index as u64 + 1,
                model: model.into(),
                group: "allowed".into(),
                speed: Speed::Standard,
                // Exactly one Credit per input token, cached or uncached.
                rates: TokenRates {
                    input: 2_000_000,
                    output: output_rate.into(),
                    cache_read: 2_000_000,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: MULTIPLIER_ONE,
                speed_multiplier_ppm: MULTIPLIER_ONE,
            })
            .collect(),
    })
    .unwrap();
    let policy = ProviderPolicy::new(contexts, GLOBAL_INPUT).unwrap();
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(routing::Router::new(config, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-selected-bound-provider-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        Limits {
            max_output_tokens: GLOBAL_OUTPUT,
            total_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: GLOBAL_INPUT,
            lease_seconds: 30,
        },
        policy,
    )
    .unwrap()
}

/// Reuse real authentication and authorization, then stage the same private
/// host operation that HTTP admission normally supplies to select/reserve.
async fn admit(
    g: &ModelGateway,
    f: &Fixture,
    id: &str,
    model: &str,
    output: u32,
) -> (RequestContext, relay::Request) {
    let body = serde_json::to_vec(&json!({
        "model":model,"messages":[{"role":"user","content":"hello"}],
        "max_tokens":output
    }))
    .unwrap();
    let request = relay::Request::parse(ClientProtocol::Chat, &body, &g.limits).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", f.key)).unwrap(),
    );
    let (principal, secret, groups) = g.authenticate(&headers).await.unwrap();
    assert_eq!(groups, ["allowed"]);
    g.authorize_request(&principal, &secret, model, "allowed", None)
        .await
        .unwrap();
    let context = RequestContext {
        request_id: id.into(),
        actor_user_id: principal.user_id(),
        key_id: principal.key_id(),
        oauth_source: principal.oauth_source().cloned(),
    };
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    assert!(
        g.bridge
            .active
            .lock()
            .unwrap()
            .insert(
                id.into(),
                Operation {
                    secret,
                    group: "allowed".into(),
                    worker,
                    fingerprint: Sha256::digest(body).into(),
                    selected: None,
                    output_token_ceiling: output,
                    peer: None,
                    oauth_issuer: None,
                },
            )
            .is_none()
    );
    (context, request)
}

fn selected_bound(g: &ModelGateway, id: &str) -> i64 {
    g.bridge.active.lock().unwrap()[id]
        .selected
        .as_ref()
        .unwrap()
        .input_token_ceiling
}

async fn receipt(g: &ModelGateway, id: &str) -> Charge {
    g.bridge.charging.resolve(id).await.unwrap().unwrap()
}

async fn requests(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn checkpoint_rows(pool: &PgPool, id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn assert_balance(f: &Fixture, available: i64, reserved: i64) {
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (available, reserved)
    );
    let unbalanced: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) entries")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(unbalanced, 0);
}

fn usage(input: u64) -> relay::Usage {
    relay::Usage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        cached_input_tokens: Some(0),
        ..relay::Usage::default()
    }
}

fn terminal(id: &str, route: &Route, usage: relay::Usage) -> relay::Report {
    relay::Report {
        request_id: id.into(),
        route_id: Some(route.id.clone()),
        price_version: Some(route.price_version),
        outcome: relay::Outcome::Completed,
        usage,
        finish_reason: Some(FinishReason::Stop),
        provider_complete: true,
        upstream_attempted: true,
        attempts: 1,
        upstream_status: Some(200),
        upstream_bytes: 128,
        emitted_bytes: 0,
    }
}

#[sqlx::test(migrations = false)]
async fn selected_bound_mixed_external_models_reserve_their_own_caps(pool: PgPool) {
    let f = fixture(pool, 2_000_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (small_context, small_request) =
        admit(&g, &f, "selected-small", "small-model", SMALL_OUTPUT).await;
    let small_route = g
        .bridge
        .select(&small_context, &small_request)
        .await
        .unwrap();
    assert_eq!(selected_bound(&g, "selected-small"), SMALL_INPUT);
    let small = g
        .bridge
        .reserve(&small_context, &small_route, &small_request)
        .await
        .unwrap();
    assert_eq!(receipt(&g, &small.id).await.reserved, SMALL_INPUT);
    assert_balance(&f, 2_000_000 - SMALL_INPUT, SMALL_INPUT).await;

    let (large_context, large_request) =
        admit(&g, &f, "selected-large", "large-model", GLOBAL_OUTPUT).await;
    let large_route = g
        .bridge
        .select(&large_context, &large_request)
        .await
        .unwrap();
    assert_eq!(selected_bound(&g, "selected-large"), GLOBAL_INPUT);
    let large = g
        .bridge
        .reserve(&large_context, &large_route, &large_request)
        .await
        .unwrap();
    assert_eq!(receipt(&g, &large.id).await.reserved, GLOBAL_INPUT);
    assert_balance(
        &f,
        2_000_000 - SMALL_INPUT - GLOBAL_INPUT,
        SMALL_INPUT + GLOBAL_INPUT,
    )
    .await;

    g.bridge
        .finalize(&small, &terminal(&small.id, &small_route, usage(100)))
        .await
        .unwrap();
    g.bridge
        .finalize(&large, &terminal(&large.id, &large_route, usage(1000)))
        .await
        .unwrap();
    assert_eq!(receipt(&g, &small.id).await.settled, 100);
    assert_eq!(receipt(&g, &large.id).await.settled, 1000);
    assert_balance(&f, 2_000_000 - 1100, 0).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_small_external_model_is_fundable_without_global_hold(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(&g, &f, "fundable-small", "small-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    assert_eq!(receipt(&g, &reservation.id).await.reserved, SMALL_INPUT);
    assert_balance(&f, 70_000 - SMALL_INPUT, SMALL_INPUT).await;

    let (context, request) = admit(&g, &f, "unfunded-large", "large-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    assert!(matches!(
        g.bridge.reserve(&context, &route, &request).await,
        Err(RelayError::NoFunds)
    ));
    assert!(
        g.bridge
            .charging
            .resolve("unfunded-large")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(requests(&f.pool).await, 1);
    assert_balance(&f, 70_000 - SMALL_INPUT, SMALL_INPUT).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_output_over_model_cap_is_rejected_before_any_hold(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(
        &g,
        &f,
        "excess-small-output",
        "small-model",
        SMALL_OUTPUT + 1,
    )
    .await;
    assert!(matches!(
        g.bridge.select(&context, &request).await,
        Err(RelayError::RouteUnavailable)
    ));
    assert_eq!(requests(&f.pool).await, 0);
    assert_balance(&f, 70_000, 0).await;
    // A larger request remains selectable for the other model in this gateway.
    let (context, request) =
        admit(&g, &f, "valid-large-output", "large-model", GLOBAL_OUTPUT).await;
    assert!(g.bridge.select(&context, &request).await.is_ok());
    assert_eq!(requests(&f.pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn selected_bound_external_contract_must_match_every_binding_field(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    for index in 0..5 {
        let mut contracts = contexts(SMALL_INPUT);
        match index {
            0 => contracts.clear(),
            1 => contracts[0].upstream_id = 2,
            2 => contracts[0].upstream_model = "different-provider-model".into(),
            3 => contracts[0].protocol = Protocol::OpenAiResponses,
            4 => contracts[0].endpoint = "https://different-bound.example/v1".into(),
            _ => unreachable!(),
        }
        let g = gateway(&f, EXTERNAL_BASE, contracts);
        let (context, request) = admit(
            &g,
            &f,
            &format!("invalid-binding-{index}"),
            "small-model",
            1,
        )
        .await;
        assert!(matches!(
            g.bridge.select(&context, &request).await,
            Err(RelayError::Unsupported(_) | RelayError::RouteUnavailable)
        ));
        assert_eq!(requests(&f.pool).await, 0);
        assert_balance(&f, 70_000, 0).await;
    }
}

#[sqlx::test(migrations = false)]
async fn selected_bound_bad_terminal_and_unknown_usage_keep_original_hold(pool: PgPool) {
    // @context-fold begin selected-bound-observation-is-audit-only
    let f = fixture(pool, 300_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let invalid = [
        relay::Usage {
            upstream_cost_atto_usd: Some(123),
            ..usage(SMALL_INPUT as u64 + 1)
        },
        relay::Usage {
            input_tokens: Some(37),
            output_tokens: Some(40),
            cached_input_tokens: Some(0),
            reasoning_output_tokens: Some(30),
            ..relay::Usage::default()
        },
        relay::Usage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            ..relay::Usage::default()
        },
        relay::Usage::default(),
    ];
    let cases = invalid.len() as i64;
    for (index, observed) in invalid.into_iter().enumerate() {
        let id = format!("unknown-selected-{index}");
        let (context, request) = admit(&g, &f, &id, "small-model", 1).await;
        let route = g.bridge.select(&context, &request).await.unwrap();
        let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
        let original = receipt(&g, &id).await;
        assert_eq!((original.reserved, original.settled), (SMALL_INPUT, 0));
        let worker = g.bridge.active.lock().unwrap().get(&id).unwrap().worker;
        let raw_usage = serde_json::to_value(observed).unwrap();
        let checkpoint = OutputCheckpoint {
            sequence: 1,
            packet_sha256: [3; 32],
            packet_bytes: 128,
            usage: observed,
            terminal_observed: true,
        };
        let finalize_only = observed == relay::Usage::default();
        for _ in 0..2 {
            let result = if finalize_only {
                g.bridge
                    .finalize(&reservation, &terminal(&id, &route, observed))
                    .await
            } else {
                g.bridge.persist_output(&reservation, &checkpoint).await
            };
            assert!(matches!(result, Err(RelayError::SettlementPending)));
        }
        let observation: serde_json::Value = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'",
        )
        .bind(&id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(
            observation,
            json!({
                "kind": "provider_observation",
                "provider_complete": true,
                "settlement_eligible": false,
                "route_id": route.id,
                "raw_usage": raw_usage,
                "input_token_ceiling": SMALL_INPUT,
                "output_token_ceiling": request.max_output_tokens,
            })
        );
        let audited = receipt(&g, &id).await;
        if finalize_only {
            assert_eq!(audited.state, "reconcile");
        } else {
            assert_eq!(audited.state, original.state);
            assert_eq!(audited.revision, original.revision);
        }
        assert_eq!(audited.reserved, original.reserved);
        assert_eq!(audited.settled, original.settled);
        assert_eq!(audited.usage, original.usage);
        let mut stale_worker = worker;
        stale_worker[0] ^= 1;
        assert_eq!(
            g.bridge
                .charging
                .record_provider_observation(
                    &id,
                    &stale_worker,
                    &route.id,
                    &raw_usage,
                    SMALL_INPUT,
                    request.max_output_tokens,
                )
                .await,
            Err(charging::Error::StaleWorker)
        );
        let mut changed_usage = raw_usage.clone();
        changed_usage["input_tokens"] = json!(0);
        assert_eq!(
            g.bridge
                .charging
                .record_provider_observation(
                    &id,
                    &worker,
                    &route.id,
                    &changed_usage,
                    SMALL_INPUT,
                    request.max_output_tokens,
                )
                .await,
            Err(charging::Error::Conflict)
        );
        assert_eq!(checkpoint_rows(&f.pool, &id).await, 0);
        assert!(matches!(
            g.bridge
                .finalize(&reservation, &terminal(&id, &route, observed))
                .await,
            Err(RelayError::SettlementPending)
        ));
        let after = receipt(&g, &id).await;
        assert_eq!(
            (after.state.as_str(), after.reserved, after.settled),
            ("reconcile", SMALL_INPUT, 0)
        );
        assert_eq!(after.price, original.price);
        assert_eq!(after.usage, original.usage);
        let observation_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'",
        )
        .bind(&id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(observation_count, 1);
        let recorded: serde_json::Value = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'",
        )
        .bind(&id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(recorded, observation);
        assert!(matches!(
            g.bridge.reserve(&context, &route, &request).await,
            Err(RelayError::Conflict)
        ));
        assert_eq!(requests(&f.pool).await, index as i64 + 1);
        assert_balance(
            &f,
            300_000 - SMALL_INPUT * (index as i64 + 1),
            SMALL_INPUT * (index as i64 + 1),
        )
        .await;
    }
    assert_eq!(
        g.reconcile_completed_streams_page(10, None).await.unwrap(),
        (0, None)
    );
    let accepted_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.usage_events WHERE event_id='final' OR event_id LIKE 'usage:%'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(accepted_events, 0);
    let open_holds: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.ledger_holds WHERE NOT closed AND captured=0 AND refunded=0",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(open_holds, cases);
    assert_balance(&f, 300_000 - cases * SMALL_INPUT, cases * SMALL_INPUT).await;
    // @context-fold end selected-bound-observation-is-audit-only
}

#[sqlx::test(migrations = false)]
async fn selected_bound_loopback_keeps_global_synthetic_reservation(pool: PgPool) {
    let f = fixture(pool, 2_000_000).await;
    let g = gateway(&f, "http://127.0.0.1:1/v1", vec![]);
    let (context, request) = admit(&g, &f, "synthetic-global", "small-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    assert_eq!(selected_bound(&g, &context.request_id), GLOBAL_INPUT);
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    assert_eq!(receipt(&g, &reservation.id).await.reserved, GLOBAL_INPUT);
    assert_balance(&f, 2_000_000 - GLOBAL_INPUT, GLOBAL_INPUT).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_recovery_uses_old_terminal_price_after_contract_change(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(&g, &f, "frozen-selected", "small-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    let original = receipt(&g, &reservation.id).await;
    g.bridge
        .persist_output(
            &reservation,
            &OutputCheckpoint {
                sequence: 1,
                packet_sha256: [4; 32],
                packet_bytes: 128,
                usage: usage(60_000),
                terminal_observed: true,
            },
        )
        .await
        .unwrap();
    let worker = g.bridge.active.lock().unwrap()[&reservation.id].worker;
    g.bridge
        .charging
        .mark_unknown(&reservation.id, &worker)
        .await
        .unwrap();
    let restarted = gateway(&f, EXTERNAL_BASE, contexts(32_768));
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(10, None)
            .await
            .unwrap(),
        (1, None)
    );
    let settled = receipt(&restarted, &reservation.id).await;
    assert_eq!(
        (settled.state.as_str(), settled.reserved, settled.settled),
        ("settled", SMALL_INPUT, 60_000)
    );
    assert_eq!(settled.price, original.price);
    assert_eq!(requests(&f.pool).await, 1);
    assert_balance(&f, 10_000, 0).await;
}

// @context-fold begin prepared-reservation-pg-tests
#[sqlx::test(migrations = false)]
async fn prepared_contract_is_durable_and_binds_wire_before_another_hold(pool: PgPool) {
    let f = fixture(pool, 2_000_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(&g, &f, "prepared-wire", "small-model", SMALL_OUTPUT).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let body = request.upstream_body(&route).unwrap();
    let reservation = g
        .bridge
        .reserve_prepared(&context, &route, &request, &body, None)
        .await
        .unwrap();
    let charge = receipt(&g, &reservation.id).await;
    let detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'",
    ).bind(&charge.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(detail["kind"], "model_reservation_contract");
    assert_eq!(detail["mode"], "context_max");
    assert_eq!(detail["bounds"]["input"], SMALL_INPUT);
    assert_eq!(detail["bounds"]["requested_output"], SMALL_OUTPUT);
    assert!(detail.get("usage").is_none());
    assert!(detail.get("counter").unwrap().is_null());
    let (frozen, _) = g
        .bridge
        .charging
        .reservation_contract(&charge.id)
        .await
        .unwrap();
    assert_eq!(frozen.price, charge.price);
    let mut changed = body.clone();
    changed.push(b' ');
    assert!(
        g.bridge
            .reserve_prepared(&context, &route, &request, &changed, None)
            .await
            .is_err()
    );
    assert_eq!(requests(&f.pool).await, 1);
    assert_balance(&f, 2_000_000 - SMALL_INPUT, SMALL_INPUT).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'")
        .bind(&charge.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(count, 1);
    // The output is free in this fixture. An amount-only limit would allow it.
    let worker = g.bridge.active.lock().unwrap()[&reservation.id].worker;
    let invalid = charging::Usage {
        output: i64::from(SMALL_OUTPUT) + 1,
        ..Default::default()
    };
    assert!(
        g.bridge
            .charging
            .checkpoint(&charge.id, &worker, "bound-test", invalid)
            .await
            .is_err()
    );
    assert!(
        g.bridge
            .charging
            .settle(&charge.id, &worker, invalid, charging::Outcome::Completed)
            .await
            .is_err()
    );
    assert_balance(&f, 2_000_000 - SMALL_INPUT, SMALL_INPUT).await;
    f.pool.close().await;
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_insert_failure_rolls_back_request_budget_and_hold(pool: PgPool) {
    let f = fixture(pool, 2_000_000).await;
    let mut setup = f.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *setup)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.budgets(scope,account_id,issuer_user_id,period,limit_credits) VALUES('account',$1,7,'day',2000000)")
        .bind(f.account).execute(&mut *setup).await.unwrap();
    setup.commit().await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION public.reject_reservation_contract() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id='reservation-contract:v1' THEN RAISE EXCEPTION 'synthetic contract failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_reservation_contract BEFORE INSERT ON core_charging.usage_events FOR EACH ROW EXECUTE FUNCTION public.reject_reservation_contract();")
        .execute(&f.pool).await.unwrap();
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(&g, &f, "prepared-rollback", "small-model", SMALL_OUTPUT).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    assert!(g.bridge.reserve(&context, &route, &request).await.is_err());
    assert_eq!(requests(&f.pool).await, 0);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);
    let budget_checks: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.budget_checks")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(budget_checks, 0);
    assert_balance(&f, 2_000_000, 0).await;
    f.pool.close().await;
}
#[sqlx::test(migrations = false)]
async fn prepared_contract_concurrent_admission_preserves_prior_hold(pool: PgPool) {
    let f = Arc::new(fixture(pool, 160).await);
    let old = gateway(&f, EXTERNAL_BASE, contexts(64));
    let (context, request) = admit(&old, &f, "old-unresolved-hold", "small-model", 8).await;
    old.bridge.select(&context, &request).await.unwrap();
    let (worker, spec) = {
        let active = old.bridge.active.lock().unwrap();
        let operation = &active[&context.request_id];
        let selected = operation.selected.as_ref().unwrap();
        (
            operation.worker,
            charging::Request {
                id: context.request_id.clone(),
                fingerprint: operation.fingerprint,
                price: selected.price.clone(),
                reserve: 64,
                lease_seconds: 30,
                maximum_seconds: 30,
            },
        )
    };
    // Exercise the actual historical path. Never delete a newly created proof
    // and pretend that the resulting row is a pre-R1 liability.
    old.bridge
        .charging
        .reserve_model_with_context(
            &f.key,
            &spec,
            &worker,
            None,
            charging::ReservationBasis::Tokens {
                input: 64,
                output: 8,
            },
        )
        .await
        .unwrap();
    assert!(old.bridge.charging.start(&spec.id, &worker).await.unwrap());
    let reservation = relay::Reservation { id: spec.id };
    old.bridge
        .charging
        .mark_unknown(&reservation.id, &worker)
        .await
        .unwrap();
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1")
            .bind(&reservation.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let original_evidence =
        prepared_financial_tests::liability_snapshot(&f.pool, &reservation.id).await;
    let legacy_proofs: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'")
        .bind(&reservation.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(legacy_proofs, 0);
    assert_balance(&f, 96, 64).await;

    let g = Arc::new(gateway_with_output_rate(
        &f,
        EXTERNAL_BASE,
        contexts(24),
        2_000_000,
    ));
    let mut attempts = tokio::task::JoinSet::new();
    for index in 0..8 {
        let g = g.clone();
        let f = f.clone();
        attempts.spawn(async move {
            let id = format!("prepared-concurrent-{index}");
            let (context, request) = admit(&g, &f, &id, "small-model", 8).await;
            let route = g.bridge.select(&context, &request).await.unwrap();
            g.bridge.reserve(&context, &route, &request).await
        });
    }
    let mut successful = 0;
    while let Some(result) = attempts.join_next().await {
        let result = result.unwrap();
        match result {
            Ok(reservation) => {
                successful += 1;
                assert_eq!(receipt(&g, &reservation.id).await.reserved, 32);
            }
            Err(error) => assert_eq!(error, RelayError::NoFunds),
        }
    }
    assert_eq!(successful, 3);
    let (context, request) = admit(&g, &f, "prepared-capacity-exhausted", "small-model", 8).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    assert!(matches!(
        g.bridge.reserve(&context, &route, &request).await,
        Err(RelayError::NoFunds)
    ));
    assert_eq!(requests(&f.pool).await, 4);
    let proofs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.usage_events WHERE event_id='reservation-contract:v1' AND request_id LIKE 'prepared-concurrent-%'",
    ).fetch_one(&f.pool).await.unwrap();
    assert_eq!(proofs, 3);
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1")
            .bind(&reservation.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(after, before);
    assert_eq!(
        prepared_financial_tests::liability_snapshot(&f.pool, &reservation.id).await,
        original_evidence
    );
    assert_balance(&f, 0, 160).await;
    f.pool.close().await;
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_resolve_waits_for_original_transaction(pool: PgPool) {
    use charging::contract::PreparedReservation;
    let f = fixture(pool, 2_000_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) =
        admit(&g, &f, "prepared-wait-commit", "small-model", SMALL_OUTPUT).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    let charge = receipt(&g, &reservation.id).await;
    let (worker, fingerprint) = {
        let active = g.bridge.active.lock().unwrap();
        let operation = &active[&reservation.id];
        (operation.worker, operation.fingerprint)
    };
    let detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'",
    ).bind(&reservation.id).fetch_one(&f.pool).await.unwrap();
    let prepared = PreparedReservation::context_max(
        detail["request_fingerprint"].as_str().unwrap().into(),
        serde_json::from_value(detail["binding"].clone()).unwrap(),
        detail["wire_sha256"].as_str().unwrap().into(),
        serde_json::from_value(detail["bounds"].clone()).unwrap(),
        serde_json::from_value(detail["semantics"].clone()).unwrap(),
    )
    .unwrap();
    let spec = charging::Request {
        id: reservation.id.clone(),
        fingerprint,
        price: charge.price.clone(),
        reserve: charge.reserved,
        lease_seconds: 30,
        maximum_seconds: 30,
    };
    let mut writer = f.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *writer)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core_charging:' || $1,0))")
        .bind(&reservation.id)
        .execute(&mut *writer)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_charging.requests SET state='reconcile',revision=revision+1 WHERE id=$1",
    )
    .bind(&reservation.id)
    .execute(&mut *writer)
    .await
    .unwrap();
    let resolving = g.bridge.charging.resolve_prepared_model(
        &spec,
        &worker,
        charging::ReservationBasis::Tokens {
            input: SMALL_INPUT as u64,
            output: u64::from(SMALL_OUTPUT),
        },
        &prepared,
    );
    tokio::pin!(resolving);
    tokio::select! {
        result = &mut resolving => panic!("resolved before original writer committed: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
    }
    writer.commit().await.unwrap();
    let resolved = tokio::time::timeout(Duration::from_secs(3), &mut resolving)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.state, "reconcile");
    assert_eq!(resolved.revision, charge.revision + 1);
    assert_balance(&f, 2_000_000 - SMALL_INPUT, SMALL_INPUT).await;
    f.pool.close().await;
}
// @context-fold end prepared-reservation-pg-tests

#[path = "prepared_financial_tests.rs"]
mod prepared_financial_tests;
