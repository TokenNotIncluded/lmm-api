//! Native System One HTTP and real disposable Core PostgreSQL money tests.
//! The only supplier is a counting numeric-loopback HTTP server.
use super::*;
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        ledger::{Action, Ledger, Request as LedgerRequest},
    },
    funding::FundingPolicy,
    identity::{BrowserAuth, BrowserRegistration, IdentityStore},
    routing::*,
};
use axum::{
    Router as HttpRouter,
    extract::{ConnectInfo, FromRequest, Multipart, State},
    http::Request as HttpRequest,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;
#[path = "../../tests/common/runtime.rs"]
mod runtime;

#[derive(Clone, Copy)]
enum Reply {
    Valid,
    MissingUsage,
    NegativeUsage,
}
struct Seen {
    headers: HeaderMap,
    wire: serde_json::Value,
    files: Vec<(String, Vec<u8>)>,
}
#[derive(Clone)]
struct SupplierState {
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
    reply: Reply,
}
struct Supplier {
    base: String,
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
    task: JoinHandle<()>,
}
impl Drop for Supplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn supplier(State(state): State<SupplierState>, request: HttpRequest<Body>) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    let headers = request.headers().clone();
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer synthetic-systemone-provider-key"
    );
    let mut files = Vec::new();
    let wire = if headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("multipart/form-data")
    {
        let mut multipart = Multipart::from_request(request, &()).await.unwrap();
        let first = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(first.name(), Some("request"));
        assert_eq!(first.content_type(), Some("application/json"));
        let wire =
            serde_json::from_slice::<serde_json::Value>(&first.bytes().await.unwrap()).unwrap();
        while let Some(field) = multipart.next_field().await.unwrap() {
            let name = field.name().unwrap().to_owned();
            assert!(field.file_name().is_some());
            files.push((name, field.bytes().await.unwrap().to_vec()));
        }
        wire
    } else {
        serde_json::from_slice::<serde_json::Value>(
            &to_bytes(request.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(wire["model"], "jev-provider");
    assert!(wire.get("messages").is_none());
    let mut answers = serde_json::Map::new();
    for (name, question) in wire["questions"].as_object().unwrap() {
        answers.insert(name.clone(),match question["type"].as_str().unwrap() {
            "noul"=>json!({"type":"noul","noul":0.3}),
            "choice"=>json!({"type":"choice","choice":"y","confidence":0.9,"probabilities":{"x":0.1,"y":0.9}}),
            "score"=>json!({"type":"score","score":0.8,"confidence":0.9,"legend":{"0":"low","1":"high"},"probabilities":{"0":0.2,"1":0.8}}),
            "box"=>json!({"type":"box","box":{"top":0.1,"bottom":0.9,"left":0.2,"right":0.7},"confidence":null}),
            _=>panic!("unexpected native question"),
        });
    }
    state.seen.lock().unwrap().push(Seen {
        headers,
        wire,
        files,
    });
    let mut response = json!({"model":"jev-1.14.0","answers":answers});
    match state.reply {
        Reply::Valid => response["usage"] = json!({"input_tokens":120,"output_tokens":456789}),
        Reply::MissingUsage => {}
        Reply::NegativeUsage => response["usage"] = json!({"input_tokens":-1,"output_tokens":12}),
    }
    Json(response).into_response()
}
impl Supplier {
    async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = HttpRouter::new()
            .route("/v1/systemone", post(supplier))
            .with_state(SupplierState {
                hits: hits.clone(),
                seen: seen.clone(),
                reply,
            });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base,
            hits,
            seen,
            task,
        }
    }
}
struct Fixture {
    pool: PgPool,
    identity: IdentityStore,
    session: String,
    key: String,
    key_id: i64,
    account: i64,
    ledger: Ledger,
}
async fn fixture(pool: PgPool) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let actor = identity.authorize(&session.secret).await.unwrap();
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
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-systemone-pg')").execute(&pool).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT granted FROM core_identity.developer_access WHERE user_id=7"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "systemone-pg".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in disposable PostgreSQL".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(10000).unwrap(),
                payment_reference: "synthetic:systemone".into(),
            },
        })
        .await
        .unwrap();
    Fixture {
        pool,
        identity,
        session: session.secret,
        key: key.secret,
        key_id: key.id,
        account: actor.owner_account_id,
        ledger,
    }
}
fn gateway(pool: &PgPool, supplier: &Supplier, key_id: i64, input_ceiling: i64) -> ModelGateway {
    let prices=PriceSnapshot::compile(serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"jev-native","group":"allowed","speed":"standard","rates":{"input":42000,"output":0,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap()).unwrap();
    let routes = RoutingConfig {
        version: 1,
        groups: vec![ModelGroup {
            name: "allowed".into(),
            enabled: true,
        }],
        upstreams: vec![UpstreamConfig {
            id: 1,
            endpoint: Endpoint::local_loopback(supplier.base.clone()).unwrap(),
            protocol: Protocol::SystemOne,
            credential: CredentialRef { id: 7, revision: 1 },
            enabled: true,
        }],
        routes: vec![ModelRoute {
            id: 1,
            model: "jev-native".into(),
            group: "allowed".into(),
            enabled: true,
            speeds: vec![Speed::Standard],
            targets: vec![RouteTarget {
                id: 1,
                upstream_id: 1,
                upstream_model: "jev-provider".into(),
                enabled: true,
                priority: 0,
                weight: 1,
                speeds: vec![Speed::Standard],
                payload: PayloadPolicy::Adapt,
                timeout_ms: 5000,
            }],
            failover: FailoverPolicy {
                max_attempts: 1,
                retry_on: vec![],
                allow_replay_after_send: false,
            },
        }],
        aliases: vec![],
    };
    ModelGateway::new(
        pool.clone(),
        Arc::new(crate::routing::Router::new(routes, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-systemone-provider-key".into(),
        }],
        BTreeMap::from([(key_id, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: input_ceiling,
            lease_seconds: 30,
        },
    )
    .unwrap()
}
fn text_request() -> serde_json::Value {
    json!({"model":"jev-native","state":{"message":"hello"},"questions":{"n":{"type":"noul","instructions":"Is this hello?"},"c":{"type":"choice","criteria":{"x":"no","y":"yes"}},"s":{"type":"score","criteria":["low","high"]}}})
}
fn four_questions() -> serde_json::Value {
    let mut v = text_request();
    v["state"]["camera"] = json!("unique-camera-reference");
    v["assets"] =
        json!({"unique-camera-reference":{"format":"png","data_base64":STANDARD.encode(png())}});
    v["questions"]["b"] = json!({"type":"box"});
    v
}
fn http_request(
    secret: &str,
    key: &str,
    path: &str,
    body: serde_json::Value,
    images: bool,
    long: bool,
    peer: Option<IpAddr>,
) -> HttpRequest<Body> {
    let mut builder = HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", key);
    if images {
        builder = builder.header("typesafe-danger-zone-images", "true");
    }
    if long {
        builder = builder.header("typesafe-danger-zone-long-context", "true");
    }
    let mut request = builder.body(Body::from(body.to_string())).unwrap();
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(peer, 1234)));
    }
    request
}
async fn call(
    gateway: &ModelGateway,
    request: HttpRequest<Body>,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    (status, headers, value)
}
async fn untouched(f: &Fixture) {
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    let b = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((b.available_units, b.reserved_units), (10000, 0));
}

