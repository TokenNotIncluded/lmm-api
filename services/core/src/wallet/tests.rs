use super::*;
use crate::{
    core_schema,
    identity::{BrowserRegistration, BrowserSessionMetadata, IdentityStore},
};
use axum::{
    body::{Body, to_bytes},
    http::{Request as HttpRequest, StatusCode},
};
use tower::ServiceExt;
struct Fixture {
    store: Store,
    sender: String,
    cookie: String,
    recipient: String,
    other: String,
    account: i64,
}
async fn setup(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_wallet.transfers') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap();
    if !exists {
        sqlx::raw_sql(SCHEMA).execute(pool).await.unwrap();
    }
    let identity = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "wallet_sender",
            password: "synthetic password",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "wallet_sender",
            "synthetic password",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let sender = bundle.data["access_token"].as_str().unwrap().to_owned();
    let account = identity.authorize(&sender).await.unwrap().owner_account_id;
    let recipient = identity.bootstrap_user(91002, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(91003, 0).await.unwrap().secret;
    Ledger::from_pool(pool.clone())
        .open_wallet(account)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    let user = identity.authorize(&sender).await.unwrap().user_id;
    post(
        &mut tx,
        user,
        "fixture-credit".into(),
        Action::Credit {
            account_id: account,
            amount_units: credits(1000).unwrap(),
            payment_reference: "wallet-synthetic-paid".into(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        store: Store::new(pool.clone()),
        sender,
        cookie: bundle.refresh_token,
        recipient,
        other,
        account,
    }
}
async fn pending(f: &Fixture, key: &str, token: &str, amount: i64) -> Transfer {
    let t = f
        .store
        .prepare(&f.sender, amount, key, token)
        .await
        .unwrap();
    assert_eq!(t.state, "awaiting_confirmation");
    let (_, csrf) = f
        .store
        .confirmation_page(&f.cookie, t.id, "create")
        .await
        .unwrap();
    f.store
        .confirm(&f.cookie, t.id, &csrf, "create")
        .await
        .unwrap()
}
#[sqlx::test(migrations = false)]
async fn only_real_cookie_confirmation_can_hold_exact_funds(pool: PgPool) {
    let f = setup(&pool).await;
    let token = "a".repeat(64);
    let key = "wallet-create-0001";
    let t = f.store.prepare(&f.sender, 300, key, &token).await.unwrap();
    let again = f.store.prepare(&f.sender, 300, key, &token).await.unwrap();
    assert_eq!(t.id, again.id);
    assert!(matches!(
        f.store.prepare(&f.sender, 301, key, &token).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        f.store.prepare(&f.sender, 300, key, &"b".repeat(64)).await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        f.store
            .balance(&f.sender)
            .await
            .unwrap()
            .balance
            .spendable_units,
        1000
    );
    assert!(
        f.store
            .confirmation_page(&f.sender, t.id, "create")
            .await
            .is_err()
    );
    assert!(matches!(
        f.store.claim(&f.recipient, &token).await,
        Err(Error::Conflict)
    ));
    let (_, csrf) = f
        .store
        .confirmation_page(&f.cookie, t.id, "create")
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .confirm(&f.cookie, t.id, &"0".repeat(64), "create")
            .await,
        Err(Error::Forbidden)
    ));
    f.store
        .confirm(&f.cookie, t.id, &csrf, "create")
        .await
        .unwrap();
    f.store
        .confirm(&f.cookie, t.id, &csrf, "create")
        .await
        .unwrap();
    let b = f.store.balance(&f.sender).await.unwrap().balance;
    assert_eq!((b.spendable_units, b.reserved_units), (700, 300));
    let holds:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-wallet-transfer' AND kind='reserve'").fetch_one(&pool).await.unwrap();
    assert_eq!(holds, 1);
    let digest: Vec<u8> =
        sqlx::query_scalar("SELECT token_digest FROM core_wallet.transfers WHERE id=$1")
            .bind(t.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(digest, hash(&token));
    assert_ne!(digest, token.as_bytes());
    assert!(matches!(
        f.store.request_cancel(&f.other, t.id).await,
        Err(Error::Conflict)
    ));
}
#[sqlx::test(migrations = false)]
async fn claim_cancel_race_has_one_money_outcome_and_replays(pool: PgPool) {
    let f = setup(&pool).await;
    let token = "c".repeat(64);
    let t = pending(&f, "wallet-race-00001", &token, 400).await;
    let (_, csrf) = f
        .store
        .confirmation_page(&f.cookie, t.id, "cancel")
        .await
        .unwrap();
    let (claim, cancel) = tokio::join!(
        f.store.claim(&f.recipient, &token),
        f.store.confirm(&f.cookie, t.id, &csrf, "cancel")
    );
    assert_ne!(claim.is_ok(), cancel.is_ok());
    let state: String = sqlx::query_scalar("SELECT state FROM core_wallet.transfers WHERE id=$1")
        .bind(t.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let b = f.store.balance(&f.sender).await.unwrap().balance;
    assert_eq!(b.reserved_units, 0);
    if state == "claimed" {
        assert_eq!(b.spendable_units, 600);
        let a = f.store.claim(&f.recipient, &token).await.unwrap();
        assert!(a.claimed_by_me);
        assert!(matches!(
            f.store.claim(&f.other, &token).await,
            Err(Error::Conflict)
        ));
        assert!(matches!(
            f.store.confirm(&f.cookie, t.id, &csrf, "cancel").await,
            Err(Error::Conflict)
        ));
        assert_eq!(
            f.store
                .balance(&f.recipient)
                .await
                .unwrap()
                .balance
                .spendable_units,
            400
        );
    } else {
        assert_eq!(state, "cancelled");
        assert_eq!(b.spendable_units, 1000);
        f.store
            .confirm(&f.cookie, t.id, &csrf, "cancel")
            .await
            .unwrap();
        assert_eq!(
            f.store.request_cancel(&f.sender, t.id).await.unwrap().state,
            "cancelled"
        );
        assert!(matches!(
            f.store.claim(&f.recipient, &token).await,
            Err(Error::Conflict)
        ));
    }
    let n:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE parent_journal_id=(SELECT hold_journal FROM core_wallet.transfers WHERE id=$1)").bind(t.id).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    assert!(matches!(
        f.store.claim(&f.sender, &token).await,
        Err(Error::Forbidden)
    ));
    let r = f.store.inspect(&f.other, &token).await.unwrap();
    let v = serde_json::to_value(r).unwrap();
    assert!(
        v.get("recipient_user").is_none()
            && v.get("token").is_none()
            && v.get("request_key").is_none()
    );
    assert_eq!(f.account, b.account_id);
}
#[sqlx::test(migrations = false)]
async fn http_origin_cookie_and_internal_host_authority_are_separate(pool: PgPool) {
    let f = setup(&pool).await;
    let t = f
        .store
        .prepare(&f.sender, 100, "wallet-http-00001", &"d".repeat(64))
        .await
        .unwrap();
    let browser = http::browser_router(f.store.clone(), "https://wallet.example".into()).unwrap();
    let path = format!("/core/v1/wallet/confirm/create/{}", t.id);
    let denied = browser
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri(&path)
                .header("Authorization", format!("Bearer {}", f.sender))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let page = browser
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri(&path)
                .header("Cookie", format!("new_api_refresh={}", f.cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()["cache-control"], "no-store");
    assert_eq!(page.headers()["referrer-policy"], "no-referrer");
    let html = String::from_utf8(to_bytes(page.into_body(), 8192).await.unwrap().to_vec()).unwrap();
    assert!(html.contains("100 CREDIT"));
    let csrf = html
        .split("csrf:'")
        .nth(1)
        .unwrap()
        .split('\'')
        .next()
        .unwrap();
    let req = |origin: &str| {
        HttpRequest::builder()
            .method("POST")
            .uri(&path)
            .header("Content-Type", "application/json")
            .header("Origin", origin)
            .header("Cookie", format!("new_api_refresh={}", f.cookie))
            .body(Body::from(format!("{{\"csrf\":\"{csrf}\"}}")))
            .unwrap()
    };
    assert_eq!(
        browser
            .clone()
            .oneshot(req("https://evil.example"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        browser
            .oneshot(req("https://wallet.example"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let internal = http::internal_router(f.store.clone(), vec![b'x'; 32]).unwrap();
    let denied = internal
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/internal/wallet/v1/balance")
                .header("Content-Type", "application/json")
                .header("X-Lmm-User-Credential", &f.sender)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
}

use crate::test_runtime as runtime;
#[sqlx::test(migrations = false)]
async fn runtime_column_authority_preserves_immutable_economics(pool: PgPool) {
    let f = setup(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_wallet TO {0}; GRANT SELECT,INSERT ON core_wallet.transfers TO {0}; GRANT UPDATE(state,hold_journal,final_journal,recipient_user,confirmed_at,finished_at,confirmation_digest,confirmation_sid,confirmation_kind,confirmation_expires) ON core_wallet.transfers TO {0}; GRANT USAGE,SELECT ON SEQUENCE core_wallet.transfers_id_seq TO {0}",role.name)).execute(&pool).await.unwrap();
    let store = Store::new(role.pool.clone());
    store.check_ready().await.unwrap();
    let readiness = http::internal_router(store.clone(), vec![b'x'; 32])
        .unwrap()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/internal/wallet/v1/ready")
                .header("content-type", "application/json")
                .header("x-lmm-protocol", "1")
                .header("authorization", format!("Bearer {}", "x".repeat(32)))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(readiness.status(), StatusCode::OK);
    assert_eq!(readiness.headers()["cache-control"], "no-store");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &to_bytes(readiness.into_body(), 4096).await.unwrap()
        )
        .unwrap(),
        serde_json::json!({"protocol":1,"service":"wallet","ready":true})
    );

    let t = store
        .prepare(&f.sender, 200, "wallet-runtime-001", &"e".repeat(64))
        .await
        .unwrap();
    let (_, csrf) = store
        .confirmation_page(&f.cookie, t.id, "create")
        .await
        .unwrap();
    store
        .confirm(&f.cookie, t.id, &csrf, "create")
        .await
        .unwrap();
    store.claim(&f.recipient, &"e".repeat(64)).await.unwrap();
    assert_eq!(
        store
            .balance(&f.recipient)
            .await
            .unwrap()
            .balance
            .spendable_units,
        200
    );
    for column in [
        "owner_user",
        "payer_account",
        "request_key",
        "amount",
        "token_digest",
    ] {
        sqlx::raw_sql(&format!(
            "GRANT UPDATE({column}) ON core_wallet.transfers TO {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(store.check_ready().await, Err(Error::Forbidden)));
        sqlx::raw_sql(&format!(
            "REVOKE UPDATE({column}) ON core_wallet.transfers FROM {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    store.check_ready().await.unwrap();
    role.cleanup().await;
}

async fn abuse_admin(pool: &PgPool) -> (crate::abuse::AbuseStore, String) {
    use crate::abuse::{AbuseStore, admin::PolicyUpdate};
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(91099, 6).await.unwrap().secret;
    let abuse = AbuseStore::new(pool.clone(), std::collections::BTreeSet::new());
    let update:PolicyUpdate=serde_json::from_value(serde_json::json!({"operation_id":"wallet-policy-01","reason":"wallet real fixture","expected_revision":0,"config":{"version":1,"policies":[{"id":"wallet-test","name":"Wallet test","mode":"fixed_text","fixed_text":"synthetic response","public_models":["public-demo"],"clients":{},"limits":{"requests_per_minute_per_user":10,"requests_per_minute_global":100,"max_concurrent_per_user":2,"max_concurrent_global":10,"max_input_bytes":1024,"max_output_bytes":1024,"max_output_tokens":128,"output_bytes_per_second":0,"total_timeout_seconds":5}}]}})).unwrap();
    abuse.update_policies(&admin, &update).await.unwrap();
    (abuse, admin)
}
async fn mark_user(abuse: &crate::abuse::AbuseStore, admin: &str, user: i64) {
    use crate::abuse::admin::{CommitRequest, PreviewRequest};
    let operation = format!("wallet-mark-{user}");
    let request:PreviewRequest=serde_json::from_value(serde_json::json!({"operation_id":operation,"reason":"wallet admission test","action":"mark","expected_policy_revision":1,"targets":[{"user_id":user,"expected_revision":0}],"policy_id":"wallet-test"})).unwrap();
    let preview = abuse.preview_users(admin, &request).await.unwrap();
    let commit:CommitRequest=serde_json::from_value(serde_json::json!({"operation_id":operation,"preview_id":preview["preview_id"],"preview_hash":preview["preview_hash"]})).unwrap();
    abuse.commit_users(admin, &commit).await.unwrap();
}
#[sqlx::test(migrations = false)]
async fn marked_admission_blocks_new_funds_but_preserves_original_hold_settlement(pool: PgPool) {
    let f = setup(&pool).await;
    let identity = IdentityStore::from_pool(pool.clone());
    let sender = identity.authorize(&f.sender).await.unwrap().user_id;
    let recipient = identity.authorize(&f.recipient).await.unwrap().user_id;
    let cancel = pending(&f, "wallet-mark-cancel1", &"1".repeat(64), 100).await;
    let settle = pending(&f, "wallet-mark-settle1", &"2".repeat(64), 100).await;
    let recipient_block = pending(&f, "wallet-mark-claim01", &"3".repeat(64), 100).await;
    let draft = f
        .store
        .prepare(&f.sender, 100, "wallet-mark-draft01", &"4".repeat(64))
        .await
        .unwrap();
    let (_, draft_csrf) = f
        .store
        .confirmation_page(&f.cookie, draft.id, "create")
        .await
        .unwrap();
    let (abuse, admin) = abuse_admin(&pool).await;
    mark_user(&abuse, &admin, sender).await;
    assert!(matches!(
        f.store
            .prepare(&f.sender, 100, "wallet-mark-new001", &"5".repeat(64))
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        f.store
            .prepare(&f.sender, 100, "wallet-mark-draft01", &"4".repeat(64))
            .await
            .unwrap()
            .id,
        draft.id
    );
    assert!(matches!(
        f.store
            .confirm(&f.cookie, draft.id, &draft_csrf, "create")
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(f.store.list(&f.sender, 0).await.unwrap().len(), 4);
    let (_, cancel_csrf) = f
        .store
        .confirmation_page(&f.cookie, cancel.id, "cancel")
        .await
        .unwrap();
    f.store
        .confirm(&f.cookie, cancel.id, &cancel_csrf, "cancel")
        .await
        .unwrap();
    // Pre-mark sender hold is a real admitted operation, so original settlement remains allowed.
    let claimed = f.store.claim(&f.recipient, &"2".repeat(64)).await.unwrap();
    assert_eq!(claimed.id, settle.id);
    mark_user(&abuse, &admin, recipient).await;
    assert!(matches!(
        f.store.claim(&f.recipient, &"3".repeat(64)).await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        f.store
            .claim(&f.recipient, &"2".repeat(64))
            .await
            .unwrap()
            .id,
        settle.id
    );
    assert!(f.store.inspect(&f.recipient, &"3".repeat(64)).await.is_ok());
    let (_, csrf) = f
        .store
        .confirmation_page(&f.cookie, recipient_block.id, "cancel")
        .await
        .unwrap();
    f.store
        .confirm(&f.cookie, recipient_block.id, &csrf, "cancel")
        .await
        .unwrap();
    let b = f.store.balance(&f.sender).await.unwrap().balance;
    assert_eq!((b.spendable_units, b.reserved_units), (900, 0));
}
#[sqlx::test(migrations = false)]
async fn database_guard_rejects_hold_receipt_swaps_and_fabricated_terminal_state(pool: PgPool) {
    let f = setup(&pool).await;
    let first = pending(&f, "wallet-dbguard-001", &"6".repeat(64), 100).await;
    let second = pending(&f, "wallet-dbguard-002", &"7".repeat(64), 100).await;
    let hold: i64 =
        sqlx::query_scalar("SELECT hold_journal FROM core_wallet.transfers WHERE id=$1")
            .bind(second.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    // Even the installation owner cannot swap a persisted hold or alter original economics.
    assert!(
        sqlx::query("UPDATE core_wallet.transfers SET hold_journal=$2 WHERE id=$1")
            .bind(first.id)
            .bind(hold)
            .execute(&pool)
            .await
            .is_err()
    );
    for column in ["amount", "payer_account", "owner_user"] {
        assert!(
            sqlx::query(&format!(
                "UPDATE core_wallet.transfers SET {column}={column}+1 WHERE id=$1"
            ))
            .bind(first.id)
            .execute(&pool)
            .await
            .is_err()
        );
    }
    assert!(sqlx::query("UPDATE core_wallet.transfers SET state='cancelled',finished_at=1,final_journal=hold_journal WHERE id=$1").bind(first.id).execute(&pool).await.is_err());
    f.store.claim(&f.recipient, &"6".repeat(64)).await.unwrap();
    let (_, csrf) = f
        .store
        .confirmation_page(&f.cookie, second.id, "cancel")
        .await
        .unwrap();
    f.store
        .confirm(&f.cookie, second.id, &csrf, "cancel")
        .await
        .unwrap();
    let terminal: i64 =
        sqlx::query_scalar("SELECT final_journal FROM core_wallet.transfers WHERE id=$1")
            .bind(second.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        sqlx::query("UPDATE core_wallet.transfers SET final_journal=$2 WHERE id=$1")
            .bind(first.id)
            .bind(terminal)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("UPDATE core_wallet.transfers SET state='pending',recipient_user=NULL,finished_at=NULL,final_journal=NULL WHERE id=$1").bind(first.id).execute(&pool).await.is_err());
    assert!(
        sqlx::query("DELETE FROM core_wallet.transfers WHERE id=$1")
            .bind(first.id)
            .execute(&pool)
            .await
            .is_err()
    );
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-wallet-transfer' AND operation_key<>'fixture-credit'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, 4);
}

#[sqlx::test(migrations = false)]
async fn request_key_recovery_is_exact_sender_only_and_never_admits_funds(pool: PgPool) {
    let f = setup(&pool).await;
    let key = "a".repeat(64);
    let t = f
        .store
        .prepare(&f.sender, 150, &key, &"9".repeat(64))
        .await
        .unwrap();
    // The original operation falls behind the first owner-history page.
    for n in 0..51 {
        f.store
            .prepare(
                &f.sender,
                1,
                &format!("wallet-later-{n:020}"),
                &format!("{n:064x}"),
            )
            .await
            .unwrap();
    }
    assert!(
        !f.store
            .list(&f.sender, 0)
            .await
            .unwrap()
            .iter()
            .any(|v| v.id == t.id)
    );
    let initial: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    let recovered = f.store.query_by_request_key(&f.sender, &key).await.unwrap();
    assert_eq!(
        (recovered.id, recovered.state.as_str()),
        (t.id, "awaiting_confirmation")
    );
    assert!(matches!(
        f.store.query_by_request_key(&f.other, &key).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        f.store
            .query_by_request_key(&f.sender, &"b".repeat(64))
            .await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        f.store.query_by_request_key(&f.sender, "short").await,
        Err(Error::Invalid)
    ));
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, initial);
    assert_eq!(
        f.store
            .balance(&f.sender)
            .await
            .unwrap()
            .balance
            .reserved_units,
        0
    );
    let app = http::internal_router(f.store.clone(), vec![b'x'; 32]).unwrap();
    let req = HttpRequest::builder()
        .method("POST")
        .uri("/internal/wallet/v1/query")
        .header("Authorization", format!("Bearer {}", "x".repeat(32)))
        .header("X-Lmm-User-Credential", &f.sender)
        .header("Content-Type", "application/json")
        .body(Body::from(format!("{{\"request_key\":\"{key}\"}}")))
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["id"], t.id);
    assert_eq!(body["request_key"], key);
    assert!(body.get("token").is_none());
    assert_eq!(f.store.list(&f.sender, 0).await.unwrap().len(), 50);
}

#[tokio::test]
async fn readiness_requires_its_purpose_host_bearer_not_personal_session() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unreachable@127.0.0.1:1/unreachable")
        .unwrap();
    let app = http::internal_router(Store::new(pool), vec![b'x'; 32]).unwrap();
    for bearer in [
        None,
        Some("personal-session"),
        Some("wrong-purpose-host-token-000000000000"),
    ] {
        let mut request = HttpRequest::builder()
            .method("POST")
            .uri("/internal/wallet/v1/ready")
            .header("content-type", "application/json")
            .header("x-lmm-protocol", "1");
        if let Some(token) = bearer {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from("{}")).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = app
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/internal/wallet/v1/ready")
                .header("content-type", "application/json")
                .header("x-lmm-protocol", "1")
                .header("authorization", format!("Bearer {}", "x".repeat(32)))
                .header("authorization", format!("Bearer {}", "x".repeat(32)))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// @mark wallet.http_identity_boundary
#[sqlx::test(migrations = false)]
async fn repeated_identity_headers_cannot_prepare_a_transfer(pool: PgPool) {
    let f = setup(&pool).await;
    let app = http::internal_router(f.store.clone(), vec![b'x'; 32]).unwrap();
    let initial: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut statuses = Vec::new();
    for header in ["authorization", "x-lmm-user-credential"] {
        for second in ["", "ambiguous-other-identity"] {
            let request = HttpRequest::builder()
                .method("POST")
                .uri("/internal/wallet/v1/prepare")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", "x".repeat(32)))
                .header("x-lmm-user-credential", &f.sender)
                .header(header, second)
                .body(Body::from(
                    serde_json::json!({
                        "amount_credits": 100,
                        "request_key": "wallet-duplicate-identity",
                        "token": "a".repeat(64)
                    })
                    .to_string(),
                ))
                .unwrap();
            statuses.push(app.clone().oneshot(request).await.unwrap().status());
        }
    }
    let transfers: i64 = sqlx::query_scalar("SELECT count(*) FROM core_wallet.transfers")
        .fetch_one(&pool)
        .await
        .unwrap();
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(statuses, vec![StatusCode::UNAUTHORIZED; 4]);
    assert_eq!(transfers, 0);
    assert_eq!(journals, initial);
}

#[tokio::test]
async fn repeated_browser_origin_is_rejected_before_cookie_or_database_access() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unreachable@127.0.0.1:1/unreachable")
        .unwrap();
    let app = http::browser_router(Store::new(pool), "https://wallet.example".into()).unwrap();
    for second in ["https://wallet.example", "https://evil.example", ""] {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/core/v1/wallet/confirm/create/1")
            .header("content-type", "application/json")
            .header("origin", "https://wallet.example")
            .header("origin", second)
            .body(Body::from(
                serde_json::json!({"csrf": "a".repeat(64)}).to_string(),
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn readiness_rejects_wrong_protocol_user_credentials_and_nonempty_body_without_db() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unreachable@127.0.0.1:1/unreachable")
        .unwrap();
    let app = http::internal_router(Store::new(pool), vec![b'x'; 32]).unwrap();
    for (protocol, user, body, status) in [
        (None, false, "{}", StatusCode::BAD_REQUEST),
        (Some("2"), false, "{}", StatusCode::BAD_REQUEST),
        (Some("1"), true, "{}", StatusCode::FORBIDDEN),
        (
            Some("1"),
            false,
            "{\"id\":17}",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (Some("1"), false, "null", StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let mut request = HttpRequest::builder()
            .method("POST")
            .uri("/internal/wallet/v1/ready")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", "x".repeat(32)));
        if let Some(value) = protocol {
            request = request.header("x-lmm-protocol", value);
        }
        if user {
            request = request.header("x-lmm-user-credential", "personal-session");
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(Body::from(body)).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
}
