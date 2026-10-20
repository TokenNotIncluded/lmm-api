//! Exact restricted runtime authority, using a new SQLx-owned database only.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError as Error},
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn assistant_confirmation_permissions_are_explicit_and_column_limited(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let required = RequiredCapabilities {
        assistant_tools: true,
        ..RequiredCapabilities::default()
    };
    runtime_database::check(&role.pool, RequiredCapabilities::default())
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    role.optional("core_assistant_tools").await;
    runtime_database::check(&role.pool, required).await.unwrap();
    sqlx::query("SELECT * FROM core_assistant_tools.confirmations")
        .fetch_all(&role.pool)
        .await
        .unwrap();
    for column in ["nonce_sha256", "state", "confirmed_at"] {
        sqlx::query(&format!(
            "UPDATE core_assistant_tools.confirmations SET {column}={column} WHERE FALSE"
        ))
        .execute(&role.pool)
        .await
        .unwrap();
        sqlx::query(&format!(
            "REVOKE UPDATE({column}) ON core_assistant_tools.confirmations FROM {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::MissingPrivileges)
        );
        sqlx::query(&format!(
            "GRANT UPDATE({column}) ON core_assistant_tools.confirmations TO {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(&format!(
        "REVOKE UPDATE(revision) ON core_humangrant.human_grant_gate FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    role.optional("core_assistant_tools").await;
    for table in [
        "intents",
        "accepted_proofs",
        "refund_holds",
        "blocked_refunds",
    ] {
        sqlx::query(&format!(
            "REVOKE SELECT ON core_payments.{table} FROM {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::MissingPrivileges)
        );
        sqlx::query(&format!(
            "GRANT SELECT ON core_payments.{table} TO {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    let columns:Vec<String>=sqlx::query_scalar("SELECT column_name::text FROM information_schema.columns WHERE table_schema='core_assistant_tools' AND table_name='confirmations' AND column_name NOT IN ('nonce_sha256','state','confirmed_at') ORDER BY ordinal_position").fetch_all(&pool).await.unwrap();
    for column in columns {
        let e = sqlx::query(&format!(
            "UPDATE core_assistant_tools.confirmations SET {column}={column} WHERE FALSE"
        ))
        .execute(&role.pool)
        .await
        .unwrap_err();
        assert_eq!(
            e.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
        sqlx::query(&format!(
            "GRANT UPDATE({column}) ON core_assistant_tools.confirmations TO {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::UnsafePrivileges),
            "{column}"
        );
        sqlx::query(&format!(
            "REVOKE UPDATE({column}) ON core_assistant_tools.confirmations FROM {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    for privilege in [
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "TRIGGER",
        "REFERENCES",
        "MAINTAIN",
    ] {
        sqlx::query(&format!(
            "GRANT {privilege} ON core_assistant_tools.confirmations TO {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::UnsafePrivileges),
            "{privilege}"
        );
        sqlx::query(&format!(
            "REVOKE {privilege} ON core_assistant_tools.confirmations FROM {}",
            role.name
        ))
        .execute(&pool)
        .await
        .unwrap();
        // Revoking whole UPDATE also removes its column grants in PostgreSQL.
        role.optional("core_assistant_tools").await;
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    sqlx::query(&format!(
        "GRANT REFERENCES(state) ON core_assistant_tools.confirmations TO {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::UnsafePrivileges)
    );
    sqlx::query(&format!(
        "REVOKE REFERENCES(state) ON core_assistant_tools.confirmations FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    // Effective grants inherited from PUBLIC are unsafe too.
    sqlx::query("GRANT UPDATE(preview) ON core_assistant_tools.confirmations TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::UnsafePrivileges)
    );
    sqlx::query("REVOKE UPDATE(preview) ON core_assistant_tools.confirmations FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    runtime_database::check(&role.pool, required).await.unwrap();
    role.cleanup().await;
}
