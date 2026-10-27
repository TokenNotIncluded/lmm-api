//! Real PostgreSQL, original NativeKey authority and zero financial writes.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use lmm_core::{
    accounts::{Account, AccountKind},
    funding::FundingPolicy,
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/runtime.rs"]
mod runtime;

async fn query(app: &Router, secret: &str, ip: &str) -> (StatusCode, Value) {
    query_route(app, "/v1/balance", secret, ip).await
}
async fn query_route(app: &Router, path: &str, secret: &str, ip: &str) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .uri(path)
        .header("Authorization", format!("Bearer {secret}"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        format!("{ip}:8123")
            .parse::<std::net::SocketAddr>()
            .unwrap(),
    ));
    let out = app.clone().oneshot(req).await.unwrap();
    assert_eq!(out.headers()["Cache-Control"], "no-store");
    let code = out.status();
    let body = serde_json::from_slice(&to_bytes(out.into_body(), 4096).await.unwrap()).unwrap();
    (code, body)
}
async fn settings(pool: &PgPool, key: i64, enabled: bool) {
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,account_balance_read) VALUES($1,'synthetic-balance-key','unused-model-group',$2)")
        .bind(key).bind(enabled).execute(pool).await.unwrap();
}
async fn financial_snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('operations',(SELECT count(*) FROM core_billing.ledger_operations),'accounts',(SELECT count(*) FROM core_billing.ledger_accounts),'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),'requests',(SELECT count(*) FROM core_charging.requests),'accessed',(SELECT jsonb_agg(jsonb_build_array(credential_id,accessed_at) ORDER BY credential_id) FROM core_identity.api_key_settings))")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn native_balance_optin_exact_usd_and_readonly_runtime(pool: PgPool) {
    let (_, _, session, key, id, _) = model_fixture::fixture(&pool, 3_500_001).await;
    settings(&pool, id, false).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    let app = lmm_core::native_key_read::router(role.pool.clone());
    let before = financial_snapshot(&pool).await;
    let (code, value) = query(&app, &key, "127.0.0.1").await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(value["error"], "account_balance_access_required");
    sqlx::query("UPDATE core_identity.api_key_settings SET account_balance_read=TRUE WHERE credential_id=$1").bind(id).execute(&pool).await.unwrap();
    let (code, value) = query(&app, &key, "127.0.0.1").await;
    assert_eq!(code, StatusCode::OK, "{value}");
    assert_eq!(value["remaining"].to_string(), "7.000002");
    assert_eq!(value["scope"], "account");
    assert_eq!(value["currency"], "USD");
    assert_eq!(value["consistency"], "persisted_snapshot");
    assert_eq!(value.as_object().unwrap().len(), 6);
    assert!(!value.to_string().contains(&key));
    assert_eq!(
        query(&app, &session, "127.0.0.1").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(before, financial_snapshot(&pool).await);
    let (code, usage) = query_route(&app, "/v1/usage", &key, "127.0.0.1").await;
    assert_eq!(code, StatusCode::OK, "{usage}");
    assert_eq!(usage["scope"], "token");
    assert_eq!(usage["used_quota"], 0);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_usage_requires_legacy_receipts_and_reports_key_quota_without_writes(pool: PgPool) {
    let (_, _, session, key, id, account) = model_fixture::fixture(&pool, 5_000_000).await;
    settings(&pool, id, false).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.budgets(scope,key_id,issuer_user_id,period,limit_credits,unlimited) VALUES('key',$1,7,'lifetime',1000000,FALSE)")
        .bind(id).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO core_charging.budget_opening_consumption(scope,key_id,period,start_at,end_at,consumed_credits,import_reference) VALUES('key',$1,'lifetime',-62135596800,253402300800,250000,'synthetic-key-opening')")
        .bind(id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'synthetic-usage',extract(epoch FROM clock_timestamp())::bigint,true)")
        .execute(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    let app = lmm_core::native_key_read::router(role.pool.clone());
    let before = financial_snapshot(&pool).await;
    let (code, body) = query_route(&app, "/v1/usage", &key, "127.0.0.1").await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "quota_query_unavailable");
    for token_id in [None, Some(id)] {
        sqlx::query("INSERT INTO core_history.scope_receipts(source_id,user_id,owner_account_id,token_id,source_archive_sha256,mapping_sha256,source_scope_sha256,complete) VALUES(1,7,$1,$2,repeat('a',64),repeat('b',64),repeat('c',64),true)")
            .bind(account).bind(token_id).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,token_id,created_at,log_type,quota) VALUES(1,1,'synthetic-today-usage',repeat('d',64),7,$1,$2,extract(epoch FROM clock_timestamp())::bigint,2,250000)")
        .bind(account).bind(id).execute(&pool).await.unwrap();
    let (code, body) = query_route(&app, "/v1/usage", &key, "127.0.0.1").await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["remaining"], 1.5);
    assert_eq!(body["used_total"], 0.5);
    assert_eq!(body["used_today"], 0.5);
    assert_eq!(body["used_quota"], 250_000);
    assert_eq!(body["total_quota"], 2.0);
    assert_eq!(body["scope"], "token");
    assert_eq!(
        query_route(&app, "/v1/usage", &session, "127.0.0.1")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(before, financial_snapshot(&pool).await);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_balance_existing_personal_key_without_developer_grant_can_read(pool: PgPool) {
    let (_, _, _, _, _, account) = model_fixture::fixture(&pool, 500_000).await;
    sqlx::query("UPDATE core_identity.developer_access SET granted=FALSE WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    // Synthetic installation-owner seed at the current identity generation;
    // old credentials remain invalidated by the actual revocation trigger.
    let secret = "sk-synthetic_migrated_balance_key_without_developer_0000";
    let id:i64=sqlx::query_scalar("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT sha256(convert_to($1,'UTF8')),'api_key',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=7 RETURNING id").bind(secret).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES($1,0,$2)").bind(id).bind(account).execute(&pool).await.unwrap();
    settings(&pool, id, true).await;
    let (code, value) = query(
        &lmm_core::native_key_read::router(pool.clone()),
        secret,
        "127.0.0.1",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{value}");
    assert_eq!(value["remaining"], 1);
}

#[sqlx::test(migrations = false)]
async fn native_balance_zero_and_exhausted_quota_do_not_require_spending_authority(pool: PgPool) {
    let (_, _, _, key, id, _) = model_fixture::fixture(&pool, 0).await;
    settings(&pool, id, true).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.budgets(scope,key_id,issuer_user_id,period,limit_credits,unlimited) VALUES('key',$1,7,'lifetime',0,FALSE)").bind(id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let before = financial_snapshot(&pool).await;
    let (code, value) = query(
        &lmm_core::native_key_read::router(pool.clone()),
        &key,
        "127.0.0.1",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{value}");
    assert_eq!(value["remaining"], 0);
    assert_eq!(before, financial_snapshot(&pool).await);
}

#[sqlx::test(migrations = false)]
async fn native_balance_live_ip_revocation_expiry_user_and_consent_are_rechecked(pool: PgPool) {
    let (identity, _, session, key, id, _) = model_fixture::fixture(&pool, 500_000).await;
    settings(&pool, id, true).await;
    let other = identity
        .issue_key(
            &session,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    settings(&pool, other.id, true).await;
    let app = lmm_core::native_key_read::router(pool.clone());
    sqlx::query("UPDATE core_identity.api_key_settings SET allow_ips=ARRAY['127.0.0.1/32'::inet] WHERE credential_id=$1").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        query(&app, &key, "127.0.0.2").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(query(&app, &key, "127.0.0.1").await.0, StatusCode::OK);
    sqlx::query("UPDATE core_identity.api_key_settings SET account_balance_read=FALSE WHERE credential_id=$1").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        query(&app, &key, "127.0.0.1").await.1["error"],
        "account_balance_access_required"
    );
    sqlx::query("UPDATE core_identity.api_key_settings SET account_balance_read=TRUE,enabled=FALSE WHERE credential_id=$1").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        query(&app, &key, "127.0.0.1").await.0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE core_identity.api_key_settings SET enabled=TRUE WHERE credential_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.credentials SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        query(&app, &key, "127.0.0.1").await.0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE core_identity.credentials SET expires_at=clock_timestamp()+interval '1 hour',revoked_at=clock_timestamp() WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        query(&app, &key, "127.0.0.1").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        query(&app, &other.secret, "127.0.0.1").await.0,
        StatusCode::OK
    );
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=7")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        query(&app, &other.secret, "127.0.0.1").await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = false)]
async fn native_balance_missing_wallet_never_creates_one_and_unknown_basis_is_not_zero(
    pool: PgPool,
) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
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
    settings(&pool, key.id, true).await;
    let app = lmm_core::native_key_read::router(pool.clone());
    let before = financial_snapshot(&pool).await;
    let (code, value) = query(&app, &key.secret, "127.0.0.1").await;
    assert_eq!(code, StatusCode::OK, "{value}");
    assert_eq!(value["remaining"], 0);
    assert_eq!(before, financial_snapshot(&pool).await);
    // Unknown imported identity must never be reclassified as an audited native zero.
    let account: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO core_identity.users(id,personal_account_id) VALUES(8,$1)")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    let secret = "lmmk_unknown_native_balance_basis_0000000000000000";
    let id:i64=sqlx::query_scalar("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) VALUES(sha256(convert_to($1,'UTF8')),'api_key',8,1,$2,clock_timestamp()+interval '1 hour') RETURNING id").bind(secret).bind(account).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES($1,0,$2)").bind(id).bind(account).execute(&pool).await.unwrap();
    settings(&pool, id, true).await;
    let (code, value) = query(&app, secret, "127.0.0.1").await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        value,
        json!({"valid":false,"error":"account_balance_unavailable"})
    );
}

#[sqlx::test(migrations = false)]
async fn native_balance_account_rate_limit_is_shared_across_keys_before_money_read(pool: PgPool) {
    let (identity, _, session, key, id, _) = model_fixture::fixture(&pool, 0).await;
    settings(&pool, id, true).await;
    let other = identity
        .issue_key(
            &session,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    settings(&pool, other.id, true).await;
    let app = lmm_core::native_key_read::router(pool.clone());
    let before = financial_snapshot(&pool).await;
    for n in 0..30 {
        assert_eq!(
            query(
                &app,
                if n % 2 == 0 { &key } else { &other.secret },
                "127.0.0.1"
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        query(&app, &other.secret, "127.0.0.1").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(before, financial_snapshot(&pool).await);
}
