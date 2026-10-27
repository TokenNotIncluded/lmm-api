//! Independent native System One acceptance: pinned official SDKs over real TCP,
//! disposable PostgreSQL, original API Key authority and transactional ledger.
//! All upstream responses come from a counting numeric-loopback fixture.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRequest, Multipart, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use lmm_core::{
    billing::{
        charging::{Budget, Charging, key_quota_snapshot},
        transactional_ledger::TransactionalLedger,
    },
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::*,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};
#[path = "common/model_fixture.rs"]
mod model_fixture;

#[derive(Clone)]
struct Supplier {
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Value>>>,
    images: Arc<Mutex<Vec<Vec<u8>>>>,
}
async fn supplier(State(s): State<Supplier>, request: Request<Body>) -> Response {
    s.hits.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        request.headers()["authorization"],
        "Bearer synthetic-independent-supplier"
    );
    let wire: Value = if request.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("multipart/form-data")
    {
        let mut parts = Multipart::from_request(request, &()).await.unwrap();
        let first = parts.next_field().await.unwrap().unwrap();
        assert_eq!(first.name(), Some("request"));
        assert_eq!(first.content_type(), Some("application/json"));
        let wire = serde_json::from_slice(&first.bytes().await.unwrap()).unwrap();
        while let Some(field) = parts.next_field().await.unwrap() {
            let bytes = field.bytes().await.unwrap().to_vec();
            s.images.lock().unwrap().push(bytes);
        }
        wire
    } else {
        serde_json::from_slice(
            &to_bytes(request.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()
    };
    match wire["state"]["scenario"].as_str() {
        Some("status-429") | Some("status-529") => {
            let code = if wire["state"]["scenario"] == "status-429" {
                429
            } else {
                529
            };
            s.seen.lock().unwrap().push(wire);
            return (
                StatusCode::from_u16(code).unwrap(),
                Json(json!({"error":"synthetic supplier refusal"})),
            )
                .into_response();
        }
        Some("malformed-success") => {
            s.seen.lock().unwrap().push(wire);
            return (StatusCode::OK, [("content-type", "application/json")], "{").into_response();
        }
        Some("caller-disconnect") => tokio::time::sleep(Duration::from_millis(500)).await,
        _ => (),
    }
    let model = wire["model"].as_str().unwrap();
    assert!(matches!(model, "jev-1.13.0" | "jev-1.14.0"));
    assert!(wire.get("messages").is_none());
    if let Some(decimal) = wire["state"].get("decimal") {
        assert_eq!(decimal, &json!(0.3));
    }
    let mut answers = serde_json::Map::new();
    for (name, q) in wire["questions"].as_object().unwrap() {
        answers.insert(name.clone(), match q["type"].as_str().unwrap(){
            "noul"=>json!({"type":"noul","noul":0.3}),
            "box"=>json!({"type":"box","box":{"top":0.1,"bottom":0.9,"left":0.2,"right":0.7},"confidence":null}),
            _=>panic!("unexpected independent question"),
        });
    }
    let mut output = json!({"model":model,"answers":answers});
    if wire["state"]["scenario"] != "missing-usage" {
        output["usage"] = json!({"input_tokens":120,"output_tokens":456789});
    }
    s.seen.lock().unwrap().push(wire);
    Json(output).into_response()
}
#[derive(Clone, Debug)]
struct Ingress {
    key: String,
    encoding: Option<String>,
    content_type: String,
    raw_size: usize,
    zstd_magic: bool,
    status: u16,
    exposes_answers: bool,
}
async fn record(
    State(seen): State<Arc<Mutex<Vec<Ingress>>>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let key = request
        .headers()
        .get("idempotency-key")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let encoding = request
        .headers()
        .get("content-encoding")
        .map(|v| v.to_str().unwrap().to_owned());
    let content_type = request
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 33 * 1024 * 1024).await.unwrap();
    let raw_size = body.len();
    let zstd_magic = body.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]);
    let response = next.run(Request::from_parts(parts, Body::from(body))).await;
    let status = response.status().as_u16();
    let (parts, body) = response.into_parts();
    let body = to_bytes(body, 2 * 1024 * 1024).await.unwrap();
    let exposes_answers = serde_json::from_slice::<Value>(&body)
        .ok()
        .is_some_and(|v| v.get("answers").is_some());
    seen.lock().unwrap().push(Ingress {
        key,
        encoding,
        content_type,
        raw_size,
        zstd_magic,
        status,
        exposes_answers,
    });
    Response::from_parts(parts, Body::from(body))
}
struct Server {
    base: String,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(app: Router) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Server { base, task }
}
fn build_gateway(pool: &PgPool, base: &str, key: i64, output_rate: u64) -> ModelGateway {
    let models = ["jev-1.13.0", "jev-1.14.0"];
    let rules:Vec<Value>=models.iter().enumerate().map(|(i,model)|json!({"id":i+1,"model":model,"group":"allowed","speed":"standard","rates":{"input":42000,"output":output_rate,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000})).collect();
    let prices =
        PriceSnapshot::compile(serde_json::from_value(json!({"version":1,"rules":rules})).unwrap())
            .unwrap();
    let config = RoutingConfig {
        version: 1,
        groups: vec![ModelGroup {
            name: "allowed".into(),
            enabled: true,
        }],
        upstreams: vec![UpstreamConfig {
            id: 1,
            endpoint: Endpoint::local_loopback(base.into()).unwrap(),
            protocol: Protocol::SystemOne,
            credential: CredentialRef { id: 7, revision: 1 },
            enabled: true,
        }],
        routes: models
            .iter()
            .enumerate()
            .map(|(i, model)| ModelRoute {
                id: i as u64 + 1,
                model: (*model).into(),
                group: "allowed".into(),
                enabled: true,
                speeds: vec![Speed::Standard],
                targets: vec![RouteTarget {
                    id: i as u64 + 1,
                    upstream_id: 1,
                    upstream_model: (*model).into(),
                    enabled: true,
                    priority: 0,
                    weight: 1,
                    speeds: vec![Speed::Standard],
                    payload: PayloadPolicy::Adapt,
                    timeout_ms: 10000,
                }],
                failover: FailoverPolicy {
                    max_attempts: 1,
                    retry_on: vec![],
                    allow_replay_after_send: false,
                },
            })
            .collect(),
        aliases: vec![],
    };
    ModelGateway::new(
        pool.clone(),
        Arc::new(lmm_core::routing::Router::new(config, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-independent-supplier".into(),
        }],
        BTreeMap::from([(key, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(5),
            hook_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 65536,
            lease_seconds: 30,
        },
    )
    .unwrap()
}
fn evidence() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../output/remaining-20261020/systemone-independent")
}
async fn sdk(program: &Path, script: &Path, base: &str, key: &str, png: &Path) {
    let mut command = std::process::Command::new(program);
    command
        .arg(script)
        .env("LMM_JEV_BASE", base)
        .env("LMM_JEV_KEY", key)
        .env("LMM_JEV_PNG", png)
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("TYPESAFE_BASE_URL")
        .env_remove("TYPESAFE_DEFAULT_MODEL")
        .env_remove("TYPESAFE_LOG_LEVEL");
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("official SDK subprocess exceeded its bounded deadline");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "official SDK failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8(output.stdout).unwrap());
}
#[sqlx::test(migrations = false)]
#[ignore = "requires pinned official SDK packages and isolated acceptance runner under output"]
async fn systemone_independent_official_sdk_raw_zstd_multipart_tcp_key_ledger(pool: PgPool) {
    let (identity, ledger, session, key, key_id, account) =
        model_fixture::fixture(&pool, 10000).await;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'independent-systemone-sdk')").execute(&pool).await.unwrap();
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    charging
        .set_budget(
            &session,
            &Budget {
                scope: "key".into(),
                account_id: None,
                user_id: None,
                key_id: Some(key_id),
                period: "lifetime".into(),
                anchor: 0,
                seconds: 0,
                limit: 10000,
            },
        )
        .await
        .unwrap();
    let supplier_state = Supplier {
        hits: Arc::new(AtomicUsize::new(0)),
        seen: Arc::new(Mutex::new(Vec::new())),
        images: Arc::new(Mutex::new(Vec::new())),
    };
    let supplier_server = serve(
        Router::new()
            .route("/v1/systemone", post(supplier))
            .with_state(supplier_state.clone()),
    )
    .await;
    let gateway = build_gateway(&pool, &format!("{}/v1", supplier_server.base), key_id, 0);
    let ingress = Arc::new(Mutex::new(Vec::new()));
    let core = serve(
        gateway
            .router()
            .layer(middleware::from_fn_with_state(ingress.clone(), record)),
    )
    .await;
    let root = evidence();
    let png = root.join("synthetic-280.png");
    assert!(png.is_file(), "independent frozen image fixture missing");
    sdk(
        Path::new("node"),
        &root.join("sdk_js.mjs"),
        &core.base,
        &key,
        &png,
    )
    .await;
    sdk(
        &root.join("sdk-venv/bin/python"),
        &root.join("sdk_python.py"),
        &core.base,
        &key,
        &png,
    )
    .await;
    assert_eq!(
        supplier_state.hits.load(Ordering::SeqCst),
        7,
        "six valid SDK calls and one unknown-usage dispatch, no retry"
    );
    let observed = ingress.lock().unwrap().clone();
    for language in ["js", "python"] {
        for mode in ["raw", "zstd", "multipart"] {
            let matched: Vec<_> = observed
                .iter()
                .filter(|v| v.key == format!("independent-{language}-{mode}"))
                .collect();
            assert_eq!(matched.len(), 1);
            let v = matched[0];
            assert_eq!(v.status, 200);
            assert!(v.exposes_answers);
            assert!(v.raw_size > 0);
            if mode == "raw" {
                assert_eq!(v.encoding, None);
                assert!(v.content_type.starts_with("application/json"));
            } else {
                assert_eq!(v.encoding.as_deref(), Some("zstd"));
                assert!(v.zstd_magic);
            }
            if mode == "multipart" {
                assert!(v.content_type.starts_with("multipart/form-data;"));
            }
        }
    }
    let unknown: Vec<_> = observed
        .iter()
        .filter(|v| v.key == "independent-js-unknown")
        .collect();
    assert_eq!(unknown.len(), 2);
    assert!(
        unknown
            .iter()
            .all(|v| v.status >= 400 && !v.exposes_answers)
    );
    let expected_png = std::fs::read(&png).unwrap();
    {
        let images = supplier_state.images.lock().unwrap();
        assert_eq!(images.len(), 2);
        assert!(images.iter().all(|v| v == &expected_png));
    }
    let rows:Vec<(String,i64,i64,i64,i64,Value)>=sqlx::query_as("SELECT state,reserved,settled,input_tokens,output_tokens,price FROM core_charging.requests WHERE key_id=$1 ORDER BY state").bind(key_id).fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 7);
    assert_eq!(rows.iter().filter(|r| r.0 == "settled").count(), 6);
    assert_eq!(rows.iter().filter(|r| r.0 == "reconcile").count(), 1);
    for (state, reserved, settled, input, output, price) in &rows {
        assert_eq!(*reserved, 1377);
        assert_eq!(price["input_per_million"], 21000);
        assert_eq!(price["output_per_million"], 0);
        if state == "settled" {
            assert_eq!((*settled, *input, *output), (3, 120, 456789));
        } else {
            assert_eq!(*settled, 0);
        }
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (8605, 1377)
    );
    let quota = key_quota_snapshot(&mut pool.acquire().await.unwrap(), key_id)
        .await
        .unwrap();
    assert_eq!(quota, (1395, 8605, false));
    let before = rows.len();
    let http = reqwest::Client::new();
    let catalog = http
        .get(format!("{}/v1/models", core.base))
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(catalog.status(), StatusCode::OK);
    let catalog: Value = catalog.json().await.unwrap();
    assert_eq!(catalog["object"], "list");
    assert!(
        catalog["data"].is_array() && catalog.get("models").is_none(),
        "the retained catalog is OpenAI-shaped, not native TypeSafe Models"
    );
    for secret in [&session, "synthetic-invalid-key"] {
        let response = http
            .post(format!("{}/v1/systemone", core.base))
            .bearer_auth(secret)
            .json(&json!({"model":"jev-1.13.0","state":"hello","questions":{"n":{"type":"noul"}}}))
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
    }
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits) VALUES($1,'independent-key','allowed',TRUE,ARRAY['another-model'])").bind(key_id).execute(&pool).await.unwrap();
    let response = http
        .post(format!("{}/v1/systemone", core.base))
        .bearer_auth(&key)
        .json(&json!({"model":"jev-1.13.0","state":"hello","questions":{"n":{"type":"noul"}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    sqlx::query("UPDATE core_identity.api_key_settings SET model_limits_enabled=FALSE WHERE credential_id=$1").bind(key_id).execute(&pool).await.unwrap();
    let authorized = identity.authorize(&key).await.unwrap();
    assert_eq!(
        authorized.credential_kind,
        lmm_core::identity::CredentialKind::ApiKey
    );
    assert_eq!(authorized.credential_id, key_id);
    let tariff_core = serve(
        build_gateway(
            &pool,
            &format!("{}/v1", supplier_server.base),
            key_id,
            100000,
        )
        .router(),
    )
    .await;
    let response = http
        .post(format!("{}/v1/systemone", tariff_core.base))
        .bearer_auth(&key)
        .json(&json!({"model":"jev-1.13.0","state":"hello","questions":{"n":{"type":"noul"}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "nonzero tariff must reject, never silently become free"
    );
    let tariff_error: Value = response.json().await.unwrap();
    assert_eq!(tariff_error["error"]["code"], "unsupported_feature");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&pool)
            .await
            .unwrap(),
        before as i64
    );
    assert_eq!(supplier_state.hits.load(Ordering::SeqCst), 7);
    assert_eq!(ledger.balance(account).await.unwrap(), balance);
}

#[sqlx::test(migrations = false)]
async fn systemone_independent_supplier_errors_and_disconnected_caller_never_replay_money(
    pool: PgPool,
) {
    let (_identity, ledger, session, key, key_id, account) =
        model_fixture::fixture(&pool, 10000).await;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'independent-systemone-adverse')").execute(&pool).await.unwrap();
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    charging
        .set_budget(
            &session,
            &Budget {
                scope: "key".into(),
                account_id: None,
                user_id: None,
                key_id: Some(key_id),
                period: "lifetime".into(),
                anchor: 0,
                seconds: 0,
                limit: 10000,
            },
        )
        .await
        .unwrap();
    let supplier_state = Supplier {
        hits: Arc::new(AtomicUsize::new(0)),
        seen: Arc::new(Mutex::new(Vec::new())),
        images: Arc::new(Mutex::new(Vec::new())),
    };
    let supplier_server = serve(
        Router::new()
            .route("/v1/systemone", post(supplier))
            .with_state(supplier_state.clone()),
    )
    .await;
    let gateway = build_gateway(&pool, &format!("{}/v1", supplier_server.base), key_id, 0);
    let core = serve(gateway.router()).await;
    let http = reqwest::Client::new();
    for (index, scenario) in ["status-429", "status-529", "malformed-success"]
        .into_iter()
        .enumerate()
    {
        let operation = format!("independent-adverse-{scenario}");
        let body = json!({"model":"jev-1.13.0","state":{"scenario":scenario},"questions":{"n":{"type":"noul"}}});
        let response = http
            .post(format!("{}/v1/systemone", core.base))
            .bearer_auth(&key)
            .header("idempotency-key", &operation)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
        let error: Value = response.json().await.unwrap();
        assert!(error.get("answers").is_none());
        assert_eq!(supplier_state.hits.load(Ordering::SeqCst), index + 1);
        let retry = http
            .post(format!("{}/v1/systemone", core.base))
            .bearer_auth(&key)
            .header("idempotency-key", &operation)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(retry.status(), StatusCode::CONFLICT);
        assert_eq!(supplier_state.hits.load(Ordering::SeqCst), index + 1);
        let balance = ledger.balance(account).await.unwrap();
        assert_eq!(
            (balance.available_units, balance.reserved_units),
            (10000 - 1377 * (index as i64 + 1), 1377 * (index as i64 + 1))
        );
    }
    let base = core.base.clone();
    let caller_key = key.clone();
    let caller = tokio::spawn(async move {
        reqwest::Client::new().post(format!("{base}/v1/systemone")).bearer_auth(caller_key).header("idempotency-key","independent-caller-disconnect").json(&json!({"model":"jev-1.13.0","state":{"scenario":"caller-disconnect"},"questions":{"n":{"type":"noul"}}})).send().await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while supplier_state.hits.load(Ordering::SeqCst) < 4 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "caller was never actually dispatched"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    loop {
        let settled: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_charging.requests WHERE key_id=$1 AND state='settled'",
        )
        .bind(key_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        if settled == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "disconnected caller stranded dispatched money"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let states: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT state,reserved,settled FROM core_charging.requests WHERE key_id=$1 ORDER BY state",
    )
    .bind(key_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(states.len(), 4);
    assert_eq!(
        states
            .iter()
            .filter(|r| r.0 == "reconcile" && r.1 == 1377 && r.2 == 0)
            .count(),
        3
    );
    assert_eq!(
        states
            .iter()
            .filter(|r| r.0 == "settled" && r.2 == 3)
            .count(),
        1
    );
    let retry=http.post(format!("{}/v1/systemone",core.base)).bearer_auth(&key).header("idempotency-key","independent-caller-disconnect").json(&json!({"model":"jev-1.13.0","state":{"scenario":"caller-disconnect"},"questions":{"n":{"type":"noul"}}})).send().await.unwrap();
    assert_eq!(retry.status(), StatusCode::CONFLICT);
    assert_eq!(supplier_state.hits.load(Ordering::SeqCst), 4);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (5866, 4131)
    );
    assert_eq!(
        key_quota_snapshot(&mut pool.acquire().await.unwrap(), key_id)
            .await
            .unwrap(),
        (4134, 5866, false)
    );
}

/// R1 behavior regression without SDK dependencies. Both HTTP hops use actual
/// numeric-loopback TCP; the provider response and image are synthetic. This is
/// not an official supplier result or independent acceptance sign-off.
#[sqlx::test(migrations = false)]
async fn systemone_prepared_json_and_zstd_multipart_tcp_capture_once(pool: PgPool) {
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[derive(Clone)]
    struct WireCapture {
        pool: PgPool,
        seen: Arc<Mutex<Vec<(String, String, Vec<u8>)>>>,
    }
    async fn capture_wire(
        State(state): State<WireCapture>,
        request: Request<Body>,
        next: Next,
    ) -> Response {
        assert!(!request.headers().contains_key("content-encoding"));
        let content_type = request.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_owned();
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, 2 * 1024 * 1024).await.unwrap();
        let wire_sha256 = format!("{:x}", Sha256::digest(&body));
        // A separate connection must observe the committed hold and BOTH
        // matching proof records before the synthetic supplier can reply.
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT r.id FROM core_charging.requests r \
             JOIN core_charging.usage_events p ON p.request_id=r.id AND p.event_id='reservation-contract:v1' \
             JOIN core_charging.usage_events a ON a.request_id=r.id AND a.event_id='reservation-admission:v1' \
             JOIN core_charging.ledger_holds h ON h.request_id=r.id \
             WHERE p.detail->>'wire_sha256'=$1 AND r.state='streaming' \
             AND r.reserved=1377 AND r.settled=0 AND h.reserved=1377 AND h.captured=0 AND NOT h.closed \
             AND p.detail->>'prepared_sha256'=a.detail->>'prepared_sha256' \
             AND p.detail->>'admission_spec_sha256'=a.detail->>'admission_spec_sha256' \
             AND p.detail->>'frozen_price_sha256'=a.detail->>'frozen_price_sha256'",
        )
        .bind(wire_sha256)
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(ids.len(), 1, "actual wire lacks one durable prepared hold");
        state
            .seen
            .lock()
            .unwrap()
            .push((ids[0].clone(), content_type, body.to_vec()));
        next.run(Request::from_parts(parts, Body::from(body))).await
    }

    let (_identity, ledger, _session, key, key_id, account) =
        model_fixture::fixture(&pool, 10000).await;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-r1-systemone-prepared')")
        .execute(&pool).await.unwrap();
    let supplier_state = Supplier {
        hits: Arc::new(AtomicUsize::new(0)),
        seen: Arc::new(Mutex::new(Vec::new())),
        images: Arc::new(Mutex::new(Vec::new())),
    };
    let capture = WireCapture {
        pool: pool.clone(),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let supplier_server = serve(
        Router::new()
            .route("/v1/systemone", post(supplier))
            .with_state(supplier_state.clone())
            .layer(middleware::from_fn_with_state(
                capture.clone(),
                capture_wire,
            )),
    )
    .await;
    let gateway = build_gateway(&pool, &format!("{}/v1", supplier_server.base), key_id, 0);
    let core = serve(gateway.router()).await;
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    // The same valid 280x280 synthetic PNG used by the native unit fixture.
    let png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAARgAAAEYCAYAAACHjumMAAACqklEQVR42u3UMQ0AAAzDsPIn3YKY9tkQciQFeBIJAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQxGAsBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBuBlknX59IehThQAAAABJRU5ErkJggg==").unwrap();
    for (index, multipart) in [false, true].into_iter().enumerate() {
        let operation = if multipart {
            "synthetic-r1-systemone-multipart"
        } else {
            "synthetic-r1-systemone-json"
        };
        let mut wire = json!({
            "model":"jev-1.14.0",
            "state":{"scenario":operation,"decimal":0.3},
            "questions":{"n":{"type":"noul"}},
        });
        let (content_type, body) = if multipart {
            wire["state"]["camera"] = json!("synthetic-camera-reference");
            wire["assets"] = json!({"synthetic-camera-reference":{"part":"camera"}});
            wire["questions"]["b"] = json!({"type":"box"});
            let form = reqwest::multipart::Form::new()
                .part(
                    "request",
                    reqwest::multipart::Part::text(wire.to_string())
                        .mime_str("application/json")
                        .unwrap(),
                )
                .part(
                    "camera",
                    reqwest::multipart::Part::bytes(png.clone())
                        .file_name("synthetic-camera.png")
                        .mime_str("image/png")
                        .unwrap(),
                );
            let mut request = http
                .post(format!("{}/v1/systemone", core.base))
                .multipart(form)
                .build()
                .unwrap();
            let content_type = request.headers()["content-type"]
                .to_str()
                .unwrap()
                .to_owned();
            let body = request.body_mut().take().unwrap();
            let raw = to_bytes(Body::new(body), 2 * 1024 * 1024).await.unwrap();
            (
                content_type,
                zstd::stream::encode_all(raw.as_ref(), 3).unwrap(),
            )
        } else {
            (
                "application/json".into(),
                serde_json::to_vec(&wire).unwrap(),
            )
        };
        if multipart {
            assert!(body.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]));
        }
        let make_request = || {
            let mut request = http
                .post(format!("{}/v1/systemone", core.base))
                .bearer_auth(&key)
                .header("idempotency-key", operation)
                .header("content-type", &content_type)
                .body(body.clone());
            if multipart {
                request = request
                    .header("content-encoding", "zstd")
                    .header("typesafe-danger-zone-images", "true");
            }
            request
        };
        let response = make_request().send().await.unwrap();
        let status = response.status();
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let output: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{output}");
        assert_eq!(
            output["usage"],
            json!({"input_tokens":120,"output_tokens":456789})
        );
        assert_eq!(output["answers"]["n"]["noul"], json!(0.3));
        if multipart {
            assert_eq!(output["answers"]["b"]["type"], "box");
        }
        let (seen_id, seen_type, actual_wire) = capture.seen.lock().unwrap()[index].clone();
        assert_eq!(seen_id, id);
        assert_eq!(seen_type.starts_with("multipart/form-data;"), multipart);
        assert_eq!(supplier_state.seen.lock().unwrap()[index], wire);
        let (state, reserved, settled, input, out, price, proof, marker, fingerprint, spec):
            (String, i64, i64, i64, i64, Value, Value, Value, String, String) = sqlx::query_as(
                "SELECT r.state,r.reserved,r.settled,r.input_tokens,r.output_tokens,r.price,p.detail,a.detail,encode(r.fingerprint,'hex'),encode(r.spec_digest,'hex') \
                 FROM core_charging.requests r \
                 JOIN core_charging.usage_events p ON p.request_id=r.id AND p.event_id='reservation-contract:v1' \
                 JOIN core_charging.usage_events a ON a.request_id=r.id AND a.event_id='reservation-admission:v1' WHERE r.id=$1",
            ).bind(&id).fetch_one(&pool).await.unwrap();
        assert_eq!(
            (state.as_str(), reserved, settled, input, out),
            ("settled", 1377, 3, 120, 456789)
        );
        assert_eq!(price["input_per_million"], 21000);
        assert_eq!(price["cached_per_million"], 0);
        assert_eq!(price["output_per_million"], 0);
        let typed_price: lmm_core::billing::charging::Price =
            serde_json::from_value(price).unwrap();
        assert_eq!(proof["mode"], "systemone_native_v1");
        assert_eq!(
            proof["wire_sha256"],
            format!("{:x}", Sha256::digest(&actual_wire))
        );
        assert_eq!(proof["request_fingerprint"], fingerprint);
        assert_eq!(proof["admission_spec_sha256"], spec);
        assert_eq!(
            proof["frozen_price_sha256"],
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&typed_price).unwrap())
            )
        );
        assert_eq!(
            proof["bounds"],
            json!({"model_input_max":65536,"input":65536,"requested_output":0,"output":u32::MAX})
        );
        assert_eq!(proof["semantics"]["output"], "unpriced_systemone_metadata");
        assert_eq!(
            proof["binding"]["generation_url"],
            format!("{}/v1/systemone", supplier_server.base)
        );
        assert_eq!(
            proof["binding"]["credential_ref"],
            json!({"id":7,"revision":1})
        );
        for field in [
            "prepared_sha256",
            "admission_spec_sha256",
            "frozen_price_sha256",
        ] {
            assert_eq!(marker[field], proof[field]);
        }
        let hold: (i64, i64, bool) = sqlx::query_as(
            "SELECT reserved,captured,closed FROM core_charging.ledger_holds WHERE request_id=$1",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(hold, (1377, 3, true));
        let finals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'")
            .bind(&id).fetch_one(&pool).await.unwrap();
        assert_eq!(finals, 1);
        let balance = ledger.balance(account).await.unwrap();
        assert_eq!(
            (balance.available_units, balance.reserved_units),
            (10000 - 3 * (index as i64 + 1), 0)
        );
        let replay = make_request().send().await.unwrap();
        assert_eq!(replay.status(), StatusCode::CONFLICT);
        assert_eq!(supplier_state.hits.load(Ordering::SeqCst), index + 1);
        assert_eq!(capture.seen.lock().unwrap().len(), index + 1);
        assert_eq!(ledger.balance(account).await.unwrap(), balance);
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM core_charging.requests), \
             (SELECT count(*) FROM core_billing.ledger_journals WHERE kind='capture'), \
             (SELECT count(*) FROM core_charging.ledger_commands)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            counts,
            (index as i64 + 1, index as i64 + 1, 2 * (index as i64 + 1))
        );
    }
    assert_eq!(*supplier_state.images.lock().unwrap(), vec![png]);
}
