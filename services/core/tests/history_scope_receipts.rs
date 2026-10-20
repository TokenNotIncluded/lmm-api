//! Real PostgreSQL scope provenance checks. A global source alone proves no identity.
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    history_read::{HistoryError, HistoryQuery, HistoryStore, ReadRoute},
    identity::IdentityStore,
};
use sqlx::PgPool;
#[path = "common/history_scope.rs"]
mod history_scope;
async fn fixture(pool: &PgPool) -> (IdentityStore, String, i64) {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(9901, 1).await.unwrap();
    let owner =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=9901")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'synthetic-scoped-source',1700000000,true)").execute(pool).await.unwrap();
    (identity, user.secret, owner)
}
async fn read(
    pool: &PgPool,
    secret: &str,
    route: ReadRoute,
) -> Result<lmm_core::history_read::HistorySnapshot, HistoryError> {
    HistoryStore::new(pool.clone())
        .read(secret, route, &HistoryQuery::default())
        .await
}
#[sqlx::test(migrations = false)]
async fn reviewed_complete_identity_mapping_proves_genuinely_empty_history(pool: PgPool) {
    let (_, user, _) = fixture(&pool).await;
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
    history_scope::install(&pool).await;
    let result = read(&pool, &user, ReadRoute::SelfSession).await.unwrap();
    assert_eq!(result.total, 0);
    assert!(result.items.is_empty());
    assert_eq!(result.summary.money.quota_usd.as_deref(), Some("0"));
    assert_eq!(result.summary.money.quota_money_basis, "empty_result");
}
#[sqlx::test(migrations = false)]
async fn source_digest_mismatch_and_incomplete_scope_never_prove_empty_history(pool: PgPool) {
    let (_, user, _) = fixture(&pool).await;
    history_scope::install(&pool).await;
    sqlx::query("UPDATE core_history.scope_receipts SET mapping_sha256=repeat('f',64)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
    sqlx::query(
        "UPDATE core_history.scope_receipts SET mapping_sha256=repeat('b',64),complete=false",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}
#[sqlx::test(migrations = false)]
async fn exact_key_mapping_is_required_beyond_complete_user_mapping(pool: PgPool) {
    let (identity, user, _) = fixture(&pool).await;
    history_scope::install(&pool).await;
    let key = identity
        .issue_key(
            &user,
            Account {
                kind: AccountKind::Personal,
                id: 9901,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&pool, &key.secret, ReadRoute::ReadOnlyKey)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
    history_scope::install(&pool).await;
    let result = read(&pool, &key.secret, ReadRoute::ReadOnlyKey)
        .await
        .unwrap();
    assert_eq!(result.total, 0);
    sqlx::query("UPDATE core_history.scope_receipts SET source_archive_sha256=repeat('f',64) WHERE token_id=$1").bind(key.id).execute(&pool).await.unwrap();
    assert_eq!(
        read(&pool, &key.secret, ReadRoute::ReadOnlyKey)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}
#[sqlx::test(migrations = false)]
async fn another_identity_or_account_receipt_never_covers_this_actor(pool: PgPool) {
    let (identity, user, _) = fixture(&pool).await;
    identity.bootstrap_user(9902, 1).await.unwrap();
    history_scope::install(&pool).await;
    sqlx::query("DELETE FROM core_history.scope_receipts WHERE user_id=9901")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
    sqlx::query("INSERT INTO core_history.scope_receipts SELECT source_id,9901,owner_account_id,token_id,source_archive_sha256,mapping_sha256,source_scope_sha256,complete FROM core_history.scope_receipts WHERE user_id=9902").execute(&pool).await.unwrap();
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}
#[sqlx::test(migrations = false)]
async fn every_installed_source_requires_its_own_matching_scope_proof(pool: PgPool) {
    let (_, user, _) = fixture(&pool).await;
    history_scope::install(&pool).await;
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('c',64),repeat('d',64),'synthetic-second-source',1700000000,true)").execute(&pool).await.unwrap();
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
    history_scope::install(&pool).await;
    assert_eq!(
        read(&pool, &user, ReadRoute::SelfSession)
            .await
            .unwrap()
            .total,
        0
    );
}
