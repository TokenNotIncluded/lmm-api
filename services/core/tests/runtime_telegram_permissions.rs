//! Exact native Telegram runtime ACLs in SQLx-owned disposable PostgreSQL.
//! Checks inspect authority only; no account, assertion or ceremony is created.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError as Error},
};
use sqlx::PgPool;

#[path = "common/runtime.rs"]
mod runtime;

const TABLES: [&str; 3] = ["bindings", "flows", "assertions"];
const MINIMUM: [&str; 17] = [
    "USAGE ON SCHEMA core_telegram_auth",
    "SELECT ON core_telegram_auth.bindings",
    "SELECT ON core_telegram_auth.flows",
    "SELECT ON core_telegram_auth.assertions",
    "INSERT(subject) ON core_telegram_auth.bindings",
    "INSERT(user_id) ON core_telegram_auth.bindings",
    "UPDATE(revoked_at) ON core_telegram_auth.bindings",
    "INSERT(token_digest) ON core_telegram_auth.flows",
    "INSERT(purpose) ON core_telegram_auth.flows",
    "INSERT(binding_digest) ON core_telegram_auth.flows",
    "INSERT(user_id) ON core_telegram_auth.flows",
    "INSERT(user_version) ON core_telegram_auth.flows",
    "INSERT(sid) ON core_telegram_auth.flows",
    "UPDATE(consumed_at) ON core_telegram_auth.flows",
    "DELETE ON core_telegram_auth.flows",
    "INSERT(digest) ON core_telegram_auth.assertions",
    "DELETE ON core_telegram_auth.assertions",
];

fn required() -> RequiredCapabilities {
    RequiredCapabilities {
        web_identity: true,
        ..Default::default()
    }
}

async fn fixture(owner: &PgPool) -> runtime::RuntimeRole {
    core_schema::init(owner).await.unwrap();
    let role = runtime::RuntimeRole::create(owner).await;
    role.optional("web_identity").await;
    // This first positive uses the shared real runtime provisioning. Tests do
    // not fill in missing Telegram ACLs before checking that provisioning.
    runtime_database::check(&role.pool, required())
        .await
        .unwrap();
    runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    role
}

async fn grant(owner: &PgPool, role: &runtime::RuntimeRole, privilege: &str) {
    sqlx::raw_sql(&format!("GRANT {privilege} TO {}", role.name))
        .execute(owner)
        .await
        .unwrap();
}

async fn revoke(owner: &PgPool, role: &runtime::RuntimeRole, privilege: &str) {
    sqlx::raw_sql(&format!("REVOKE {privilege} FROM {}", role.name))
        .execute(owner)
        .await
        .unwrap();
}

async fn restore_minimum(owner: &PgPool, role: &runtime::RuntimeRole) {
    // REVOKE of table INSERT/UPDATE also removes its column ACLs. Restore the
    // entire narrow contract rather than accidentally granting table writes.
    for privilege in MINIMUM {
        grant(owner, role, privilege).await;
    }
    runtime_database::check(&role.pool, required())
        .await
        .unwrap();
}

async fn assert_unsafe(role: &runtime::RuntimeRole, privilege: &str) {
    for capabilities in [Default::default(), required()] {
        assert_eq!(
            runtime_database::check(&role.pool, capabilities).await,
            Err(Error::UnsafePrivileges),
            "unexpected authority: {privilege}"
        );
    }
}

async fn assert_no_business_rows(owner: &PgPool) {
    let rows: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_telegram_auth.bindings),
                (SELECT count(*) FROM core_telegram_auth.flows),
                (SELECT count(*) FROM core_telegram_auth.assertions)",
    )
    .fetch_one(owner)
    .await
    .unwrap();
    assert_eq!(rows, (0, 0, 0));
}

