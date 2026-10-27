// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
//! Compile the unwired module directly; main.rs and lib.rs remain untouched.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
pub use lmm_core::identity;
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use tower::ServiceExt;

#[path = "common/database_snapshot.rs"]
mod database_snapshot;
#[path = "common/runtime.rs"]
mod runtime;
#[path = "../src/wallet_history_read_http.rs"]
mod wallet_history_read_http;
#[path = "../src/wallet_topup_snapshot.rs"]
mod wallet_topup_snapshot;

fn app(identity: &IdentityStore) -> Router {
    wallet_history_read_http::router(wallet_history_read_http::WalletHistoryReadHttp::new(
        identity.clone(),
    ))
}

async fn read(app: &Router, path: &str, secret: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let body = to_bytes(response.into_body(), 16_384).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn missing_or_ambiguous_session_is_rejected_before_database_access() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = app(&IdentityStore::from_pool(pool));
    for path in ["/api/user/topup/self"] {
        for authorization in [
            vec![],
            vec!["Bearer "],
            vec!["Basic unrelated"],
            vec!["Bearer a b"],
            vec!["Bearer first", "Bearer second"],
        ] {
            let mut request = Request::builder().uri(path);
            for value in authorization {
                request = request.header(header::AUTHORIZATION, value);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
}

#[tokio::test]
async fn no_payment_write_route_or_configuration_route_is_exposed() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = app(&IdentityStore::from_pool(pool));
    for path in ["/api/user/topup/self"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/user/topup/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn authenticated_wallet_reads_are_honest_and_leave_all_state_unchanged(pool: PgPool) {
    // @context-fold begin wallet-read-real-postgres-contract
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(71, 1).await.unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 71,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO core_payments.intents \
         (account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,\
         amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash) \
         SELECT personal_account_id,id,1,'stripe','test-merchant','test','USD',\
         100,500000,5000,'wallet-read-prepared-order',decode(repeat('00',32),'hex') \
         FROM core_identity.users WHERE id=71",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = database_snapshot::snapshot(&pool).await;
    let disabled = app(&identity);

    let (status, body) = read(
        &disabled,
        "/api/user/topup/self?p=1&page_size=10",
        &session.secret,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], "TOPUP_HISTORY_SOURCE_UNAVAILABLE");
    assert!(body.get("data").is_none());

    for path in ["/api/user/topup/self"] {
        let (status, _) = read(&disabled, path, &key.secret).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert_eq!(database_snapshot::snapshot(&pool).await, before);

    identity.revoke(&session.secret, session.id).await.unwrap();
    let revoked = database_snapshot::snapshot(&pool).await;
    for path in ["/api/user/topup/self"] {
        let (status, _) = read(&disabled, path, &session.secret).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(database_snapshot::snapshot(&pool).await, revoked);
    // @context-fold end wallet-read-real-postgres-contract
}

#[sqlx::test]
async fn imported_legacy_history_is_exact_scoped_and_read_only(pool: PgPool) {
    // @context-fold begin wallet-history-imported-snapshot-read
    core_schema::init(&pool).await.unwrap();
    let owner_identity = IdentityStore::from_pool(pool.clone());
    let user = owner_identity.bootstrap_user(81, 1).await.unwrap();
    let empty_user = owner_identity.bootstrap_user(82, 1).await.unwrap();
    let uncovered_user = owner_identity.bootstrap_user(83, 1).await.unwrap();
    let other_user = owner_identity.bootstrap_user(84, 1).await.unwrap();
    let other_account_id: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=84")
            .fetch_one(&pool)
            .await
            .unwrap();
    let account_id: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=81")
            .fetch_one(&pool)
            .await
            .unwrap();
    let empty_account_id: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=82")
            .fetch_one(&pool)
            .await
            .unwrap();

    let mut rows = json!([
        {
            "source_user_id": 1801, "target_user_id": 81,
            "target_personal_account_id": account_id, "source_row_sha256": "a".repeat(64),
            "money_text": "2.000", "record": {
                "id": 901, "user_id": 81, "amount": 100, "amount_unit": "LEGACY",
                "money_text": "2.000", "trade_no": "ORD%_901",
                "payment_method": "stripe", "status": "success",
                "create_time": 1700000100_i64, "complete_time": 1700000101_i64,
                "credited_quota": 500000, "platform_amount_micros": 0,
                "currency": "USD", "settlement_currency": "USD"
            }
        },
        {
            "source_user_id": 1801, "target_user_id": 81,
            "target_personal_account_id": account_id, "source_row_sha256": "b".repeat(64),
            "money_text": "1.25", "record": {
                "id": 902, "user_id": 81, "amount": 50, "amount_unit": "CREDIT",
                "money_text": "1.25", "trade_no": "ORD-902",
                "payment_method": "creem", "status": "failed",
                "create_time": 1700000102_i64, "complete_time": 1700000103_i64,
                "credited_quota": 0, "platform_amount_micros": 0,
                "currency": "", "settlement_currency": ""
            }
        }
    ]);
    for (id, user_id, source_user, target_account, money, status) in [
        (903, 81, 1801, account_id, "3.50", "pending"),
        (904, 81, 1801, account_id, "4.00", "expired"),
        (905, 84, 1804, other_account_id, "9.99", "success"),
    ] {
        rows.as_array_mut().unwrap().push(json!({
            "source_user_id": source_user, "target_user_id": user_id,
            "target_personal_account_id": target_account, "source_row_sha256": "f".repeat(64),
            "money_text": money, "record": {
                "id": id, "user_id": user_id, "amount": 100, "amount_unit": "LEGACY",
                "money_text": money, "trade_no": format!("ORD-{id}"),
                "payment_method": "stripe", "status": status,
                "create_time": 1700000100_i64 + id,
                "complete_time": if status == "success" {1700001100_i64} else {0},
                "credited_quota": 0, "platform_amount_micros": 0,
                "currency": "USD", "settlement_currency": "USD"
            }
        }));
    }
    let migration_id = "wallet-topup-history-v1-9a7a0a63e17900a89e73c4a95cb629cf";
    let manifest = json!({
        "migration_id":migration_id,
        "scope":"wallet-topup-history.v1",
        "source_namespace":"lmm_api.lmm_prod_20260802.top_ups",
        "observed_at":1700000200_i64,
        "source_row_count":5,
        "source_archive_sha256":"c".repeat(64),
        "source_export_sha256":"d".repeat(64),
        "mapping_sha256":"e".repeat(64),
        "complete":true,
        "identity_map":[
            {"source_user_id":1801,"target_user_id":81,"target_personal_account_id":account_id},
            {"source_user_id":1802,"target_user_id":82,"target_personal_account_id":empty_account_id},
            {"source_user_id":1804,"target_user_id":84,"target_personal_account_id":other_account_id}
        ],
        "per_source_user_counts":{"1801":4,"1802":0,"1804":1},
        "counts_by_status":{"success":2,"failed":1,"pending":1,"expired":1},
        "legacy_money_sums_by_currency":{"USD":"19.490","<unknown>":"1.25"},
        "rows":rows
    });
    let generator_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/prepare_wallet_history_snapshot.py");
    let mut generator = Command::new("python3")
        .arg("-c")
        .arg("import importlib.util,json,sys; s=importlib.util.spec_from_file_location('wallet_import',sys.argv[1]); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); print(m.import_sql(json.load(sys.stdin)))")
        .arg(generator_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    generator
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    let generated = generator.wait_with_output().unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let apply_sql = String::from_utf8(generated.stdout).unwrap();
    let before_import = database_snapshot::snapshot(&pool).await;
    sqlx::raw_sql(&apply_sql).execute(&pool).await.unwrap();
    let after_import = database_snapshot::snapshot(&pool).await;
    for (table, rows) in &before_import {
        if table != "r:core_migration.import_runs" {
            assert_eq!(
                after_import.get(table),
                Some(rows),
                "unexpected import write: {table}"
            );
        }
    }
    core_schema::check(&pool).await.unwrap();

    let role = crate::runtime::RuntimeRole::create(&pool).await;
    sqlx::raw_sql(&format!(
        "GRANT USAGE ON SCHEMA core_migration TO {0}; \
         GRANT SELECT (migration_id,manifest) ON core_migration.import_runs TO {0}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    let runtime_identity = IdentityStore::from_pool(role.pool.clone());
    let history =
        wallet_topup_snapshot::WalletTopupHistory::new(role.pool.clone(), migration_id.to_owned());
    let history_app = wallet_history_read_http::router(
        wallet_history_read_http::WalletHistoryReadHttp::new(runtime_identity)
            .with_history(history),
    );
    let before = database_snapshot::snapshot(&pool).await;

    let (status, body) = read(
        &history_app,
        "/api/user/topup/self?p=1&page_size=10&keyword=ORD%25_",
        &user.secret,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["total"], 1);
    assert_eq!(body["data"]["items"][0]["id"], 901);
    assert_eq!(body["data"]["items"][0]["user_id"], 81);
    assert_eq!(body["data"]["items"][0]["money"].to_string(), "2.000");
    assert_eq!(body["data"]["items"][0]["trade_no"], "ORD%_901");
    assert_eq!(body["data"]["coverage"]["source_items"], 4);
    assert_eq!(body["data"]["coverage"]["imported_items"], 4);
    assert_eq!(body["data"]["coverage"]["missing_items"], 0);
    assert_eq!(
        body["data"]["coverage"]["native_payment_orders_included"],
        false
    );

    let (status, body) = read(
        &history_app,
        "/api/user/topup/self?p=1&page_size=10&sort_by=money&sort_order=asc",
        &user.secret,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["items"][0]["id"], 902);
    assert_eq!(body["data"]["items"][1]["id"], 901);
    assert_eq!(body["data"]["total"], 4);
    for (index, state) in ["failed", "success", "pending", "expired"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(body["data"]["items"][index]["status"], state);
        assert_eq!(body["data"]["items"][index]["user_id"], 81);
    }
    assert_eq!(body["data"]["items"][2]["complete_time"], 0);
    assert_eq!(body["data"]["items"][3]["complete_time"], 0);
    let (status, body) = read(
        &history_app,
        "/api/user/topup/self?p=2&page_size=2&sort_by=money&sort_order=asc",
        &user.secret,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["data"]["items"][0]["id"], 903);
    let (status, body) = read(&history_app, "/api/user/topup/self", &other_user.secret).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["total"], 1);
    assert_eq!(body["data"]["items"][0]["id"], 905);
    assert_eq!(body["data"]["items"][0]["user_id"], 84);

    let (status, body) = read(&history_app, "/api/user/topup/self", &empty_user.secret).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["total"], 0);
    assert_eq!(body["data"]["coverage"]["source_items"], 0);
    let (status, _) = read(&history_app, "/api/user/topup/self", &uncovered_user.secret).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(database_snapshot::snapshot(&pool).await, before);

    // A concurrent-safe retry of the exact receipt changes neither rows nor ledger.
    let before_retry = database_snapshot::snapshot(&pool).await;
    sqlx::raw_sql(&apply_sql).execute(&pool).await.unwrap();
    let retry = sqlx::query(
        "SELECT count(*) FROM core_migration.import_runs WHERE migration_id=$1 AND manifest=$2",
    )
    .bind(migration_id)
    .bind(&manifest)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retry.get::<i64, _>(0), 1);
    assert_eq!(database_snapshot::snapshot(&pool).await, before_retry);
    role.cleanup().await;
    // @context-fold end wallet-history-imported-snapshot-read
}
