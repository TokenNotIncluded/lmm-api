//! Actual offline command and BrowserAuth against SQLx-owned fresh PostgreSQL.
use lmm_core::{
    console_compat::ConsoleCompat,
    core_schema,
    identity::{
        BrowserAuth, BrowserSessionMetadata, IdentityStore,
        admin_users::{AdminUsersStore, UsersQuery},
    },
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
#[path = "common/runtime.rs"]
mod runtime;

const PASSWORD: &str = "synthetic first-owner private password";

struct PrivateConfig {
    dir: PathBuf,
    path: PathBuf,
}
impl PrivateConfig {
    fn new(url: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "lmm-first-owner-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let url_file = dir.join("database.url");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&url_file)
            .unwrap()
            .write_all(url.as_bytes())
            .unwrap();
        let path = dir.join("installer.toml");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(
                format!(
                    "version=1\n[database]\nurl_file={}\nmax_connections=1\n",
                    json!(url_file.to_str().unwrap())
                )
                .as_bytes(),
            )
            .unwrap();
        Self { dir, path }
    }
    fn command(&self, input: &str) -> Output {
        invoke(
            &[
                "--config",
                self.path.to_str().unwrap(),
                "create-first-owner",
            ],
            input,
        )
    }
}
impl Drop for PrivateConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn invoke(arguments: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lmm-core"))
        .args(arguments)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("first-owner command exceeded its test deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn input() -> String {
    json!({"username":"site-owner","display_name":"Site Owner","password":PASSWORD}).to_string()
}
fn owner_url(pool: &PgPool) -> reqwest::Url {
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(url.host_str(), Some("localhost" | "127.0.0.1")));
    let database = pool.connect_options().get_database().unwrap().to_owned();
    assert!(database.starts_with("_sqlx_test_"));
    url.set_path(&database);
    url
}
fn refused(output: &Output) {
    assert!(!output.status.success(), "command unexpectedly accepted");
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!error.contains(PASSWORD));
    assert!(!error.contains("postgres://"));
    assert!(!error.contains("postgresql://"));
}
async fn owner_state(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('accounts',(SELECT count(*) FROM core_identity.accounts),'users',(SELECT count(*) FROM core_identity.users),'profiles',(SELECT count(*) FROM core_identity.user_profiles),'passwords',(SELECT count(*) FROM core_identity.password_logins),'policies',(SELECT count(*) FROM core_billing.account_policies),'setup',(SELECT count(*) FROM core_console.initialization),'audit',(SELECT count(*) FROM core_identity.audit),'grants',(SELECT count(*) FROM core_identity.developer_access),'credentials',(SELECT count(*) FROM core_identity.credentials),'journals',(SELECT count(*) FROM core_billing.ledger_journals),'entries',(SELECT count(*) FROM core_billing.ledger_entries),'gifts',(SELECT count(*) FROM core_gift.decisions))")
        .fetch_one(pool).await.unwrap()
}

#[test]
fn explicit_installer_config_and_closed_secret_input_are_required() {
    for arguments in [
        vec!["create-first-owner"],
        vec!["--legacy-env", "create-first-owner"],
    ] {
        refused(&invoke(&arguments, &input()));
    }
    let config = PrivateConfig::new("postgres://invalid/never-connect");
    for invalid in [
        "x".repeat(4097),
        input().replace('}', ",\"unexpected-secret-field\":true}"),
        input().replace('}', ",\"password\":\"duplicate-password\"}"),
        input().replace(PASSWORD, "short"),
    ] {
        let output = config.command(&invalid);
        refused(&output);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("unexpected-secret-field"));
    }
}

#[sqlx::test(migrations = false)]
async fn first_owner_logs_in_to_web_with_real_root_role_and_no_financial_grant(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let before = owner_state(&pool).await;
    let config = PrivateConfig::new(owner_url(&pool).as_str());
    let output = config.command(&input());
    assert!(output.status.success(), "offline first-owner failed");
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(PASSWORD));
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    let user = receipt["user_id"].as_i64().unwrap();
    assert_eq!(
        receipt,
        json!({"protocol":"core.first-owner.v1","user_id":user,"username":"site-owner","platform_role":"superadmin","setup_completed":true})
    );
    let row = sqlx::query("SELECT u.platform_role,u.active,a.service_level,a.active AS account_active,p.password_login_allowed,p.second_factor_required,p.legacy_bcrypt_allowed,l.password_hash,b.preference FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id JOIN core_identity.password_logins l ON l.user_id=u.id JOIN core_billing.account_policies b ON b.account_id=a.id")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<String, _>("platform_role"), "superadmin");
    assert!(row.get::<bool, _>("active") && row.get::<bool, _>("account_active"));
    assert_eq!(row.get::<i16, _>("service_level"), 0);
    assert_eq!(row.get::<String, _>("preference"), "wallet_only");
    assert!(row.get::<bool, _>("password_login_allowed"));
    assert!(!row.get::<bool, _>("second_factor_required"));
    assert!(!row.get::<bool, _>("legacy_bcrypt_allowed"));
    assert!(
        row.get::<String, _>("password_hash")
            .starts_with("$argon2id$v=19$m=19456,t=2,p=1$")
    );
    let state = owner_state(&pool).await;
    for name in [
        "accounts",
        "users",
        "profiles",
        "passwords",
        "policies",
        "setup",
        "audit",
    ] {
        assert_eq!(state[name], 1);
    }
    for name in ["grants", "credentials", "journals", "entries", "gifts"] {
        assert_eq!(before[name], 0);
        assert_eq!(state[name], 0);
    }
    let balances: Vec<(i64, i64)> =
        sqlx::query_as("SELECT balance_units,revision FROM core_billing.balance_state")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(balances, vec![(0, 0), (0, 0)]);
    let identity = IdentityStore::from_pool(pool.clone());
    assert!(
        ConsoleCompat::new(pool.clone(), identity.clone())
            .setup_status()
            .await
            .unwrap()
            .status
    );
    let browser = BrowserAuth::new(identity.clone(), &[71; 32]).unwrap();
    assert!(
        browser
            .login(
                "site-owner",
                "wrong synthetic password",
                &BrowserSessionMetadata::default()
            )
            .await
            .is_err()
    );
    let session = browser
        .login("site-owner", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    assert_eq!(session.data["user"]["role"], 100);
    assert_eq!(session.data["user"]["developer_access_granted"], true);
    let secret = session.data["access_token"].as_str().unwrap();
    assert_eq!(identity.authorize(secret).await.unwrap().platform_level, 6);
    AdminUsersStore::new(identity)
        .users(secret, &UsersQuery::default())
        .await
        .unwrap();
    let prior = owner_state(&pool).await;
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, prior);
}

