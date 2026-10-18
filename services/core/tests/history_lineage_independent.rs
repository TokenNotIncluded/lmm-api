//! Independent required failures against the frozen A04 v2 schema/input.
//! A global complete source does not prove per-user/account/key coverage.
//! Preserve actual v2 FAILs; v3 requires its newly frozen schema and explicit
//! precondition checks, not success caused by a missing relation/storage fault.
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    history_read::{HistoryError, HistoryQuery, HistoryStore, ReadRoute},
    identity::IdentityStore,
};
use sqlx::PgPool;

async fn fixture(pool: &PgPool) -> IdentityStore {
    core_schema::init(pool).await.unwrap();
    sqlx::raw_sql(include_str!("identity_history_source.sql"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'independent-missing-lineage',1700000000,true)").execute(pool).await.unwrap();
    let has_global_source: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_history.sources WHERE complete)")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        has_global_source,
        "global source is genuinely installed, not missing-table 503"
    );
    IdentityStore::from_pool(pool.clone())
}

#[sqlx::test(migrations = false)]
async fn missing_user_account_lineage_cannot_be_reported_as_proven_empty_history(pool: PgPool) {
    let identity = fixture(&pool).await;
    let user = identity.bootstrap_user(9301, 1).await.unwrap();
    let result = HistoryStore::new(pool.clone())
        .read(
            &user.secret,
            ReadRoute::SelfSession,
            &HistoryQuery::default(),
        )
        .await;
    match result {
        Err(HistoryError::Unavailable) => {}
        Err(other) => panic!("missing scope lineage must be Unavailable, got {other:?}"),
        Ok(snapshot) => panic!(
            "FAIL: unproven user/account lineage became successful history: total={}, source={}, money_basis={}",
            snapshot.total, snapshot.source, snapshot.summary.money.quota_money_basis
        ),
    }
}

#[sqlx::test(migrations = false)]
async fn native_key_without_authoritative_historical_key_mapping_cannot_be_empty_success(
    pool: PgPool,
) {
    let identity = fixture(&pool).await;
    let user = identity.bootstrap_user(9301, 1).await.unwrap();
    let key = identity
        .issue_key(
            &user.secret,
            Account {
                kind: AccountKind::Personal,
                id: 9301,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let key_is_real:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.credentials WHERE id=$1 AND kind='api_key' AND user_id=9301 AND revoked_at IS NULL)").bind(key.id).fetch_one(&pool).await.unwrap();
    assert!(
        key_is_real,
        "fixture uses a genuine native Key, not an invalid-secret rejection"
    );
    let result = HistoryStore::new(pool.clone())
        .read(
            &key.secret,
            ReadRoute::ReadOnlyKey,
            &HistoryQuery::default(),
        )
        .await;
    match result {
        Err(HistoryError::Unavailable) => {}
        Err(other) => panic!("missing historical key mapping must be Unavailable, got {other:?}"),
        Ok(snapshot) => panic!(
            "FAIL: unproven key lineage became successful history: total={}, source={}, money_basis={}",
            snapshot.total, snapshot.source, snapshot.summary.money.quota_money_basis
        ),
    }
}
