//! Installer/runtime separation for the native wallet and bounty components.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError as Error},
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[test]
fn wallet_and_bounty_install_contract_uses_frozen_guarded_definitions() {
    assert_eq!(
        lmm_core::wallet::SCHEMA,
        include_str!("../schema/wallet.sql")
    );
    assert_eq!(
        lmm_core::bounties::SCHEMA,
        include_str!("../schema/bounties.sql")
    );
    let manifest = core_schema::schema_manifest();
    for name in ["wallet", "bounties"] {
        assert_eq!(
            manifest
                .components
                .iter()
                .filter(|c| c.name == name)
                .count(),
            1
        );
    }
    assert_eq!(manifest.components.len(), core_schema::component_count());
}
async fn grant(pool: &PgPool, role: &str, sql: &str) {
    sqlx::raw_sql(&sql.replace("ROLE", role))
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn wallet_runtime_requires_exact_mutable_columns_and_rejects_source_authority(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let required = RequiredCapabilities {
        wallet: true,
        ..Default::default()
    };
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    grant(&pool,&role.name,"GRANT USAGE ON SCHEMA core_wallet TO ROLE; GRANT SELECT,INSERT ON core_wallet.transfers TO ROLE; GRANT UPDATE(state,hold_journal,final_journal,recipient_user,confirmed_at,finished_at,confirmation_digest,confirmation_sid,confirmation_kind,confirmation_expires) ON core_wallet.transfers TO ROLE; GRANT USAGE,SELECT ON SEQUENCE core_wallet.transfers_id_seq TO ROLE;").await;
    runtime_database::check(&role.pool, required).await.unwrap();
    lmm_core::wallet::Store::new(role.pool.clone())
        .check_ready()
        .await
        .unwrap();
    for privilege in [
        "UPDATE(owner_user)",
        "UPDATE(payer_account)",
        "UPDATE(amount)",
        "UPDATE(request_key)",
        "UPDATE(token_digest)",
        "UPDATE(id)",
        "UPDATE(created_at)",
        "DELETE",
        "TRUNCATE",
        "UPDATE",
    ] {
        grant(
            &pool,
            &role.name,
            &format!("GRANT {privilege} ON core_wallet.transfers TO ROLE"),
        )
        .await;
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::UnsafePrivileges),
            "{privilege}"
        );
        grant(
            &pool,
            &role.name,
            &format!("REVOKE {privilege} ON core_wallet.transfers FROM ROLE"),
        )
        .await;
        if privilege == "UPDATE" {
            // PostgreSQL table UPDATE revocation also clears column grants.
            // Reapply the original minimal role before testing its health.
            grant(&pool,&role.name,"GRANT UPDATE(state,hold_journal,final_journal,recipient_user,confirmed_at,finished_at,confirmation_digest,confirmation_sid,confirmation_kind,confirmation_expires) ON core_wallet.transfers TO ROLE").await;
        }
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    grant(
        &pool,
        &role.name,
        "GRANT UPDATE ON SEQUENCE core_wallet.transfers_id_seq TO ROLE",
    )
    .await;
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::UnsafePrivileges)
    );
    grant(&pool,&role.name,"REVOKE UPDATE ON SEQUENCE core_wallet.transfers_id_seq FROM ROLE; REVOKE UPDATE(final_journal) ON core_wallet.transfers FROM ROLE").await;
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn bounty_runtime_cannot_rewrite_policy_original_confirmation_or_receipt(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let required = RequiredCapabilities {
        bounties: true,
        ..Default::default()
    };
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    grant(&pool,&role.name,"GRANT USAGE ON SCHEMA core_bounties TO ROLE; GRANT SELECT ON core_bounties.policy TO ROLE; GRANT SELECT,INSERT,UPDATE(body) ON core_bounties.projects TO ROLE; GRANT SELECT,INSERT(id,actor,credential,request,expires,policy),UPDATE(approved,csrf,browser_sid) ON core_bounties.confirmations TO ROLE; GRANT SELECT,INSERT ON core_bounties.operations TO ROLE").await;
    grant(&pool,&role.name,"GRANT USAGE ON SCHEMA core_charging TO ROLE; GRANT SELECT ON core_charging.budgets,core_charging.requests,core_charging.external_consumption,core_charging.budget_opening_consumption TO ROLE; GRANT UPDATE(revision) ON core_charging.budgets TO ROLE; GRANT INSERT,UPDATE(refunded) ON core_charging.external_consumption TO ROLE; GRANT EXECUTE ON FUNCTION core_charging.period_window(text,bigint,bigint,bigint),core_charging.committed_credits(text,bigint,bigint,bigint) TO ROLE").await;
    runtime_database::check(&role.pool, required).await.unwrap();
    let model_write:bool=sqlx::query_scalar("SELECT has_table_privilege(current_user,'core_charging.requests','INSERT') OR has_any_column_privilege(current_user,'core_charging.requests','INSERT,UPDATE')").fetch_one(&role.pool).await.unwrap();
    assert!(
        !model_write,
        "bounty must not inherit model traffic write authority"
    );
    for (relation, privilege) in [
        ("policy", "INSERT"),
        ("policy", "UPDATE(fee_bps)"),
        ("policy", "UPDATE(fee_account_id)"),
        ("operations", "UPDATE(receipt)"),
        ("operations", "DELETE"),
        ("projects", "UPDATE(id)"),
        ("projects", "DELETE"),
        ("confirmations", "UPDATE(request)"),
        ("confirmations", "UPDATE(actor)"),
        ("confirmations", "TRUNCATE"),
    ] {
        grant(
            &pool,
            &role.name,
            &format!("GRANT {privilege} ON core_bounties.{relation} TO ROLE"),
        )
        .await;
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::UnsafePrivileges),
            "{relation}.{privilege}"
        );
        grant(
            &pool,
            &role.name,
            &format!("REVOKE {privilege} ON core_bounties.{relation} FROM ROLE"),
        )
        .await;
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    grant(
        &pool,
        &role.name,
        "REVOKE UPDATE(approved) ON core_bounties.confirmations FROM ROLE",
    )
    .await;
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    role.cleanup().await;
}
