//! Compile the owned module directly until its separate production wiring lands.
use lmm_core::{accounts, identity};
#[path = "../src/topup_info_http.rs"]
mod topup_info_http;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use identity::{IdentityStore, IssuedCredential};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

const PATH: &str = "/api/user/topup/info";

async fn request(app: &Router, method: Method, credentials: &[&str]) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(PATH);
    for credential in credentials {
        builder = builder.header("authorization", *credential);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

fn authorization(credential: &IssuedCredential) -> String {
    format!("Bearer {}", credential.secret)
}

#[tokio::test]
async fn missing_or_ambiguous_authentication_is_rejected_without_database_access() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = topup_info_http::unconfigured_router(IdentityStore::from_pool(pool));
    for headers in [
        vec![],
        vec!["Basic unused"],
        vec!["Bearer "],
        vec!["Bearer first", "Bearer second"],
    ] {
        let (status, body) = request(&app, Method::GET, &headers).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], "AUTH_UNAUTHORIZED");
        assert!(body.get("data").is_none());
    }
}

#[sqlx::test(migrations = false)]
async fn unconfigured_catalog_is_truthful_read_only_and_session_scoped(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 0).await.unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            accounts::Account {
                kind: accounts::AccountKind::Personal,
                id: 7,
            },
            lmm_core::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let app = topup_info_http::unconfigured_router(identity.clone());
    let before = persisted_state(&pool).await;
    let (status, body) = request(&app, Method::GET, &[&authorization(&session)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    let data = body["data"].as_object().unwrap();
    assert_eq!(data["payment_available"], false);
    assert_eq!(
        data["payment_unavailable_reason"],
        "web_checkout_not_configured"
    );
    assert_eq!(data["credit_metadata_available"], false);
    for (name, value) in data {
        if name.starts_with("enable_") {
            assert_eq!(value, &json!(false), "{name}");
        }
    }
    for name in [
        "pay_methods",
        "creem_products",
        "waffo_pay_methods",
        "amount_options",
    ] {
        assert_eq!(data[name], json!([]), "{name}");
    }
    assert_eq!(data["discount"], json!({}));
    for name in [
        "min_topup",
        "stripe_min_topup",
        "waffo_min_topup",
        "waffo_pancake_min_topup",
    ] {
        assert_eq!(data[name], 0, "{name}");
    }
    assert_eq!(data["amount_unit"], "CREDIT");
    assert!(data.get("developer_access_granted").is_none());
    let (status, body) = request(&app, Method::GET, &[&authorization(&key)]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = request(&app, Method::POST, &[&authorization(&session)]).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(persisted_state(&pool).await, before);
    identity.logout(&session.secret).await.unwrap();
    let (status, body) = request(&app, Method::GET, &[&authorization(&session)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

async fn persisted_state(pool: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'credentials',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM core_identity.credentials c),
          'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),
          'entries',(SELECT jsonb_agg(to_jsonb(e) ORDER BY journal_id,ledger_account_id) FROM core_billing.ledger_entries e),
          'charges',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM core_charging.requests r))",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
