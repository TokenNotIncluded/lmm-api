//! Synthetic live-mode proof acceptance through independent Core and Go processes.
//! Provider transport exists only in the Go bootstrap test; no production Stripe coverage.
#![cfg(unix)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use lmm_core::{
    billing::ledger::Ledger,
    core_schema,
    identity::IdentityStore,
    internal_rpc::pb,
    payments::{Channel, PaymentsStore},
};
use prost::Message;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[path = "common/runtime.rs"]
mod runtime;
const SECRET: &str = "whsec_live_process_synthetic_only";
const MERCHANT: &str = "acct_live_process_synthetic";
pub(crate) struct Process(
    pub(crate) Child,
    pub(crate) Value,
    pub(crate) PathBuf,
    pub(crate) String,
);

fn binary_sha(binary: &Path, variable: &str, required: bool) -> String {
    assert!(binary.is_absolute() && binary.is_file());
    let actual = format!("{:x}", Sha256::digest(fs::read(binary).unwrap()));
    match std::env::var(variable) {
        Ok(expected) => {
            assert!(expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()));
            assert_eq!(
                actual,
                expected.to_ascii_lowercase(),
                "binary digest differs from frozen input"
            );
        }
        Err(std::env::VarError::NotPresent) => {
            assert!(!required, "external Core requires its frozen SHA256")
        }
        Err(error) => panic!("invalid binary digest environment: {error}"),
    }
    actual
}

pub(crate) fn observation(pid: u32, expected: &str) -> Value {
    use std::io::Read;
    let stat = || {
        let text = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        text.rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let startticks = stat();
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    let mut file = fs::File::open(&path).unwrap();
    let metadata = file.metadata().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        digest, expected,
        "actual product executable differs from frozen input"
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!((metadata.dev(), metadata.ino()), (after.dev(), after.ino()));
    assert_eq!(
        startticks,
        stat(),
        "product identity changed during observation"
    );
    json!({"pid":pid,"startticks":startticks,"executed_sha256":digest,
        "device":metadata.dev(),"inode":metadata.ino(),
        "executed_path":fs::read_link(&path).unwrap(),
        "boot_id":fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim()})
}

pub(crate) fn record_identity(process: &Process, stage: &str) {
    let actual = observation(
        process.0.id(),
        process.1["executed_sha256"].as_str().unwrap(),
    );
    assert_eq!(
        actual, process.1,
        "product identity changed before/after readiness"
    );
    private(
        &process.2,
        &format!("{}.identity.{stage}.json", process.3),
        &serde_json::to_vec(&json!({"format":"lmm-commerce-process-identity-v1",
            "stage":stage,"identity":actual}))
        .unwrap(),
    );
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn private(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    use std::io::Write;
    let path = root.join(name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    path
}

pub(crate) fn spawn(
    binary: &Path,
    expected: &str,
    config: &Path,
    root: &Path,
    log: &str,
) -> Process {
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(binary).unwrap())),
        expected,
        "product binary changed before spawn"
    );
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(log))
        .unwrap();
    let mut command = Command::new(binary);
    // Neither process inherits a core DSN, caller-selected runtime settings or
    // unrelated credentials. File mode exercises the production config boundary.
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GOMAXPROCS", "2")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output);
    // Install the kill/wait guard before any fallible /proc observation.
    let mut process = Process(
        command.spawn().expect("product process must start"),
        Value::Null,
        root.to_owned(),
        log.to_owned(),
    );
    process.1 = observation(process.0.id(), expected);
    record_identity(&process, "spawn");
    process
}

fn address() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

