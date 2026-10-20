//! Real Core product and Go module-worker OS recovery with a counted TCP Stripe
//! fixture. This target does not claim lmm-extensions main/provider deployment.
#![cfg(unix)]
#[path = "payments_process.rs"]
mod shared;

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path as ApiPath, State},
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use hmac::{Hmac, Mac};
use lmm_core::{billing::ledger::Ledger, internal_rpc::pb};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const SECRET: &str = "whsec_local_synthetic_endpoint_only";
const CASE: &str = "real_refund_dispatch_response_loss_restarts_core_and_go_worker";
fn signed(event: Value) -> Value {
    let payload = serde_json::to_string(&event).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(payload.as_bytes());
    let signature: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect();
    json!({"payload":payload,"signature":format!("t={now},v1={signature}")})
}
fn refund_event(
    intent: &pb::PaymentIntent,
    form: &BTreeMap<String, String>,
    key: &str,
    id: &str,
    status: &str,
) -> Value {
    json!({"id":id,"type":if status=="pending"{"refund.created"}else{"refund.updated"},"livemode":false,"request":if status=="pending"{json!({"id":"req_refund_synthetic","idempotency_key":key})}else{Value::Null},"data":{"object":{"id":"re_refund_os_001","object":"refund","status":status,"amount":40,"currency":"usd","livemode":false,"payment_intent":format!("pi_{}",intent.intent_id),"metadata":{"lmm_core_intent_id":intent.intent_id.to_string(),"lmm_merchant":intent.merchant,"lmm_order_id":format!("top_{}",intent.intent_id),"lmm_refund_id":form["metadata[lmm_refund_id]"],"lmm_hold_receipt_id":form["metadata[lmm_hold_receipt_id]"]}}}})
}
#[derive(Clone)]
struct Provider {
    core: PgPool,
    go: PgPool,
    intent: pb::PaymentIntent,
    root: PathBuf,
    posts: Arc<AtomicUsize>,
    events: Arc<Mutex<BTreeMap<String, Value>>>,
}
fn response(value: Value) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&value).unwrap()))
        .unwrap()
}
async fn provider(
    State(state): State<Provider>,
    ApiPath(path): ApiPath<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let i = &state.intent;
    let order = format!("top_{}", i.intent_id);
    if method == Method::GET && path == "v1/account" {
        return response(json!({"id":i.merchant}));
    }
    if method == Method::GET && path == format!("v1/payment_intents/pi_{}", i.intent_id) {
        return response(
            json!({"id":format!("pi_{}",i.intent_id),"object":"payment_intent","status":"succeeded","amount":100,"amount_received":100,"currency":"usd","livemode":false,"metadata":{"lmm_core_intent_id":i.intent_id.to_string(),"lmm_merchant":i.merchant,"lmm_order_id":order}}),
        );
    }
    if method == Method::POST && path == "v1/checkout/sessions" {
        return response(
            json!({"id":"cs_refund_os","object":"checkout.session","mode":"payment","client_reference_id":order,"metadata":{"lmm_order_id":order,"lmm_core_intent_id":i.intent_id.to_string(),"lmm_merchant":i.merchant},"amount_total":100,"currency":"usd","livemode":false,"payment_intent":format!("pi_{}",i.intent_id),"url":"https://checkout.stripe.com/c/pay/refund-os-fixture"}),
        );
    }
    if method == Method::GET && path.starts_with("v1/events/") {
        let id = path.trim_start_matches("v1/events/");
        let events = state.events.lock().await;
        return response(
            events
                .get(id)
                .expect("only pre-signed synthetic event IDs may be queried")
                .clone(),
        );
    }
    if method == Method::POST && path == "v1/refunds" {
        let count = state.posts.fetch_add(1, Ordering::SeqCst) + 1;
        let query = std::str::from_utf8(&body).unwrap();
        let url = reqwest::Url::parse(&format!("http://127.0.0.1/?{query}")).unwrap();
        let form: BTreeMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let key = headers.get("idempotency-key").unwrap().to_str().unwrap();
        let hold = &form["metadata[lmm_hold_receipt_id]"];
        assert_eq!(form["payment_intent"], format!("pi_{}", i.intent_id));
        assert_eq!(form["amount"], "40");
        let registered:Option<String>=sqlx::query_scalar("SELECT d.receipt_id FROM core_payments.refund_dispatches d JOIN core_payments.refund_holds h ON h.receipt_id=d.hold_receipt_id WHERE d.intent_id=$1 AND d.hold_receipt_id=$2 AND d.operation_key=$3 AND h.state='held'").bind(i.intent_id).bind(hold).bind(key).fetch_optional(&state.core).await.unwrap();
        let registered = registered.expect("provider POST preceded Core dispatch COMMIT");
        let snapshot: Value = sqlx::query_scalar("SELECT snapshot FROM payment_orders WHERE id=$1")
            .bind(&order)
            .fetch_one(&state.go)
            .await
            .unwrap();
        let refund_id = &form["metadata[lmm_refund_id]"];
        assert_eq!(
            snapshot["refunds"][refund_id]["dispatch_receipt"], registered,
            "Go did not persist the exact Core registration ACK"
        );
        assert_eq!(snapshot["refunds"][refund_id]["dispatch"], "unknown");
        assert!(
            snapshot["refunds"][refund_id]["dispatch_receipt"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "Core registration ACK was not persisted before POST"
        );
        let created = refund_event(i, &form, key, "evt_refund_os_created", "pending");
        let failed = refund_event(i, &form, key, "evt_refund_os_failed", "failed");
        let late = refund_event(i, &form, key, "evt_refund_os_late_success", "succeeded");
        let mut conflict = created.clone();
        conflict["data"]["object"]["metadata"]["synthetic_changed_payload"] = json!("changed");
        for event in [&created, &failed, &late] {
            state
                .events
                .lock()
                .await
                .insert(event["id"].as_str().unwrap().to_owned(), event.clone());
        }
        shared::private(&state.root,&format!("post-{count}.json"),&serde_json::to_vec(&json!({"posts":count,"hold":hold,"key":key,"refund_id":refund_id,"go_snapshot":snapshot,"core_dispatch_commit_witness":true,"go_registration_ack_witness":true,"evidence":{"created":signed(created),"failed":signed(failed),"late_success":signed(late),"conflicting_id":signed(conflict)}})).unwrap());
        // Receive and count the real TCP POST but never return its response. Parent
        // kills both spawned OS processes before the worker's bounded call expires.
        return std::future::pending().await;
    }
    panic!("unexpected synthetic provider route: {method} {path}");
}
fn worker(
    binary: &Path,
    expected: &str,
    fixture: &Path,
    root: &Path,
    phase: &str,
) -> shared::Process {
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(binary).unwrap())),
        expected
    );
    let log = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join(format!("worker-{phase}.log")))
        .unwrap();
    let child = Command::new(binary)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GOMAXPROCS", "2")
        .env("LMM_REFUND_PROCESS_FIXTURE", fixture)
        .env("LMM_REFUND_PROCESS_PHASE", phase)
        .args([
            "-test.run",
            "^TestRefundProcessWorkerFixture$",
            "-test.v",
            "-test.timeout=160s",
        ])
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut process = shared::Process(
        child,
        Value::Null,
        root.to_owned(),
        format!("worker-{phase}"),
    );
    process.1 = shared::observation(process.0.id(), expected);
    shared::private(
        root,
        &format!("worker-{phase}.identity.json"),
        &serde_json::to_vec(&process.1).unwrap(),
    );
    process
}
async fn marker(path: &Path, process: &mut shared::Process, timeout: u64) -> Value {
    let end = Instant::now() + Duration::from_secs(timeout);
    loop {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "worker exited before checkpoint; inspect private log"
        );
        if let Ok(bytes) = fs::read(path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return value;
            }
        }
        assert!(
            Instant::now() < end,
            "refund checkpoint deadline: {}",
            path.file_name().unwrap().to_string_lossy()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn balances(pool: &PgPool, account: i64, available: i64, reserved: i64) {
    let b = Ledger::from_pool(pool.clone())
        .balance(account)
        .await
        .unwrap();
    assert_eq!(b.available_units, available);
    assert_eq!(b.reserved_units, reserved);
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) j").fetch_one(pool).await.unwrap();
    assert_eq!(unbalanced, 0);
}

