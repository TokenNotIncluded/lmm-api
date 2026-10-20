//! Real PostgreSQL checks for the host assembly and retained consent lifecycle.
use lmm_core::{
    core_schema,
    humangrant::{self, HumanGrantEngine, HumanGrantStore, types::*},
};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn host_open_requires_existing_schema_and_seed(pool: PgPool) {
    assert!(
        humangrant::runtime::open(pool.clone(), "https://auth.example.test", false)
            .await
            .is_err()
    );
    core_schema::init(&pool).await.unwrap();
    // Kept until the fresh-install component is integrated into core_schema.
    sqlx::raw_sql(humangrant::DDL).execute(&pool).await.unwrap();
    assert!(
        humangrant::runtime::open(pool.clone(), "https://auth.example.test", false)
            .await
            .is_ok()
    );
    assert!(
        humangrant::runtime::open(pool.clone(), "http://public.example.test", true)
            .await
            .is_err()
    );
    assert!(
        humangrant::runtime::open(pool.clone(), "http://127.0.0.1:8765", true)
            .await
            .is_ok()
    );
    sqlx::query("DELETE FROM core_humangrant.human_grant_gate")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        humangrant::runtime::open(pool, "https://auth.example.test", false)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
async fn consent_rejects_retained_terminal_expired_and_old_epoch_requests(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    sqlx::raw_sql(humangrant::DDL).execute(&pool).await.unwrap();
    let engine = HumanGrantEngine::new(
        HumanGrantStore::new(pool),
        "https://auth.example.test".into(),
        false,
    );
    let now = chrono::Utc::now().timestamp();
    let mut request = Request {
        id: "request".into(),
        link: "link".into(),
        actor: "actor".into(),
        public_key: "key".into(),
        input: CreateRequest {
            client_id: CLIENT_ID.into(),
            target_service: SERVICE_ID.into(),
            purpose: "read profile".into(),
            requested_permissions: vec![Permission {
                action: PROFILE_READ.into(),
                resource: "1".into(),
            }],
            expected_human_id: None,
            session_ttl_seconds: 600,
        },
        epoch: 1,
        created: now,
        updated: now,
        expires: now + 600,
        phase: "waiting_human".into(),
        poll_after: 5,
        last_poll: 0,
        email_status: "not_requested".into(),
        last_email: 0,
        grant_id: None,
    };
    let mut tx = engine.store.begin().await.unwrap();
    assert!(
        engine
            .check_pending_request(&mut tx, &request, now)
            .await
            .is_ok()
    );
    for phase in ["approved", "succeeded", "rejected", "cancelled", "expired"] {
        request.phase = phase.into();
        assert_eq!(
            engine
                .check_pending_request(&mut tx, &request, now)
                .await
                .unwrap_err()
                .error,
            "invalid_state"
        );
    }
    request.phase = "awaiting_consent".into();
    request.expires = now;
    assert_eq!(
        engine
            .check_pending_request(&mut tx, &request, now)
            .await
            .unwrap_err()
            .error,
        "expired"
    );
    request.expires = now + 600;
    request.epoch = 0;
    assert_eq!(
        engine
            .check_pending_request(&mut tx, &request, now)
            .await
            .unwrap_err()
            .error,
        "feature_disabled"
    );
    tx.rollback().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn concurrent_limits_cannot_lose_increments(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    sqlx::raw_sql(humangrant::DDL).execute(&pool).await.unwrap();
    let engine = std::sync::Arc::new(HumanGrantEngine::new(
        HumanGrantStore::new(pool),
        "https://auth.example.test".into(),
        false,
    ));
    let now = chrono::Utc::now().timestamp();
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let engine = engine.clone();
        tasks.push(tokio::spawn(async move {
            let mut tx = engine.store.begin().await.unwrap();
            let allowed = engine
                .check_limits(&mut tx, &[("concurrent", 60, 1)], now)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            allowed
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        accepted += usize::from(task.await.unwrap());
    }
    assert_eq!(accepted, 1);
}
