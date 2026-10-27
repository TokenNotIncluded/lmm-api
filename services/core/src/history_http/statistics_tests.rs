use super::*;
use axum::{body::to_bytes, http::Request};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn request(store: HistoryStore, path: &str, secret: Option<&str>) -> Response {
    let mut request = Request::builder().uri(path);
    if let Some(secret) = secret {
        request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    router(store)
        .oneshot(request.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn statistics_require_authentication_before_reporting_source_dependencies() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    for path in ["/api/finance/site-statistics", "/api/perf-metrics/summary"] {
        let response = request(HistoryStore::new(pool.clone()), path, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(value["code"], "AUTH_UNAUTHORIZED");
        assert!(value.get("missing_sources").is_none());
    }
}

#[sqlx::test(migrations = false)]
async fn missing_statistics_preserve_admin_boundary_and_do_not_fabricate_totals(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    let store = HistoryStore::new(pool);
    let response = request(
        store.clone(),
        "/api/finance/site-statistics",
        Some(&user.secret),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    for (path, secret, code, missing) in [
        (
            "/api/finance/site-statistics",
            admin.secret.as_str(),
            "SITE_STATISTICS_SOURCE_UNAVAILABLE",
            "historical_top_ups_with_original_currency_paid_and_refunded_amounts",
        ),
        (
            "/api/perf-metrics/summary?hours=24",
            user.secret.as_str(),
            "PERFORMANCE_METRICS_SOURCE_UNAVAILABLE",
            "historical_perf_metrics",
        ),
    ] {
        let response = request(store.clone(), path, Some(secret)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(value["success"], false);
        assert_eq!(value["availability"], "unavailable");
        assert_eq!(value["code"], code);
        assert!(
            value["missing_sources"]
                .as_array()
                .unwrap()
                .contains(&json!(missing))
        );
        assert!(value.get("data").is_none());
        assert!(value.get("models").is_none());
    }
}