#[sqlx::test(migrations = false)]
async fn real_refund_dispatch_response_loss_restarts_core_and_go_worker(pool: PgPool) {
    let f = shared::prepare_payment_fixture(&pool).await;
    let shared::PaymentFixture {
        root,
        account,
        intent,
        role,
        go_owner,
        core_bin,
        core_sha,
        go_bin: _,
        go_sha: _,
        boot_bin,
        boot_sha,
        fixture,
        core_config,
        go_config: _,
        client,
        core_endpoint,
        go_endpoint: _,
        core,
        login: _,
    } = f;
    // The shared bootstrap initially starts a top-up-only Core. Reap that setup
    // process and start an explicitly refund-enabled runtime before the scenario.
    shared::crash(core);
    let permission = root.join("payments.toml");
    let permission_text = fs::read_to_string(&permission).unwrap();
    fs::write(
        &permission,
        permission_text.replace(
            "credit_evidence=true",
            "credit_evidence=true\nrefund_user=true\nrefund_evidence=true",
        ),
    )
    .unwrap();
    let mut core = shared::spawn(
        &core_bin,
        &core_sha,
        &core_config,
        &root,
        "core-refund-initial.log",
    );
    shared::ready(&client, &core_endpoint, &mut core).await;
    let initial: Value = serde_json::from_slice(&fs::read(&fixture).unwrap()).unwrap();
    let mut events = BTreeMap::new();
    for event in initial["events"].as_array().unwrap() {
        let parsed: Value = serde_json::from_str(event["payload"].as_str().unwrap()).unwrap();
        events.insert(parsed["id"].as_str().unwrap().into(), parsed);
    }
    let state = Provider {
        core: pool.clone(),
        go: go_owner.clone(),
        intent: intent.clone(),
        root: root.clone(),
        posts: Arc::new(AtomicUsize::new(0)),
        events: Arc::new(Mutex::new(events)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_url = format!("http://{}", listener.local_addr().unwrap());
    let provider_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/{*path}", axum::routing::any(provider))
                .with_state(provider_state),
        )
        .await
        .unwrap();
    });
    let mut worker_fixture = initial;
    worker_fixture["provider_url"] = json!(provider_url);
    worker_fixture["marker_dir"] = json!(root);
    worker_fixture["evidence_file"] = json!(root.join("recovery-evidence.json"));
    let worker_fixture = shared::private(
        &root,
        "refund-worker.json",
        &serde_json::to_vec(&worker_fixture).unwrap(),
    );
    let mut go = worker(&boot_bin, &boot_sha, &worker_fixture, &root, "dispatch");
    let observed = marker(&root.join("post-1.json"), &mut go, 30).await;
    assert_eq!(observed["core_dispatch_commit_witness"], true);
    assert_eq!(observed["go_registration_ack_witness"], true);
    balances(&pool, account, 300_000, 200_000).await;
    let before_core = core.0.id();
    let before_go = go.0.id();
    shared::record_identity(&core, "refund-before-crash");
    shared::record_identity(&go, "refund-before-crash");
    shared::crash(go);
    shared::crash(core);
    assert!(!Path::new(&format!("/proc/{before_core}")).exists());
    assert!(!Path::new(&format!("/proc/{before_go}")).exists());
    let mut core = shared::spawn(
        &core_bin,
        &core_sha,
        &core_config,
        &root,
        "core-refund-restart.log",
    );
    shared::ready(&client, &core_endpoint, &mut core).await;
    let mut go = worker(&boot_bin, &boot_sha, &worker_fixture, &root, "recover");
    assert_ne!(core.0.id(), before_core);
    assert_ne!(go.0.id(), before_go);
    // Honor the original persisted lease; do not rewrite timestamps/attempts.
    let unknown = marker(&root.join("unknown.json"), &mut go, 60).await;
    assert_eq!(unknown["pid"], go.0.id());
    assert_eq!(unknown["job"]["attempts"], 2);
    assert_eq!(
        unknown["job"]["error_code"],
        "payment_operation_unsupported"
    );
    let original_job = &observed["go_snapshot"]["jobs"][observed["key"].as_str().unwrap()];
    assert_eq!(
        unknown["job"]["first_attempt"],
        original_job["first_attempt"]
    );
    let lease = chrono::DateTime::parse_from_rfc3339(original_job["lease_until"].as_str().unwrap())
        .unwrap();
    assert!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            >= lease.timestamp().try_into().unwrap(),
        "original lease was bypassed"
    );

    assert_eq!(
        state.posts.load(Ordering::SeqCst),
        1,
        "UNKNOWN caused another provider POST"
    );
    assert_eq!(unknown["submit_key"], observed["key"]);
    balances(&pool, account, 300_000, 200_000).await;
    shared::private(
        &root,
        "recovery-evidence.json",
        &serde_json::to_vec(&observed["evidence"]).unwrap(),
    );
    let settled = marker(&root.join("settled.json"), &mut go, 30).await;
    assert_eq!(settled["duplicate_events_checked"], true);
    assert_eq!(settled["conflicting_event_id_denied"], true);
    assert_eq!(settled["success_after_release_denied"], true);
    assert_eq!(settled["submit_key"], observed["key"]);
    assert_eq!(state.posts.load(Ordering::SeqCst), 1);
    balances(&pool, account, 500_000, 0).await;
    let releases: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_payments.receipts WHERE method='release'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(releases, 1);
    let commits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_payments.receipts WHERE method='commit'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(commits, 0);
    let bindings: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_payments.refund_dispatch_bindings")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bindings, 1);
    let after_core = core.0.id();
    let after_go = go.0.id();
    shared::record_identity(&core, "refund-recovered");
    shared::record_identity(&go, "refund-recovered");
    shared::crash(go);
    shared::crash(core);
    server.abort();
    let _ = server.await;
    shared::private(&root,"refund-result.json",&serde_json::to_vec(&json!({"case":CASE,"result":"PASS","scope":"real Core main and Go payment module worker; synthetic counted TCP provider; not Go main acceptance","core_os_restart":true,"go_module_worker_os_restart":true,"provider_posts":1,"core_ack_before_provider_post":true,"unknown_original_key_preserved":true,"refund_releases":releases,"refund_commits":commits,"dispatch_bindings":bindings,"duplicate_event_replay_checked":true,"signed_event_id_payload_conflict_denied":true,"success_after_release_denied":true,"journal_balance_checked":true,"product_pids":[before_core,before_go,after_core,after_go],"children_reaped":true})).unwrap());
    // Preserve all failure state automatically. Successful outer fixture cleanup
    // is performed by the independent orchestrator after checking this receipt.
    go_owner.close().await;
    role.pool.close().await;
    println!("REFUND_PROCESS_EVIDENCE {}", root.display());
}
