//! Native A03 Key Quota Acceptance Tests.
//!
//! Verifies:
//! 1. Complete Rust Core installer & independent installer LOGIN SET ROLE NOLOGIN owner.
//! 2. IdentityStore real key issuance & secret digest binding (no forged secret/status).
//! 3. Real offline SQL writer consumption & restricted runtime ChargingEngine: historical used + new reserve merged admission.
//! 4. Refuse cross-key and cross-account quota borrowing.
//! 5. Finite / unlimited switching preserves historical opening consumed credits.
//! 6. Reject an active competing client & verify strict replay idempotency.
//! 7. Halfway failure marker rolls back all tables and sequence state.
//! 8. UNKNOWN queries by original migration_id in core_migration.import_runs.
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{Charging, Error as ChargingError, Outcome, Price, Request, Usage},
        ledger::{Action, Ledger, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, PgPool, Row, postgres::PgConnectOptions};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

const WORKER: [u8; 32] = [7; 32];

fn test_price() -> Price {
    Price {
        search: None,
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
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

const POSTGRES_IMAGE: &str =
    "postgres:18-alpine@sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873";

#[derive(Clone)]
struct OwnedFixture {
    id: String,
    nonce: String,
    host_port: u16,
    password: String,
}

static FIXTURES: OnceLock<Mutex<HashMap<String, OwnedFixture>>> = OnceLock::new();

fn docker(arguments: &[&str]) -> std::process::Output {
    std::process::Command::new("docker")
        .args(["--host", "unix:///var/run/docker.sock"])
        .args(arguments)
        .output()
        .expect("Docker is required for the owned offline PostgreSQL fixture")
}

async fn setup_a03_roles() -> PgPool {
    let suffix = format!("{:016x}", rand::random::<u64>());
    let container = format!("lmm-native-kq-{suffix}");
    let label = format!("lmm.test.native-kq={container}");
    let password = format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    );
    let nonce = format!("lmm.migration_instance_nonce={container}");
    let started = std::process::Command::new("docker")
        .args(["--host", "unix:///var/run/docker.sock"])
        .env("POSTGRES_PASSWORD", &password)
        .args([
            "run",
            "--detach",
            "--pull=never",
            "--name",
            &container,
            "--label",
            &label,
            "--memory=384m",
            "--cpus=1",
            "--pids-limit=128",
            "--publish",
            "127.0.0.1::5432",
            "--tmpfs",
            "/var/lib/postgresql:rw,size=512m",
            "--tmpfs",
            "/var/run/postgresql:rw,size=16m",
            "--env",
            "POSTGRES_PASSWORD",
            POSTGRES_IMAGE,
            "postgres",
            "-c",
            "autovacuum=off",
            "-c",
            "max_connections=20",
            "-c",
            "shared_buffers=32MB",
            "-c",
            &nonce,
        ])
        .output()
        .expect("start owned PostgreSQL fixture");
    assert!(
        started.status.success(),
        "fixture startup failed: {}",
        String::from_utf8_lossy(&started.stderr)
    );
    let container_id = String::from_utf8(started.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(container_id.len() == 64 && container_id.bytes().all(|b| b.is_ascii_hexdigit()));
    eprintln!(
        "owned PostgreSQL failure scene: {container}; id: {container_id}; pinned image: {POSTGRES_IMAGE}"
    );
    let mut ready = false;
    for _ in 0..300 {
        let result = docker(&[
            "exec",
            &container,
            "sh",
            "-c",
            "test \"$(cat /proc/1/comm)\" = postgres && pg_isready -q -U postgres",
        ]);
        if result.status.success() {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        ready,
        "owned PostgreSQL fixture did not become ready: {container}"
    );
    let inspected = docker(&["port", &container, "5432/tcp"]);
    assert!(inspected.status.success());
    let address = String::from_utf8(inspected.stdout).unwrap();
    let port: u16 = address
        .trim()
        .strip_prefix("127.0.0.1:")
        .expect("loopback binding")
        .parse()
        .unwrap();
    FIXTURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(
            container.clone(),
            OwnedFixture {
                id: container_id,
                nonce: container.clone(),
                host_port: port,
                password: password.clone(),
            },
        );
    verify_owned_container(&container);
    let admin = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username("postgres")
        .password(&password)
        .database("postgres")
        .application_name(&container);
    let pool = PgPool::connect_with(admin).await.unwrap();
    let instance: (String, i32) = sqlx::query_as(
        "SELECT current_setting('lmm.migration_instance_nonce'), current_setting('port')::int",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        instance,
        (container.clone(), 5432),
        "published endpoint must reach the newly created PostgreSQL instance"
    );
    let owner = format!("kq_owner_{suffix}");
    let operator = format!("kq_operator_{suffix}");
    let database = format!("kq_{suffix}");
    let runtime = format!("kq_runtime_{suffix}");
    // Role names, database and password material are generated hexadecimal.
    sqlx::raw_sql(&format!(
        "CREATE ROLE {owner} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; \
         CREATE ROLE {operator} LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{password}'; \
         GRANT {owner} TO {operator}; \
         CREATE ROLE {runtime} LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{password}';"
    )).execute(&pool).await.expect("provision isolated installer roles");
    // CREATE DATABASE cannot be part of a multi-statement transaction.
    sqlx::raw_sql(&format!("CREATE DATABASE {database} OWNER {owner}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(&format!("REVOKE CONNECT ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {operator}"))
        .execute(&pool).await.unwrap();
    let admin_options = (*pool.connect_options()).clone().database(&database);
    pool.close().await;
    let mut connection = PgConnection::connect_with(&admin_options).await.unwrap();
    sqlx::raw_sql(&format!(
        "GRANT EXECUTE ON FUNCTION pg_control_system() TO {owner}"
    ))
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();
    let options = admin_options
        .username(&operator)
        .options([("role", owner.as_str())]);
    let pool = PgPool::connect_with(options)
        .await
        .expect("independent LOGIN must SET ROLE to NOLOGIN owner");
    let identity: (String, String, bool, bool, bool) = sqlx::query_as(
        "SELECT current_user::text, session_user::text, \
         (SELECT rolcanlogin FROM pg_roles WHERE rolname=current_user), \
         (SELECT rolcanlogin FROM pg_roles WHERE rolname=session_user), \
         (SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname=current_user) \
         OR (SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname=session_user)"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(identity, (owner.clone(), operator, false, true, false));
    core_schema::init(&pool).await.unwrap();
    core_schema::check(&pool).await.unwrap();
    // Reuse the reviewed deployment grant matrix; only the test's owner name differs.
    let grants = include_str!("../../../deployment/postgres/grant-core-runtime.sql")
        .replace("lmm_core_owner", &owner);
    let script = format!("\\set runtime_role {runtime}\n\\set enable_models true\n{grants}");
    run_writer_sql(&pool.connect_options(), &script)
        .await
        .expect("explicit production runtime grants");
    pool
}

fn verify_owned_container(container: &str) -> OwnedFixture {
    let fixture = FIXTURES
        .get()
        .expect("fixture creation receipt")
        .lock()
        .unwrap()
        .get(container)
        .expect("container must have been created by this test process")
        .clone();
    let result = docker(&[
        "inspect",
        &fixture.id,
        "--format",
        "{{.Id}}|{{index .Config.Labels \"lmm.test.native-kq\"}}|{{(index (index .NetworkSettings.Ports \"5432/tcp\") 0).HostIp}}|{{(index (index .NetworkSettings.Ports \"5432/tcp\") 0).HostPort}}",
    ]);
    assert!(result.status.success());
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().trim(),
        format!(
            "{}|{}|127.0.0.1|{}",
            fixture.id, fixture.nonce, fixture.host_port
        ),
        "container id, label and published endpoint must match the creation receipt"
    );
    fixture
}

async fn wait_for_no_clients(fixture: &OwnedFixture) {
    // SQLx has closed its sockets, but backend exit can lag pool.close().
    // Observe a bounded drain; never terminate clients to manufacture exclusivity.
    for _ in 0..50 {
        let clients = docker(&[
            "exec",
            &fixture.id,
            "psql",
            "-X",
            "--no-password",
            "-qAt",
            "-v",
            "ON_ERROR_STOP=1",
            "-U",
            "postgres",
            "-c",
            "SELECT count(*) FROM pg_stat_activity WHERE backend_type='client backend' AND pid<>pg_backend_pid()",
        ]);
        assert!(clients.status.success());
        if String::from_utf8(clients.stdout).unwrap().trim() == "0" {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!(
        "retain owned fixture with active clients after bounded drain: {}",
        fixture.id
    );
}

async fn finish_fixture(pool: PgPool) {
    let container = pool
        .connect_options()
        .get_application_name()
        .unwrap()
        .to_string();
    pool.close().await;
    let fixture = verify_owned_container(&container);
    wait_for_no_clients(&fixture).await;
    let result = docker(&["rm", "--force", "--volumes", &fixture.id]);
    assert!(result.status.success(), "owned fixture cleanup failed");
    FIXTURES.get().unwrap().lock().unwrap().remove(&container);
}

async fn charging_runtime(owner_pool: &PgPool) -> PgPool {
    let options = owner_pool.connect_options();
    let runtime = options
        .get_username()
        .replace("kq_operator_", "kq_runtime_");
    let owner = options.get_username().replace("kq_operator_", "kq_owner_");
    let fixture = verify_owned_container(options.get_application_name().unwrap());
    // Build fresh runtime options: copying installer options also copies SET ROLE owner.
    let options = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(fixture.host_port)
        .username(&runtime)
        .password(&fixture.password)
        .database(options.get_database().unwrap())
        .application_name(&fixture.nonce);
    let pool = PgPool::connect_with(options).await.unwrap();
    let permissions: (bool, bool, bool) = sqlx::query_as(
        "SELECT pg_has_role(current_user, $1, 'SET'), \
         has_table_privilege(current_user, 'core_charging.budget_opening_consumption', 'INSERT,UPDATE,DELETE'), \
         has_table_privilege(current_user, 'core_billing.balance_state', 'INSERT,UPDATE,DELETE')"
    ).bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(
        permissions,
        (false, false, false),
        "runtime must not acquire installation or historical/funds write authority"
    );
    core_schema::check(&pool).await.unwrap();
    pool
}

async fn import_state(pool: &PgPool) -> serde_json::Value {
    let relations: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT n.nspname::text, c.relname::text, c.relkind::text FROM pg_class c \
         JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE left(n.nspname,5)='core_' AND c.relkind IN ('r','p','S') ORDER BY n.nspname,c.relname"
    ).fetch_all(pool).await.unwrap();
    let mut snapshot = serde_json::Map::new();
    for (schema, name, kind) in relations {
        let relation = format!(
            "\"{}\".\"{}\"",
            schema.replace('"', "\"\""),
            name.replace('"', "\"\"")
        );
        let query = if kind == "S" {
            format!(
                "SELECT jsonb_build_object('last_value',last_value,'is_called',is_called) FROM {relation}"
            )
        } else {
            format!(
                "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM {relation} t"
            )
        };
        let value = sqlx::query_scalar(&query).fetch_one(pool).await.unwrap();
        snapshot.insert(format!("{schema}.{name}"), value);
    }
    serde_json::Value::Object(snapshot)
}

const SCRIPTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts");

async fn get_target_receipt(pool: &PgPool) -> serde_json::Value {
    let ident: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
            'database', current_database(),
            'owner', current_user,
            'operator', session_user,
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
            "role": ident["owner"],
            "login_role": ident["operator"],
            "set_role": ident["owner"],
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

data = json.load(sys.stdin)
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
        scripts_dir = SCRIPTS_DIR
    );

    let mut child = std::process::Command::new("python3")
        .args(["-B", "-c", &script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("invoke legacy_key_quota_opening");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Python build_apply_sql failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 output")
}

async fn run_writer_sql(options: &PgConnectOptions, sql: &str) -> Result<String, String> {
    let container = options
        .get_application_name()
        .expect("owned fixture identity")
        .to_string();
    let operator = options.get_username().to_string();
    let owner = operator.replace("kq_operator_", "kq_owner_");
    let database = options.get_database().unwrap().to_string();
    let sql = sql.to_string();
    tokio::task::spawn_blocking(move || {
        let fixture = verify_owned_container(&container);
        let role_option = format!("PGOPTIONS=-c role={owner}");
        let mut child = std::process::Command::new("docker")
            .args(["--host", "unix:///var/run/docker.sock"])
            .args([
                "exec",
                "-i",
                "--env",
                &role_option,
                &fixture.id,
                "psql",
                "-X",
                "--no-password",
                "-qAt",
                "-v",
                "ON_ERROR_STOP=1",
                "-h",
                "/var/run/postgresql",
                "-U",
                &operator,
                "-d",
                &database,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(sql.as_bytes())
            .map_err(|e| e.to_string())?;
        let result = child.wait_with_output().map_err(|e| e.to_string())?;
        if result.status.success() {
            Ok(String::from_utf8_lossy(&result.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&result.stderr).to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn apply_writer_sql(pool: PgPool, sql: &str) -> (Result<String, String>, PgPool) {
    let options = (*pool.connect_options()).clone();
    pool.close().await;
    let fixture = verify_owned_container(options.get_application_name().unwrap());
    wait_for_no_clients(&fixture).await;
    let result = run_writer_sql(&options, sql).await;
    let pool = PgPool::connect_with(options).await.unwrap();
    (result, pool)
}

#[tokio::test]
async fn fresh_installer_and_independent_owner_contract() {
    let pool = setup_a03_roles().await;
    let components: Vec<String> =
        sqlx::query_scalar("SELECT name FROM core_meta.component_contract ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    let manifest = core_schema::schema_manifest();
    let mut expected: Vec<String> = manifest
        .components
        .iter()
        .map(|c| c.name.to_string())
        .collect();
    expected.sort();
    assert_eq!(components, expected);
    assert_eq!(components.len(), core_schema::component_count());
    // Required assembled features remain explicit while the installer manifest
    // supplies the complete count as independently owned modules are added.
    for added in [
        "identity",
        "ledger",
        "payments",
        "charging",
        "routing",
        "events",
        "charging-ledger",
        "migration",
        "console",
        "model-gateway",
        "commerce",
        "abuse",
        "oauth-grants",
        "history",
        "humangrant",
        "tool-market",
        "assistant-model",
        "gift",
    ] {
        assert!(
            components.iter().any(|name| name == added),
            "missing required fresh-install component: {added}"
        );
    }
    let version: i32 =
        sqlx::query_scalar("SELECT version FROM core_meta.schema_contract WHERE singleton")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, manifest.identity.version);
    let foreign_owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE left(n.nspname,5)='core_' AND c.relowner<>(SELECT oid FROM pg_roles WHERE rolname=current_user)"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(
        foreign_owners, 0,
        "NOLOGIN owner must own the installed relations and sequences"
    );
    finish_fixture(pool).await;
}

#[tokio::test]
async fn identity_store_real_key_issuance_and_digest_binding() {
    let pool = setup_a03_roles().await;
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
    finish_fixture(pool).await;
}

#[tokio::test]
async fn offline_writer_plan_and_charging_engine_admission() {
    let pool = setup_a03_roles().await;

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

    let runtime_pool = charging_runtime(&pool).await;
    let charging = Charging::new(runtime_pool.clone(), Arc::new(TransactionalLedger));

    // Case A: Reserve 300 credits -> 600 + 300 = 900 <= 1000 -> SUCCEEDS
    let req1 = charge_request("kq-charge-1", 300);
    charging
        .reserve(&key.secret, &req1, &WORKER)
        .await
        .expect("Reservation of 300 must succeed within remaining 400 credits");

    // Case B: Additional reserve of 200 credits -> 600 + 300 + 200 = 1100 > 1000 -> FAILS
    let req2 = charge_request("kq-charge-2", 200);
    let err = charging.reserve(&key.secret, &req2, &WORKER).await;
    assert_eq!(
        err.unwrap_err(),
        ChargingError::BudgetExceeded,
        "Reservation exceeding budget limit must fail for its quota, not storage or permissions"
    );

    // Settle req1 with 150 actual units
    charging.start(&req1.id, &WORKER).await.unwrap();
    charging
        .settle(
            &req1.id,
            &WORKER,
            Usage {
                search: None,
                cache_creation: None,
                image: None,
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
    drop(charging);
    runtime_pool.close().await;
    finish_fixture(pool).await;
}

#[tokio::test]
async fn refuse_cross_key_and_cross_account_borrowing() {
    let pool = setup_a03_roles().await;

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

    let runtime_pool = charging_runtime(&pool).await;
    let charging = Charging::new(runtime_pool.clone(), Arc::new(TransactionalLedger));

    // Key 1 attempting to reserve 100 credits MUST fail
    // Key 1 CANNOT borrow from Key 2's 9,900 remaining credits
    let req_k1 = charge_request("req-key1-overflow", 100);
    assert_eq!(
        charging
            .reserve(&key1.secret, &req_k1, &WORKER)
            .await
            .unwrap_err(),
        ChargingError::BudgetExceeded,
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
    drop(charging);
    runtime_pool.close().await;
    finish_fixture(pool).await;
}

#[tokio::test]
async fn finite_unlimited_switching_does_not_clear_consumed_credits() {
    let pool = setup_a03_roles().await;

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
         SET unlimited = TRUE, limit_credits = 0, display_limit_credits = 1000, revision = revision + 1 \
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
         SET unlimited = FALSE, limit_credits = 1000, display_limit_credits = NULL, revision = revision + 1 \
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
    finish_fixture(pool).await;
}

#[tokio::test]
async fn competing_client_rejected_and_strict_replay() {
    let pool = setup_a03_roles().await;

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

    let options = (*pool.connect_options()).clone();
    let before = import_state(&pool).await;
    // This live backend deterministically overlaps the real writer connection.
    // Offline import intentionally rejects any competing client; it does not promise
    // that two simultaneous callers produce a winner.
    let mut blocker = pool.acquire().await.unwrap();
    let _: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let rejected = run_writer_sql(&options, &apply_sql).await.unwrap_err();
    assert!(
        rejected.contains("opening target has another client"),
        "{rejected}"
    );
    drop(blocker);
    assert_eq!(
        import_state(&pool).await,
        before,
        "competing-client rejection must not mutate import state"
    );
    let (first, mut pool) = apply_writer_sql(pool, &apply_sql).await;
    assert!(first.unwrap().contains("succeeded"));

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_migration.import_runs WHERE migration_id = 'kq-conc-001'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Exactly one import run must be recorded");

    // Strict Replay: Running identical SQL again sequentially is a valid no-op
    let before_replay = import_state(&pool).await;
    let (replay_res, p) = apply_writer_sql(pool, &apply_sql).await;
    pool = p;
    assert!(replay_res.unwrap().contains("succeeded"));
    assert_eq!(
        import_state(&pool).await,
        before_replay,
        "exact replay must preserve all import state"
    );

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
    let (conflict_err, pool) = apply_writer_sql(pool, &conflict_sql).await;
    assert!(
        conflict_err.is_err(),
        "Changed material with same migration ID must be rejected"
    );
    let err_str = conflict_err.unwrap_err();
    assert!(
        err_str.contains("key_quota_import_conflict"),
        "Error message must specify key_quota_import_conflict: {err_str}"
    );
    assert_eq!(
        import_state(&pool).await,
        before_replay,
        "conflicting replay must preserve every table and sequence"
    );
    finish_fixture(pool).await;
}

#[tokio::test]
async fn injected_failure_marker_rolls_back_every_table_and_sequence() {
    let pool = setup_a03_roles().await;

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

    let before_failure = import_state(&pool).await;
    let (res, pool) = apply_writer_sql(pool, &failing_sql).await;
    assert!(res.is_err(), "Injected failure must cause SQL abort");
    let err_str = res.unwrap_err();
    assert!(
        err_str.contains("independent-injected-failure"),
        "Injected error must be reached: {err_str}"
    );

    assert_eq!(
        import_state(&pool).await,
        before_failure,
        "injected failure must preserve all three import tables and sequence state"
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
    finish_fixture(pool).await;
}

#[tokio::test]
async fn unknown_import_queries_by_original_migration_id() {
    let pool = setup_a03_roles().await;

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
    finish_fixture(pool).await;
}
