//! Comprehensive HTTP and synthetic PostgreSQL tests for history HTTP endpoints.
//! Verifies authentication, authorization, pagination, filtering, export security,
//! missing/incomplete source 503 rejection, unknown money handling, JS-safe overflow handling,
//! admin Session vs ApiKey boundary, and database invariance.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    http::{Capabilities, configured_router},
    identity::{IdentityStore, IssuedCredential},
};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

#[path = "common/database_snapshot.rs"]
mod database_snapshot;

fn app(pool: &PgPool) -> Router {
    configured_router(
        pool.clone(),
        Capabilities {
            identity: true,
            ..Capabilities::default()
        },
    )
}

struct TestContext {
    user: IssuedCredential,
    admin: IssuedCredential,
    admin_key: IssuedCredential,
    key1: IssuedCredential,
    key2: IssuedCredential,
    app: Router,
}

async fn fixture(pool: &PgPool) -> TestContext {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let admin = identity.bootstrap_user(8, 5).await.unwrap();

    let owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=7")
            .fetch_one(pool)
            .await
            .unwrap();
    let admin_owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=8")
            .fetch_one(pool)
            .await
            .unwrap();

    let key1 = identity
        .issue_key(
            &user.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();

    let key2 = identity
        .issue_key(
            &user.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();

    let admin_key = identity
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

    sqlx::query(
        "INSERT INTO core_history.sources (contract_version, source_archive_sha256, mapping_sha256, source_namespace, observed_at, complete) \
         VALUES ('core.history-source.v1', repeat('a', 64), repeat('b', 64), 'synthetic-only', 1700000000, true)",
    )
    .execute(pool)
    .await
    .unwrap();

    // Row 1: Key 1 log, user 7, priced at 500000 credits/usd, formula injection username
    sqlx::query(
        "INSERT INTO core_history.log_records (source_id, source_log_id, canonical_event_key, source_row_sha256, \
         user_id, owner_account_id, token_id, created_at, log_type, quota, pricing_credits_per_usd, prompt_tokens, completion_tokens, \
         username, token_name, model_name, channel_id, group_name, request_id, upstream_request_id) \
         VALUES (1, 1, 'synth-ev-1', repeat('c', 64), 7, $1, $2, 1700000001, 2, 100, 500000, 10, 20, \
         '=HYPERLINK(\"http://evil.com\")', 'key-one', 'gpt-4', 1, 'default', 'req-1', 'up-1')",
    )
    .bind(owner)
    .bind(key1.id)
    .execute(pool)
    .await
    .unwrap();

    // Row 2: Key 2 log, user 7, priced at 500000 credits/usd
    sqlx::query(
        "INSERT INTO core_history.log_records (source_id, source_log_id, canonical_event_key, source_row_sha256, \
         user_id, owner_account_id, token_id, created_at, log_type, quota, pricing_credits_per_usd, prompt_tokens, completion_tokens, \
         username, token_name, model_name, channel_id, group_name, request_id, upstream_request_id) \
         VALUES (1, 2, 'synth-ev-2', repeat('d', 64), 7, $1, $2, 1700000002, 2, 200, 500000, 30, 40, \
         'alice', 'key-two', 'gpt-4', 2, 'default', 'req-2', 'up-2')",
    )
    .bind(owner)
    .bind(key2.id)
    .execute(pool)
    .await
    .unwrap();

    // Row 3: Admin log, user 8, pricing_credits_per_usd is NULL (unknown basis)
    sqlx::query(
        "INSERT INTO core_history.log_records (source_id, source_log_id, canonical_event_key, source_row_sha256, \
         user_id, owner_account_id, token_id, created_at, log_type, quota, pricing_credits_per_usd, prompt_tokens, completion_tokens, \
         username, token_name, model_name, channel_id, group_name, request_id, upstream_request_id) \
         VALUES (1, 3, 'synth-ev-3', repeat('e', 64), 8, $1, NULL, 1700000003, 2, 50, NULL, 5, 15, \
         'admin-user', '', 'claude-3', 0, 'admin', 'req-3', '')",
    )
    .bind(admin_owner)
    .execute(pool)
    .await
    .unwrap();

    let app = app(pool);

    TestContext {
        user,
        admin,
        admin_key,
        key1,
        key2,
        app,
    }
}

async fn req(
    app: &Router,
    method: &str,
    uri: &str,
    secret: Option<&str>,
    accept: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(s) = secret {
        builder = builder.header("authorization", format!("Bearer {s}"));
    }
    if let Some(a) = accept {
        builder = builder.header("accept", a);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, headers, body)
}

fn json_data(bytes: &[u8]) -> Value {
    let val: Value = serde_json::from_slice(bytes).expect("valid json expected");
    val
}

#[sqlx::test(migrations = false)]
async fn unauthenticated_requests_return_401(pool: PgPool) {
    let ctx = fixture(&pool).await;

    for path in [
        "/api/log",
        "/api/log/",
        "/api/log/stat",
        "/api/log/token",
        "/api/finance/export",
    ] {
        let (status, headers, body) = req(&ctx.app, "GET", path, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "path: {path}");
        assert_eq!(headers["cache-control"], "no-store");
        let val = json_data(&body);
        assert_eq!(val["success"], false);
        assert_eq!(val["code"], "AUTH_UNAUTHORIZED");

        let builder = Request::builder()
            .method("GET")
            .uri(path)
            .header("authorization", "Basic dXNlcjpwYXNz");
        let resp = ctx
            .app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let builder = Request::builder()
            .method("GET")
            .uri(path)
            .header("authorization", "Bearer ");
        let resp = ctx
            .app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

#[sqlx::test(migrations = false)]
async fn authorization_boundaries_reject_non_admin_and_wrong_scopes(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // Regular user session calling admin routes -> 403 Forbidden
    for admin_path in [
        "/api/log",
        "/api/log/",
        "/api/log/stat",
        "/api/finance/export",
    ] {
        let (status, _, body) =
            req(&ctx.app, "GET", admin_path, Some(&ctx.user.secret), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "path: {admin_path}");
        let val = json_data(&body);
        assert_eq!(val["success"], false);
        assert_eq!(val["code"], "HISTORY_FORBIDDEN");
    }

    // Read-only API key calling admin routes -> 403 Forbidden
    for admin_path in ["/api/log", "/api/log/stat", "/api/finance/export"] {
        let (status, _, body) =
            req(&ctx.app, "GET", admin_path, Some(&ctx.key1.secret), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "path: {admin_path}");
        let val = json_data(&body);
        assert_eq!(val["success"], false);
        assert_eq!(val["code"], "HISTORY_FORBIDDEN");
    }

    // Admin user's API Key calling admin routes -> 403 Forbidden!
    // Administrative endpoints strictly require CredentialKind::Session.
    for admin_path in ["/api/log", "/api/log/stat", "/api/finance/export"] {
        let (status, _, body) = req(
            &ctx.app,
            "GET",
            admin_path,
            Some(&ctx.admin_key.secret),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Admin ApiKey must be rejected for admin routes; Session is required"
        );
        let val = json_data(&body);
        assert_eq!(val["success"], false);
        assert_eq!(val["code"], "HISTORY_FORBIDDEN");
    }

    // User session calling token route -> 403 Forbidden (requires API key)
    let (status, _, body) = req(
        &ctx.app,
        "GET",
        "/api/log/token",
        Some(&ctx.user.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let val = json_data(&body);
    assert_eq!(val["success"], false);
    assert_eq!(val["code"], "HISTORY_FORBIDDEN");
}

#[sqlx::test(migrations = false)]
async fn admin_history_logs_and_stats_query_success(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // Admin calling /api/log
    let (status, headers, body) =
        req(&ctx.app, "GET", "/api/log", Some(&ctx.admin.secret), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let val = json_data(&body);
    assert_eq!(val["success"], true);
    let data = &val["data"];
    assert_eq!(data["contract"], "core.history-read.v1");
    assert_eq!(data["total"], 3);
    assert_eq!(data["items"].as_array().unwrap().len(), 3);
    assert_eq!(data["source"], "legacy_log_only");

    // Trailing slash /api/log/ returns identical data
    let (status2, _, body2) =
        req(&ctx.app, "GET", "/api/log/", Some(&ctx.admin.secret), None).await;
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(json_data(&body2)["data"]["total"], 3);

    // Admin calling /api/log/stat
    let (status, headers, body) = req(
        &ctx.app,
        "GET",
        "/api/log/stat",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let val = json_data(&body);
    assert_eq!(val["success"], true);
    let stat = &val["data"];
    assert_eq!(stat["quota"], 350);
    assert_eq!(stat["quota_numeric_representable"], true);
    assert_eq!(stat["raw_consumption_quota"], "350");
    assert_eq!(stat["unpriced_quota_records"], 1);
    assert_eq!(stat["quota_projection_truncated"], false);
    // Unpriced record prevents aggregate money projection: must not fake zero
    assert_eq!(stat["quota_money_available"], false);
    assert!(stat["quota_usd"].is_null());
    assert!(
        stat["rpm"].is_null(),
        "rpm must be null for historical traffic"
    );
    assert!(
        stat["tpm"].is_null(),
        "tpm must be null for historical traffic"
    );
    assert_eq!(stat["rates_available"], false);
    assert_eq!(stat["source"], "legacy_log_only");
}

#[sqlx::test(migrations = false)]
async fn token_history_logs_scoped_to_key(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // Calling with Key 1 secret -> sees only Row 1
    let (status, headers, body) = req(
        &ctx.app,
        "GET",
        "/api/log/token",
        Some(&ctx.key1.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let val = json_data(&body);
    assert_eq!(val["success"], true);
    let data = &val["data"];
    assert_eq!(data["total"], 1);
    let items = data["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["source_log_id"], 1);
    assert_eq!(items[0]["quota"], 100);
    assert_eq!(items[0]["quota_usd"], "0.0002");
    assert_eq!(items[0]["quota_money_available"], true);
    assert_eq!(items[0]["quota_money_basis"], "captured_credits_per_usd");
    // Non-admin token read does not expose channel or upstream_request_id
    assert!(items[0].get("channel").is_none());
    assert!(items[0].get("upstream_request_id").is_none());

    // Calling with Key 2 secret -> sees only Row 2
    let (status, _, body) = req(
        &ctx.app,
        "GET",
        "/api/log/token",
        Some(&ctx.key2.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let val = json_data(&body);
    assert_eq!(val["data"]["total"], 1);
    let items = val["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["source_log_id"], 2);
    assert_eq!(items[0]["quota"], 200);
    assert_eq!(items[0]["quota_usd"], "0.0004");
}

#[sqlx::test(migrations = false)]
async fn pagination_and_filtering_consistency(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // Pagination: 1 per page across 3 items
    let (s1, _, b1) = req(
        &ctx.app,
        "GET",
        "/api/log?p=1&page_size=1",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    let d1 = json_data(&b1)["data"].clone();
    assert_eq!(d1["total"], 3);
    assert_eq!(d1["page"], 1);
    assert_eq!(d1["page_size"], 1);
    assert_eq!(d1["items"][0]["source_log_id"], 3);

    let (s2, _, b2) = req(
        &ctx.app,
        "GET",
        "/api/log?p=2&page_size=1",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s2, StatusCode::OK);
    let d2 = json_data(&b2)["data"].clone();
    assert_eq!(d2["total"], 3);
    assert_eq!(d2["page"], 2);
    assert_eq!(d2["items"][0]["source_log_id"], 2);

    let (s3, _, b3) = req(
        &ctx.app,
        "GET",
        "/api/log?p=3&page_size=1",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s3, StatusCode::OK);
    let d3 = json_data(&b3)["data"].clone();
    assert_eq!(d3["total"], 3);
    assert_eq!(d3["page"], 3);
    assert_eq!(d3["items"][0]["source_log_id"], 1);

    // Filtering by model_name
    let (s_model, _, b_model) = req(
        &ctx.app,
        "GET",
        "/api/log?model_name=gpt-4",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s_model, StatusCode::OK);
    let d_model = json_data(&b_model)["data"].clone();
    assert_eq!(d_model["total"], 2);
    for item in d_model["items"].as_array().unwrap() {
        assert_eq!(item["model_name"], "gpt-4");
    }

    // Filtering by request_id
    let (s_req, _, b_req) = req(
        &ctx.app,
        "GET",
        "/api/log?request_id=req-1",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s_req, StatusCode::OK);
    let d_req = json_data(&b_req)["data"].clone();
    assert_eq!(d_req["total"], 1);
    assert_eq!(d_req["items"][0]["request_id"], "req-1");

    // Filtering by group
    let (s_grp, _, b_grp) = req(
        &ctx.app,
        "GET",
        "/api/log?group=admin",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s_grp, StatusCode::OK);
    let d_grp = json_data(&b_grp)["data"].clone();
    assert_eq!(d_grp["total"], 1);
    assert_eq!(d_grp["items"][0]["group"], "admin");
}

#[sqlx::test(migrations = false)]
async fn finance_export_json_and_csv_allowlisted_and_formula_defense(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // JSON export
    let (status, headers, body) = req(
        &ctx.app,
        "GET",
        "/api/finance/export?format=json",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json; charset=utf-8");
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("lmm-historical-usage.json")
    );
    assert_eq!(headers["cache-control"], "no-store");

    let val: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["schema_version"], "core.finance-usage-export.v1");
    assert_eq!(val["complete_financial_export"], false);
    assert_eq!(val["total"], 3);

    let usage = val["usage"].as_array().unwrap();
    assert_eq!(usage.len(), 3);
    for item in usage {
        for forbidden in [
            "other",
            "source_row_sha256",
            "mapping_sha256",
            "source_archive_sha256",
            "password",
            "secret",
            "ip",
            "provider_payload",
        ] {
            assert!(
                item.get(forbidden).is_none(),
                "Forbidden field present: {forbidden}"
            );
        }
    }

    // CSV export
    let (status_csv, headers_csv, body_csv) = req(
        &ctx.app,
        "GET",
        "/api/finance/export?format=csv",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(status_csv, StatusCode::OK);
    assert_eq!(headers_csv["content-type"], "text/csv; charset=utf-8");
    assert!(
        headers_csv["content-disposition"]
            .to_str()
            .unwrap()
            .contains("lmm-historical-usage.csv")
    );
    assert_eq!(headers_csv["cache-control"], "no-store");

    let csv_text = String::from_utf8(body_csv).expect("valid utf-8 csv");
    // Formula injection defense
    assert!(
        csv_text.contains("\"'=HYPERLINK(\"\"http://evil.com\"\")\""),
        "CSV must protect against spreadsheet formula injection"
    );
    let first_line = csv_text.lines().next().unwrap();
    assert!(!first_line.contains("source_archive_sha256"));
    assert!(!first_line.contains("password"));
    assert!(!first_line.contains("secret"));
    assert!(first_line.contains("source_log_id"));
    assert!(first_line.contains("quota_usd"));
}

#[sqlx::test(migrations = false)]
async fn missing_or_incomplete_source_returns_503_never_faked_empty(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 5).await.unwrap();

    let app = app(&pool);

    let (status, _, body) = req(&app, "GET", "/api/log", Some(&admin.secret), None).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "Installed schema without an imported source must reject with 503"
    );
    let val = json_data(&body);
    assert_eq!(val["success"], false);
    assert_eq!(val["code"], "HISTORY_UNAVAILABLE");

    let (status_empty, _, body_empty) =
        req(&app, "GET", "/api/log", Some(&admin.secret), None).await;
    assert_eq!(
        status_empty,
        StatusCode::SERVICE_UNAVAILABLE,
        "Empty sources must reject with 503, never fake 200 with empty list"
    );
    assert_eq!(json_data(&body_empty)["code"], "HISTORY_UNAVAILABLE");

    sqlx::query(
        "INSERT INTO core_history.sources (contract_version, source_archive_sha256, mapping_sha256, source_namespace, observed_at, complete) \
         VALUES ('core.history-source.v1', repeat('a', 64), repeat('b', 64), 'synthetic-only', 1700000000, false)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status_incomplete, _, body_incomplete) =
        req(&app, "GET", "/api/log", Some(&admin.secret), None).await;
    assert_eq!(
        status_incomplete,
        StatusCode::SERVICE_UNAVAILABLE,
        "Incomplete source must reject with 503"
    );
    assert_eq!(json_data(&body_incomplete)["code"], "HISTORY_UNAVAILABLE");

    sqlx::query("DROP SCHEMA core_history CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    let (status_missing, _, body_missing) =
        req(&app, "GET", "/api/log", Some(&admin.secret), None).await;
    assert_eq!(status_missing, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_data(&body_missing)["code"], "HISTORY_UNAVAILABLE");
}

#[sqlx::test(migrations = false)]
async fn unknown_money_not_coerced_to_zero(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // Row 3 has NULL pricing_credits_per_usd
    let (status, _, body) = req(
        &ctx.app,
        "GET",
        "/api/log?request_id=req-3",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let val = json_data(&body);
    let item = &val["data"]["items"][0];
    assert_eq!(item["quota_money_available"], false);
    assert!(item.get("quota_usd").is_none() || item["quota_usd"].is_null());
    assert_eq!(item["quota_money_basis"], "missing_credit_basis");

    // Summary across all 3 rows
    let (stat_status, _, stat_body) = req(
        &ctx.app,
        "GET",
        "/api/log/stat",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(stat_status, StatusCode::OK);
    let stat = json_data(&stat_body)["data"].clone();
    assert_eq!(stat["unpriced_quota_records"], 1);
    assert_eq!(stat["quota_money_available"], false);
    assert!(stat["quota_usd"].is_null());
    assert_eq!(stat["quota_money_basis"], "aggregate_missing_credit_basis");
}

#[sqlx::test(migrations = false)]
async fn quota_sums_exceeding_js_safe_do_not_fabricate_zero_or_negative(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();

    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    let admin_owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=8")
            .fetch_one(&pool)
            .await
            .unwrap();

    sqlx::query(
        "INSERT INTO core_history.sources (contract_version, source_archive_sha256, mapping_sha256, source_namespace, observed_at, complete) \
         VALUES ('core.history-source.v1', repeat('a', 64), repeat('b', 64), 'synthetic-only', 1700000000, true)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Every row is representable by legacy JSON; their aggregate exceeds i64.
    let exact = i128::from(9_007_199_254_740_991_i64) * 1025;
    assert!(exact > i128::from(i64::MAX));
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,pricing_credits_per_usd,username,model_name,request_id) SELECT 1,n,'large-key-'||n,repeat('f',64),8,$1,1700000000,2,9007199254740991,500000,'admin','gpt-4','large-req-'||n FROM generate_series(1,1025) n")
        .bind(admin_owner)
        .execute(&pool)
        .await
        .unwrap();
    let app = app(&pool);

    let (status, _, body) = req(&app, "GET", "/api/log/stat", Some(&admin.secret), None).await;
    assert_eq!(status, StatusCode::OK);
    let val = json_data(&body);
    let stat = &val["data"];

    // Exact raw string must be preserved
    assert_eq!(stat["raw_consumption_quota"], exact.to_string());
    // Numeric field must be null (NEVER coerced to 0 or overflowed)
    assert!(
        stat["quota"].is_null(),
        "Numeric quota must be null when exceeding JS-safe range"
    );
    assert_eq!(stat["quota_numeric_representable"], false);
}

#[sqlx::test(migrations = false)]
async fn parameter_validation_and_size_limits(pool: PgPool) {
    let ctx = fixture(&pool).await;

    // Query exceeding 4096 bytes -> 400 Bad Request
    let large_pad = "a".repeat(4100);
    let large_uri = format!("/api/log?model_name={large_pad}");
    let (s_len, _, b_len) = req(&ctx.app, "GET", &large_uri, Some(&ctx.admin.secret), None).await;
    assert_eq!(s_len, StatusCode::BAD_REQUEST);
    assert_eq!(json_data(&b_len)["code"], "HISTORY_INVALID");

    // page_size > 100 -> 400 Bad Request
    let (s_ps, _, b_ps) = req(
        &ctx.app,
        "GET",
        "/api/log?page_size=101",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s_ps, StatusCode::BAD_REQUEST);
    assert_eq!(json_data(&b_ps)["code"], "HISTORY_INVALID");

    // p = 0 -> 400 Bad Request
    let (s_p0, _, b_p0) = req(
        &ctx.app,
        "GET",
        "/api/log?p=0",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s_p0, StatusCode::BAD_REQUEST);
    assert_eq!(json_data(&b_p0)["code"], "HISTORY_INVALID");

    // Invalid type > 7 -> 400 Bad Request
    let (s_t, _, b_t) = req(
        &ctx.app,
        "GET",
        "/api/log?type=8",
        Some(&ctx.admin.secret),
        None,
    )
    .await;
    assert_eq!(s_t, StatusCode::BAD_REQUEST);
    assert_eq!(json_data(&b_t)["code"], "HISTORY_INVALID");
}

#[sqlx::test(migrations = false)]
async fn database_invariance_before_and_after_reads(pool: PgPool) {
    let ctx = fixture(&pool).await;
    let owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=7")
            .fetch_one(&pool)
            .await
            .unwrap();
    let ledger = lmm_core::billing::ledger::Ledger::from_pool(pool.clone());
    ledger.open_wallet(owner).await.unwrap();
    ledger
        .execute(&lmm_core::billing::ledger::Request {
            scope: "history-read-invariance".into(),
            operation_key: "seed-wallet".into(),
            actor_user_id: Some(7),
            reason: "Synthetic nonempty ledger in isolated read-only regression".into(),
            action: lmm_core::billing::ledger::Action::Credit {
                account_id: owner,
                amount_units: lmm_core::billing::Credits::try_from(1000).unwrap(),
                payment_reference: "synthetic-history-read-invariance".into(),
            },
        })
        .await
        .unwrap();
    let before = database_snapshot::snapshot(&pool).await;

    for _ in 0..5 {
        for (uri, secret) in [
            ("/api/log", &ctx.admin.secret),
            ("/api/log/stat", &ctx.admin.secret),
            ("/api/log/token", &ctx.key1.secret),
            ("/api/finance/export?format=json", &ctx.admin.secret),
            ("/api/finance/export?format=csv", &ctx.admin.secret),
        ] {
            let (status, headers, _) = req(&ctx.app, "GET", uri, Some(secret), None).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(headers["cache-control"], "no-store");
        }
    }

    assert_eq!(database_snapshot::snapshot(&pool).await, before);
    for (uri, secret, expected) in [
        ("/api/log", None, StatusCode::UNAUTHORIZED),
        ("/api/log/stat", None, StatusCode::UNAUTHORIZED),
        ("/api/log/token", None, StatusCode::UNAUTHORIZED),
        (
            "/api/finance/export?format=json",
            None,
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/api/finance/export?format=csv",
            None,
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/api/log",
            Some(ctx.user.secret.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/log/stat",
            Some(ctx.admin_key.secret.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/log/token",
            Some(ctx.user.secret.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/finance/export",
            Some(ctx.user.secret.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/log?p=0",
            Some(ctx.admin.secret.as_str()),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/log/stat?type=8",
            Some(ctx.admin.secret.as_str()),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/log/token?page_size=101",
            Some(ctx.key1.secret.as_str()),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/finance/export?format=unknown",
            Some(ctx.admin.secret.as_str()),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_unchanged_response(&pool, &ctx.app, uri, secret, expected).await;
    }
    sqlx::query("UPDATE core_history.sources SET complete=false")
        .execute(&pool)
        .await
        .unwrap();
    for missing_schema in [false, true] {
        if missing_schema {
            sqlx::query("DROP SCHEMA core_history CASCADE")
                .execute(&pool)
                .await
                .unwrap();
        }
        for (uri, secret) in [
            ("/api/log", &ctx.admin.secret),
            ("/api/log/stat", &ctx.admin.secret),
            ("/api/log/token", &ctx.key1.secret),
            ("/api/finance/export?format=json", &ctx.admin.secret),
            ("/api/finance/export?format=csv", &ctx.admin.secret),
        ] {
            assert_unchanged_response(
                &pool,
                &ctx.app,
                uri,
                Some(secret),
                StatusCode::SERVICE_UNAVAILABLE,
            )
            .await;
        }
    }
}

async fn assert_unchanged_response(
    pool: &PgPool,
    app: &Router,
    uri: &str,
    secret: Option<&str>,
    expected: StatusCode,
) {
    let before = database_snapshot::snapshot(pool).await;
    let (status, headers, _) = req(app, "GET", uri, secret, None).await;
    assert_eq!(status, expected, "{uri}");
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(
        database_snapshot::snapshot(pool).await,
        before,
        "{uri}: {expected}"
    );
}

#[sqlx::test(migrations = false)]
async fn read_invariance_snapshot_detects_same_count_updates_and_sequence_changes(pool: PgPool) {
    let _ = fixture(&pool).await;
    let before = database_snapshot::snapshot(&pool).await;
    sqlx::query(
        "UPDATE core_history.log_records SET request_id='same-count-change' WHERE source_log_id=1",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_ne!(database_snapshot::snapshot(&pool).await, before);
    let changed = database_snapshot::snapshot(&pool).await;
    sqlx::query("SELECT nextval('core_history.sources_source_id_seq')")
        .execute(&pool)
        .await
        .unwrap();
    assert_ne!(database_snapshot::snapshot(&pool).await, changed);
}