pub(crate) async fn ready(client: &Client, endpoint: &str, process: &mut Process) -> Value {
    record_identity(process, "before-ready");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "product exited before readiness; inspect private process log"
        );
        if let Ok(response) = client.get(format!("{endpoint}/health/ready")).send().await
            && response.status() == StatusCode::OK
        {
            let body = response.json().await.unwrap();
            record_identity(process, "ready");
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "product exited at readiness"
            );
            return body;
        }
        assert!(Instant::now() < deadline, "product readiness timed out");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn signed_event(intent: &pb::PaymentIntent, id: &str) -> Value {
    let event = json!({"id":id,"type":"payment_intent.succeeded","livemode":true,
        "data":{"object":{"id":format!("pi_{}",intent.intent_id),"object":"payment_intent",
        "status":"succeeded","amount":intent.amount_minor,"amount_received":intent.amount_minor,
        "currency":"usd","livemode":true,"metadata":{"lmm_core_intent_id":intent.intent_id.to_string(),
        "lmm_merchant":MERCHANT,"lmm_order_id":format!("top_{}",intent.intent_id)}}}});
    let payload = serde_json::to_string(&event).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(payload.as_bytes());
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"payload":payload,"signature":format!("t={now},v1={hex}")})
}
fn bootstrap_observation(pid: u32, expected: &str) -> Value {
    use std::io::Read;
    let ticks = || {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .unwrap()
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let startticks = ticks();
    let mut executed = fs::File::open(format!("/proc/{pid}/exe")).unwrap();
    let metadata = executed.metadata().unwrap();
    assert_eq!(
        startticks,
        ticks(),
        "bootstrap PID identity changed before pinned-image hashing"
    );
    let mut bytes = Vec::new();
    executed.read_to_end(&mut bytes).unwrap();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        digest, expected,
        "actual bootstrap executed image differs from sealed artifact"
    );
    let after = executed.metadata().unwrap();
    assert_eq!((metadata.dev(), metadata.ino()), (after.dev(), after.ino()));
    json!({"pid":pid,"startticks":startticks,"device":metadata.dev(),"inode":metadata.ino(),"executed_sha256":digest,"attestation":"actual /proc/PID/exe FD pinned and startticks rechecked before hashing","short_process_may_exit_during_hash":true,"boot_id":fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim()})
}
async fn bootstrap(binary: &Path, sha: &str, fixture: &Path, root: &Path, phase: &str) {
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(binary).unwrap())),
        sha
    );
    let log = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(format!("bootstrap-{phase}.log")))
        .unwrap();
    let child = Command::new(binary)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("PAYMENT_CORE_PROOF_FIXTURE", fixture)
        .env("PAYMENT_CORE_PROOF_PHASE", phase)
        .args([
            "-test.run",
            "^TestSyntheticLiveProofAcrossCoreProcess$",
            "-test.v",
            "-test.timeout=50s",
        ])
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut guard = Process(
        child,
        Value::Null,
        root.to_owned(),
        format!("bootstrap-{phase}"),
    );
    guard.1 = bootstrap_observation(guard.0.id(), sha);
    private(
        root,
        &format!("bootstrap-{phase}.identity.fd.json"),
        &serde_json::to_vec(&json!({"stage":"fd-attested","identity":guard.1})).unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(55);
    let status = loop {
        if let Some(status) = guard.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < end, "bootstrap deadline");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    use std::os::unix::process::ExitStatusExt;
    let exit_signal = status.signal();
    private(root,&format!("bootstrap-{phase}-exit.json"),&serde_json::to_vec(&json!({"exit_code":status.code(),"exit_signal":exit_signal,"fd_based_attestation":guard.1,"sha256":sha,"provider_transport":"synthetic in-process only","core_rpc":"real product OS process"})).unwrap());
    assert!(status.success(), "bootstrap failed; inspect private log");
}
async fn financial(pool: &PgPool, account: i64, credits: i64) -> Value {
    let balance = Ledger::from_pool(pool.clone())
        .balance(account)
        .await
        .unwrap();
    assert_eq!(balance.available_units, credits);
    assert_eq!(balance.reserved_units, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_payments.receipts WHERE method='credit'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, if credits == 0 { 0 } else { 1 });
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(journals, count);
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT j.id FROM core_billing.ledger_journals j LEFT JOIN core_billing.ledger_entries e ON e.journal_id=j.id GROUP BY j.id HAVING count(e.journal_id)<>2 OR sum(e.delta_units)<>0) x").fetch_one(pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    json!({"wallet":credits,"reserved":0,"credit_receipts":count,"journals":journals,"unbalanced":unbalanced})
}
// Bind immutable payment receipt bytes to the original job and exact ledger
// legs. A balanced unrelated journal must never satisfy this money assertion.
async fn credit_association(
    pool: &PgPool,
    account: i64,
    intent_id: i64,
    order: &Value,
    local_persisted: bool,
) -> Value {
    let jobs: Vec<_> = order["jobs"]
        .as_object()
        .unwrap()
        .values()
        .filter(|j| j["kind"] == "credit")
        .collect();
    assert_eq!(jobs.len(), 1);
    let job = jobs[0];
    let key = job["key"].as_str().unwrap();
    let row:(String,i64,String,Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT id,intent_id,operation_key,request_hash,receipt_wire FROM core_payments.receipts WHERE method='credit'").fetch_one(pool).await.unwrap();
    let receipt = pb::PaymentReceipt::decode(row.4.as_slice()).unwrap();
    assert_eq!(receipt.receipt_id, row.0);
    assert_eq!(receipt.intent_id, row.1);
    assert_eq!(receipt.intent_id, intent_id);
    assert_eq!(receipt.idempotency_key, row.2);
    assert_eq!(receipt.idempotency_key, key);
    assert_eq!(receipt.state, pb::PaymentReceiptState::CreditApplied as i32);
    assert!(receipt.rejection_code.is_empty());
    assert_eq!(receipt.request_sha256.len(), 32);
    assert_eq!(receipt.request_sha256, row.3);
    let intent: (i64, i64, i64) = sqlx::query_as(
        "SELECT account_id,credit_units,credit_journal_id FROM core_payments.intents WHERE id=$1",
    )
    .bind(intent_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(intent, (account, 500000, receipt.journal_id));
    assert!(receipt.journal_id > 0);
    let journal:(String,i64,i64,i64,String,String)=sqlx::query_as("SELECT kind,amount_units,payer_id,payee_id,scope,operation_key FROM core_billing.ledger_journals WHERE id=$1").bind(receipt.journal_id).fetch_one(pool).await.unwrap();
    assert_eq!(journal.0, "credit");
    assert_eq!(journal.1, 500000);
    let legs:Vec<(i64,Option<i64>,String,i64)>=sqlx::query_as("SELECT a.id,a.owner_account_id,a.bucket,e.delta_units FROM core_billing.ledger_entries e JOIN core_billing.ledger_accounts a ON a.id=e.ledger_account_id WHERE e.journal_id=$1 ORDER BY a.id").bind(receipt.journal_id).fetch_all(pool).await.unwrap();
    assert_eq!(legs.len(), 2);
    assert!(
        legs.iter()
            .any(|(id, owner, bucket, delta)| *id == journal.3
                && *owner == Some(account)
                && bucket == "wallet"
                && *delta == 500000)
    );
    assert!(
        legs.iter()
            .any(|(id, owner, bucket, delta)| *id == journal.2
                && owner.is_none()
                && bucket == "clearing"
                && *delta == -500000)
    );
    if local_persisted {
        let local = &job["receipt"];
        assert_eq!(order["credit_receipt"], receipt.receipt_id);
        assert_eq!(local["id"], receipt.receipt_id);
        assert_eq!(local["intent_id"], intent_id);
        assert_eq!(local["key"], key);
        assert_eq!(local["state"], "credit_applied");
        assert_eq!(local["journal_id"], receipt.journal_id);
        assert_eq!(
            local["request_hash"],
            STANDARD.encode(&receipt.request_sha256)
        );
    } else {
        assert!(job["receipt"].is_null());
    }
    json!({"receipt_id":receipt.receipt_id,"intent_id":intent_id,"original_operation_key":key,"request_sha256":receipt.request_sha256,"receipt_wire_sha256":format!("{:x}",Sha256::digest(&row.4)),"journal_id":receipt.journal_id,"journal_kind":journal.0,"journal_amount":journal.1,"payer_ledger_account":journal.2,"payee_ledger_account":journal.3,"journal_scope":journal.4,"journal_operation_key":journal.5,"exact_legs":legs})
}
async fn snapshot(go: &PgPool, id: &str) -> Value {
    sqlx::query_scalar("SELECT snapshot FROM public.payment_orders WHERE id=$1")
        .bind(id)
        .fetch_one(go)
        .await
        .unwrap()
}
struct PaymentFixture {
    pub(crate) root: PathBuf,
    pub(crate) account: i64,
    pub(crate) intent: pb::PaymentIntent,
    pub(crate) role: runtime::RuntimeRole,
    pub(crate) go_owner: PgPool,
    pub(crate) go_bin: PathBuf,
    pub(crate) go_sha: String,
    pub(crate) boot_bin: PathBuf,
    pub(crate) boot_sha: String,
    pub(crate) fixture: PathBuf,
    pub(crate) go_config: PathBuf,
    pub(crate) client: Client,
    pub(crate) core_endpoint: String,
    pub(crate) go_endpoint: String,
    pub(crate) core: Process,
}
// Cooperative launch contract, not an adversarial same-user security boundary.
fn runner_context() -> (PathBuf, Value) {
    // This target may only allocate resources under the owning external runner.
    let managed = PathBuf::from(
        std::env::var_os("LMM_LIVE_PROCESS_ROOT")
            .expect("run through tests/payments/verify_live_process.py"),
    );
    assert!(managed.is_absolute() && managed.is_dir());
    assert_eq!(
        managed.metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    let marker: Value =
        serde_json::from_slice(&fs::read(managed.join("owner.json")).unwrap()).unwrap();
    let parent = fs::read_to_string(format!("/proc/{}/status", std::process::id())).unwrap();
    let ppid = parent
        .lines()
        .find_map(|s| s.strip_prefix("PPid:\t"))
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert_eq!(
        marker["runner_pid"].as_u64(),
        Some(ppid),
        "owning runner must be the test's actual parent"
    );
    (managed, marker)
}
async fn prepare_payment_fixture(pool: &PgPool) -> PaymentFixture {
    let (managed, _) = runner_context();
    let required = |name: &str| {
        PathBuf::from(
            std::env::var_os(name).unwrap_or_else(|| panic!("required sealed input {name}")),
        )
    };
    let core_bin = required("LMM_PAYMENT_CORE_BIN");
    let core_sha = binary_sha(&core_bin, "LMM_PAYMENT_CORE_SHA256", true);
    let go_bin = required("LMM_PAYMENT_GO_BIN");
    let go_sha = binary_sha(&go_bin, "LMM_PAYMENT_GO_SHA256", true);
    let boot_bin = required("LMM_PAYMENT_BOOTSTRAP_BIN");
    let boot_sha = binary_sha(&boot_bin, "LMM_PAYMENT_BOOTSTRAP_SHA256", true);
    let root = managed.join(format!(
        "pay-{}-{:08x}",
        std::process::id(),
        rand::random::<u32>()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(101, 0).await.unwrap().secret;
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    Ledger::from_pool(pool.clone())
        .open_wallet(account)
        .await
        .unwrap();
    let key = "prepare_real_go_durable";
    let store = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_live(
            1,
            MERCHANT.into(),
            SECRET.into(),
            100000,
        )],
    )
    .unwrap();
    let intent = store
        .prepare(
            &user,
            pb::PrepareTopupRequest {
                mutation: Some(pb::MutationContext {
                    idempotency_key: key.into(),
                    expected_version: None,
                }),
                account_id: account,
                payment_channel_id: 1,
                currency: "USD".into(),
                amount_minor: 100,
            },
        )
        .await
        .unwrap();
    let db = pool.connect_options().get_database().unwrap().to_owned();
    assert!(db.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    sqlx::query(&format!("REVOKE CONNECT ON DATABASE \"{db}\" FROM PUBLIC"))
        .execute(pool)
        .await
        .unwrap();
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("core_payments").await;
    role.optional("core_events").await;
    let database = format!("lmm_payments_proof_{:016x}", rand::random::<u64>());
    let owner = format!("{database}_owner");
    let login = format!("{database}_runtime");
    let password = format!("{:032x}", rand::random::<u128>());
    sqlx::raw_sql(&format!("CREATE ROLE {owner} NOLOGIN;CREATE ROLE {login} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{password}';")).execute(pool).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {database} OWNER {owner} TEMPLATE template0"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(&format!("REVOKE ALL ON DATABASE {database} FROM PUBLIC;GRANT CONNECT ON DATABASE {database} TO {login};")).execute(pool).await.unwrap();
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(url.host_str(), Some("127.0.0.1" | "localhost")));
    url.set_path(&database);
    let go_owner = PgPoolOptions::new()
        .max_connections(2)
        .connect(url.as_str())
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../extensions/internal/modules/payments/schema.sql"
    ))
    .execute(&go_owner)
    .await
    .unwrap();
    sqlx::raw_sql(&format!("REVOKE ALL ON SCHEMA public FROM PUBLIC;GRANT USAGE ON SCHEMA public TO {login};GRANT SELECT ON payment_storage_guard TO {login};GRANT SELECT,INSERT,UPDATE ON payment_orders,payment_events TO {login};REVOKE CREATE,TEMPORARY ON DATABASE {database} FROM PUBLIC;")).execute(&go_owner).await.unwrap();
    url.set_username(&login).unwrap();
    url.set_password(Some(&password)).unwrap();
    // This URL was allocated above in this fresh owned loopback cluster only.
    url.set_query(Some("sslmode=disable"));
    let go_url = private(&root, "go.url", url.as_str().as_bytes());
    let core_url = private(&root, "core.url", role.url.as_bytes());
    let host = "synthetic-payment-host-012345678901234567890123456789";
    let token = "payment_fixture_service_token_0123456789abcdef";
    let host_file = private(&root, "host.token", host.as_bytes());
    let identity_file = private(
        &root,
        "identity.token",
        b"payment-identity-only-012345678901234567890123456789",
    );
    let payment_file = private(&root, "payment.token", token.as_bytes());
    let secret_file = private(&root, "webhook.secret", SECRET.as_bytes());
    let permission=private(&root,"payments.toml",format!("version=1\nprovider='stripe-live'\n[permissions]\nprepare=true\ncredit_evidence=true\nread_user=true\nreconcile=true\nproof_ingress=true\nproof_read=true\n[[channels]]\nid=1\nmerchant='{MERCHANT}'\nwebhook_secret_file='{}'\nmax_usd_cents=100000\n",secret_file.display()).as_bytes());
    let adapters = private(
        &root,
        "go-payments.toml",
        b"version=1\nprovider='stripe-live'\nchannels=[]\n",
    );
    let socket = root.join("core.sock");
    let core_address = address();
    let go_address = address();
    let core_config=private(&root,"core.toml",format!("version=1\n[server]\nlisten='{core_address}'\n[database]\nurl_file='{}'\n[rpc]\nsocket='{}'\ntoken_file='{}'\n[payments]\nconfig_file='{}'\ntoken_file='{}'\n",core_url.display(),socket.display(),identity_file.display(),permission.display(),payment_file.display()).as_bytes());
    let go_config=private(&root,"go.toml",format!("version=1\n[host]\nlisten='{go_address}'\ntoken_file='{}'\nmodules=['payments']\n[core]\nsocket='{}'\nidentity_token_file='{}'\npayment_token_file='{}'\n[payments]\ndatabase_name='{database}'\ndatabase_url_file='{}'\nconfig_file='{}'\ncallbacks_enabled=false\n",host_file.display(),socket.display(),identity_file.display(),payment_file.display(),go_url.display(),adapters.display()).as_bytes());
    let fixture=private(&root,"bootstrap.json",&serde_json::to_vec(&json!({"socket":socket,"token":token,"user":user,"account":account,"channel":{"id":1,"provider":"stripe","merchant":MERCHANT,"environment":"live","currency":"USD","enabled":true},"intent_id":intent.intent_id,"prepare_key":key,"currency":"USD","amount_minor":100,"go_database_url_file":go_url,"go_database_name":database,"events":[signed_event(&intent,"evt_live_process_credit"),signed_event(&intent,"evt_live_process_replay")]})).unwrap());
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let core_endpoint = format!("http://{core_address}");
    let go_endpoint = format!("http://{go_address}");
    role.pool.close().await;
    let mut core = spawn(
        &core_bin,
        &core_sha,
        &core_config,
        &root,
        "core-initial.log",
    );
    assert_eq!(
        ready(&client, &core_endpoint, &mut core).await["capabilities"]["payments"],
        true
    );
    PaymentFixture {
        root,
        account,
        intent,
        role,
        go_owner,
        go_bin,
        go_sha,
        boot_bin,
        boot_sha,
        fixture,
        go_config,
        client,
        core_endpoint,
        go_endpoint,
        core,
    }
}

#[tokio::test]
async fn synthetic_live_proof_crosses_core_go_processes_and_posts_once() {
    // No SQLx macro may connect or create a database before the launch check.
    let (_, marker) = runner_context();
    let url = std::env::var("DATABASE_URL").expect("owned runner DSN required");
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert_eq!(parsed.port().map(u64::from), marker["port"].as_u64());
    assert_eq!(parsed.path(), "/postgres");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let physical: (String, String) = sqlx::query_as(
        "SELECT system_identifier::text,pg_postmaster_start_time()::text FROM pg_control_system()",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(physical.0, marker["system_id"].as_str().unwrap());
    assert_eq!(physical.1, marker["postmaster_start"].as_str().unwrap());
    let database = format!("lmm_live_core_{:016x}", rand::random::<u64>());
    sqlx::query(&format!("CREATE DATABASE {database} TEMPLATE template0"))
        .execute(&admin)
        .await
        .unwrap();
    let mut core_url = parsed;
    core_url.set_path(&database);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(core_url.as_str())
        .await
        .unwrap();
    admin.close().await;
    let f = prepare_payment_fixture(&pool).await;
    assert_eq!(f.intent.environment, "live");
    // First callback is signed for test mode, including nested object mode. Core
    // must reject it before any durable proof or money; Go cannot override mode.
    let original: Value = serde_json::from_slice(&fs::read(&f.fixture).unwrap()).unwrap();
    let mut invalid = original.clone();
    let payload = invalid["events"][0]["payload"].as_str().unwrap();
    let mut value: Value = serde_json::from_str(payload).unwrap();
    value["livemode"] = json!(false);
    value["data"]["object"]["livemode"] = json!(false);
    let payload = serde_json::to_string(&value).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(payload.as_bytes());
    let signature: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    invalid["events"][0] = json!({"payload":payload,"signature":format!("t={now},v1={signature}")});
    let rejected = private(
        &f.root,
        "wrong-mode.json",
        &serde_json::to_vec(&invalid).unwrap(),
    );
    let before_orders: i64 = sqlx::query_scalar("SELECT count(*) FROM payment_orders")
        .fetch_one(&f.go_owner)
        .await
        .unwrap();
    assert_eq!(before_orders, 0);
    bootstrap(&f.boot_bin, &f.boot_sha, &rejected, &f.root, "reject-mode").await;
    let binding: (Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT provider_transaction,credit_journal_id FROM core_payments.intents WHERE id=$1",
    )
    .bind(f.intent.intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        binding,
        (None, None),
        "negative controls changed intent authority"
    );
    let after_orders: i64 = sqlx::query_scalar("SELECT count(*) FROM payment_orders")
        .fetch_one(&f.go_owner)
        .await
        .unwrap();
    assert_eq!(
        after_orders, before_orders,
        "negative controls created orders or jobs"
    );
    financial(&pool, f.account, 0).await;
    let proofs: i64 = sqlx::query_scalar("SELECT count(*) FROM core_payments.accepted_proofs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(proofs, 0, "wrong-mode signed proof persisted");
    let inbox: i64 = sqlx::query_scalar("SELECT count(*) FROM payment_events")
        .fetch_one(&f.go_owner)
        .await
        .unwrap();
    assert_eq!(inbox, 0, "wrong-mode callback persisted inbox");
    bootstrap(&f.boot_bin, &f.boot_sha, &f.fixture, &f.root, "recv").await;
    financial(&pool, f.account, 0).await;
    let mut go = spawn(&f.go_bin, &f.go_sha, &f.go_config, &f.root, "go-main.log");
    ready(&f.client, &f.go_endpoint, &mut go).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    let order_id = format!("top_{}", f.intent.intent_id);
    let order = loop {
        let order = snapshot(&f.go_owner, &order_id).await;
        if order
            .get("credit_receipt")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            break order;
        }
        assert!(
            Instant::now() < deadline,
            "real Go main did not credit accepted live proof"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let association = credit_association(&pool, f.account, f.intent.intent_id, &order, true).await;
    let funds = financial(&pool, f.account, 500000).await;
    private(&f.root,"main-live-credit.json",&serde_json::to_vec(&json!({"association":association,"funds":funds,"core_pid":f.core.0.id(),"go_pid":go.0.id()})).unwrap());
    bootstrap(&f.boot_bin, &f.boot_sha, &f.fixture, &f.root, "settle").await;
    let after = snapshot(&f.go_owner, &order_id).await;
    assert_eq!(
        credit_association(&pool, f.account, f.intent.intent_id, &after, true).await,
        association
    );
    financial(&pool, f.account, 500000).await;
    let proofs: i64 = sqlx::query_scalar("SELECT count(*) FROM core_payments.accepted_proofs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let inbox: (i64, i64) =
        sqlx::query_as("SELECT count(*),count(*) FILTER (WHERE status='done') FROM payment_events")
            .fetch_one(&f.go_owner)
            .await
            .unwrap();
    assert_eq!(proofs, 2);
    assert_eq!(inbox, (2, 2));
    record_identity(&f.core, "final");
    record_identity(&go, "final");
    let pids = [f.core.0.id(), go.0.id()];
    drop(go);
    drop(f.core);
    for pid in pids {
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }
    private(&f.root,"result.json",&serde_json::to_vec(&json!({"result":"PASS","mode":"synthetic-live","funds":funds,"association":association,"accepted_proofs":proofs,"inbox":inbox,"product_pids":pids,"products_reaped":true,"wrong_mode_rejected_without_proof_or_money":true,"full_payment_acceptance":false,"limits":["test-only bootstrap provider transport; not production public callback ingress","no checkout/refund or OS restart acceptance","formal artifact runner preflight not satisfied"]})).unwrap());
    f.go_owner.close().await;
    f.role.cleanup().await;
}
