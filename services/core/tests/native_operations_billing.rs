//! Real Core HTTP + isolated PostgreSQL holds, evidence, settlement and recovery.
//! Suppliers are bounded local protocol fixtures, not live-provider acceptance.
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
        ChargePolicy, ExternalModelContext, ModelGateway, ProviderPolicy, UpstreamCredential,
    },
    relay::Limits,
    routing::{self, CredentialRef, Endpoint, PriceConfig, PriceSnapshot, Protocol, RoutingConfig},
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Search,
    Fetch,
    Embedding,
}
impl Kind {
    fn provider(self) -> &'static str {
        match self {
            Self::Search => "deep_seek_search_v1",
            Self::Fetch => "deep_seek_fetch_v1",
            Self::Embedding => "ali_separate_input_v1",
        }
    }
    fn protocol(self) -> Protocol {
        match self {
            Self::Search | Self::Fetch => Protocol::DeepSeekTools,
            Self::Embedding => Protocol::AliMultimodalEmbedding,
        }
    }
    fn hold(self) -> i64 {
        if self == Self::Embedding { 129 } else { 500 }
    }
    fn cost(self) -> i64 {
        if self == Self::Embedding { 4 } else { 500 }
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Success,
    Empty,
    Unknown,
    Error,
}
struct Supplier {
    endpoint: String,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Supplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Supplier {
    async fn start(kind: Kind, mode: Mode) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/native", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let handler = move |uri: Uri, headers: HeaderMap, Json(request): Json<Value>| {
            let hits = observed.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                assert_eq!(headers["authorization"], "Bearer native-test-upstream-key");
                if kind == Kind::Embedding {
                    assert_eq!(request["model"], "provider-model");
                    assert_eq!(request["parameters"]["dimension"], 4);
                    assert_eq!(request["parameters"]["enable_fusion"], true);
                    assert_eq!(request["input"]["contents"].as_array().unwrap().len(), 2);
                    assert!(request.get("max_input_tokens").is_none());
                } else {
                    assert!(request.get("model").is_none());
                    assert!(uri.path().ends_with(if kind == Kind::Search {
                        "/search"
                    } else {
                        "/fetch"
                    }));
                }
                if matches!(mode, Mode::Error) {
                    return (StatusCode::NOT_FOUND, Json(json!({"error":{"message":"private upstream failure text","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}))).into_response();
                }
                let value = match kind {
                    Kind::Search if matches!(mode, Mode::Unknown) => json!({}),
                    Kind::Search if matches!(mode, Mode::Empty) => json!({"results":[]}),
                    Kind::Search => {
                        json!({"results":[{"rerank_score":2.4,"title":"Synthetic","url":"https://example.test/result","snippet":"summary","summary":"summary","content":"","site_name":null,"site_icon":null,"date":null,"authority":1}]})
                    }
                    Kind::Fetch if matches!(mode, Mode::Unknown) => {
                        json!({"title":"missing content"})
                    }
                    Kind::Fetch => {
                        json!({"title":"Synthetic","content":if matches!(mode, Mode::Empty) { "" } else { "# Synthetic page" }})
                    }
                    Kind::Embedding => {
                        let usage = match mode {
                            Mode::Unknown => json!({"input_tokens":10}),
                            Mode::Empty => {
                                json!({"input_tokens":0,"image_tokens":0,"total_tokens":0})
                            }
                            _ => json!({"input_tokens":10,"image_tokens":20,"total_tokens":30}),
                        };
                        json!({"request_id":"synthetic-native-vector","output":{"embeddings":[{"index":0,"type":"fusion","embedding":[0.125,-0.25,0.5,1.0]}]},"usage":usage})
                    }
                };
                Json(value).into_response()
            }
        };
        let app = Router::new()
            .route("/native/v1/tools/{operation}", post(handler.clone()))
            .route(
                "/native/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding",
                post(handler),
            );
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

fn gateway(
    pool: &PgPool,
    supplier: &Supplier,
    key_id: i64,
    kind: Kind,
    version: u64,
) -> ModelGateway {
    let protocol = kind.protocol();
    let mut routes: RoutingConfig = serde_json::from_value(json!({
        "version":version,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/native","protocol":protocol,"credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"public-model","group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":1,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
    })).unwrap();
    routes.upstreams[0].endpoint = Endpoint::local_loopback(supplier.endpoint.clone()).unwrap();
    let mut rule = json!({"id":1,"model":"public-model","group":"allowed","speed":"standard","rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000});
    let mut context = json!({"upstream_id":1,"upstream_model":"provider-model","protocol":protocol,"endpoint":supplier.endpoint,"max_input_tokens":if kind==Kind::Embedding {1000} else {0},"max_output_tokens":0,"contract_revision":"synthetic-exact-native-v1","missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true});
    if kind == Kind::Embedding {
        rule["embedding"] = json!({"revision":1,"provider":kind.provider(),"currency":"USD","text_usd_per_million":{"numerator":version,"denominator":10},"image_usd_per_million":{"numerator":258*version,"denominator":1000}});
        context["embedding"] = json!({"provider":kind.provider(),"contract_revision":"synthetic-modal-partition-v1","max_contents":20,"max_images":5,"max_videos":1,"max_text_bytes":32000,"max_inline_image_bytes":5242880,"default_dimensions":4,"dimensions":[4],"supports_dimension_parameter":true,"supports_fusion":true,"allow_remote_media":true});
    } else {
        rule["search"] = json!({"revision":1,"provider":kind.provider(),"unit":"call","currency":"USD","usd_per_unit":{"numerator":version,"denominator":1000}});
        context["search"] = json!({"provider":kind.provider(),"limits":{"calls":1},"contract_revision":"synthetic-one-native-call-v1"});
    }
    let prices: PriceConfig =
        serde_json::from_value(json!({"version":version,"rules":[rule]})).unwrap();
    let context: ExternalModelContext = serde_json::from_value(context).unwrap();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            routing::Router::new(routes, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "native-test-upstream-key".into(),
        }],
        BTreeMap::from([(key_id, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 30,
        },
        ProviderPolicy::new(vec![context], 1000).unwrap(),
    )
    .unwrap()
}
fn input(kind: Kind, native: bool) -> (&'static str, Value) {
    match kind {
        Kind::Search => (
            "/v1/tools/search",
            json!({"model":"public-model","query":"synthetic query","count":2}),
        ),
        Kind::Fetch => (
            "/v1/tools/fetch",
            json!({"model":"public-model","url":"https://example.test/page"}),
        ),
        Kind::Embedding if native => (
            "/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding",
            json!({"model":"public-model","input":{"contents":[{"text":"text"},{"image":"https://example.test/image.png"}]},"parameters":{"enable_fusion":true}}),
        ),
        Kind::Embedding => (
            "/v1/embeddings",
            json!({"model":"public-model","input":[{"type":"text","text":"text"},{"type":"image_url","image_url":{"url":"https://example.test/image.png"}}],"enable_fusion":true}),
        ),
    }
}
async fn invoke(
    gateway: &ModelGateway,
    key: &str,
    id: &str,
    path: &str,
    value: Value,
) -> (StatusCode, Value, Option<String>) {
    let response = gateway
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
        .unwrap();
    let status = response.status();
    let id = response
        .headers()
        .get("x-request-id")
        .map(|v| v.to_str().unwrap().to_owned());
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap(), id)
}
fn charging(pool: &PgPool) -> Charging {
    Charging::new(pool.clone(), Arc::new(TransactionalLedger))
}
async fn balanced(pool: &PgPool) {
    let count:i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e").fetch_one(pool).await.unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
async fn native_tools_and_both_embedding_envelopes_settle_exact_frozen_evidence(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10000).await;
    for kind in [Kind::Search, Kind::Fetch, Kind::Embedding] {
        let supplier = Supplier::start(kind, Mode::Success).await;
        let service = gateway(&pool, &supplier, key_id, kind, 1);
        let clients = if kind == Kind::Embedding {
            vec![false, true]
        } else {
            vec![false]
        };
        for native in clients {
            let (path, value) = input(kind, native);
            let (status, body, id) = invoke(
                &service,
                &key,
                &format!("native-{kind:?}-{native}"),
                path,
                value,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let receipt = charging(&pool)
                .resolve(&id.unwrap())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (receipt.reserved, receipt.settled, receipt.state.as_str()),
                (kind.hold(), kind.cost(), "settled")
            );
            if kind == Kind::Embedding {
                assert_eq!(
                    (
                        receipt.usage.input,
                        receipt.usage.cached,
                        receipt.usage.output
                    ),
                    (30, 0, 0)
                );
                let measured = receipt.usage.embedding.unwrap();
                assert_eq!(
                    (
                        measured.text_tokens,
                        measured.image_tokens,
                        measured.total_input_tokens
                    ),
                    (Some(10), Some(20), Some(30))
                );
                assert_eq!(
                    receipt.price.embedding.as_ref().unwrap().input_token_limit,
                    1000
                );
                if !native {
                    assert_eq!(body["usage"]["prompt_tokens"], 30);
                    assert_eq!(body["model"], "public-model");
                }
            } else {
                assert_eq!(receipt.usage.search.unwrap().calls, Some(1));
            }
            assert_eq!(receipt.price.cost(receipt.usage).unwrap(), kind.cost());
        }
        assert_eq!(
            supplier.hits.load(Ordering::SeqCst),
            if kind == Kind::Embedding { 2 } else { 1 }
        );
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (8992, 0));
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn explicit_empty_receipts_cost_zero_but_missing_native_evidence_retains_hold(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10000).await;
    for kind in [Kind::Search, Kind::Fetch, Kind::Embedding] {
        for (mode, label) in [(Mode::Empty, "empty"), (Mode::Unknown, "unknown")] {
            let supplier = Supplier::start(kind, mode).await;
            let service = gateway(&pool, &supplier, key_id, kind, 1);
            let (path, value) = input(kind, false);
            let id = format!("native-{label}-{kind:?}");
            let (status, body, operation) = invoke(&service, &key, &id, path, value.clone()).await;
            let operation = operation.unwrap();
            let receipt = charging(&pool).resolve(&operation).await.unwrap().unwrap();
            assert_eq!(receipt.settled, 0);
            if label == "empty" {
                assert_eq!(status, StatusCode::OK, "{body}");
                assert_eq!(receipt.state, "settled");
            } else {
                assert!(!status.is_success(), "{body}");
                assert_eq!(receipt.state, "reconcile");
                assert_eq!(receipt.reserved, kind.hold());
                let restart = gateway(&pool, &supplier, key_id, kind, 7);
                assert_eq!(restart.reconcile_completed_streams(20).await.unwrap(), 0);
                let duplicate = invoke(&restart, &key, &id, path, value).await;
                assert_eq!(duplicate.0, StatusCode::CONFLICT);
                assert_eq!(duplicate.2, Some(operation));
            }
            assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
        }
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (8871, 1129)
    );
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn native_duplicate_budget_and_auth_rejections_never_repeat_supplier_work(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10000).await;
    let supplier = Supplier::start(Kind::Embedding, Mode::Success).await;
    let service = gateway(&pool, &supplier, key_id, Kind::Embedding, 1);
    let (path, mut value) = input(Kind::Embedding, false);
    value["max_input_tokens"] = json!(999);
    let too_small = invoke(&service, &key, "native-budget", path, value).await;
    assert_eq!(too_small.0, StatusCode::BAD_REQUEST);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let (path, value) = input(Kind::Embedding, false);
    let denied = invoke(
        &service,
        "invalid-synthetic-key",
        "native-unauthorized",
        path,
        value.clone(),
    )
    .await;
    assert_eq!(denied.0, StatusCode::UNAUTHORIZED);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let (one, two) = tokio::join!(
        invoke(&service, &key, "native-concurrent", path, value.clone()),
        invoke(&service, &key, "native-concurrent", path, value.clone())
    );
    assert_eq!(one.2, two.2);
    assert!(
        (one.0 == StatusCode::OK && two.0 == StatusCode::CONFLICT)
            || (two.0 == StatusCode::OK && one.0 == StatusCode::CONFLICT)
    );
    let restarted = gateway(&pool, &supplier, key_id, Kind::Embedding, 20);
    assert_eq!(
        invoke(&restarted, &key, "native-concurrent", path, value)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9996, 0));
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn native_success_and_documented_failed_terminal_recover_once_with_original_outcome(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10000).await;
    sqlx::raw_sql("CREATE FUNCTION public.reject_native_final() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id='final' THEN RAISE EXCEPTION 'synthetic native final failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_native_final BEFORE INSERT ON core_charging.usage_events FOR EACH ROW EXECUTE FUNCTION public.reject_native_final()")
        .execute(&pool).await.unwrap();
    let mut pending = Vec::new();
    for (kind, mode, expected, outcome) in [
        (Kind::Embedding, Mode::Success, 4, "completed"),
        (Kind::Fetch, Mode::Error, 0, "failed"),
    ] {
        let supplier = Supplier::start(kind, mode).await;
        let service = gateway(&pool, &supplier, key_id, kind, 1);
        let (path, value) = input(kind, false);
        let id = format!("native-recover-{kind:?}");
        let (status, body, operation) = invoke(&service, &key, &id, path, value.clone()).await;
        assert!(!status.is_success());
        assert!(!body.to_string().contains("private upstream failure text"));
        let operation = operation.unwrap();
        let receipt = charging(&pool).resolve(&operation).await.unwrap().unwrap();
        assert_eq!(
            (receipt.reserved, receipt.settled, receipt.state.as_str()),
            (kind.hold(), 0, "reconcile")
        );
        let terminal:bool=sqlx::query_scalar("SELECT bool_or(terminal_observed) FROM core_model.output_checkpoints WHERE request_id=$1").bind(&operation).fetch_one(&pool).await.unwrap();
        assert!(terminal);
        pending.push((
            supplier,
            kind,
            expected,
            outcome,
            id,
            path,
            value,
            operation,
            receipt.price,
        ));
    }
    sqlx::query("DROP TRIGGER reject_native_final ON core_charging.usage_events")
        .execute(&pool)
        .await
        .unwrap();
    for (supplier, kind, expected, outcome, id, path, value, operation, price) in pending {
        let restarted = gateway(&pool, &supplier, key_id, kind, 99);
        let (a, b) = tokio::join!(
            restarted.reconcile_completed_streams(20),
            restarted.reconcile_completed_streams(20)
        );
        a.unwrap();
        b.unwrap();
        let restored = charging(&pool).resolve(&operation).await.unwrap().unwrap();
        assert_eq!(
            (restored.settled, restored.state.as_str()),
            (expected, "settled")
        );
        assert_eq!(restored.price, price);
        assert_eq!(restored.outcome.as_deref(), Some(outcome));
        assert_eq!(
            invoke(&restarted, &key, &id, path, value).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9996, 0));
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn native_reserve_failure_stops_before_supplier_work(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 499).await;
    let supplier = Supplier::start(Kind::Search, Mode::Success).await;
    let service = gateway(&pool, &supplier, key_id, Kind::Search, 1);
    let (path, value) = input(Kind::Search, false);
    assert_eq!(
        invoke(&service, &key, "native-no-money", path, value)
            .await
            .0,
        StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (499, 0));
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn documented_native_error_receipts_release_hold_without_exposing_private_errors(
    pool: PgPool,
) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    for kind in [Kind::Search, Kind::Fetch] {
        let supplier = Supplier::start(kind, Mode::Error).await;
        let service = gateway(&pool, &supplier, key_id, kind, 1);
        let (path, value) = input(kind, false);
        let id = format!("native-failed-{kind:?}");
        let (status, body, operation) = invoke(&service, &key, &id, path, value.clone()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["code"], "upstream_error");
        assert!(!body.to_string().contains("private upstream failure text"));
        let receipt = charging(&pool)
            .resolve(&operation.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (receipt.reserved, receipt.settled, receipt.state.as_str()),
            (500, 0, "settled")
        );
        assert_eq!(receipt.usage.search.unwrap().calls, Some(0));
        assert_eq!(receipt.outcome.as_deref(), Some("failed"));
        assert_eq!(
            invoke(&service, &key, &id, path, value).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (1000, 0));
    balanced(&pool).await;
}