#[sqlx::test(migrations = false)]
async fn systemone_native_pg_four_questions_real_input_only_charge_and_no_replay(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::Valid).await;
    let gateway = gateway(&f.pool, &supplier, f.key_id, 65536);
    let request = four_questions();
    let (status, headers, output) = call(
        &gateway,
        http_request(
            &f.key,
            "native-four",
            "/v1/systemone",
            request.clone(),
            true,
            false,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output["model"], "jev-1.14.0");
    assert_eq!(output["answers"].as_object().unwrap().len(), 4);
    assert_eq!(output["answers"]["b"]["box"]["left"], 0.2);
    assert_eq!(
        output["answers"]["s"]["legend"],
        json!({"0":"low","1":"high"})
    );
    assert_eq!(output["usage"]["output_tokens"], 456789);
    assert!(output.get("choices").is_none());
    let id = headers["x-request-id"].to_str().unwrap();
    let charge = gateway.bridge.charging.resolve(id).await.unwrap().unwrap();
    assert_eq!(charge.key_id, Some(f.key_id));
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.reserved, 1377);
    assert_eq!(charge.price.input_per_million, 21000);
    assert_eq!(charge.price.output_per_million, 0);
    assert_eq!(charge.settled, 3);
    let b = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((b.available_units, b.reserved_units), (9997, 0));
    let (status, _, _) = call(
        &gateway,
        http_request(
            &f.key,
            "native-four",
            "/typesafe/v1/systemone",
            request,
            true,
            false,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let seen = supplier.seen.lock().unwrap();
    assert_eq!(seen[0].wire["questions"].as_object().unwrap().len(), 4);
    assert_eq!(seen[0].headers["typesafe-danger-zone-images"], "true");
}
#[sqlx::test(migrations = false)]
async fn systemone_native_pg_bad_usage_retains_original_hold_without_exposing_answers(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    for (index, reply) in [Reply::MissingUsage, Reply::NegativeUsage]
        .into_iter()
        .enumerate()
    {
        let supplier = Supplier::start(reply).await;
        let gateway = gateway(&f.pool, &supplier, f.key_id, 65536);
        let key = format!("bad-usage-{index}");
        let (status, headers, output) = call(
            &gateway,
            http_request(
                &f.key,
                &key,
                "/v1/systemone",
                text_request(),
                false,
                false,
                None,
            ),
        )
        .await;
        assert!(!status.is_success());
        assert!(output.get("answers").is_none());
        assert!(output.get("choices").is_none());
        let charge = gateway
            .bridge
            .charging
            .resolve(headers["x-request-id"].to_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge.state, "reconcile");
        assert_eq!(charge.reserved, 1377);
        assert_eq!(charge.settled, 0);
        let b = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(b.reserved_units, (index as i64 + 1) * 1377);
        assert_eq!(b.available_units, 10000 - (index as i64 + 1) * 1377);
        let (status, _, _) = call(
            &gateway,
            http_request(
                &f.key,
                &key,
                "/v1/systemone",
                text_request(),
                false,
                false,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    }
}
#[sqlx::test(migrations = false)]
async fn systemone_native_pg_current_key_model_group_and_peer_limits_precede_hold(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::Valid).await;
    let gateway = gateway(&f.pool, &supplier, f.key_id, 65536);
    let peer = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits,allow_ips) VALUES($1,'synthetic-systemone-key','allowed',TRUE,ARRAY['other-model'],ARRAY['127.0.0.1/32'::inet])").bind(f.key_id).execute(&f.pool).await.unwrap();
    for (index, (model, group, ip)) in [
        ("other-model", "allowed", peer),
        ("jev-native", "different-group", peer),
        (
            "jev-native",
            "allowed",
            Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
        ),
        ("jev-native", "allowed", None),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query("UPDATE core_identity.api_key_settings SET model_limits=ARRAY[$2],group_name=$3 WHERE credential_id=$1").bind(f.key_id).bind(model).bind(group).execute(&f.pool).await.unwrap();
        let (status, _, _) = call(
            &gateway,
            http_request(
                &f.key,
                &format!("blocked-{index}"),
                "/v1/systemone",
                text_request(),
                false,
                false,
                ip,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        untouched(&f).await;
    }
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
}
#[sqlx::test(migrations = false)]
async fn systemone_native_pg_real_session_is_rejected_before_hold(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::Valid).await;
    let gateway = gateway(&f.pool, &supplier, f.key_id, 65536);
    let (status, _, _) = call(
        &gateway,
        http_request(
            &f.session,
            "session-denied",
            "/v1/systemone",
            text_request(),
            false,
            false,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    untouched(&f).await;
    let oauth = real_pi_access(&f, &gateway).await;
    let (status, _, _) = call(
        &gateway,
        http_request(
            &oauth,
            "issued-pi-denied",
            "/v1/systemone",
            text_request(),
            false,
            false,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    untouched(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
}
#[sqlx::test(migrations = false)]
async fn systemone_native_pg_restricted_role_uses_real_ledger_and_permissions(pool: PgPool) {
    let f = fixture(pool).await;
    let role = runtime::RuntimeRole::create(&f.pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    crate::runtime_database::check(
        &role.pool,
        crate::runtime_database::RequiredCapabilities {
            models: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let supplier = Supplier::start(Reply::Valid).await;
    let gateway = gateway(&role.pool, &supplier, f.key_id, 65536);
    let (status, _, output) = call(
        &gateway,
        http_request(
            &f.key,
            "restricted-systemone",
            "/v1/systemone",
            text_request(),
            false,
            false,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    let b = Ledger::from_pool(role.pool.clone())
        .balance(f.account)
        .await
        .unwrap();
    assert_eq!((b.available_units, b.reserved_units), (9997, 0));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    drop(gateway);
    role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn systemone_native_pg_zstd_multipart_box_long_context_and_insufficient_contract(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::Valid).await;
    for (ceiling, long) in [(65535, false), (65536, true)] {
        let gateway = gateway(&f.pool, &supplier, f.key_id, ceiling);
        let (status, _, _) = call(
            &gateway,
            http_request(
                &f.key,
                &format!("insufficient-{ceiling}"),
                "/v1/systemone",
                text_request(),
                false,
                long,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        untouched(&f).await;
    }
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let gateway = gateway(&f.pool, &supplier, f.key_id, 131072);
    let mut wire = four_questions();
    wire["assets"]["unique-camera-reference"] = json!({"part":"camera"});
    let form = reqwest::multipart::Form::new()
        .part(
            "request",
            reqwest::multipart::Part::text(wire.to_string())
                .mime_str("application/json")
                .unwrap(),
        )
        .part(
            "camera",
            reqwest::multipart::Part::bytes(png())
                .file_name("camera.png")
                .mime_str("image/png")
                .unwrap(),
        );
    let mut unsent = reqwest::Client::new()
        .post("http://127.0.0.1:1/unsent-systemone-fixture")
        .multipart(form)
        .build()
        .unwrap();
    let headers = unsent.headers().clone();
    let body = unsent.body_mut().take().unwrap();
    let raw = to_bytes(Body::new(body), 32 * 1024 * 1024).await.unwrap();
    let encoded = zstd::stream::encode_all(raw.as_ref(), 3).unwrap();
    let encoded_len = encoded.len();
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri("/typesafe/v1/systemone")
        .body(Body::from(encoded))
        .unwrap();
    *request.headers_mut() = headers;
    request
        .headers_mut()
        .insert(header::CONTENT_ENCODING, "zstd".parse().unwrap());
    request.headers_mut().insert(
        header::CONTENT_LENGTH,
        encoded_len.to_string().parse().unwrap(),
    );
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", f.key).parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("idempotency-key", "multipart-systemone".parse().unwrap());
    request
        .headers_mut()
        .insert("typesafe-danger-zone-images", "true".parse().unwrap());
    request
        .headers_mut()
        .insert("typesafe-danger-zone-long-context", "true".parse().unwrap());
    let (status, headers, output) = call(&gateway, request).await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output["answers"]["b"]["type"], "box");
    let charge = gateway
        .bridge
        .charging
        .resolve(headers["x-request-id"].to_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.reserved, 2753);
    assert_eq!(charge.settled, 3);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let seen = supplier.seen.lock().unwrap();
    assert!(!seen[0].headers.contains_key(header::CONTENT_ENCODING));
    assert_eq!(seen[0].headers["typesafe-danger-zone-images"], "true");
    assert_eq!(seen[0].headers["typesafe-danger-zone-long-context"], "true");
    assert_eq!(seen[0].files.len(), 1);
    assert_eq!(seen[0].files[0], ("camera".into(), png()));
    assert_eq!(
        seen[0].wire["assets"]["unique-camera-reference"]["part"],
        "camera"
    );
}

fn png() -> Vec<u8> {
    STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAARgAAAEYCAYAAACHjumMAAACqklEQVR42u3UMQ0AAAzDsPIn3YKY9tkQciQFeBIJAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQxGAsBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBuBlknX59IehThQAAAABJRU5ErkJggg==").unwrap()
}

// Issue a real bounded Pi token through browser identity, Core human consent and
// PKCE. This verifies an existing OAuth authority is not silently widened to the
// new native System One route merely because its bearer prefix is recognized.
async fn real_pi_access(f: &Fixture, gateway: &ModelGateway) -> String {
    use crate::identity::grants::{AuthorizationRequest, LmmIssuer, LmmProfile, TokenRequest};
    const ISSUER: &str = "https://systemone-pi.example.test";
    const REDIRECT: &str = "http://127.0.0.1:38423/oauth/lmm/callback";
    const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-._~";
    let identity = f
        .identity
        .clone()
        .with_registration_enabled(true)
        .with_dashboard_routing(gateway.bridge.routing.clone());
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "systemone-pi",
            password: "test passphrase",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login("systemone-pi", "test passphrase", &Default::default())
        .await
        .unwrap();
    let actor = identity
        .authorize(bundle.data["access_token"].as_str().unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES($1,TRUE,'synthetic-systemone-pi')").bind(actor.user_id).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,'allowed',TRUE,'synthetic-systemone-pi')").bind(actor.user_id).execute(&f.pool).await.unwrap();
    let profile = LmmProfile::new(ISSUER, vec!["allowed".into()])
        .unwrap()
        .with_pi_budget_limit(1000)
        .unwrap();
    let issuer = LmmIssuer::open(identity, profile, 900, 86400)
        .await
        .unwrap();
    let page = issuer
        .begin(AuthorizationRequest {
            response_type: "code".into(),
            client_id: "lmm-pi".into(),
            redirect_uri: REDIRECT.into(),
            scope: "catalog:read balance:read usage:read models:invoke".into(),
            resource: format!("{ISSUER}/api/oauth2"),
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER)),
            code_challenge_method: "S256".into(),
            state: "synthetic-systemone-pi".into(),
        })
        .await
        .unwrap();
    let page = issuer
        .continue_authorization(&page.binding, &page.csrf, Some(&bundle.refresh_token))
        .await
        .unwrap();
    let redirect = issuer
        .consent_with_budget(
            &page.binding,
            &page.csrf,
            Some(&bundle.refresh_token),
            true,
            Some(100),
        )
        .await
        .unwrap();
    let uri = reqwest::Url::parse(&redirect.uri).unwrap();
    let code = uri
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned();
    let token = issuer
        .token(TokenRequest::Code {
            client_id: "lmm-pi".into(),
            code,
            redirect: REDIRECT.into(),
            verifier: VERIFIER.into(),
        })
        .await
        .unwrap();
    let mut transaction = f.pool.begin().await.unwrap();
    let source = issuer
        .authorize_model_in_transaction(&mut transaction, &token.access_token, None, None)
        .await
        .unwrap();
    assert_eq!(source.user_id(), actor.user_id);
    transaction.commit().await.unwrap();
    token.access_token
}
