use super::*;
use crate::routing::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;

fn document() -> ReleaseDocument {
    let routing = RoutingConfig {
        version: 1,
        groups: vec![ModelGroup {
            name: "default".into(),
            enabled: true,
        }],
        upstreams: (1..=2)
            .map(|id| UpstreamConfig {
                id,
                endpoint: format!("https://provider-{id}.invalid/v1")
                    .try_into()
                    .unwrap(),
                protocol: Protocol::OpenAiChat,
                credential: CredentialRef {
                    id: id * 100,
                    revision: 1,
                },
                enabled: true,
            })
            .collect(),
        routes: vec![ModelRoute {
            id: 1,
            model: "a".into(),
            group: "default".into(),
            enabled: true,
            speeds: vec![Speed::Standard],
            targets: (1..=2)
                .map(|id| RouteTarget {
                    id,
                    upstream_id: id,
                    upstream_model: "a".into(),
                    enabled: true,
                    priority: if id == 1 { 0 } else { 10 },
                    weight: 1,
                    speeds: vec![Speed::Standard],
                    payload: PayloadPolicy::Adapt,
                    timeout_ms: 30_000,
                })
                .collect(),
            failover: FailoverPolicy::default(),
        }],
        aliases: vec![],
    };
    let pricing = PriceConfig {
        version: 1,
        rules: ["a", "b"]
            .iter()
            .enumerate()
            .map(|(index, model)| PriceRule {
                id: index as u64 + 1,
                model: (*model).into(),
                group: "default".into(),
                speed: Speed::Standard,
                billing_mode: BillingMode::Tokens,
                tiered: None,
                search: None,
                embedding: None,
                rates: TokenRates {
                    input: 7_000_000,
                    output: 14_000_000,
                    cache_read: 0,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: MULTIPLIER_ONE,
                speed_multiplier_ppm: MULTIPLIER_ONE,
            })
            .collect(),
    };
    ReleaseDocument {
        contract_version: CONTRACT_VERSION,
        routing,
        pricing,
    }
}
fn fx_value(value: &str) -> Option<plan::FxSnapshot> {
    Some(plan::FxSnapshot {
        key: "USDExchangeRate".into(),
        revision: "1".into(),
        raw: value.into(),
    })
}
fn request(
    before: &ReleaseDocument,
    source: &ReleaseDocument,
    adjustment: plan::Adjustment,
) -> plan::PricePreviewRequest {
    plan::PricePreviewRequest {
        expected: plan::Expected::from_release(before).unwrap(),
        source_json: serde_json::to_string(&plan::PriceSource::from_release(source)).unwrap(),
        selections: vec![plan::PriceSelection {
            model: "a".into(),
            adjustment,
        }],
    }
}
#[test]
fn fx_is_exact_selected_models_only_and_zero_stays_zero() {
    let before = document();
    let planned = plan::prices(
        &before,
        request(&before, &before, plan::Adjustment::Divide),
        fx_value("7"),
    )
    .unwrap();
    assert_eq!(planned.after.pricing.rules[0].rates.input, 1_000_000);
    assert_eq!(planned.after.pricing.rules[0].rates.cache_read, 0);
    assert_eq!(planned.after.pricing.rules[1], before.pricing.rules[1]);
    assert_eq!(before.pricing.rules[0].rates.input, 7_000_000);
    assert_eq!(planned.after.routing.version, 2);
    assert_eq!(planned.after.pricing.version, 2);
    assert_eq!(planned.fx, fx_value("7"));
}
#[test]
fn keep_imports_source_price_without_fx_and_does_not_flatten_rules() {
    let before = document();
    let mut source = before.clone();
    source.pricing.rules[0].rates.input = 3_000_000;
    let planned = plan::prices(
        &before,
        request(&before, &source, plan::Adjustment::Keep),
        None,
    )
    .unwrap();
    assert_eq!(planned.after.pricing.rules[0].rates.input, 3_000_000);
    assert!(planned.fx.is_none());
    assert_eq!(
        planned.changes[0]["source"]["prices"][0]["amount"]["numerator"],
        "3000000"
    );
}
#[test]
fn missing_invalid_and_inexact_fx_are_rejected_without_rounding() {
    let before = document();
    assert_eq!(
        plan::prices(
            &before,
            request(&before, &before, plan::Adjustment::Multiply),
            None
        )
        .unwrap_err(),
        AdminError::ExchangeRateUnavailable
    );
    for bad in ["0", "-1", "NaN", "1e3", "1000001", "1.1234567890123"] {
        assert_eq!(
            plan::prices(
                &before,
                request(&before, &before, plan::Adjustment::Multiply),
                fx_value(bad)
            )
            .unwrap_err(),
            AdminError::ExchangeRateUnavailable
        );
    }
    assert_eq!(
        plan::prices(
            &before,
            request(&before, &before, plan::Adjustment::Divide),
            fx_value("3")
        )
        .unwrap_err(),
        AdminError::Unrepresentable
    );
}
#[test]
fn native_import_rejects_foreign_currency_and_semantic_changes() {
    let before = document();
    let mut input = request(&before, &before, plan::Adjustment::Multiply);
    input.source_json = input.source_json.replace("\"USD\"", "\"CNY\"");
    assert_eq!(
        plan::prices(&before, input, fx_value("7")).unwrap_err(),
        AdminError::InvalidSource
    );
    let mut source = before.clone();
    source.pricing.rules[0].group_multiplier_ppm = 500_000;
    assert_eq!(
        plan::prices(
            &before,
            request(&before, &source, plan::Adjustment::Multiply),
            fx_value("7")
        )
        .unwrap_err(),
        AdminError::UnsupportedPrice
    );
}
#[test]
fn stale_version_digest_or_duplicate_selection_cannot_preview() {
    let before = document();
    let mut input = request(&before, &before, plan::Adjustment::Divide);
    input.expected.snapshot_sha256 = "0".repeat(64);
    assert_eq!(
        plan::prices(&before, input, fx_value("7")).unwrap_err(),
        AdminError::Conflict
    );
    let mut input = request(&before, &before, plan::Adjustment::Divide);
    input.selections.push(input.selections[0].clone());
    assert_eq!(
        plan::prices(&before, input, fx_value("7")).unwrap_err(),
        AdminError::Invalid
    );
}
#[test]
fn embedding_fractions_scale_without_changing_metering_contract() {
    let mut before = document();
    let rule = &mut before.pricing.rules[0];
    rule.rates = TokenRates {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        request: 0,
    };
    rule.embedding = Some(EmbeddingTariff {
        revision: 1,
        provider: EmbeddingProvider::AliSeparateInputV1,
        currency: EmbeddingCurrency::Usd,
        text_usd_per_million: DecimalRate {
            numerator: 1,
            denominator: 10,
        },
        image_usd_per_million: DecimalRate {
            numerator: 3,
            denominator: 10,
        },
    });
    let planned = plan::prices(
        &before,
        request(&before, &before, plan::Adjustment::Divide),
        fx_value("3"),
    )
    .unwrap();
    let after = planned.after.pricing.rules[0].embedding.unwrap();
    assert_eq!(
        after.text_usd_per_million,
        DecimalRate {
            numerator: 1,
            denominator: 30
        }
    );
    assert_eq!(
        after.image_usd_per_million,
        DecimalRate {
            numerator: 1,
            denominator: 10
        }
    );
    assert!(plan::same_host_contract(&before, &planned.after).unwrap());
    let mut invalid = planned.after.clone();
    invalid.pricing.rules[0]
        .embedding
        .as_mut()
        .unwrap()
        .provider = EmbeddingProvider::AliInclusiveInputV1;
    assert!(!plan::same_host_contract(&before, &invalid).unwrap());
}
#[test]
fn priorities_change_only_one_route_preserve_prices_and_allow_equal_tiers() {
    let before = document();
    let planned = plan::priorities(
        &before,
        plan::PriorityPreviewRequest {
            expected: Expected::from_release(&before).unwrap(),
            route_id: "1".into(),
            changes: vec![plan::PriorityChange {
                target_id: "2".into(),
                priority: 0,
            }],
        },
    )
    .unwrap();
    assert_eq!(planned.after.pricing, before.pricing);
    assert_eq!(planned.after.routing.routes[0].targets[1].priority, 0);
    assert_eq!(planned.changes[0]["before"], 10);
    assert_eq!(planned.changes[0]["after"], 0);
    assert!(plan::same_host_contract(&before, &planned.after).unwrap());
    let mut invalid = planned.after;
    invalid.routing.upstreams[0].endpoint = "https://changed.invalid/v1".try_into().unwrap();
    assert!(!plan::same_host_contract(&before, &invalid).unwrap());
}
#[test]
fn public_catalog_has_no_endpoints_credentials_or_unsafe_numeric_ids() {
    let mut before = document();
    before.routing.upstreams[0].credential.id = 9_007_199_254_740_993;
    let view = plan::catalog(&before, fx_value("7"), true, true).unwrap();
    let text = view.to_string();
    assert!(!text.contains("provider-1.invalid"));
    assert!(!text.contains("credential"));
    assert_eq!(view["routes"][0]["route_id"], "1");
    assert_eq!(
        view["prices"][0]["prices"][0]["amount"]["numerator"],
        "7000000"
    );
}
async fn call(app: &Router, secret: &str, path: &str, input: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .uri(path)
        .header("Authorization", format!("Bearer {secret}"));
    let body = if let Some(input) = input {
        request = request
            .method("POST")
            .header("Content-Type", "application/json");
        Body::from(input.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn setup(
    pool: &PgPool,
) -> (
    crate::test_runtime::RuntimeRole,
    RoutingAdmin,
    identity::IssuedCredential,
) {
    crate::core_schema::init(pool).await.unwrap();
    let identity = identity::IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 6).await.unwrap();
    install_owner_baseline(pool, document()).await.unwrap();
    let runtime = crate::test_runtime::RuntimeRole::create(pool).await;
    runtime.optional("core_console").await;
    runtime.optional("core_events").await;
    let state = RoutingAdmin::new(
        runtime.pool.clone(),
        Arc::new(document().into_router().unwrap()),
    )
    .unwrap();
    (runtime, state, session)
}
#[sqlx::test]
async fn native_publication_is_cas_idempotent_and_durable_on_recreation(pool: PgPool) {
    let (runtime, state, session) = setup(&pool).await;
    let app = state.http_router();
    let before = state.current().unwrap();
    let input = json!({"expected":Expected::from_release(&before).unwrap(),"route_id":"1","changes":[{"target_id":"2","priority":0}]});
    let (status, preview) = call(
        &app,
        &session.secret,
        "/api/core/routing-admin/priorities/preview",
        Some(input.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let (_, other) = call(
        &app,
        &session.secret,
        "/api/core/routing-admin/priorities/preview",
        Some(input),
    )
    .await;
    let commit = json!({"preview_id":preview["data"]["preview_id"],"confirmation_sha256":preview["data"]["confirmation_sha256"],"operation_id":"a".repeat(64)});
    let (status, result) = call(
        &app,
        &session.secret,
        "/api/core/routing-admin/commit",
        Some(commit.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["replayed"], false);
    let (_, result) = call(
        &app,
        &session.secret,
        "/api/core/routing-admin/commit",
        Some(commit),
    )
    .await;
    assert_eq!(result["data"]["replayed"], true);
    let (status,_)=call(&app,&session.secret,"/api/core/routing-admin/commit",Some(json!({"preview_id":other["data"]["preview_id"],"confirmation_sha256":other["data"]["confirmation_sha256"],"operation_id":"b".repeat(64)}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let restarted = RoutingAdmin::new(
        runtime.pool.clone(),
        Arc::new(document().into_router().unwrap()),
    )
    .unwrap();
    restarted.refresh().await.unwrap();
    assert_eq!(restarted.current().unwrap().routing.version, 2);
    assert_eq!(
        restarted.current().unwrap().routing.routes[0].targets[1].priority,
        0
    );
    let cannot_write:bool=sqlx::query_scalar("SELECT NOT has_table_privilege('core_routing_admin.operations','INSERT,UPDATE,DELETE') AND NOT has_table_privilege('routing_active_release','UPDATE')").fetch_one(&runtime.pool).await.unwrap();
    assert!(cannot_write);
    drop(app);
    drop(state);
    drop(restarted);
    runtime.cleanup().await;
}
#[sqlx::test]
async fn fx_and_actor_changes_force_new_preview_without_advancing_release(pool: PgPool) {
    let (runtime, state, session) = setup(&pool).await;
    sqlx::query("INSERT INTO core_console.options(key,value) VALUES('USDExchangeRate','7')")
        .execute(&pool)
        .await
        .unwrap();
    let before = state.current().unwrap();
    let input = request(&before, &before, plan::Adjustment::Divide);
    let app = state.http_router();
    let (_, preview) = call(
        &app,
        &session.secret,
        "/api/core/routing-admin/prices/preview",
        Some(serde_json::to_value(input).unwrap()),
    )
    .await;
    sqlx::query("UPDATE core_console.options SET value='8',revision=2 WHERE key='USDExchangeRate'")
        .execute(&pool)
        .await
        .unwrap();
    let (status,_)=call(&app,&session.secret,"/api/core/routing-admin/commit",Some(json!({"preview_id":preview["data"]["preview_id"],"confirmation_sha256":preview["data"]["confirmation_sha256"],"operation_id":"c".repeat(64)}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let active: Option<Value> = sqlx::query_scalar("SELECT core_routing_admin.read_active()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(active.unwrap()["routing"]["version"], 1);
    let user = identity::IdentityStore::from_pool(pool.clone())
        .bootstrap_user(8, 4)
        .await
        .unwrap();
    let (status, _) = call(&app, &user.secret, "/api/core/routing-admin/catalog", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    drop(app);
    drop(state);
    runtime.cleanup().await;
}

#[sqlx::test]
async fn runtime_cannot_create_a_forged_initial_baseline_and_owner_bootstrap_is_atomic(
    pool: PgPool,
) {
    crate::core_schema::init(&pool).await.unwrap();
    let session = identity::IdentityStore::from_pool(pool.clone())
        .bootstrap_user(7, 6)
        .await
        .unwrap();
    let runtime = crate::test_runtime::RuntimeRole::create(&pool).await;
    let mut authorization = runtime.pool.begin().await.unwrap();
    let actor = authorize(&mut authorization, &session.secret)
        .await
        .unwrap();
    authorization.rollback().await.unwrap();
    let mut forged = document();
    forged.routing.upstreams[0].endpoint = "https://unreviewed.invalid/v1".try_into().unwrap();
    forged.routing.upstreams[0].credential.id = 999;
    let plan = plan::priorities(
        &forged,
        plan::PriorityPreviewRequest {
            expected: Expected::from_release(&forged).unwrap(),
            route_id: "1".into(),
            changes: vec![plan::PriorityChange {
                target_id: "2".into(),
                priority: 0,
            }],
        },
    )
    .unwrap();
    let id = "e".repeat(64);
    let stored = StoredPlan {
        contract: "core.routing-admin.preview.v1".into(),
        preview_id: id.clone(),
        authority: actor.binding,
        plan,
    };
    let payload = serde_json::to_string(&stored).unwrap();
    let digest = plan::hex(Sha256::digest(payload.as_bytes()).as_slice());
    let error = sqlx::query("SELECT core_routing_admin.store_preview($1,$2,$3,$4)")
        .bind(secret_digest(&session.secret))
        .bind(&id)
        .bind(&payload)
        .bind(&digest)
        .execute(&runtime.pool)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("P2703")
    );
    // Even owner-injected private test data cannot make the runtime publish a
    // first release through its second wrapper.
    sqlx::query("INSERT INTO core_routing_admin.previews(preview_id,actor_user_id,actor_credential_id,kind,payload,digest) VALUES($1,7,$2,'priorities',$3,$4)")
        .bind(&id).bind(stored.authority.credential_id.parse::<i64>().unwrap())
        .bind(&payload).bind(&digest).execute(&pool).await.unwrap();
    let error = sqlx::query("SELECT core_routing_admin.publish_preview($1,$2,$3,$4)")
        .bind(secret_digest(&session.secret))
        .bind(&id)
        .bind(&digest)
        .bind("f".repeat(64))
        .execute(&runtime.pool)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("P2703")
    );
    let untouched: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM public.routing_credential_refs) AND NOT EXISTS(SELECT FROM public.routing_price_versions) AND NOT EXISTS(SELECT FROM public.routing_config_versions) AND NOT EXISTS(SELECT FROM core_routing_admin.operations)")
        .fetch_one(&pool).await.unwrap();
    assert!(untouched);
    assert!(matches!(
        install_owner_baseline(&runtime.pool, document()).await,
        Err(AdminError::Forbidden)
    ));
    let first = install_owner_baseline(&pool, document()).await.unwrap();
    let replay = install_owner_baseline(&pool, document()).await.unwrap();
    assert!(first.created);
    assert!(!replay.created);
    assert_eq!(first.snapshot_sha256, replay.snapshot_sha256);
    assert!(matches!(
        install_owner_baseline(&pool, forged).await,
        Err(AdminError::Conflict)
    ));
    let mut next = stored;
    next.preview_id = "d".repeat(64);
    let next_payload = serde_json::to_string(&next).unwrap();
    let next_digest = plan::hex(Sha256::digest(next_payload.as_bytes()).as_slice());
    let error = sqlx::query("SELECT core_routing_admin.store_preview($1,$2,$3,$4)")
        .bind(secret_digest(&session.secret))
        .bind(&next.preview_id)
        .bind(next_payload)
        .bind(next_digest)
        .execute(&runtime.pool)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("40001")
    );
    runtime.cleanup().await;
}
