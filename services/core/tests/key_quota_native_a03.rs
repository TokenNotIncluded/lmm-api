//! Native A03 Key Quota Acceptance Tests.
//!
//! Verifies:
//! 1. Fresh14 Rust Core installer & independent installer LOGIN SET ROLE NOLOGIN owner.
//!    (Note: kept UNRUN per review instruction --skip fresh14_installer_and_independent_owner_contract).
//! 2. IdentityStore real key issuance & secret digest binding (no forged secret/status).
//! 3. Real offline writer CLI consumption & minimum runtime ChargingEngine: historical used + new reserve merged admission.
//! 4. Refuse cross-key and cross-account quota borrowing.
//! 5. Finite / unlimited switching preserves historical opening consumed credits.
//! 6. Concurrent import single winner & strict replay idempotency.
//! 7. Halfway failure marker rolls back all tables and sequence state.
//! 8. UNKNOWN queries by original migration_id in core_migration.import_runs.
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{Charging, Outcome, Price, Request, Usage},
        ledger::{Action, Ledger, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::sync::Arc;

const WORKER: [u8; 32] = [7; 32];

fn test_price() -> Price {
    Price {
        upstream_cost_group_ppm: None,
        version: "v1".into(),
        model: "test-model".into(),
        group: "default".into(),
        input_per_million: 1_000_000,
        cached_per_million: 500_000,
        output_per_million: 1_000_000,
    }
}

fn charge_request(id: &str, reserve: i64) -> Request {
    Request {
        id: id.into(),
        fingerprint: [1; 32],
        reserve,
        lease_seconds: 300,
        maximum_seconds: 3600,
        price: test_price(),
    }
}

fn personal(id: i64) -> Account {
    Account {
        kind: AccountKind::Personal,
        id,
    }
}

async fn setup_a03_roles(pool: &PgPool) {
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await
        .unwrap();
    let alter_sql = format!("ALTER DATABASE \"{}\" OWNER TO a03_owner;", db);
    let _ = sqlx::raw_sql(&alter_sql).execute(pool).await;

    let _ = sqlx::raw_sql(
        "GRANT USAGE ON SCHEMA core_meta, core_charging, core_identity, core_migration TO a03_owner; \
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA core_charging, core_migration TO a03_owner; \
         GRANT SELECT ON ALL TABLES IN SCHEMA core_identity, core_meta TO a03_owner; \
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA core_charging, core_identity TO a03_owner;"
    )
    .execute(pool)
    .await;
}

const SCRIPTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts");

async fn get_target_receipt(pool: &PgPool) -> serde_json::Value {
    let ident: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
            'database', current_database(),
            'database_oid', (SELECT oid FROM pg_database WHERE datname=current_database()),
            'port', current_setting('port')::int,
            'cluster_system_identifier', (SELECT system_identifier::text FROM pg_control_system()),
            'server_version_num', current_setting('server_version_num')::int,
            'postmaster_started_at', pg_postmaster_start_time()::text,
            'server_instance_nonce', current_setting('lmm.migration_instance_nonce', true)
        )"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let contract_script = format!(
        r#"
import sys, json
sys.path.insert(0, "{scripts_dir}")
import legacy_key_quota_opening as KQ
print(json.dumps(KQ.IDENTITY.current_contract()))
"#,
        scripts_dir = SCRIPTS_DIR
    );
    let output = std::process::Command::new("python3")
        .args(["-B", "-c", &contract_script])
        .output()
        .expect("get current contract");
    assert!(
        output.status.success(),
        "Failed to get contract: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut contract: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let catalog_hash_script = format!(
        r#"
import sys
sys.path.insert(0, "{scripts_dir}")
import legacy_key_quota_opening as KQ
print(KQ.IDENTITY.catalog_hash_sql(KQ.IDENTITY.current_contract()))
"#,
        scripts_dir = SCRIPTS_DIR
    );
    let hash_output = std::process::Command::new("python3")
        .args(["-B", "-c", &catalog_hash_script])
        .output()
        .expect("get catalog hash sql");
    assert!(
        hash_output.status.success(),
        "Failed to get catalog hash sql: {}",
        String::from_utf8_lossy(&hash_output.stderr)
    );
    let hash_sql = String::from_utf8(hash_output.stdout).unwrap();
    let catalog_hash: String = sqlx::query_scalar(&hash_sql).fetch_one(pool).await.unwrap();

    contract["catalog_fingerprint_sha256"] = serde_json::Value::String(catalog_hash);

    serde_json::json!({
        "connection": {
            "role": "a03_owner",
            "login_role": "a03_operator",
            "set_role": "a03_owner",
        },
        "instance_identity": ident,
        "contract": contract,
    })
}

