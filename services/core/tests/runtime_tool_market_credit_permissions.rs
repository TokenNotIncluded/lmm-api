//! Narrow CREDIT v2 runtime grants. These checks do not perform business writes.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError},
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;
#[sqlx::test]
async fn credit_runtime_rejects_missing_and_excess_authority(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let required = RequiredCapabilities {
        tool_market: true,
        ..Default::default()
    };
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    role.optional("core_tool_market").await;
    runtime_database::check(&role.pool, required).await.unwrap();
    for grant in [
        "SELECT ON core_tool_market.credit_result_contracts",
        "INSERT ON core_tool_market.credit_supplier_receipts",
        "INSERT ON core_tool_market.credit_fixed_witness_receipts",
        "UPDATE(receipt) ON core_tool_market.credit_calls",
        "INSERT(seller_account_id) ON core_tool_market.credit_quotes",
        "UPDATE(active_version_id) ON core_tool_market.credit_services",
        "EXECUTE ON FUNCTION core_tool_market.lock_builtin_context(text)",
    ] {
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(RuntimeDatabaseError::MissingPrivileges),
            "{grant}"
        );
        role.optional("core_tool_market").await;
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    for grant in [
        "INSERT ON core_tool_market.credit_policy",
        "INSERT ON core_tool_market.credit_builtin_bindings",
        "UPDATE ON core_tool_market.credit_result_contracts",
        "UPDATE ON core_tool_market.credit_supplier_receipts",
        "UPDATE(market_oauth_family_id) ON core_tool_market.credit_calls",
        "UPDATE(payer_account_id) ON core_tool_market.credit_calls",
        "UPDATE(snapshot) ON core_tool_market.credit_quotes",
        "INSERT(approved) ON core_tool_market.credit_quotes",
        "UPDATE(owner_user_id) ON core_tool_market.credit_services",
        "DELETE ON core_tool_market.credit_services",
        "REFERENCES(actor_user_id) ON core_tool_market.credit_calls",
        "CREATE ON SCHEMA core_tool_market",
        "EXECUTE ON FUNCTION core_billing.import_opening_balance(jsonb)",
    ] {
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, Default::default()).await,
            Err(RuntimeDatabaseError::UnsafePrivileges),
            "{grant}"
        );
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        role.optional("core_tool_market").await;
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    sqlx::query("GRANT EXECUTE ON FUNCTION core_tool_market.lock_builtin_context(text) TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, Default::default()).await,
        Err(RuntimeDatabaseError::UnsafePrivileges)
    );
    sqlx::query(
        "REVOKE EXECUTE ON FUNCTION core_tool_market.lock_builtin_context(text) FROM PUBLIC",
    )
    .execute(&pool)
    .await
    .unwrap();
    runtime_database::check(&role.pool, required).await.unwrap();
    role.cleanup().await;
}
