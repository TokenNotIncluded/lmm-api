//! Real Session admission for unsupported categories. No historical import or
//! history-table permission is needed; no common-log projection substitutes.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    http::{Capabilities, configured_router},
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

#[path = "common/database_snapshot.rs"]
mod database_snapshot;

async fn request(app: &Router, path: &str, secret: Option<&str>, expected: StatusCode) {
    let mut builder = Request::builder().uri(path);
    if let Some(secret) = secret {
        builder = builder.header("authorization", format!("Bearer {secret}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), expected, "{path}");
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["success"], false);
    if expected == StatusCode::SERVICE_UNAVAILABLE {
        assert_eq!(
            body,
            json!({
                "success": false,
                "code": "USAGE_HISTORY_UNAVAILABLE",
                "availability": "unavailable",
                "category": if path.starts_with("/api/mj") { "drawing" } else { "task" },
                "message": "This usage history category is unavailable"
            })
        );
    }
}

#[sqlx::test(migrations = false)]
async fn category_routes_authenticate_without_history_or_write_permissions(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    let key = identity
        .issue_key(
            &admin.secret,
            Account {
                kind: AccountKind::Personal,
                id: 8,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_history.sources")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let role = format!("history_category_{:016x}", rand::random::<u64>());
    let password = format!("{:032x}", rand::random::<u128>());
    let database = pool.connect_options().get_database().unwrap().to_owned();
    assert!(
        database
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    sqlx::raw_sql(&format!(
        "CREATE ROLE {role} LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; GRANT CONNECT ON DATABASE \"{database}\" TO {role}; GRANT USAGE ON SCHEMA core_identity TO {role}; GRANT SELECT ON ALL TABLES IN SCHEMA core_identity TO {role};"
    )).execute(&pool).await.unwrap();
    let options = pool
        .connect_options()
        .as_ref()
        .clone()
        .username(&role)
        .password(&password);
    let readonly = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let can_write: bool = sqlx::query_scalar("SELECT has_table_privilege(current_user,'core_identity.credentials','INSERT,UPDATE,DELETE')")
        .fetch_one(&readonly).await.unwrap();
    assert!(!can_write);
    let app = configured_router(
        readonly.clone(),
        Capabilities {
            identity: true,
            ..Capabilities::default()
        },
    );
    let before = database_snapshot::snapshot(&pool).await;
    for path in ["/api/mj", "/api/mj/", "/api/task", "/api/task/"] {
        request(&app, path, None, StatusCode::UNAUTHORIZED).await;
        request(&app, path, Some("invalid"), StatusCode::UNAUTHORIZED).await;
        request(&app, path, Some(&user.secret), StatusCode::FORBIDDEN).await;
        request(&app, path, Some(&key.secret), StatusCode::FORBIDDEN).await;
        request(
            &app,
            path,
            Some(&admin.secret),
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
    }
    for path in [
        "/api/mj/self",
        "/api/mj/self/",
        "/api/task/self",
        "/api/task/self/",
    ] {
        request(&app, path, None, StatusCode::UNAUTHORIZED).await;
        request(&app, path, Some(&key.secret), StatusCode::FORBIDDEN).await;
        request(
            &app,
            path,
            Some(&user.secret),
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
        request(
            &app,
            path,
            Some(&admin.secret),
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
    }
    request(
        &app,
        "/api/task/self?user_id=8",
        Some(&user.secret),
        StatusCode::SERVICE_UNAVAILABLE,
    )
    .await;
    assert_eq!(before, database_snapshot::snapshot(&pool).await);
    sqlx::query("UPDATE core_identity.credentials SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1")
        .bind(user.id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(admin.id)
        .execute(&pool)
        .await
        .unwrap();
    let expired = database_snapshot::snapshot(&pool).await;
    request(
        &app,
        "/api/mj/self",
        Some(&user.secret),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    request(
        &app,
        "/api/task",
        Some(&admin.secret),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_eq!(expired, database_snapshot::snapshot(&pool).await);
    readonly.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
        .execute(&pool)
        .await
        .unwrap();
}
