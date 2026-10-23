//! Real PostgreSQL transport checks with synthetic installation-owned funds.
//! These do not authorize any real snapshot or prove an application cutover.
#[path = "common/runtime.rs"]
mod runtime;

use axum::http::StatusCode;
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        ledger::Ledger,
        legacy_escrow::{LegacyEscrowStore, http::internal_router},
    },
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sqlx::{PgPool, types::Json};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    time::Duration,
};

const MIGRATION: &str = "synthetic-legacy-http-v1";
const PROJECT: i64 = 9_007_199_254_740_993;
const CLAIM: i64 = 9_007_199_254_740_995;

async fn owner_import(pool: &PgPool, function: &str, payload: Value) {
    assert!(matches!(
        function,
        "import_opening_balance" | "import_legacy_escrow_batch"
    ));
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let _: Json<Value> = sqlx::query_scalar(&format!("SELECT core_billing.{function}($1)"))
        .bind(Json(payload))
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn post(
    client: &reqwest::Client,
    session: &str,
    action: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let response = client
        .post(format!(
            "http://core/internal/bounties/v1/legacy-escrow/{action}"
        ))
        .bearer_auth("s".repeat(32))
        .header("x-lmm-protocol", "1")
        .header("x-lmm-user-credential", session)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    let body = response.bytes().await.unwrap();
    assert!(body.len() <= 65536);
    (status, serde_json::from_slice(&body).unwrap())
}

#[sqlx::test(migrations = false)]
async fn original_participant_submits_and_original_owner_pays_over_private_http(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identities = IdentityStore::from_pool(pool.clone());
    let publisher = identities.bootstrap_user(101, 1).await.unwrap();
    let participant = identities.bootstrap_user(202, 1).await.unwrap();
    let stranger = identities.bootstrap_user(303, 1).await.unwrap();
    let payer = identities
        .authorize(&publisher.secret)
        .await
        .unwrap()
        .owner_account_id;
    let receiver = identities
        .authorize(&participant.secret)
        .await
        .unwrap()
        .owner_account_id;
    let stranger_account = identities
        .authorize(&stranger.secret)
        .await
        .unwrap()
        .owner_account_id;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch from clock_timestamp())::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    for (user, account, units) in [
        (101, payer, 100),
        (202, receiver, 0),
        (303, stranger_account, 0),
    ] {
        owner_import(
            &pool,
            "import_opening_balance",
            json!({
                "migration_id":MIGRATION,"source_archive_sha256":"11".repeat(32),
                "mapping_sha256":"22".repeat(32),"source_entity_key":format!("users:{user}"),
                "source_row_sha256":"33".repeat(32),"account_id":account,
                "available_units":units,"reserved_units":0,"affiliate_units":0,"observed_at":now,
            }),
        )
        .await;
    }
    owner_import(
        &pool,
        "import_legacy_escrow_batch",
        json!({
            "migration_id":MIGRATION,"source_archive_sha256":"11".repeat(32),
            "mapping_sha256":"22".repeat(32),"review_proof_sha256":"77".repeat(32),
            "unit":"credit_500k_usd","observed_at":now,
            "projects":[{"source_project_id":PROJECT,"owner_user_id":101,"payer_account_id":payer,
                "source_row_sha256":"55".repeat(32),"source_parent_plan_sha256":"66".repeat(32),
                "escrow_units":20,"status":"published","disputes":[],
                "claims":[{"source_challenge_id":CLAIM,"participant_user_id":202,
                    "recipient_account_id":receiver,"source_row_sha256":"44".repeat(32),
                    "rights_status":"active_future_reward","source_reward_quota":7000,
                    "payout_units":7,"status":"accepted","rejected_at":0,"paid_at":0}]}]
        }),
    )
    .await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    let app = internal_router(
        LegacyEscrowStore::from_pool(runtime.pool.clone()),
        &"s".repeat(32),
    )
    .unwrap();
    let socket_root = std::env::temp_dir().join(format!(
        "lmm-escrow-http-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&socket_root)
        .unwrap();
    let socket = socket_root.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .unix_socket(socket.as_path())
        .no_proxy()
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    assert_eq!(ledger.balance(payer).await.unwrap().available_units, 100);
    assert_eq!(ledger.balance(payer).await.unwrap().reserved_units, 20);
    let original_claim: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM core_billing.legacy_escrow_claims c WHERE migration_id=$1 AND source_challenge_id=$2")
        .bind(MIGRATION).bind(CLAIM).fetch_one(&pool).await.unwrap();
    let submit = json!({"migration_id":MIGRATION,"source_project_id":PROJECT,
        "source_challenge_id":CLAIM,"operation_key":"submit-proof",
        "issue_url":"","pull_request_url":"https://github.com/synthetic/rehearsal/pull/1",
        "submission_note":"Synthetic participant evidence"});
    let (status, _) = post(&client, &publisher.secret, "submit", submit.clone()).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "publisher cannot submit as participant"
    );
    let api = identities
        .issue_key(
            &participant.secret,
            Account {
                kind: AccountKind::Personal,
                id: 202,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let (status, _) = post(&client, &api.secret, "submit", submit.clone()).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "host token plus API key is not a participant Session"
    );
    let (first, replay) = tokio::join!(
        post(&client, &participant.secret, "submit", submit.clone()),
        post(&client, &participant.secret, "submit", submit.clone()),
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(
        first, replay,
        "concurrent duplicate must return the exact original receipt"
    );
    assert_eq!(first.1["status"], "submitted");
    assert!(first.1["journal_id"].is_null());
    let mut expected_submit = submit.clone();
    expected_submit["kind"] = "submit".into();
    let query = json!({"migration_id":MIGRATION,"source_project_id":PROJECT,
        "operation_key":"submit-proof","expected_command":expected_submit});
    for session in [&publisher.secret, &participant.secret] {
        let (status, result) = post(&client, session, "query", query.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["receipt"], first.1);
        assert_eq!(result["actor_user_id"], 202);
    }
    assert_eq!(
        post(&client, &stranger.secret, "query", query.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut collision = query.clone();
    collision["expected_command"]["pull_request_url"] =
        "https://github.com/synthetic/rehearsal/pull/2".into();
    assert_eq!(
        post(&client, &publisher.secret, "query", collision).await.0,
        StatusCode::CONFLICT
    );
    let payout = json!({"migration_id":MIGRATION,"source_project_id":PROJECT,
        "source_challenge_id":CLAIM,"operation_key":"pay-submitted","submission_operation_key":"submit-proof"});
    assert_eq!(
        post(&client, &participant.secret, "payout", payout.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, paid) = post(&client, &publisher.secret, "payout", payout.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(paid["status"], "posted");
    assert!(paid["journal_id"].as_i64().unwrap() > 0);
    assert_eq!(paid["remaining_units"], 13);
    assert_eq!(
        post(&client, &publisher.secret, "payout", payout.clone())
            .await
            .1,
        paid
    );
    assert_eq!(ledger.balance(receiver).await.unwrap().available_units, 7);
    assert_eq!(ledger.balance(payer).await.unwrap().available_units, 100);
    assert_eq!(ledger.balance(payer).await.unwrap().reserved_units, 13);
    let mut expected_payout = payout;
    expected_payout["kind"] = "payout".into();
    let (status, observed) = post(
        &client,
        &publisher.secret,
        "query",
        json!({
            "migration_id":MIGRATION,"source_project_id":PROJECT,"operation_key":"pay-submitted",
            "expected_command":expected_payout,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(observed["receipt"], paid);
    let (status, closed) = post(&client, &publisher.secret, "close", json!({
        "migration_id":MIGRATION,"source_project_id":PROJECT,"operation_key":"release-remainder",
    })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(closed["status"], "posted");
    assert_eq!(ledger.balance(payer).await.unwrap().available_units, 113);
    assert_eq!(ledger.balance(payer).await.unwrap().reserved_units, 0);
    let after_claim: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM core_billing.legacy_escrow_claims c WHERE migration_id=$1 AND source_challenge_id=$2")
        .bind(MIGRATION).bind(CLAIM).fetch_one(&pool).await.unwrap();
    assert_eq!(
        after_claim, original_claim,
        "runtime appends events without rewriting source facts"
    );
    let unbalanced: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) bad")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    drop(client);
    stop.send(()).unwrap();
    server.await.unwrap();
    std::fs::remove_file(socket).unwrap();
    std::fs::remove_dir(socket_root).unwrap();
    runtime.cleanup().await;
}
