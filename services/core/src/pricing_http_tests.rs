use super::*;
use crate::{
    identity::IdentityStore,
    model_http::pricing::{PricingSurface, tests::gateway},
};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;
#[path = "../tests/common/runtime.rs"]
mod runtime;
async fn call(
    router: &Router,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri(path);
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"))
    }
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json")
    }
    let response = router
        .clone()
        .oneshot(
            req.body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 3 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn counts(pool: &PgPool) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_charging.requests),(SELECT count(*) FROM core_billing.ledger_entries),(SELECT count(*) FROM core_identity.audit)").fetch_one(pool).await.unwrap()
}
async fn setup(
    pool: &PgPool,
) -> (
    IdentityStore,
    crate::identity::IssuedCredential,
    Arc<PricingSurface>,
) {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'pricing-private-test')").execute(pool).await.unwrap();
    let surface = Arc::new(PricingSurface::from_gateway(&gateway(pool.clone())).unwrap());
    (identity, session, surface)
}
#[sqlx::test(migrations = false)]
async fn pricing_pg_readonly_role_public_session_mark_and_revocation(pool: PgPool) {
    let (identity, session, surface) = setup(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_console").await;
    let router = PricingHttp::new(role.pool.clone(), surface).router();
    let before = counts(&pool).await;
    let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["visibility"], "public");
    assert_eq!(public["groups"], json!(["allowed", "private"]));
    assert_eq!(public["routing_version"], 9);
    assert_eq!(public["pricing_version"], 11);
    assert_eq!(public["invocation_authorized"], false);
    let (status, own) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["visibility"], "session");
    assert_eq!(own["groups"], json!(["allowed"]));
    assert!(!own.to_string().contains("provider-model"));
    let key = identity
        .issue_key(
            &session.secret,
            crate::accounts::Account {
                kind: crate::accounts::AccountKind::Personal,
                id: 7,
            },
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        call(&router, "/core/v1/pricing/session", Some(&key.secret), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &router,
            "/core/v1/pricing/session",
            Some("lmm_at_not_a_session"),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, "/core/v1/pricing/session", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let policy = json!({"version":1,"policies":[{"id":"marked","name":"pricing policy","mode":"fixed_text","fixed_text":"bounded","public_models":["model"],"clients":{},"limits":{"requests_per_minute_per_user":10,"requests_per_minute_global":10,"max_concurrent_per_user":1,"max_concurrent_global":1,"max_input_bytes":4096,"max_output_bytes":4096,"max_output_tokens":1024,"output_bytes_per_second":0,"total_timeout_seconds":5}}]});
    sqlx::query("UPDATE core_abuse.settings SET revision=1,document=$1 WHERE singleton")
        .bind(policy)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_abuse.user_marks(user_id,marked,policy_id,revision) VALUES(7,TRUE,'marked',1)").execute(&pool).await.unwrap();
    let (status, marked) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{marked}");
    let models = marked["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["model"], "model");
    assert_eq!(models[0]["group"], "allowed");
    let (status, runtime) = call(
        &router,
        "/core/v1/pricing/runtime",
        Some(&session.secret),
        Some(json!({"models":["model","claude","absent"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runtime}");
    assert_eq!(runtime["data"]["model"]["status"], "unknown");
    assert_eq!(runtime["data"]["claude"]["status"], "no_access");
    assert_eq!(runtime["data"]["absent"]["status"], "no_access");
    // Exclude explicit fixture key issuance, whose audit is a real authorized write.
    let after_fixture = counts(&pool).await;
    assert_eq!((before.0, before.1), (after_fixture.0, after_fixture.1));
    let _ = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(counts(&pool).await, after_fixture);
    sqlx::query("UPDATE core_identity.user_routing_groups SET enabled=FALSE WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &router,
            "/core/v1/pricing/session",
            Some(&session.secret),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(counts(&pool).await, after_fixture);
    role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn pricing_pg_header_nav_policy_bounds_and_public_runtime_unknown(pool: PgPool) {
    let (_, session, surface) = setup(&pool).await;
    let router = PricingHttp::new(pool.clone(), surface).router();
    let before = counts(&pool).await;
    let unavailable = PricingHttp::unavailable(pool.clone()).router();
    let (status, error) = call(&unavailable, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"]["code"], "pricing_unavailable");

    for (raw, public_code, session_code) in [
        (
            r#"{"pricing":{"enabled":true,"requireAuth":true}}"#,
            StatusCode::FORBIDDEN,
            StatusCode::OK,
        ),
        (
            r#"{"pricing":false}"#,
            StatusCode::FORBIDDEN,
            StatusCode::FORBIDDEN,
        ),
        (r#"{"pricing":true}"#, StatusCode::OK, StatusCode::OK),
    ] {
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES('HeaderNavModules',$1) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value").bind(raw).execute(&pool).await.unwrap();
        assert_eq!(
            call(&router, "/core/v1/pricing/public", None, None).await.0,
            public_code
        );
        assert_eq!(
            call(
                &router,
                "/core/v1/pricing/session",
                Some(&session.secret),
                None
            )
            .await
            .0,
            session_code
        );
    }
    let (status, runtime) = call(
        &router,
        "/core/v1/pricing/runtime",
        None,
        Some(json!({"models":["model","unknown-model"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runtime}");
    assert_eq!(runtime["data"]["model"]["status"], "unknown");
    assert_eq!(runtime["data"]["unknown-model"]["status"], "not_listed");
    assert_eq!(
        call(&router, "/core/v1/pricing/public?group=private", None, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &router,
            "/core/v1/pricing/runtime",
            None,
            Some(json!({"models":["model","model"]}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(counts(&pool).await, before);
}
