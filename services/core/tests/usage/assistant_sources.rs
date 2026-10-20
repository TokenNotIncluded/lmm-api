use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

async fn read(pool: &PgPool, session: &str, path: &str) -> Value {
    let app = lmm_core::usage::router(lmm_core::usage::UsageStore::new(pool.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .header("Authorization", format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let code = response.status();
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(code, StatusCode::OK, "{value}");
    value["data"].clone()
}

pub(super) async fn assert_sponsored_usage(pool: &PgPool, session: &str) {
    let data = read(pool, session, "/api/log/self").await;
    assert_eq!(data["total"], 2);
    for row in data["items"].as_array().unwrap() {
        assert_eq!(row["platform_sponsored"], true);
        let other: Value = serde_json::from_str(row["other"].as_str().unwrap()).unwrap();
        assert_eq!(other["credential_source"], "assistant_session");
        assert!(row["token_id"].is_null());
    }
    let stats = read(pool, session, "/api/log/self/stat").await;
    assert_eq!(stats["gross_settled_credits"], 98);
    assert_eq!(stats["platform_sponsored_quota"], 98);
    assert_eq!(
        stats["personal_wallet_quota"], 0,
        "platform spending must not be presented as user's wallet consumption"
    );
    assert_eq!(stats["reserved_credits"], 0);
}

#[sqlx::test]
async fn usage_assistant_paid_and_unknown_sources_preserve_consumption_and_holds(pool: PgPool) {
    let (_, ledger, session, _, _, account) = fixture(&pool, 100_000).await;
    let good = Provider::start(true).await;
    let complete_service = service(&pool, &good);
    let complete_binding = binding(
        &complete_service,
        &session,
        "usage-assistant-complete",
        "hello",
    )
    .await;
    let complete = complete_service
        .reserve_text(&session, &complete_binding, "hello")
        .await
        .unwrap();
    assert_eq!(complete.state, State::Settled);
    let incomplete_provider = Provider::start(false).await;
    let incomplete_service = service(&pool, &incomplete_provider);
    let incomplete_binding = binding(
        &incomplete_service,
        &session,
        "usage-assistant-unknown",
        "hello",
    )
    .await;
    let unknown = incomplete_service
        .reserve_text(&session, &incomplete_binding, "hello")
        .await
        .unwrap();
    assert_eq!(unknown.state, State::Unknown);
    let before = ledger.balance(account).await.unwrap();
    let logs = read(&pool, &session, "/api/log/self").await;
    assert_eq!(logs["total"], 2);
    let rows = logs["items"].as_array().unwrap();
    let settled = rows
        .iter()
        .find(|r| r["request_id"] == complete.request_id)
        .unwrap();
    assert_eq!(settled["quota"], complete.settled_credits.unwrap());
    assert!(settled["token_id"].is_null());
    let pending = rows
        .iter()
        .find(|r| r["request_id"] == unknown.request_id)
        .unwrap();
    assert!(pending["quota"].is_null());
    assert!(pending["net_consumption_credits"].is_null());
    assert_eq!(pending["quota_money_available"], false);
    let other: Value = serde_json::from_str(pending["other"].as_str().unwrap()).unwrap();
    assert_eq!(other["credential_source"], "assistant_session");
    assert_eq!(other["reserved_credits"], unknown.reserved_credits);
    let stats = read(&pool, &session, "/api/log/self/stat").await;
    assert_eq!(
        stats["net_consumption_credits"],
        complete.settled_credits.unwrap()
    );
    assert_eq!(stats["reserved_credits"], unknown.reserved_credits);
    assert_eq!(stats["incomplete_count"], 1);
    assert_eq!(before, ledger.balance(account).await.unwrap());
    assert_eq!(good.hits.load(Ordering::SeqCst), 1);
    assert_eq!(incomplete_provider.hits.load(Ordering::SeqCst), 1);
}
