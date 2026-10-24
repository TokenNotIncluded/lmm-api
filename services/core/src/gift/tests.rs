use super::*;
use crate::test_runtime as runtime;
#[test]
fn canonical_identity_and_strict_tool_decision() {
    assert_eq!(
        canonical_email(" A.B+c@googlemail.com ").as_deref(),
        Some("ab@gmail.com")
    );
    assert_eq!(
        canonical_email("A.B+c@outlook.com").as_deref(),
        Some("a.b@outlook.com")
    );
    assert_eq!(
        canonical_email("A.B+c@example.org").as_deref(),
        Some("a.b+c@example.org")
    );
    for v in ["a@b@c", "@a", "a@", "a b@x"] {
        assert!(canonical_email(v).is_none())
    }
    assert!(
        serde_json::from_str::<GiftDecision>(r#"{"amount_credits":1,"reason":"hi","owner":9}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<GiftDecision>(r#"{"amount_credits":1.5,"reason":"hi"}"#).is_err()
    );
}
#[test]
fn bounded_fingerprints_match_registration_policy_semantics() {
    for s in [
        "hi",
        "我就是想用QQ邮箱注册",
        "名字随便起的",
        "嗯，先等等",
        "复制好了，谢谢",
    ] {
        assert!(fingerprints(b"secret", s).unwrap().1.is_empty())
    }
    let text = "I need help connecting my existing desktop application to this service and running a minimal request with the correct endpoint.";
    let a = fingerprints(b"secret", text).unwrap();
    let b = fingerprints(b"secret", &format!("{text}!!!")).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.1.len(), 8);
    assert_ne!(a, fingerprints(b"different", text).unwrap());
    assert!(fingerprints(b"", text).is_err());
    let secret = "sk-thisisaverylongprivatesecretwhichmustneverbecomeacorrelationtoken";
    assert!(fingerprints(b"secret", secret).unwrap().1.is_empty());
}
async fn fixture(pool: &PgPool) -> (String, i64, String) {
    crate::core_schema::init(pool).await.unwrap();
    let identity = identity::IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(101, 0).await.unwrap().secret;
    let a = identity.authorize(&session).await.unwrap();
    let cv = format!("lmm_cv_{}", "a".repeat(64));
    sqlx::query("INSERT INTO core_assistant.conversations(id,owner_user_id,owner_account_id,creation_key,created_at) VALUES($1,101,$2,'gift-test',1)").bind(&cv).bind(a.owner_account_id).execute(pool).await.unwrap();
    sqlx::query("UPDATE core_gift.policy SET enabled=TRUE,exclusive_cap=1000")
        .execute(pool)
        .await
        .unwrap();
    (session, a.owner_account_id, cv)
}
async fn qualify(pool: &PgPool) {
    sqlx::query("INSERT INTO core_gift.qualifications(user_id,email_hash,network_hash,evidence_digest,evidence_kind,verified_at,registration_hold,reward_used) VALUES(101,$1,$2,$3,'owner_reviewed_registration_v1',clock_timestamp(),FALSE,FALSE)").bind(vec![1u8;32]).bind(vec![2u8;32]).bind(vec![3u8;32]).execute(pool).await.unwrap();
}
#[sqlx::test]
async fn unverified_profile_is_never_qualification(pool: PgPool) {
    let (_, _, cv) = fixture(&pool).await;
    sqlx::query("UPDATE core_identity.user_profiles SET email='verified-looking@gmail.com' WHERE user_id=101").execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let _ = cv;
    assert_eq!(
        eligibility(&mut tx, 101, 100, false).await.unwrap().1,
        "qualification_missing"
    );
    tx.rollback().await.unwrap();
}
#[sqlx::test]
async fn missing_offer_cannot_be_created_by_claim(pool: PgPool) {
    let (session, _, _) = fixture(&pool).await;
    qualify(&pool).await;
    assert_eq!(
        GiftStore::new(pool.clone()).claim_stored(&session).await,
        Err(Error::NotFound)
    );
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE kind='gift'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
}
#[sqlx::test]
async fn zero_and_exclusive_cap_never_consume_identity(pool: PgPool) {
    let (_, _, cv) = fixture(&pool).await;
    qualify(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        execute_in_transaction(
            &mut tx,
            101,
            &cv,
            "unused-zero-decision",
            GiftDecision {
                amount_credits: 0,
                reason: "later".into()
            }
        )
        .await
        .unwrap(),
        None
    );
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        eligibility(&mut tx, 101, 1000, false).await.unwrap().1,
        "cap_changed"
    );
    tx.rollback().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM core_gift.decisions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
#[sqlx::test]
async fn held_or_rewarded_registration_rejected(pool: PgPool) {
    fixture(&pool).await;
    qualify(&pool).await;
    for (column, code) in [
        ("registration_hold", "registration_hold"),
        ("reward_used", "reward_used"),
    ] {
        sqlx::query(&format!(
            "UPDATE core_gift.qualifications SET {column}=TRUE WHERE user_id=101"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let mut tx = pool.begin().await.unwrap();
        assert_eq!(eligibility(&mut tx, 101, 100, false).await.unwrap().1, code);
        tx.rollback().await.unwrap();
        sqlx::query(&format!(
            "UPDATE core_gift.qualifications SET {column}=FALSE WHERE user_id=101"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
}
#[sqlx::test]
async fn eligibility_locks_work_without_runtime_fact_update_privileges(pool: PgPool) {
    let (session, _, _) = fixture(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_gift").await;
    for table in [
        "core_gift.policy",
        "core_gift.qualifications",
        "core_abuse.registration_observations",
    ] {
        let can_update:bool=sqlx::query_scalar("SELECT has_table_privilege(current_user,$1,'UPDATE') OR has_any_column_privilege(current_user,$1,'UPDATE')").bind(table).fetch_one(&role.pool).await.unwrap();
        assert!(!can_update, "{table}");
    }
    let mut tx = role.pool.begin().await.unwrap();
    sqlx::query("SELECT core_gift.lock_eligibility(101)")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        eligibility(&mut tx, 101, 100, true).await.unwrap().1,
        "qualification_missing"
    );
    tx.commit().await.unwrap();
    assert_eq!(
        GiftStore::new(role.pool.clone())
            .get(&session)
            .await
            .unwrap(),
        None
    );
    role.cleanup().await;
}
