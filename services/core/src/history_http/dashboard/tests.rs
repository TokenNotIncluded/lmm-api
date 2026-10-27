//! Real disposable PostgreSQL + actual Charging/TransactionalLedger receipts.
//! No production accounts, providers, schema migrations or success-only adapters.
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{Charging, Outcome, Price, Request as ChargeRequest, Usage},
        ledger::{Action, Ledger, Outcome as LedgerOutcome, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    core_schema,
    funding::FundingPolicy,
    http::{self, Capabilities},
    identity::{IdentityStore, IssuedCredential},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

#[sqlx::test(migrations = false)]
async fn dashboard_views_use_settled_receipts_and_preserve_scopes(pool: PgPool) {
    let f = fixture(&pool).await;
    let other = f.identity.bootstrap_user(8, 0).await.unwrap();
    let admin = f.identity.bootstrap_user(9, 5).await.unwrap();
    let other_key = f
        .identity
        .issue_key(
            &other.secret,
            Account {
                kind: AccountKind::Personal,
                id: 8,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'Other key','default')")
        .bind(other_key.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(8,'default','dashboard-test')")
        .execute(&pool)
        .await
        .unwrap();
    let other_account = f
        .identity
        .authorize(&other_key.secret)
        .await
        .unwrap()
        .owner_account_id;
    credit(&f.ledger, other_account, "dashboard-other-credit").await;

    let alternate_key = f
        .identity
        .issue_key(
            &f.session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'Alternate key','default')")
        .bind(alternate_key.id)
        .execute(&pool)
        .await
        .unwrap();

    settled(&f, &f.key.secret, "dashboard-personal", Outcome::Completed).await;
    settled(&f, &f.key.secret, "dashboard-refunded", Outcome::Completed).await;
    f.charging
        .refund("dashboard-refunded", "partial", 20, "dashboard test refund")
        .await
        .unwrap();
    settled(
        &f,
        &alternate_key.secret,
        "dashboard-alternate",
        Outcome::Completed,
    )
    .await;
    settled(&f, &other_key.secret, "dashboard-other", Outcome::Completed).await;

    let team_id = f.identity.create_team(&f.session.secret).await.unwrap();
    let team_key = f
        .identity
        .issue_key(
            &f.session.secret,
            Account {
                kind: AccountKind::Team,
                id: team_id,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    let team_account = f
        .identity
        .authorize(&team_key.secret)
        .await
        .unwrap()
        .owner_account_id;
    credit(&f.ledger, team_account, "dashboard-team-credit").await;
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'Team key','default')")
        .bind(team_key.id)
        .execute(&pool)
        .await
        .unwrap();
    settled(&f, &team_key.secret, "dashboard-team", Outcome::Completed).await;

    f.charging
        .reserve(&f.key.secret, &spec("dashboard-held"), &WORKER)
        .await
        .unwrap();

    let history_source: i64 = sqlx::query_scalar(
        "INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete)
         VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'dashboard-test',1,true) RETURNING source_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,model_name)
         VALUES($1,1,'dashboard-history-event',repeat('c',64),7,$2,floor(extract(epoch FROM clock_timestamp()))::bigint,2,9000000,'historical-model')",
    )
    .bind(history_source)
    .bind(f.account)
    .execute(&pool)
    .await
    .unwrap();

    let before = money_state(&pool).await;
    let (status, flow) = get(&f.app, "/api/data/flow", Some(&admin.secret)).await;
    assert_eq!(status, StatusCode::OK, "{flow}");
    assert_eq!(flow["availability"], "partial");
    assert_eq!(flow["coverage"]["historical_imports"], false);
    assert_eq!(flow["coverage"]["complete"], false);
    assert_eq!(
        flow["coverage"]["missing_sources"][0],
        "historical_usage_receipts"
    );
    assert_eq!(
        flow["coverage"]["unavailable_dimensions"],
        json!(["node", "channel"])
    );
    let flows = flow["data"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    let personal_flow = flows
        .iter()
        .find(|row| row["token_id"] == f.key.id)
        .unwrap();
    assert_eq!(personal_flow["user_id"], 7);
    assert_eq!(personal_flow["token_name"], "Test key");
    assert_eq!(personal_flow["count"], 2);
    assert_eq!(personal_flow["gross_settled_credits"], 98);
    assert_eq!(personal_flow["refunded_credits"], 20);
    assert_eq!(personal_flow["quota"], 78);
    assert!(personal_flow.get("node_name").is_none());
    assert!(personal_flow.get("channel_id").is_none());

    let (status, self_flow) = get(&f.app, "/api/data/flow/self", Some(&f.session.secret)).await;
    assert_eq!(status, StatusCode::OK, "{self_flow}");
    assert_eq!(self_flow["coverage"]["scope"], "actor_personal_key");
    assert_eq!(self_flow["data"].as_array().unwrap().len(), 2);
    assert!(
        self_flow["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["user_id"] == 7 && row["payer_kind"] == "personal")
    );
    assert_eq!(
        self_flow["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["quota"].as_i64().unwrap())
            .sum::<i64>(),
        127
    );

    let (status, users) = get(&f.app, "/api/data/users", Some(&admin.secret)).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    assert_eq!(users["coverage"]["bucket_seconds"], 3600);
    assert_eq!(users["data"].as_array().unwrap().len(), 3);
    let personal = users["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["user_id"] == 7)
        .unwrap();
    assert_eq!(personal["count"], 3);
    assert_eq!(personal["gross_settled_credits"], 147);
    assert_eq!(personal["refunded_credits"], 20);
    assert_eq!(personal["quota"], 127);
    assert_eq!(personal["token_used"], 90);
    assert!(
        users["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["model_name"] != "historical-model")
    );
    assert_eq!(money_state(&pool).await, before);

    for path in ["/api/data/flow", "/api/data/users"] {
        let (status, body) = get(&f.app, path, Some(&f.session.secret)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
    let (status, body) = get(&f.app, "/api/data/flow/self", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, body) = get(
        &f.app,
        "/api/data/flow/self?username=other",
        Some(&f.session.secret),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

async fn money_state(pool: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'requests',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM core_charging.requests r),
          'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY operation_id) FROM core_charging.ledger_commands c),
          'refunds',(SELECT jsonb_agg(to_jsonb(r) ORDER BY request_id,refund_id) FROM core_charging.refunds r),
          'journals',(SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM core_billing.ledger_journals j),
          'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),
          'entries',(SELECT jsonb_agg(to_jsonb(e) ORDER BY journal_id,ledger_account_id) FROM core_billing.ledger_entries e))",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn dashboard_fails_closed_on_a_missing_final_receipt(pool: PgPool) {
    let f = fixture(&pool).await;
    settled(
        &f,
        &f.key.secret,
        "dashboard-corrupt-final",
        Outcome::Completed,
    )
    .await;
    corrupt_final_event(&pool, "dashboard-corrupt-final").await;
    let admin = f.identity.bootstrap_user(9, 5).await.unwrap();
    for path in ["/api/data/flow", "/api/data/users"] {
        let (status, body) = get(&f.app, path, Some(&admin.secret)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}: {body}");
        assert_eq!(body["success"], false);
        assert!(body.get("data").is_none());
    }
    let (status, body) = get(&f.app, "/api/data/flow/self", Some(&f.session.secret)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

// Corrupt only the disposable SQLx fixture, then restore the append-only trigger.
async fn corrupt_final_event(pool: &PgPool, request_id: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events DISABLE TRIGGER usage_immutable")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("DELETE FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'")
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events ENABLE TRIGGER usage_immutable")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}
const WORKER: [u8; 32] = [9; 32];
struct Fixture {
    identity: IdentityStore,
    charging: Charging,
    ledger: Ledger,
    session: IssuedCredential,
    key: IssuedCredential,
    account: i64,
    app: Router,
}
async fn credit(ledger: &Ledger, account: i64, id: &str) {
    ledger.open_wallet(account).await.unwrap();
    assert!(matches!(
        ledger
            .execute(&LedgerRequest {
                scope: "usage-test".into(),
                operation_key: id.into(),
                actor_user_id: Some(7),
                reason: "Synthetic private fixture".into(),
                action: Action::Credit {
                    account_id: account,
                    amount_units: Credits::try_from(1_000_000).unwrap(),
                    payment_reference: format!("usage-fixture:{id}")
                }
            })
            .await
            .unwrap(),
        LedgerOutcome::Posted { .. }
    ));
}
async fn fixture(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 0).await.unwrap();
    let account = identity
        .authorize(&session.secret)
        .await
        .unwrap()
        .owner_account_id;
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
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'Test key','default')").bind(key.id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(7,'default','usage-fixture')").execute(pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    credit(&ledger, account, "personal-credit").await;
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    // Exercise the production mount, not only the standalone module router.
    let app = http::configured_router(
        pool.clone(),
        Capabilities {
            identity: true,
            ..Default::default()
        },
    );
    Fixture {
        identity,
        charging,
        ledger,
        session,
        key,
        account,
        app,
    }
}
fn spec(id: &str) -> ChargeRequest {
    ChargeRequest {
        id: id.into(),
        fingerprint: [3; 32],
        reserve: 100,
        lease_seconds: 300,
        maximum_seconds: 600,
        price: Price {
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "v1".into(),
            model: "test-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 500_000,
            output_per_million: 2_000_000,
        },
    }
}
fn counts() -> Usage {
    Usage {
        input: 10,
        cached: 2,
        output: 20,
        ..Default::default()
    }
}
async fn settled(f: &Fixture, key: &str, id: &str, outcome: Outcome) {
    f.charging.reserve(key, &spec(id), &WORKER).await.unwrap();
    f.charging.start(id, &WORKER).await.unwrap();
    f.charging
        .checkpoint(id, &WORKER, "provider-final", counts())
        .await
        .unwrap();
    f.charging
        .settle(id, &WORKER, counts(), outcome)
        .await
        .unwrap();
}
async fn get(app: &Router, path: &str, secret: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(path);
    if let Some(secret) = secret {
        request = request.header("authorization", format!("Bearer {secret}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    (status, data)
}
