use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;

async fn setup(pool: &PgPool) -> (Router, IdentityStore, String) {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap();
    let app = router(OptionsHttp::new(
        pool.clone(),
        identity.clone(),
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            registration_enabled: false,
        },
    ));
    (app, identity, root.secret)
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    secret: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn notice(pool: &PgPool) -> Option<(String, i64)> {
    sqlx::query_as("SELECT value,revision FROM core_console.options WHERE key='Notice'")
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn audits(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='console.options.update'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

fn announcements() -> String {
    json!([{
        "id":1,"content":"Maintenance complete","publishDate":"2026-10-27T00:00:00Z",
        "mandatory":true,"ackRevision":"1","type":"success","extra":""
    }])
    .to_string()
}

#[sqlx::test(migrations = false)]
async fn saves_roundtrip_aliases_revisions_and_audit(pool: PgPool) {
    let (app, _, secret) = setup(&pool).await;
    let body = json!({"key":"Notice","value":"New notice"});
    for path in ["/api/option", "/api/option/"] {
        let (status, body) = request(&app, "PUT", path, &secret, body.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["saved"], true);
    }
    assert_eq!(notice(&pool).await, Some(("New notice".into(), 1)));
    assert_eq!(audits(&pool).await, 1);
    let items = announcements();
    let (status, _) = request(
        &app,
        "PUT",
        "/api/option",
        &secret,
        json!({"key":"Announcements","value":items}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = request(&app, "GET", "/api/option/", &secret, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["capabilities"]["read_only"], false);
    assert_eq!(body["capabilities"]["partial"], true);
    for key in ["Announcements", "console_setting.announcements"] {
        assert!(
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["key"] == key && item["value"] == items)
        );
    }
    // Exercise the actual consumer, including mandatory acknowledgement generation.
    let console = crate::console_compat::ConsoleCompat::new(
        pool.clone(),
        IdentityStore::from_pool(pool.clone()),
    );
    assert_eq!(console.notice().await.unwrap(), "New notice");
    let listed = console.announcements(&secret, None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].ack_revision, "1");
    let target: serde_json::Value = sqlx::query_scalar(
        "SELECT target FROM core_identity.audit WHERE action='console.options.update' ORDER BY id LIMIT 1"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(target["changes"][0]["key"], "Notice");
    assert_eq!(target["changes"][0]["before"], serde_json::Value::Null);
    assert_eq!(target["changes"][0]["after"], json!(["New notice", 1]));
    let (status, _) = request(
        &app,
        "PUT",
        "/api/option/",
        &secret,
        json!({"key":"Notice","value":""}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(notice(&pool).await, Some(("".into(), 2)));
}

#[sqlx::test(migrations = false)]
async fn rejects_non_root_api_key_and_revoked_session(pool: PgPool) {
    let (app, identity, secret) = setup(&pool).await;
    let admin = identity.bootstrap_user(2, 5).await.unwrap();
    let actor = identity.authorize(&secret).await.unwrap();
    let key = identity
        .issue_key(
            &secret,
            actor.owner,
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    for denied in [&admin.secret, &key.secret] {
        for (method, path, body) in [
            (
                "PUT",
                "/api/option/",
                json!({"key":"Notice","value":"Denied"}),
            ),
            (
                "POST",
                "/api/option/bulk",
                json!({"values":{"Notice":"Denied"}}),
            ),
            (
                "POST",
                "/api/option/validate",
                json!({"values":{"Notice":"Denied"}}),
            ),
        ] {
            let (status, _) = request(&app, method, path, denied, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    }
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=1 AND kind='session'")
        .execute(&pool).await.unwrap();
    let (status, _) = request(
        &app,
        "PUT",
        "/api/option",
        &secret,
        json!({"key":"Notice","value":"Denied"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(notice(&pool).await, None);
    assert_eq!(audits(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn unsupported_and_invalid_batches_are_atomic(pool: PgPool) {
    let (app, _, secret) = setup(&pool).await;
    for body in [
        json!({"values":{"Notice":"Not saved","ServerAddress":"https://other.test"}}),
        json!({"values":{"Notice":"Not saved","RegisterEnabled":"true"}}),
        json!({"values":{"Notice":"Not saved","OAuthSecret":"secret"}}),
        json!({"values":{"Notice":"Not saved","console_setting.announcements_enabled":"false"}}),
    ] {
        let (status, body) = request(&app, "POST", "/api/option/bulk", &secret, body).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "SYSTEM_OPTION_NOT_WRITABLE");
    }
    for invalid in [
        "not-json".to_owned(),
        "{}".to_owned(),
        json!([{"id":1,"content":"x","publishDate":"invalid"}]).to_string(),
        json!([{"id":1,"content":"x","publishDate":"2026-10-27T00:00:00Z","mandatory":"true"}]).to_string(),
        json!([{"id":1,"content":"x","publishDate":"2026-10-27T00:00:00Z","ackRevision":"invalid generation"}]).to_string(),
        format!("[{},{}]",&announcements()[1..announcements().len()-1],&announcements()[1..announcements().len()-1]),
    ] {
        let (status,_) = request(&app,"POST","/api/option/bulk",&secret,
            json!({"values":{"Notice":"Not saved","Announcements":invalid}})).await;
        assert_eq!(status,StatusCode::BAD_REQUEST);
    }
    let (status, _) = request(
        &app,
        "POST",
        "/api/option/bulk",
        &secret,
        json!({"values":{"Announcements":"[]","console_setting.announcements":announcements()}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(notice(&pool).await, None);
    assert_eq!(audits(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn validation_and_concurrent_compare_and_swap(pool: PgPool) {
    let (app, identity, first) = setup(&pool).await;
    let second = identity.bootstrap_user(2, 6).await.unwrap().secret;
    let payload = json!({"values":{"Notice":"New"},"expected_values":{"Notice":""}});
    let (status, body) = request(&app, "POST", "/api/option/validate", &first, payload).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["saved"], false);
    assert_eq!(notice(&pool).await, None);
    assert_eq!(audits(&pool).await, 0);
    let a = request(
        &app,
        "POST",
        "/api/option/bulk",
        &first,
        json!({"values":{"Notice":"First"},"expected_values":{"Notice":""}}),
    );
    let b = request(
        &app,
        "POST",
        "/api/option/bulk",
        &second,
        json!({"values":{"Notice":"Second"},"expected_values":{"Notice":""}}),
    );
    let (a, b) = tokio::join!(a, b);
    let mut statuses = [a.0.as_u16(), b.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(notice(&pool).await.unwrap().1, 1);
    assert_eq!(audits(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn audit_failure_rolls_back_all_changes(pool: PgPool) {
    let (app, _, secret) = setup(&pool).await;
    sqlx::query("ALTER TABLE core_identity.audit ADD CONSTRAINT reject_option_audit CHECK (action <> 'console.options.update')")
        .execute(&pool).await.unwrap();
    let (status, body) = request(
        &app,
        "POST",
        "/api/option/bulk",
        &secret,
        json!({"values":{"Notice":"Not saved","Announcements":announcements()}}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["success"], false);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_console.options")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(audits(&pool).await, 0);
}
