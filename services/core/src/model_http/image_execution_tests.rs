//! Real Core schema, identity, native keys and ledger, with a counting loopback
//! image supplier. No external provider request or handwritten schema.
use super::*;
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        ledger::{Action, Ledger, Outcome as LedgerOutcome, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    funding::FundingPolicy,
    identity::IdentityStore,
    model_http::{
        image_price::{ExactRatio, ImageCountBasis, ImagePriceRule},
        image_provider::{ImageMediaType, ImageMultipartInput, ImageUpload},
    },
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::post,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

#[derive(Clone, Copy)]
pub(super) enum Reply {
    OneImage,
    SlowOneImage,
    Empty,
    Truncated,
}
#[derive(Clone)]
struct SupplierState {
    hits: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
    reply: Reply,
}
pub(super) struct Supplier {
    base_url: String,
    pub(super) hits: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
    task: JoinHandle<()>,
}
impl Drop for Supplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn supplier_reply(
    State(state): State<SupplierState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        headers["authorization"],
        "Bearer synthetic-image-provider-key"
    );
    assert_eq!(input["model"], "supplier-image");
    assert_eq!(input["size"], "1024x1024");
    assert_eq!(input["quality"], "standard");
    if matches!(state.reply, Reply::SlowOneImage) {
        // Release only after the test observes dispatch and drain, eliminating
        // dependence on scheduler speed or an arbitrary response delay.
        state.release.acquire().await.unwrap().forget();
    }
    match state.reply {
        // A completed, syntactically valid provider image payload is the image
        // usage evidence. The relay does not infer image cost from text tokens.
        Reply::OneImage | Reply::SlowOneImage => {
            Json(json!({"created":1,"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lZkAAAAASUVORK5CYII="}]})).into_response()
        }
        Reply::Empty => Json(json!({"created":1,"data":[]})).into_response(),
        Reply::Truncated => {
            let stream = tokio_stream::iter([
                Ok::<Bytes, std::io::Error>(Bytes::from_static(b"{\"created\":1,\"data\":[")),
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "synthetic truncated image body",
                )),
            ]);
            (
                [("content-type", "application/json")],
                Body::from_stream(stream),
            )
                .into_response()
        }
    }
}
async fn supplier_edit_reply(
    State(state): State<SupplierState>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        headers["authorization"],
        "Bearer synthetic-image-provider-key"
    );
    let mut images = 0usize;
    let mut model = None;
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_owned();
        if matches!(name.as_str(), "image" | "image[]") {
            assert_eq!(field.content_type(), Some("image/png"));
            let bytes = field.bytes().await.unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            images += 1;
        } else if name == "model" {
            model = Some(field.text().await.unwrap());
        }
    }
    assert!(images > 0);
    assert_eq!(model.as_deref(), Some("supplier-image"));
    if matches!(state.reply, Reply::Truncated) {
        return (axum::http::StatusCode::OK, "{\"data\":[").into_response();
    }
    Json(json!({"created":1,"data":[{"b64_json":"AQID"}]})).into_response()
}
impl Supplier {
    pub(super) async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let app = Router::new()
            .route("/v1/images/generations", post(supplier_reply))
            .route("/v1/images/edits", post(supplier_edit_reply))
            .with_state(SupplierState {
                hits: hits.clone(),
                release: release.clone(),
                reply,
            });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base_url,
            hits,
            release,
            task,
        }
    }
}
struct Fixture {
    pool: PgPool,
    ledger: Ledger,
    secret: String,
    key_id: i64,
    account: i64,
}
async fn fixture(pool: PgPool) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::check(&pool).await.unwrap();
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
    let developer: bool =
        sqlx::query_scalar("SELECT granted FROM core_identity.developer_access WHERE user_id=7")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(developer);
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-image-pg-test')")
        .execute(&pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    let credited = ledger
        .execute(&LedgerRequest {
            scope: "image-api-pg-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in disposable PostgreSQL".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(10000).unwrap(),
                payment_reference: "synthetic:image-api-pg-test".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(credited, LedgerOutcome::Posted { .. }));
    Fixture {
        pool,
        ledger,
        secret: key.secret,
        key_id: key.id,
        account: actor.owner_account_id,
    }
}
fn image_service(f: &Fixture, supplier: &Supplier) -> ImageService {
    image_service_for_pool(&f.pool, supplier)
}
pub(super) fn image_service_for_pool(pool: &PgPool, supplier: &Supplier) -> ImageService {
    let rules = [ImageEndpoint::Generations, ImageEndpoint::Edits]
        .into_iter()
        .map(|endpoint| ImagePriceRule {
            upstream_id: 1,
            vendor: "openai".into(),
            billing_group: "allowed".into(),
            upstream_model: "supplier-image".into(),
            semantic_model: "dall-e-3".into(),
            endpoint,
            size: "1024x1024".into(),
            quality: "standard".into(),
            // 0.004 USD * 500000 credits/USD = 2000 credits per image.
            usd_micros_per_image: ExactRatio {
                numerator: 4000,
                denominator: 1,
            },
            group_multiplier: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            max_n: 4,
            count_basis: ImageCountBasis::ReturnedPayloads,
            max_budget: Credits::try_from(10000).unwrap(),
            source_revision: "synthetic-image-tariff-v1".into(),
        })
        .collect();
    ImageService::new(
        pool.clone(),
        Charging::new(pool.clone(), Arc::new(TransactionalLedger)),
        ImagePriceConfig { revision: 1, rules },
        vec![ProviderSpec {
            upstream_id: 1,
            vendor: "openai".into(),
            upstream_model: "supplier-image".into(),
            base_url: supplier.base_url.clone(),
            secret: "synthetic-image-provider-key".into(),
            local_http: true,
        }],
        &Limits {
            total_timeout: Duration::from_secs(3),
            ..Limits::default()
        },
    )
    .unwrap()
}
pub(super) fn input(n: u32) -> DrawingInput {
    DrawingInput {
        prompt: "a synthetic image".into(),
        model: "dall-e-3".into(),
        group: String::new(),
        size: "1024x1024".into(),
        quality: "standard".into(),
        n,
    }
}
fn groups() -> Vec<String> {
    vec!["allowed".into()]
}
fn operation(key: &str) -> String {
    format!("image:{key}")
}
async fn no_charges(f: &Fixture) {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(funds.available_units, 10000);
    assert_eq!(funds.reserved_units, 0);
}

#[sqlx::test(migrations = false)]
async fn image_api_actual_payload_count_settlement_replay_and_conflict(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    let key = format!("{:064x}", 1);
    let receipt = service
        .execute_api(&f.secret, &key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(receipt.operation_id, operation(&key));
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.reservation_credits, 6000);
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        funds.available_units, 8000,
        "the unused two-image reservation must be returned"
    );
    assert_eq!(funds.reserved_units, 0);
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.key_id, Some(f.key_id));
    assert_eq!(charge.owner_account_id, f.account);
    assert_eq!(charge.payer_account_id, f.account);
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.reserved, 6000);
    assert_eq!(charge.settled, 2000);
    assert_eq!(charge.usage.image.unwrap().output_images, 1);
    assert_eq!(
        (charge.usage.input, charge.usage.cached, charge.usage.output),
        (0, 0, 0)
    );
    let repeated = service
        .execute_api(&f.secret, &key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(repeated.output, receipt.output);
    assert_eq!(repeated.charged_credits, Some(2000));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let mut changed = input(3);
    changed.prompt.push_str(" changed");
    assert_eq!(
        service
            .execute_api(&f.secret, &key, &groups(), changed, None, None)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE key_id=$1")
            .bind(f.key_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        8000
    );
}

#[sqlx::test(migrations = false)]
async fn image_api_empty_or_truncated_provider_retains_full_hold_without_retry(pool: PgPool) {
    let f = fixture(pool).await;
    for (position, reply) in [Reply::Empty, Reply::Truncated].into_iter().enumerate() {
        let supplier = Supplier::start(reply).await;
        let service = image_service(&f, &supplier);
        let key = format!("{:064x}", position + 10);
        assert_eq!(
            service
                .execute_api(&f.secret, &key, &groups(), input(2), None, None)
                .await
                .unwrap_err(),
            Error::Pending
        );
        let charge = service
            .charging
            .resolve(&operation(&key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge.state, "reconcile");
        assert_eq!(charge.reserved, 4000);
        assert_eq!(charge.settled, 0);
        assert!(charge.usage.image.is_none());
        let funds = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(funds.reserved_units, (position as i64 + 1) * 4000);
        assert_eq!(funds.available_units, 10000 - (position as i64 + 1) * 4000);
        let repeated = service
            .execute_api(&f.secret, &key, &groups(), input(2), None, None)
            .await
            .unwrap();
        assert_ne!(repeated.state, "completed");
        assert!(repeated.output.is_none());
        assert!(repeated.charged_credits.is_none());
        assert_eq!(
            supplier.hits.load(Ordering::SeqCst),
            1,
            "unknown provider outcome must never trigger another POST"
        );
        assert!(
            service
                .charging
                .recover_expired(10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service.recover_receipts(10).await.unwrap(),
            0,
            "missing payload is not settlement evidence"
        );
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().reserved_units,
            (position as i64 + 1) * 4000
        );
    }
}

#[sqlx::test(migrations = false)]
async fn image_api_real_key_model_group_peer_policies_reject_before_hold(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits,allow_ips) VALUES($1,'synthetic-image-restricted-key','allowed',TRUE,ARRAY['other-model'],ARRAY['127.0.0.1/32'::inet])")
        .bind(f.key_id).execute(&f.pool).await.unwrap();
    let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let forbidden = Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
    let first = format!("{:064x}", 20);
    assert_eq!(
        service
            .execute_api(&f.secret, &first, &groups(), input(1), None, loopback)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_charges(&f).await;
    sqlx::query("UPDATE core_identity.api_key_settings SET model_limits=ARRAY['dall-e-3'],group_name='different-group' WHERE credential_id=$1")
        .bind(f.key_id).execute(&f.pool).await.unwrap();
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 21),
                &groups(),
                input(1),
                None,
                loopback
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_charges(&f).await;
    sqlx::query(
        "UPDATE core_identity.api_key_settings SET group_name='allowed' WHERE credential_id=$1",
    )
    .bind(f.key_id)
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 22),
                &groups(),
                input(1),
                None,
                forbidden
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 23),
                &groups(),
                input(1),
                None,
                None
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_charges(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let completed = service
        .execute_api(
            &f.secret,
            &format!("{:064x}", 24),
            &groups(),
            input(1),
            None,
            loopback,
        )
        .await
        .unwrap();
    assert_eq!(completed.charged_credits, Some(2000));
    assert_eq!(
        service
            .query_api_with_peer(&f.secret, &completed.operation_id, forbidden)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .query_api_with_peer(&f.secret, &completed.operation_id, None)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test(migrations = false)]
async fn image_api_malformed_edit_upload_rejects_before_hold_and_post(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    let edits = ImageMultipartInput {
        images: vec![ImageUpload {
            bytes: b"not a PNG image".to_vec(),
            media_type: ImageMediaType::Png,
        }],
        mask: None,
    };
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 30),
                &groups(),
                input(1),
                Some(edits),
                None
            )
            .await
            .unwrap_err(),
        Error::Invalid
    );
    no_charges(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test(migrations = false)]
