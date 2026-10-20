use super::*;
use crate::{billing::ledger::Ledger, identity::IdentityStore};
#[test]
fn native_microusd_exact_policy_and_canonical_hash() {
    assert_eq!(credits(2), Ok(1));
    assert_eq!(credits(1_000_000), Ok(crate::billing::CREDITS_PER_USD));
    for v in [-2, 0, 1, 3, i64::MAX] {
        assert_eq!(credits(v), Err(Error::Invalid));
    }
    assert_eq!(credits(i64::MAX - 1), Ok((i64::MAX - 1) / 2));
    let i = intent(1000);
    assert_eq!(
        hash(&i).unwrap(),
        "38b525d0afe0c63030c847ca24b977a1648027d115860f73b3c0879989c9cd35"
    );
}
fn intent(expires: i64) -> Intent {
    Intent {
        audience: AUDIENCE.into(),
        grant: "grant".into(),
        owner: Owner {
            user: "7".into(),
            account: "7".into(),
        },
        installation: "install".into(),
        release: "release".into(),
        tool: "echo".into(),
        handle: "a".repeat(64),
        quote: "c".repeat(64),
        arguments_hash: "b".repeat(64),
        currency: "USD".into(),
        generation: 1,
        amount: 20,
        max_calls: 2,
        max_amount: 40,
        expires,
    }
}
struct Fixture {
    store: ToolMarketStore,
    identity: IdentityStore,
    buyer: String,
    seller: String,
    other: String,
    ledger: Ledger,
    buyer_id: i64,
    seller_id: i64,
    intent: Intent,
    evidence: Evidence,
}
async fn fixture(pool: &PgPool) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let present: bool =
        sqlx::query_scalar("SELECT to_regclass('core_tool_market.calls') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap();
    if !present {
        sqlx::raw_sql(SCHEMA).execute(pool).await.unwrap();
    }
    let identity = IdentityStore::from_pool(pool.clone());
    let buyer = identity.bootstrap_user(7, 1).await.unwrap().secret;
    let seller = identity.bootstrap_user(9, 1).await.unwrap().secret;
    let other = identity.bootstrap_user(11, 1).await.unwrap().secret;
    let buyer_id = identity.authorize(&buyer).await.unwrap().owner_account_id;
    let seller_id = identity.authorize(&seller).await.unwrap().owner_account_id;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(buyer_id).await.unwrap();
    ledger.open_wallet(seller_id).await.unwrap();
    let outcome = ledger
        .execute(&Request {
            scope: "tool-market-test".into(),
            operation_key: "opening".into(),
            actor_user_id: Some(7),
            reason: "Synthetic disposable fixture".into(),
            action: Action::Credit {
                account_id: buyer_id,
                amount_units: Credits::try_from(100).unwrap(),
                payment_reference: "synthetic-tool-market-opening".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(outcome, Outcome::Posted { .. }));
    sqlx::query("INSERT INTO core_tool_market.publishers(user_id,active) VALUES(9,TRUE)")
        .execute(pool)
        .await
        .unwrap();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(pool)
            .await
            .unwrap();
    let store = ToolMarketStore::from_pool(pool.clone());
    store
        .register_quote(
            &seller,
            &Quote {
                id: "c".repeat(64),
                release: "release".into(),
                tool: "echo".into(),
                version: "1".into(),
                price_hash: "c".repeat(64),
                amount: 20,
                currency: "USD".into(),
                expires: now + 3600,
            },
        )
        .await
        .unwrap();
    let intent = intent(now + 1800);
    let evidence = store.confirm(&buyer, &intent).await.unwrap();
    Fixture {
        store,
        identity,
        buyer,
        seller,
        other,
        ledger,
        buyer_id,
        seller_id,
        intent,
        evidence,
    }
}
fn call(f: &Fixture, key: &str) -> Call {
    Call {
        id: hash(&["toolmarket/v1", "7:7", key]).unwrap(),
        installation: f.intent.installation.clone(),
        tool: f.intent.tool.clone(),
        quote: f.intent.quote.clone(),
        currency: "USD".into(),
        amount: f.intent.amount,
        release: f.intent.release.clone(),
        grant: f.intent.grant.clone(),
        handle: f.intent.handle.clone(),
        arguments_hash: f.intent.arguments_hash.clone(),
        approval: f.evidence.id.clone(),
        generation: f.intent.generation,
        idempotency_key: key.into(),
    }
}
async fn journal_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-tool-market'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
#[sqlx::test(migrations = false)]
async fn real_ledger_restart_replay_and_frozen_result(pool: PgPool) {
    let f = fixture(&pool).await;
    let c = call(&f, "one");
    let r = f.store.reserve(&f.buyer, &c).await.unwrap();
    assert_eq!(r.credits, 10);
    assert_eq!(r.state, "reserved");
    assert_eq!(f.store.reserve(&f.buyer, &c).await.unwrap(), r);
    assert_eq!(journal_count(&pool).await, 1);
    let reopened = ToolMarketStore::from_pool(pool.clone());
    assert_eq!(reopened.reconcile(&c).await.unwrap(), Some(r.clone()));
    assert_eq!(reopened.query(&f.other, &c.id).await.unwrap(), None);
    assert_eq!(
        reopened
            .commit(&c.id, &r.reservation, &"d".repeat(64))
            .await,
        Err(Error::Conflict)
    );
    reopened
        .mark_dispatched(&f.buyer, &c.id, &r.reservation)
        .await
        .unwrap();
    assert_eq!(
        reopened.release_undispatched(&c.id, &r.reservation).await,
        Err(Error::Conflict)
    );
    let done = reopened
        .commit(&c.id, &r.reservation, &"d".repeat(64))
        .await
        .unwrap();
    assert_eq!(done.state, "committed");
    assert_eq!(
        reopened
            .commit(&c.id, &r.reservation, &"d".repeat(64))
            .await
            .unwrap(),
        done
    );
    assert_eq!(
        reopened
            .commit(&c.id, &r.reservation, &"e".repeat(64))
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(journal_count(&pool).await, 2);
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) s").fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    // Seller receives a real ledger transfer; no fabricated store order exists.
    assert_eq!(
        f.ledger.balance(f.seller_id).await.unwrap().available_units,
        10
    );
    assert_eq!(
        f.ledger.balance(f.buyer_id).await.unwrap().available_units,
        90
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM core_commerce.orders")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
#[sqlx::test(migrations = false)]
async fn reject_tampering_revoke_and_release_only_before_dispatch(pool: PgPool) {
    let f = fixture(&pool).await;
    let c = call(&f, "one");
    let mut bad = c.clone();
    bad.amount += 2;
    assert_eq!(f.store.reserve(&f.buyer, &bad).await, Err(Error::Forbidden));
    assert_eq!(f.store.reserve(&f.other, &c).await, Err(Error::Invalid));
    bad = c.clone();
    bad.id = "invented".into();
    assert_eq!(f.store.reserve(&f.buyer, &bad).await, Err(Error::Invalid));
    assert_eq!(journal_count(&pool).await, 0);
    let r = f.store.reserve(&f.buyer, &c).await.unwrap();
    let released = f
        .store
        .release_undispatched(&c.id, &r.reservation)
        .await
        .unwrap();
    assert_eq!(released.state, "released");
    assert_eq!(
        f.store
            .release_undispatched(&c.id, &r.reservation)
            .await
            .unwrap(),
        released
    );
    assert_eq!(
        f.store
            .mark_dispatched(&f.buyer, &c.id, &r.reservation)
            .await,
        Err(Error::Conflict)
    );
    let c2 = call(&f, "two");
    let r2 = f.store.reserve(&f.buyer, &c2).await.unwrap();
    f.store.revoke(&f.buyer, &f.evidence.id).await.unwrap();
    assert_eq!(
        f.store
            .mark_dispatched(&f.buyer, &c2.id, &r2.reservation)
            .await,
        Err(Error::Forbidden)
    );
    let c3 = call(&f, "three");
    assert_eq!(f.store.reserve(&f.buyer, &c3).await, Err(Error::Forbidden));
    assert_eq!(
        f.ledger.balance(f.buyer_id).await.unwrap().available_units,
        90
    );
}
#[sqlx::test(migrations = false)]
async fn session_publisher_and_quote_integrity(pool: PgPool) {
    let f = fixture(&pool).await;
    assert_eq!(
        f.store.authorize(&f.buyer, "9", "toolmarket.execute").await,
        Err(Error::Forbidden)
    );
    assert_eq!(
        f.store.authorize(&f.buyer, "7", "unknown").await,
        Err(Error::Forbidden)
    );
    assert_eq!(
        f.store.authorize(&f.buyer, "7", "toolmarket.review").await,
        Err(Error::Forbidden)
    );
    let mut i = f.intent.clone();
    i.owner.account = "9".into();
    assert_eq!(f.store.confirm(&f.buyer, &i).await, Err(Error::Forbidden));
    let q = Quote {
        id: "d".repeat(64),
        release: "new-release".into(),
        tool: "echo".into(),
        version: "1".into(),
        price_hash: "d".repeat(64),
        amount: 21,
        currency: "USD".into(),
        expires: f.intent.expires,
    };
    assert_eq!(
        f.store.register_quote(&f.seller, &q).await,
        Err(Error::Invalid)
    );
    let mut q = q;
    q.amount = 20;
    assert_eq!(
        f.store.register_quote(&f.buyer, &q).await,
        Err(Error::Forbidden)
    );
    let mut i = f.intent.clone();
    i.amount += 2;
    assert_eq!(f.store.confirm(&f.buyer, &i).await, Err(Error::Conflict));
    // Session-only check rejects an otherwise valid real API key.
    let key = f
        .identity
        .issue_key(
            &f.buyer,
            crate::accounts::Account {
                kind: crate::accounts::AccountKind::Personal,
                id: 7,
            },
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await;
    let key = key.unwrap();
    assert_eq!(
        f.store
            .authorize(&key.secret, "7", "toolmarket.execute")
            .await,
        Err(Error::Forbidden)
    );
}

#[sqlx::test(migrations = false)]
async fn free_invocation_keeps_durable_consent_and_never_posts_zero_money(pool: PgPool) {
    let mut f = fixture(&pool).await;
    let q = Quote {
        id: "f".repeat(64),
        release: "free-release".into(),
        tool: "echo".into(),
        version: "1".into(),
        price_hash: "f".repeat(64),
        amount: 0,
        currency: "USD".into(),
        expires: f.intent.expires + 100,
    };
    // A publisher may test its own free tool; no self-transfer is posted.
    sqlx::query("INSERT INTO core_tool_market.publishers(user_id,active) VALUES(7,TRUE)")
        .execute(&pool)
        .await
        .unwrap();
    f.store.register_quote(&f.buyer, &q).await.unwrap();
    f.intent.grant = "free-grant".into();
    f.intent.quote = q.id;
    f.intent.release = q.release;
    f.intent.amount = 0;
    f.intent.max_amount = 0;
    f.intent.max_calls = 1;
    f.evidence = f.store.confirm(&f.buyer, &f.intent).await.unwrap();
    let c = call(&f, "free-one");
    let r = f.store.reserve(&f.buyer, &c).await.unwrap();
    assert_eq!(r.journal_id, 0);
    assert_eq!(r.credits, 0);
    f.store
        .mark_dispatched(&f.buyer, &c.id, &r.reservation)
        .await
        .unwrap();
    let done = f
        .store
        .commit(&c.id, &r.reservation, &"e".repeat(64))
        .await
        .unwrap();
    assert_eq!(done.state, "committed");
    assert_eq!(
        f.store.reserve(&f.buyer, &call(&f, "free-two")).await,
        Err(Error::Forbidden)
    );
    assert_eq!(journal_count(&pool).await, 0);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.external_consumption")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = false)]
async fn concurrent_grant_limit_is_atomic(pool: PgPool) {
    let f = fixture(&pool).await;
    async fn retry(f: &Fixture, c: Call) -> Result<Receipt> {
        for _ in 0..8 {
            let r = f.store.reserve(&f.buyer, &c).await;
            if r != Err(Error::Retry) {
                return r;
            }
            tokio::task::yield_now().await;
        }
        Err(Error::Retry)
    }
    let (a, b, c) = tokio::join!(
        retry(&f, call(&f, "a")),
        retry(&f, call(&f, "b")),
        retry(&f, call(&f, "c"))
    );
    let results = [a, b, c];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(Error::Forbidden))
            .count(),
        1
    );
    assert_eq!(journal_count(&pool).await, 2);
    assert_eq!(
        f.ledger.balance(f.buyer_id).await.unwrap().reserved_units,
        20
    );
    let used: i64 =
        sqlx::query_scalar("SELECT used_calls FROM core_tool_market.approvals WHERE id=$1")
            .bind(&f.evidence.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(used, 2);
}

#[tokio::test]
async fn internal_transport_requires_distinct_host_and_user_credentials() {
    use axum::{
        body::Body,
        http::{Request as HttpRequest, StatusCode},
    };
    use tower::ServiceExt;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let router = super::http::router(
        ToolMarketStore::from_pool(pool),
        b"synthetic-host-token-at-least-32-chars",
    )
    .unwrap();
    for (token, protocol, expected) in [
        ("wrong-token", "1", StatusCode::UNAUTHORIZED),
        (
            "synthetic-host-token-at-least-32-chars",
            "2",
            StatusCode::BAD_REQUEST,
        ),
        (
            "synthetic-host-token-at-least-32-chars",
            "1",
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let req = HttpRequest::builder()
            .method("POST")
            .uri("/internal/tool-market/v1/authorize")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("x-lmm-protocol", protocol)
            .body(Body::from(
                r#"{"account":"7","action":"toolmarket.execute"}"#,
            ))
            .unwrap();
        assert_eq!(
            router.clone().oneshot(req).await.unwrap().status(),
            expected
        );
    }
}

#[path = "split_tests.rs"]
mod split;