#[sqlx::test(migrations = false)]
async fn telegram_runtime_requires_each_exact_minimum_privilege(pool: PgPool) {
    let role = fixture(&pool).await;
    for privilege in MINIMUM {
        revoke(&pool, &role, privilege).await;
        assert_eq!(
            runtime_database::check(&role.pool, required()).await,
            Err(Error::MissingPrivileges),
            "required authority: {privilege}"
        );
        // An optional feature may lack an otherwise safe privilege. Missing
        // schema USAGE must be Missing, never a name-resolution Unavailable.
        runtime_database::check(&role.pool, Default::default())
            .await
            .unwrap();
        restore_minimum(&pool, &role).await;
    }
    assert_no_business_rows(&pool).await;
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn telegram_runtime_rejects_every_extra_table_and_column_authority(pool: PgPool) {
    let role = fixture(&pool).await;
    let mut excessive = Vec::new();
    for table in TABLES {
        for privilege in [
            "INSERT",
            "UPDATE",
            "TRUNCATE",
            "TRIGGER",
            "REFERENCES",
            "MAINTAIN",
        ] {
            excessive.push(format!("{privilege} ON core_telegram_auth.{table}"));
        }
    }
    excessive.extend(
        [
            "DELETE ON core_telegram_auth.bindings",
            "INSERT(created_at) ON core_telegram_auth.bindings",
            "INSERT(revoked_at) ON core_telegram_auth.bindings",
            "UPDATE(subject) ON core_telegram_auth.bindings",
            "UPDATE(user_id) ON core_telegram_auth.bindings",
            "UPDATE(created_at) ON core_telegram_auth.bindings",
            "INSERT(created_at) ON core_telegram_auth.flows",
            "INSERT(expires_at) ON core_telegram_auth.flows",
            "INSERT(consumed_at) ON core_telegram_auth.flows",
            "UPDATE(token_digest) ON core_telegram_auth.flows",
            "UPDATE(purpose) ON core_telegram_auth.flows",
            "UPDATE(binding_digest) ON core_telegram_auth.flows",
            "UPDATE(user_id) ON core_telegram_auth.flows",
            "UPDATE(user_version) ON core_telegram_auth.flows",
            "UPDATE(sid) ON core_telegram_auth.flows",
            "UPDATE(created_at) ON core_telegram_auth.flows",
            "UPDATE(expires_at) ON core_telegram_auth.flows",
            "INSERT(created_at) ON core_telegram_auth.assertions",
            "INSERT(expires_at) ON core_telegram_auth.assertions",
            "UPDATE(digest) ON core_telegram_auth.assertions",
            "UPDATE(created_at) ON core_telegram_auth.assertions",
            "UPDATE(expires_at) ON core_telegram_auth.assertions",
            "REFERENCES(subject) ON core_telegram_auth.bindings",
            "REFERENCES(token_digest) ON core_telegram_auth.flows",
            "REFERENCES(digest) ON core_telegram_auth.assertions",
            "CREATE ON SCHEMA core_telegram_auth",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    for privilege in excessive {
        grant(&pool, &role, &privilege).await;
        assert_unsafe(&role, &privilege).await;
        revoke(&pool, &role, &privilege).await;
        restore_minimum(&pool, &role).await;
    }
    assert_no_business_rows(&pool).await;
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn telegram_runtime_rejects_grant_options_on_otherwise_allowed_authority(pool: PgPool) {
    let role = fixture(&pool).await;
    let mut privileges = MINIMUM.to_vec();
    privileges.extend([
        "SELECT(subject) ON core_telegram_auth.bindings",
        "SELECT(token_digest) ON core_telegram_auth.flows",
        "SELECT(digest) ON core_telegram_auth.assertions",
    ]);
    for privilege in privileges {
        sqlx::raw_sql(&format!(
            "GRANT {privilege} TO {} WITH GRANT OPTION",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert_unsafe(&role, privilege).await;
        sqlx::raw_sql(&format!(
            "REVOKE GRANT OPTION FOR {privilege} FROM {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        restore_minimum(&pool, &role).await;
    }
    assert_no_business_rows(&pool).await;
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn telegram_runtime_optional_inaccessible_schema_still_rejects_hidden_excess(pool: PgPool) {
    let role = fixture(&pool).await;
    sqlx::raw_sql(&format!(
        "REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA core_telegram_auth FROM {0};
         REVOKE ALL PRIVILEGES ON SCHEMA core_telegram_auth FROM {0}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    let access: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege(n.oid,'USAGE,CREATE')
             OR EXISTS(SELECT FROM pg_class c WHERE c.relnamespace=n.oid
                 AND c.relkind IN ('r','p','v','m','f')
                 AND (has_table_privilege(c.oid,'SELECT,INSERT,UPDATE,DELETE')
                     OR has_any_column_privilege(c.oid,'SELECT,INSERT,UPDATE,REFERENCES')))
         FROM pg_namespace n WHERE n.nspname='core_telegram_auth'",
    )
    .fetch_one(&role.pool)
    .await
    .unwrap();
    assert!(!access);
    runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required()).await,
        Err(Error::MissingPrivileges)
    );
    // Grant a forbidden column without restoring schema visibility. OID-based
    // inspection must still reject authority while Telegram is disabled.
    let excessive = "UPDATE(user_id) ON core_telegram_auth.bindings";
    grant(&pool, &role, excessive).await;
    assert_unsafe(&role, excessive).await;
    revoke(&pool, &role, excessive).await;
    runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    sqlx::query("GRANT UPDATE(subject) ON core_telegram_auth.bindings TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    assert_unsafe(
        &role,
        "PUBLIC UPDATE(subject) ON core_telegram_auth.bindings",
    )
    .await;
    sqlx::query("REVOKE UPDATE(subject) ON core_telegram_auth.bindings FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    assert_no_business_rows(&pool).await;
    role.cleanup().await;
}