async fn image_api_disconnected_caller_drain_preserves_worker_and_settlement(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::SlowOneImage).await;
    let service = image_service(&f, &supplier);
    let request_key = format!("{:064x}", 40);
    let caller_service = service.clone();
    let caller_secret = f.secret.clone();
    let caller_key = request_key.clone();
    let caller = tokio::spawn(async move {
        caller_service
            .execute_api(&caller_secret, &caller_key, &groups(), input(3), None, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let hits = supplier.hits.load(Ordering::SeqCst);
            assert!(hits <= 1);
            if hits == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the provider must observe the original POST before caller disconnect");
    let dispatched = service
        .charging
        .resolve(&operation(&request_key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dispatched.state, "streaming");
    assert_eq!(dispatched.reserved, 6000);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(funds.available_units, 4000);
    assert_eq!(funds.reserved_units, 6000);
    // Abort the HTTP caller's awaiting future, never the service-owned worker.
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(service.drain(Duration::from_millis(10)).await, 1);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    supplier.release.add_permits(1);
    let receipt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipt = match service
                .query_api_with_peer(&f.secret, &operation(&request_key), None)
                .await
            {
                Ok(receipt) => receipt,
                Err(Error::Retry) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(error) => panic!("receipt query failed during detached completion: {error:?}"),
            };
            if receipt.state == "completed" {
                break receipt;
            }
            assert!(
                receipt.output.is_none(),
                "unsettled output must not be delivered"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect(
        "the detached worker must persist and settle the original image after caller disconnect",
    );
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(funds.available_units, 8000);
    assert_eq!(funds.reserved_units, 0);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(service.drain(Duration::from_secs(1)).await, 0);
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 41),
                &groups(),
                input(1),
                None,
                None
            )
            .await
            .unwrap_err(),
        Error::Pending
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let charges: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(charges, 1);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        8000
    );
}

#[path = "../../tests/common/runtime.rs"]
pub(super) mod runtime_role;

#[sqlx::test(migrations = false)]
async fn image_api_restricted_runtime_role_generates_and_settles_real_funds(pool: PgPool) {
    let f = fixture(pool).await;
    let role = runtime_role::RuntimeRole::create(&f.pool).await;
    for capability in ["core_charging", "core_model", "core_image"] {
        role.optional(capability).await;
    }
    crate::runtime_database::check(
        &role.pool,
        crate::runtime_database::RequiredCapabilities {
            models: true,
            images: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&role.pool, &supplier);
    service.ready().await.unwrap();
    use tower::ServiceExt;
    let readiness =
        crate::model_http::image_internal::internal_router(service.clone(), vec![b'r'; 32])
            .unwrap();
    for (body, status) in [
        ("{}", axum::http::StatusCode::OK),
        ("{\"ready\":true}", axum::http::StatusCode::BAD_REQUEST),
    ] {
        let response = readiness
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/internal/models/images/v1/ready")
                    .header("authorization", format!("Bearer {}", "r".repeat(32)))
                    .header("x-lmm-protocol", "2")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status.is_success() {
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                json!({"protocol":2,"service":"images","ready":true})
            );
        }
    }
    let request_key = format!("{:064x}", 50);
    let receipt = service
        .execute_api(&f.secret, &request_key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.reservation_credits, 6000);
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.key_id, Some(f.key_id));
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.reserved, 6000);
    assert_eq!(charge.settled, 2000);
    let ledger = Ledger::from_pool(role.pool.clone());
    let balance = ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let repeated = service
        .execute_api(&f.secret, &request_key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(repeated.output, receipt.output);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(service.drain(Duration::from_secs(1)).await, 0);
    assert_eq!(service.ready().await, Err(Error::Pending));
    drop(ledger);
    drop(service);
    role.cleanup().await;
}