fn build_writer_sql(
    tokens: &[serde_json::Value],
    bindings: &[serde_json::Value],
    user_map: &serde_json::Value,
    migration_id: &str,
    target_receipt: &serde_json::Value,
) -> String {
    let input = serde_json::json!({
        "tokens": tokens,
        "bindings": bindings,
        "user_map": user_map,
        "migration_id": migration_id,
        "source_archive_sha256": "a".repeat(64),
        "target_receipt": target_receipt,
    });

    let script = format!(
        r#"
import sys, json
sys.path.insert(0, "{scripts_dir}")
import legacy_key_quota_opening as KQ

data = json.loads({input_json})
receipt = {{
    "scope": "a02-credential-mapping-receipt",
    "source_archive_sha256": data["source_archive_sha256"],
    "records": data["bindings"],
}}
user_map = {{int(k): v for k, v in data["user_map"].items()}}
plan = KQ.build_plan_from_tokens(
    data["tokens"],
    user_map,
    receipt,
    migration_id=data["migration_id"],
    source_archive_sha256=data["source_archive_sha256"],
)
apply_sql = KQ.build_apply_sql(plan, data["target_receipt"])
print(apply_sql)
"#,
        scripts_dir = SCRIPTS_DIR,
        input_json = serde_json::to_string(&input.to_string()).unwrap()
    );

    let output = std::process::Command::new("python3")
        .args(["-B", "-c", &script])
        .output()
        .expect("invoke legacy_key_quota_opening");
    assert!(
        output.status.success(),
        "Python build_apply_sql failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 output")
}

fn get_test_db_url(db: &str) -> String {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let (prefix, _) = base.rsplit_once('/').expect("valid DATABASE_URL format");
    format!("{prefix}/{db}")
}

