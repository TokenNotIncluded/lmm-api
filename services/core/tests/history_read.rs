//! Development regressions. These require new isolated PostgreSQL databases;
//! source presence or a skipped database run is not historical migration proof.
#[path = "common/history_scope.rs"]
mod history_scope;
use lmm_core::{
    core_schema,
    history_read::{HistoryError, HistoryQuery, HistoryStore, ReadRoute},
    identity::IdentityStore,
};
use sqlx::PgPool;

async fn fixture(pool: &PgPool) -> (String, String, i64, i64) {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    let owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=7")
            .fetch_one(pool)
            .await
            .unwrap();
    let other: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=8")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'synthetic-only',1700000000,true)").execute(pool).await.unwrap();
    history_scope::install(pool).await;
    (user.secret, admin.secret, owner, other)
}
async fn row(
    pool: &PgPool,
    id: i64,
    user: i64,
    owner: i64,
    quota: Option<i64>,
    basis: Option<i64>,
) {
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,pricing_credits_per_usd,username,model_name) VALUES(1,$1,$2,repeat('c',64),$3,$4,1700000000,2,$5,$6,'synthetic-user','synthetic-model')")
        .bind(id).bind(format!("synthetic-event-{id}")).bind(user).bind(owner).bind(quota).bind(basis).execute(pool).await.unwrap();
}
#[sqlx::test(migrations = false)]
async fn readonly_key_cannot_use_session_or_other_key_scope(pool: PgPool) {
    use lmm_core::{
        accounts::{Account, AccountKind},
        funding::FundingPolicy,
    };
    let (user, _, owner, _) = fixture(&pool).await;
    let identity = IdentityStore::from_pool(pool.clone());
    let key = identity
        .issue_key(
            &user,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    let other = identity
        .issue_key(
            &user,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    row(&pool, 1, 7, owner, Some(1), Some(1)).await;
    row(&pool, 2, 7, owner, Some(9), Some(1)).await;
    sqlx::query("UPDATE core_history.log_records SET token_id=CASE WHEN source_log_id=1 THEN $1 ELSE $2 END").bind(key.id).bind(other.id).execute(&pool).await.unwrap();
    history_scope::install(&pool).await;
    let store = HistoryStore::new(pool);
    let result = store
        .read(
            &key.secret,
            ReadRoute::ReadOnlyKey,
            &HistoryQuery::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.summary.money.quota_usd.as_deref(), Some("1"));
    assert_eq!(
        store
            .read(
                &key.secret,
                ReadRoute::SelfSession,
                &HistoryQuery::default()
            )
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    assert_eq!(
        store
            .read(&user, ReadRoute::ReadOnlyKey, &HistoryQuery::default())
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
}
#[sqlx::test(migrations = false)]
async fn mixed_bases_are_summed_exactly_once_and_other_user_is_not_in_page_or_total(pool: PgPool) {
    let (user, admin, owner, other) = fixture(&pool).await;
    row(&pool, 1, 7, owner, Some(1), Some(3)).await;
    row(&pool, 2, 7, owner, Some(1), Some(6)).await;
    row(&pool, 3, 8, other, Some(100), Some(1)).await;
    let store = HistoryStore::new(pool.clone());
    let query = HistoryQuery {
        page_size: Some(1),
        ..Default::default()
    };
    let result = store
        .read(&user, ReadRoute::SelfSession, &query)
        .await
        .unwrap();
    assert_eq!(result.total, 2);
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].source_log_id, 2);
    assert_eq!(result.items[0].quota, Some(1));
    assert_eq!(result.summary.raw_consumption_quota, "2");
    assert_eq!(result.summary.money.quota_usd.as_deref(), Some("0.5"));
    assert!(result.items[0].channel.is_none());
    let all = store
        .read(&admin, ReadRoute::Administrator, &HistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(all.total, 3);
    assert_eq!(all.summary.money.quota_usd.as_deref(), Some("100.5"));
    assert_eq!(
        store
            .read(&user, ReadRoute::Administrator, &query)
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    let cross = HistoryQuery {
        username: Some("other".into()),
        ..Default::default()
    };
    assert_eq!(
        store
            .read(&user, ReadRoute::SelfSession, &cross)
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
}
#[sqlx::test(migrations = false)]
async fn null_negative_missing_basis_and_empty_are_distinct_without_any_writes(pool: PgPool) {
    let (user, _, owner, _) = fixture(&pool).await;
    let store = HistoryStore::new(pool.clone());
    let empty = store
        .read(&user, ReadRoute::SelfSession, &HistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(empty.summary.money.quota_money_basis, "empty_result");
    assert_eq!(empty.summary.money.quota_usd.as_deref(), Some("0"));
    row(&pool, 1, 7, owner, Some(3), Some(3)).await;
    row(&pool, 2, 7, owner, None, Some(3)).await;
    row(&pool, 3, 7, owner, Some(-1), Some(3)).await;
    row(&pool, 4, 7, owner, Some(5), None).await;
    let before: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_journals),(SELECT count(*) FROM core_identity.audit),(SELECT count(*) FROM core_history.log_records)").fetch_one(&pool).await.unwrap();
    let result = store
        .read(&user, ReadRoute::SelfSession, &HistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(result.summary.unpriced_quota_records, 3);
    assert_eq!(result.summary.invalid_quota_records, 1);
    assert_eq!(result.summary.raw_consumption_quota, "7");
    assert!(!result.summary.money.quota_money_available);
    assert!(result.summary.money.quota_usd.is_none());
    let after: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_journals),(SELECT count(*) FROM core_identity.audit),(SELECT count(*) FROM core_history.log_records)").fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
    sqlx::query("UPDATE core_history.sources SET complete=false")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store
            .read(&user, ReadRoute::SelfSession, &HistoryQuery::default())
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}
#[sqlx::test(migrations = false)]
async fn anchor_limit_and_record_limit_do_not_expose_a_partial_total(pool: PgPool) {
    let (user, _, owner, _) = fixture(&pool).await;
    for id in 1..=65 {
        row(&pool, id, 7, owner, Some(1), Some(id)).await;
    }
    let store = HistoryStore::new(pool.clone());
    let result = store
        .read(&user, ReadRoute::SelfSession, &HistoryQuery::default())
        .await
        .unwrap();
    assert!(result.summary.quota_projection_truncated);
    assert!(!result.summary.money.quota_money_available);
    sqlx::query("UPDATE core_history.log_records SET pricing_credits_per_usd=3")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,pricing_credits_per_usd) SELECT 1,n,'dense-'||n,repeat('d',64),7,$1,1700000000,2,1,3 FROM generate_series(66,10001) n").bind(owner).execute(&pool).await.unwrap();
    let result = store
        .read(&user, ReadRoute::SelfSession, &HistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(result.total, 10001);
    assert_eq!(result.items.len(), 20);
    assert!(result.summary.quota_projection_truncated);
    assert!(result.summary.money.quota_usd.is_none());
}
#[sqlx::test(migrations = false)]
async fn concurrent_owner_changes_cannot_split_count_page_and_money_snapshots(pool: PgPool) {
    let (user, _, owner, other) = fixture(&pool).await;
    row(&pool, 1, 7, owner, Some(3), Some(3)).await;
    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        for n in 0..40 {
            let (uid, aid) = if n % 2 == 0 { (8, other) } else { (7, owner) };
            sqlx::query("UPDATE core_history.log_records SET user_id=$1,owner_account_id=$2")
                .bind(uid)
                .bind(aid)
                .execute(&writer_pool)
                .await
                .unwrap();
            tokio::task::yield_now().await;
        }
    });
    history_scope::install(&pool).await;
    let store = HistoryStore::new(pool);
    for _ in 0..40 {
        let result = store
            .read(&user, ReadRoute::SelfSession, &HistoryQuery::default())
            .await
            .unwrap();
        assert_eq!(result.total as usize, result.items.len());
        assert_eq!(
            result.summary.money.quota_usd.as_deref(),
            Some(if result.total == 0 { "0" } else { "1" })
        );
    }
    writer.await.unwrap();
}