#[sqlx::test(migrations = false)]
async fn simultaneous_first_owner_commands_have_exactly_one_winner(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let config_a = PrivateConfig::new(owner_url(&pool).as_str());
    let config_b = PrivateConfig::new(owner_url(&pool).as_str());
    let (a, b) = tokio::join!(
        tokio::task::spawn_blocking(move || config_a.command(&input())),
        tokio::task::spawn_blocking(move || config_b.command(&input()))
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(
        usize::from(a.status.success()) + usize::from(b.status.success()),
        1
    );
    refused(if a.status.success() { &b } else { &a });
    let state = owner_state(&pool).await;
    for name in [
        "accounts",
        "users",
        "profiles",
        "passwords",
        "policies",
        "setup",
        "audit",
    ] {
        assert_eq!(state[name], 1);
    }
}

#[sqlx::test(migrations = false)]
async fn existing_user_setup_or_application_data_is_not_a_fresh_install(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let config = PrivateConfig::new(owner_url(&pool).as_str());
    sqlx::query(
        "INSERT INTO core_console.options(key,value) VALUES ('sentinel','preserve existing site')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = owner_state(&pool).await;
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, before);
    sqlx::query("DELETE FROM core_console.options WHERE key='sentinel'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_console.initialization(version) VALUES ('existing-site')")
        .execute(&pool)
        .await
        .unwrap();
    let before = owner_state(&pool).await;
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, before);
    sqlx::query("DELETE FROM core_console.initialization")
        .execute(&pool)
        .await
        .unwrap();
    IdentityStore::from_pool(pool.clone())
        .bootstrap_user(411, 0)
        .await
        .unwrap();
    let before = owner_state(&pool).await;
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn restricted_runtime_cannot_create_first_owner(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_console").await;
    let config = PrivateConfig::new(&role.url);
    let before = owner_state(&pool).await;
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, before);
    drop(config);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn missing_or_tampered_oauth_schema_receipt_is_not_fresh(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let config = PrivateConfig::new(owner_url(&pool).as_str());
    sqlx::query("UPDATE core_oauth.contract SET catalog_fingerprint=decode(repeat('00',32),'hex')")
        .execute(&pool)
        .await
        .unwrap();
    core_schema::check(&pool).await.unwrap();
    let before = owner_state(&pool).await;
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, before);
    sqlx::query("DELETE FROM core_oauth.contract")
        .execute(&pool)
        .await
        .unwrap();
    refused(&config.command(&input()));
    assert_eq!(owner_state(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn explicit_set_role_to_nologin_database_and_relation_owner_is_supported(pool: PgPool) {
    let name = format!("lmm_first_owner_{:016x}", rand::random::<u64>());
    let database = pool.connect_options().get_database().unwrap().to_owned();
    let original: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(database.starts_with("_sqlx_test_"));
    assert!(
        original
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    sqlx::raw_sql(&format!("CREATE ROLE {name} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE; GRANT {name} TO \"{original}\"; ALTER DATABASE \"{database}\" OWNER TO {name}; GRANT USAGE,CREATE ON SCHEMA public TO {name}"))
        .execute(&pool).await.unwrap();
    let mut url = owner_url(&pool);
    url.query_pairs_mut()
        .append_pair("options", &format!("-c role={name}"));
    let installer = PgPoolOptions::new()
        .max_connections(1)
        .connect(url.as_str())
        .await
        .unwrap();
    core_schema::init(&installer).await.unwrap();
    let config = PrivateConfig::new(url.as_str());
    let output = config.command(&input());
    let success = output.status.success();
    installer.close().await;
    drop(config);
    sqlx::raw_sql(&format!("ALTER DATABASE \"{database}\" OWNER TO \"{original}\"; DROP OWNED BY {name}; REVOKE {name} FROM \"{original}\"; DROP ROLE {name}"))
        .execute(&pool).await.unwrap();
    assert!(success, "explicit NOLOGIN installer owner was rejected");
}