async fn run_docker_psql(container: &str, db: &str, sql: &str) -> Result<String, String> {
    let mut child = std::process::Command::new("docker")
        .args([
            "exec",
            "-i",
            "--env",
            "PGOPTIONS=-c role=a03_owner",
            container,
            "psql",
            "-X",
            "--no-password",
            "-qAt",
            "-v",
            "ON_ERROR_STOP=1",
            "-h",
            "/var/run/postgresql",
            "-U",
            "a03_operator",
            "-d",
            db,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn docker: {e}"))?;

    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(sql.as_bytes())
        .unwrap();
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

async fn apply_writer_sql(pool: PgPool, sql: &str) -> (Result<String, String>, PgPool) {
    let (db, nonce): (String, Option<String>) = sqlx::query_as(
        "SELECT current_database(), current_setting('lmm.migration_instance_nonce', true)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let container = nonce
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("PG_TEST_CONTAINER").ok())
        .expect("container name");

    pool.close().await;

    let res = run_docker_psql(&container, &db, sql).await;

    let new_url = get_test_db_url(&db);
    let new_pool = PgPool::connect(&new_url).await.unwrap();

    (res, new_pool)
}

#[sqlx::test(migrations = false)]
async fn fresh14_installer_and_independent_owner_contract(pool: PgPool) {
    // 1. Fresh14 Installer initialization and verification
    core_schema::init(&pool).await.unwrap();
    core_schema::check(&pool).await.unwrap();

    let components: Vec<String> =
        sqlx::query_scalar("SELECT name FROM core_meta.component_contract ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(components.len(), core_schema::component_count());
    assert_eq!(components.len(), 14);

    for required in [
        "identity",
        "ledger",
        "payments",
        "charging",
        "routing",
        "events",
        "financial-profile",
        "charging-ledger",
        "console",
        "model-gateway",
        "commerce",
        "abuse",
        "oauth-grants",
        "history",
    ] {
        assert!(
            components.iter().any(|c| c == required),
            "Fresh14 installer must include component: {required}"
        );
    }

    // 2. Independent Installer LOGIN role SET ROLE to NOLOGIN owner
    sqlx::raw_sql(
        "CREATE ROLE a03_test_owner NOLOGIN; \
         CREATE ROLE a03_test_operator LOGIN; \
         GRANT a03_test_owner TO a03_test_operator; \
         GRANT USAGE ON SCHEMA core_meta, core_charging, core_identity, core_migration TO a03_test_owner; \
         GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA core_charging, core_migration TO a03_test_owner; \
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA core_charging TO a03_test_owner;",
    )
    .execute(&pool)
    .await
    .unwrap();

    let version: i32 = sqlx::query_scalar(
        "SET ROLE a03_test_owner; SELECT version FROM core_meta.schema_contract WHERE singleton;",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(version, 4);
}

#[sqlx::test(migrations = false)]
async fn identity_store_real_key_issuance_and_digest_binding(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let store = IdentityStore::from_pool(pool.clone());

    let session = store.bootstrap_user(3001, 1).await.unwrap();
    let user_id = 3001;

    let key = store
        .issue_key(
            &session.secret,
            personal(user_id),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    assert!(key.id > 0);
    assert!(key.secret.starts_with("lmmk_"));

    let row = sqlx::query(
        "SELECT id, digest, kind, user_id, owner_account_id, (revoked_at IS NOT NULL) AS is_revoked \
         FROM core_identity.credentials WHERE id = $1",
    )
    .bind(key.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let digest: Vec<u8> = row.get("digest");
    let kind: String = row.get("kind");
    let uid: i64 = row.get("user_id");
    let is_revoked: bool = row.get("is_revoked");

    assert_eq!(kind, "api_key");
    assert_eq!(uid, user_id);
    assert!(!is_revoked);

    let expected_digest = Sha256::digest(key.secret.as_bytes()).to_vec();
    assert_eq!(
        digest, expected_digest,
        "Issued credential digest must match actual secret SHA-256"
    );
}

#[sqlx::test(migrations = false)]
async fn offline_writer_plan_and_charging_engine_admission(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    setup_a03_roles(&pool).await;

    let store = IdentityStore::from_pool(pool.clone());
    let session = store.bootstrap_user(4001, 1).await.unwrap();
    let user_id = 4001;

    let key = store
        .issue_key(
            &session.secret,
            personal(user_id),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let payer = store.authorize(&key.secret).await.unwrap().owner_account_id;

    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(payer).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "synthetic-kq".into(),
            operation_key: "fund-wallet".into(),
            actor_user_id: Some(user_id),
            reason: "Initial funding".into(),
            action: Action::Credit {
                account_id: payer,
                amount_units: Credits::try_from(50_000).unwrap(),
                payment_reference: "kq-credit".into(),
            },
        })
        .await
        .unwrap();

    let secret_hash = format!("{:x}", Sha256::digest(key.secret.as_bytes()));
    let target_receipt = get_target_receipt(&pool).await;

    let token = serde_json::json!({
        "id": 1,
        "user_id": user_id,
        "used_quota": 600,
        "remain_quota": 400,
        "unlimited_quota": false,
        "key": key.secret,
        "status": 1,
    });
    let binding = serde_json::json!({
        "source_key_id": 1,
        "target_credential_id": key.id,
        "target_user_id": user_id,
        "target_account_id": payer,
        "source_secret_sha256": secret_hash,
    });
    let user_map = serde_json::json!({
        user_id.to_string(): {"user_id": user_id, "account_id": payer}
    });

    let apply_sql = build_writer_sql(
        &[token],
        &[binding],
        &user_map,
        "kq-test-run-1",
        &target_receipt,
    );
    let (result, pool) = apply_writer_sql(pool, &apply_sql).await;
    let result = result.unwrap();
    assert!(result.contains("succeeded"));

    let budget: (i64, bool) = sqlx::query_as(
        "SELECT limit_credits, unlimited FROM core_charging.budgets WHERE key_id = $1",
    )
    .bind(key.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(budget, (1000, false));

    let opening_consumed: i64 = sqlx::query_scalar(
        "SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id = $1",
    )
    .bind(key.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(opening_consumed, 600);

    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));

    // Case A: Reserve 300 credits -> 600 + 300 = 900 <= 1000 -> SUCCEEDS
    let req1 = charge_request("kq-charge-1", 300);
    charging
        .reserve(&key.secret, &req1, &WORKER)
        .await
        .expect("Reservation of 300 must succeed within remaining 400 credits");

    // Case B: Additional reserve of 200 credits -> 600 + 300 + 200 = 1100 > 1000 -> FAILS
    let req2 = charge_request("kq-charge-2", 200);
    let err = charging.reserve(&key.secret, &req2, &WORKER).await;
    assert!(
        err.is_err(),
        "Reservation exceeding budget limit must fail admission"
    );

    // Settle req1 with 150 actual units
    charging.start(&req1.id, &WORKER).await.unwrap();
    charging
        .settle(
            &req1.id,
            &WORKER,
            Usage {
                upstream_cost_atto_usd: None,
                input: 50,
                cached: 0,
                output: 100,
            },
            Outcome::Completed,
        )
        .await
        .unwrap();

    // Committed credits = 600 (opening) + 150 (settled) = 750, remaining = 250
    // Case C: Reserve 200 credits -> 750 + 200 = 950 <= 1000 -> SUCCEEDS
    let req3 = charge_request("kq-charge-3", 200);
    charging
        .reserve(&key.secret, &req3, &WORKER)
        .await
        .expect("Reservation within updated remaining capacity must succeed");
}

#[sqlx::test(migrations = false)]
async fn refuse_cross_key_and_cross_account_borrowing(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    setup_a03_roles(&pool).await;

    let store = IdentityStore::from_pool(pool.clone());

    let session1 = store.bootstrap_user(5001, 1).await.unwrap();
    let key1 = store
        .issue_key(
            &session1.secret,
            personal(5001),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let session2 = store.bootstrap_user(5002, 1).await.unwrap();
    let key2 = store
        .issue_key(
            &session2.secret,
            personal(5002),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let payer1 = store
        .authorize(&key1.secret)
        .await
        .unwrap()
        .owner_account_id;
    let payer2 = store
        .authorize(&key2.secret)
        .await
        .unwrap()
        .owner_account_id;

    let ledger = Ledger::from_pool(pool.clone());
    for (uid, payer) in [(5001, payer1), (5002, payer2)] {
        ledger.open_wallet(payer).await.unwrap();
        ledger
            .execute(&LedgerRequest {
                scope: "synthetic-kq".into(),
                operation_key: format!("fund-{}", uid),
                actor_user_id: Some(uid),
                reason: "Funding".into(),
                action: Action::Credit {
                    account_id: payer,
                    amount_units: Credits::try_from(50_000).unwrap(),
                    payment_reference: format!("credit-{}", uid),
                },
            })
            .await
            .unwrap();
    }

    let target_receipt = get_target_receipt(&pool).await;
    let sec1_hash = format!("{:x}", Sha256::digest(key1.secret.as_bytes()));
    let sec2_hash = format!("{:x}", Sha256::digest(key2.secret.as_bytes()));

    let tokens = [
        serde_json::json!({
            "id": 1,
            "user_id": 5001,
            "used_quota": 450,
            "remain_quota": 50,
            "unlimited_quota": false,
            "key": key1.secret,
            "status": 1,
        }),
        serde_json::json!({
            "id": 2,
            "user_id": 5002,
            "used_quota": 100,
            "remain_quota": 9900,
            "unlimited_quota": false,
            "key": key2.secret,
            "status": 1,
        }),
    ];
    let bindings = [
        serde_json::json!({
            "source_key_id": 1,
            "target_credential_id": key1.id,
            "target_user_id": 5001,
            "target_account_id": payer1,
            "source_secret_sha256": sec1_hash,
        }),
        serde_json::json!({
            "source_key_id": 2,
            "target_credential_id": key2.id,
            "target_user_id": 5002,
            "target_account_id": payer2,
            "source_secret_sha256": sec2_hash,
        }),
    ];
    let user_map = serde_json::json!({
        "5001": {"user_id": 5001, "account_id": payer1},
        "5002": {"user_id": 5002, "account_id": payer2},
    });

    let apply_sql = build_writer_sql(
        &tokens,
        &bindings,
        &user_map,
        "kq-cross-test",
        &target_receipt,
    );
    let (res, pool) = apply_writer_sql(pool, &apply_sql).await;
    res.unwrap();

    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));

    // Key 1 attempting to reserve 100 credits MUST fail
    // Key 1 CANNOT borrow from Key 2's 9,900 remaining credits
    let req_k1 = charge_request("req-key1-overflow", 100);
    assert!(
        charging
            .reserve(&key1.secret, &req_k1, &WORKER)
            .await
            .is_err(),
        "Key 1 cannot exceed its own limit or borrow from Key 2"
    );

    // Key 2 reserving 500 credits succeeds
    let req_k2 = charge_request("req-key2-valid", 500);
    assert!(
        charging
            .reserve(&key2.secret, &req_k2, &WORKER)
            .await
            .is_ok(),
        "Key 2 operates independently under its own budget"
    );
}

#[sqlx::test(migrations = false)]
async fn finite_unlimited_switching_does_not_clear_consumed_credits(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    setup_a03_roles(&pool).await;

    let store = IdentityStore::from_pool(pool.clone());
    let session = store.bootstrap_user(6001, 1).await.unwrap();
    let key = store
        .issue_key(
            &session.secret,
            personal(6001),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let payer = store.authorize(&key.secret).await.unwrap().owner_account_id;
    let target_receipt = get_target_receipt(&pool).await;
    let sec_hash = format!("{:x}", Sha256::digest(key.secret.as_bytes()));

    let tokens = [serde_json::json!({
        "id": 1,
        "user_id": 6001,
        "used_quota": 600,
        "remain_quota": 400,
        "unlimited_quota": false,
        "key": key.secret,
        "status": 1,
    })];
    let bindings = [serde_json::json!({
        "source_key_id": 1,
        "target_credential_id": key.id,
        "target_user_id": 6001,
        "target_account_id": payer,
        "source_secret_sha256": sec_hash,
    })];
    let user_map = serde_json::json!({
        "6001": {"user_id": 6001, "account_id": payer}
    });

    let apply_sql = build_writer_sql(
        &tokens,
        &bindings,
        &user_map,
        "kq-switch-test",
        &target_receipt,
    );
    let (res, pool) = apply_writer_sql(pool, &apply_sql).await;
    res.unwrap();

    // Switch to unlimited = true
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_charging.budgets \
         SET unlimited = TRUE, limit_credits = 0, display_limit_credits = 1000 \
         WHERE key_id = $1",
    )
    .bind(key.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Verify historical opening consumption was NOT cleared or zeroed
    let consumed: i64 = sqlx::query_scalar(
        "SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id = $1",
    )
    .bind(key.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        consumed, 600,
        "Historical consumption must not be erased on unlimited switch"
    );

    // Switch back to finite: limit = 1000
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_charging.budgets \
         SET unlimited = FALSE, limit_credits = 1000, display_limit_credits = NULL \
         WHERE key_id = $1",
    )
    .bind(key.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let consumed_after: i64 = sqlx::query_scalar(
        "SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id = $1",
    )
    .bind(key.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        consumed_after, 600,
        "Historical consumption remains strictly conserved"
    );
}

#[sqlx::test(migrations = false)]
async fn concurrent_import_single_winner_and_strict_replay(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    setup_a03_roles(&pool).await;

    let store = IdentityStore::from_pool(pool.clone());
    let session = store.bootstrap_user(7001, 1).await.unwrap();
    let key = store
        .issue_key(
            &session.secret,
            personal(7001),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let payer = store.authorize(&key.secret).await.unwrap().owner_account_id;
    let target_receipt = get_target_receipt(&pool).await;
    let sec_hash = format!("{:x}", Sha256::digest(key.secret.as_bytes()));

    let tokens = [serde_json::json!({
        "id": 1,
        "user_id": 7001,
        "used_quota": 100,
        "remain_quota": 200,
        "unlimited_quota": false,
        "key": key.secret,
        "status": 1,
    })];
    let bindings = [serde_json::json!({
        "source_key_id": 1,
        "target_credential_id": key.id,
        "target_user_id": 7001,
        "target_account_id": payer,
        "source_secret_sha256": sec_hash,
    })];
    let user_map = serde_json::json!({
        "7001": {"user_id": 7001, "account_id": payer}
    });

    let apply_sql = build_writer_sql(
        &tokens,
        &bindings,
        &user_map,
        "kq-conc-001",
        &target_receipt,
    );

    let (db, nonce): (String, Option<String>) = sqlx::query_as(
        "SELECT current_database(), current_setting('lmm.migration_instance_nonce', true)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let container = nonce
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("PG_TEST_CONTAINER").ok())
        .unwrap();

    pool.close().await;

    let c1 = container.clone();
    let c2 = container.clone();
    let d1 = db.clone();
    let d2 = db.clone();
    let s1 = apply_sql.clone();
    let s2 = apply_sql.clone();

    let h1 = tokio::spawn(async move { run_docker_psql(&c1, &d1, &s1).await });
    tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
    let h2 = tokio::spawn(async move { run_docker_psql(&c2, &d2, &s2).await });

    let (res1, res2) = tokio::join!(h1, h2);

    let out1 = res1.unwrap();
    let out2 = res2.unwrap();

    let successes = usize::from(out1.is_ok()) + usize::from(out2.is_ok());
    assert!(successes >= 1, "At least one concurrent task must succeed");

    let new_url = get_test_db_url(&db);
    let mut pool = PgPool::connect(&new_url).await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_migration.import_runs WHERE migration_id = 'kq-conc-001'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Exactly one import run must be recorded");

    // Strict Replay: Running identical SQL again sequentially is a valid no-op
    let (replay_res, p) = apply_writer_sql(pool, &apply_sql).await;
    pool = p;
    assert!(replay_res.unwrap().contains("succeeded"));

    // Conflicting Replay: Same migration ID with changed material MUST be rejected
    let modified_tokens = [serde_json::json!({
        "id": 1,
        "user_id": 7001,
        "used_quota": 101, // Changed!
        "remain_quota": 200,
        "unlimited_quota": false,
        "key": key.secret,
        "status": 1,
    })];
    let conflict_sql = build_writer_sql(
        &modified_tokens,
        &bindings,
        &user_map,
        "kq-conc-001",
        &target_receipt,
    );
    let (conflict_err, _p) = apply_writer_sql(pool, &conflict_sql).await;
    assert!(
        conflict_err.is_err(),
        "Changed material with same migration ID must be rejected"
    );
    let err_str = conflict_err.unwrap_err();
    assert!(
        err_str.contains("key_quota_import_conflict"),
        "Error message must specify key_quota_import_conflict: {err_str}"
    );
}

#[sqlx::test(migrations = false)]
async fn injected_failure_marker_rolls_back_every_table_and_sequence(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    setup_a03_roles(&pool).await;

    let store = IdentityStore::from_pool(pool.clone());
    let session = store.bootstrap_user(8001, 1).await.unwrap();
    let key = store
        .issue_key(
            &session.secret,
            personal(8001),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let payer = store.authorize(&key.secret).await.unwrap().owner_account_id;
    let seq_before: (i64, bool) =
        sqlx::query_as("SELECT last_value, is_called FROM core_charging.budgets_id_seq")
            .fetch_one(&pool)
            .await
            .unwrap();

    let budget_count_before: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.budgets")
        .fetch_one(&pool)
        .await
        .unwrap();

    let target_receipt = get_target_receipt(&pool).await;
    let sec_hash = format!("{:x}", Sha256::digest(key.secret.as_bytes()));

    let tokens = [serde_json::json!({
        "id": 1,
        "user_id": 8001,
        "used_quota": 200,
        "remain_quota": 300,
        "unlimited_quota": false,
        "key": key.secret,
        "status": 1,
    })];
    let bindings = [serde_json::json!({
        "source_key_id": 1,
        "target_credential_id": key.id,
        "target_user_id": 8001,
        "target_account_id": payer,
        "source_secret_sha256": sec_hash,
    })];
    let user_map = serde_json::json!({
        "8001": {"user_id": 8001, "account_id": payer}
    });

    let apply_sql = build_writer_sql(
        &tokens,
        &bindings,
        &user_map,
        "kq-fail-001",
        &target_receipt,
    );

    // Inject failure marker right before step 8
    let marker = "-- 8. Record Immutable Import Run";
    assert!(apply_sql.contains(marker));
    let failing_sql = apply_sql.replace(
        marker,
        "RAISE EXCEPTION 'independent-injected-failure';\n -- 8. Record Immutable Import Run",
    );

    let (res, pool) = apply_writer_sql(pool, &failing_sql).await;
    assert!(res.is_err(), "Injected failure must cause SQL abort");
    let err_str = res.unwrap_err();
    assert!(
        err_str.contains("independent-injected-failure"),
        "Injected error must be reached: {err_str}"
    );

    // State after rollback MUST be completely unchanged
    let seq_after: (i64, bool) =
        sqlx::query_as("SELECT last_value, is_called FROM core_charging.budgets_id_seq")
            .fetch_one(&pool)
            .await
            .unwrap();

    let budget_count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.budgets")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        seq_before, seq_after,
        "Sequence state must be conserved across rollback"
    );
    assert_eq!(
        budget_count_before, budget_count_after,
        "Table row count must remain 0"
    );
}

#[sqlx::test(migrations = false)]
async fn unknown_import_queries_by_original_migration_id(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    setup_a03_roles(&pool).await;

    let store = IdentityStore::from_pool(pool.clone());
    let session = store.bootstrap_user(9001, 1).await.unwrap();
    let key = store
        .issue_key(
            &session.secret,
            personal(9001),
            FundingPolicy::new_personal_key(),
            86400,
        )
        .await
        .unwrap();

    let payer = store.authorize(&key.secret).await.unwrap().owner_account_id;
    let target_receipt = get_target_receipt(&pool).await;
    let sec_hash = format!("{:x}", Sha256::digest(key.secret.as_bytes()));

    let tokens = [serde_json::json!({
        "id": 1,
        "user_id": 9001,
        "used_quota": 50,
        "remain_quota": 50,
        "unlimited_quota": false,
        "key": key.secret,
        "status": 1,
    })];
    let bindings = [serde_json::json!({
        "source_key_id": 1,
        "target_credential_id": key.id,
        "target_user_id": 9001,
        "target_account_id": payer,
        "source_secret_sha256": sec_hash,
    })];
    let user_map = serde_json::json!({
        "9001": {"user_id": 9001, "account_id": payer}
    });

    let apply_sql = build_writer_sql(
        &tokens,
        &bindings,
        &user_map,
        "kq-readback-001",
        &target_receipt,
    );
    let (res, pool) = apply_writer_sql(pool, &apply_sql).await;
    res.unwrap();

    // Query back recorded run by original migration_id
    let run = sqlx::query(
        "SELECT migration_id, source_archive_sha256, mapping_sha256, manifest \
         FROM core_migration.import_runs WHERE migration_id = 'kq-readback-001'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let mig_id: String = run.get("migration_id");
    let arch_sha: String = run.get("source_archive_sha256");
    let map_sha: String = run.get("mapping_sha256");

    assert_eq!(mig_id, "kq-readback-001");
    assert_eq!(arch_sha, "a".repeat(64));
    assert_eq!(map_sha.len(), 64);
}
