//! Narrow runtime authority: offline history and campaign facts are immutable.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError as Error},
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn gift_and_history_reject_excess_privileges_even_when_feature_disabled(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_gift").await;
    runtime_database::check(&role.pool, RequiredCapabilities::default())
        .await
        .unwrap();
    for grant in [
        "UPDATE ON core_gift.policy",
        "UPDATE(exclusive_cap) ON core_gift.policy",
        "INSERT(singleton) ON core_gift.risk_key",
        "UPDATE(reward_used) ON core_gift.qualifications",
        "DELETE ON core_gift.qualifications",
        "UPDATE(amount_credits) ON core_gift.decisions",
        "UPDATE(user_id) ON core_gift.registration_profiles",
        "UPDATE(token) ON core_gift.fingerprints",
        "DELETE ON core_gift.decisions",
        "TRUNCATE ON core_gift.fingerprints",
        "REFERENCES(user_id) ON core_gift.registration_profiles",
        "UPDATE ON SEQUENCE core_gift.fingerprints_id_seq",
        "INSERT ON core_history.scope_receipts",
        "UPDATE(source_scope_sha256) ON core_history.scope_receipts",
        "DELETE ON core_history.scope_receipts",
        "UPDATE(request_id) ON core_assistant.operations",
    ] {
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, RequiredCapabilities::default()).await,
            Err(Error::UnsafePrivileges),
            "{grant}"
        );
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        runtime_database::check(&role.pool, RequiredCapabilities::default())
            .await
            .unwrap();
    }
    sqlx::query("GRANT EXECUTE ON FUNCTION core_gift.lock_eligibility(bigint) TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, RequiredCapabilities::default()).await,
        Err(Error::UnsafePrivileges)
    );
    sqlx::query("REVOKE EXECUTE ON FUNCTION core_gift.lock_eligibility(bigint) FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    runtime_database::check(&role.pool, RequiredCapabilities::default())
        .await
        .unwrap();
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn gift_identity_and_execution_require_exact_minimum_grants(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    runtime_database::check(&role.pool, RequiredCapabilities::default())
        .await
        .unwrap();
    for grant in [
        "SELECT ON core_history.scope_receipts",
        "SELECT ON core_gift.risk_key",
        "SELECT ON core_gift.policy",
        "SELECT ON core_gift.qualifications",
        "INSERT ON core_gift.qualifications",
        "EXECUTE ON FUNCTION core_gift.lock_eligibility(bigint)",
    ] {
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, RequiredCapabilities::default()).await,
            Err(Error::MissingPrivileges),
            "{grant}"
        );
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    role.optional("core_charging").await;
    role.optional("core_assistant").await;
    let required = RequiredCapabilities {
        assistant: true,
        ..Default::default()
    };
    runtime_database::check(&role.pool, required).await.unwrap();
    for grant in [
        "INSERT ON core_gift.decisions",
        "UPDATE(claimed_at) ON core_gift.decisions",
        "UPDATE(ledger_journal_id) ON core_gift.decisions",
        "UPDATE(observed_at) ON core_gift.registration_profiles",
        "INSERT ON core_gift.fingerprints",
        "DELETE ON core_gift.fingerprints",
        "USAGE ON SEQUENCE core_gift.fingerprints_id_seq",
        "SELECT ON SEQUENCE core_gift.fingerprints_id_seq",
        "UPDATE(gift_receipt) ON core_assistant.operations",
        "UPDATE(gift_tool_id) ON core_assistant.operations",
        "UPDATE(gift_response_sha256) ON core_assistant.operations",
    ] {
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::MissingPrivileges),
            "{grant}"
        );
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    role.cleanup().await;
}
