//! PostgreSQL acceptance for evidence-only historical pending liabilities.
//! These rows never touch a ledger account or grant spendable units.

use lmm_core::identity::IdentityStore;
use serde_json::{Value, json};
use sqlx::{PgPool, Row, types::Json};

fn request(user_id: i64, account_id: i64) -> Value {
    json!({
        "migration_id": "pending-liability-test",
        "source_archive_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "mapping_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "source_table": "legacy.top_ups",
        "source_row_key": "topup-11",
        "source_row_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "source_parent_plan_sha256": null,
        "owner_user_id": user_id,
        "owner_account_id": account_id,
        "status": "pending",
        "settlement_state": "unknown",
        "payload_json": {"provider": "epay", "status": "pending"},
        "amount_raw_json": {"money": "1.23"},
        "currency_raw_json": {"currency": null},
        "spendable_units": 0,
        "spendable": false,
        "observed_at": 1_700_000_000_i64,
        "request": {"source": "synthetic"},
        "result": {"status": "imported", "spendable": false}
    })
}

async fn owner_import(pool: &PgPool, value: &Value) -> Value {
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await
        .unwrap();
    let Json(result): Json<Value> =
        sqlx::query_scalar("SELECT core_billing.import_legacy_pending_liability($1)")
            .bind(Json(value.clone()))
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    result
}

#[sqlx::test(migrations = false)]
async fn pending_liabilities_are_owner_only_append_only_and_non_spendable(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let credential = identity.bootstrap_user(9001, 1).await.unwrap();
    let principal = identity.authorize(&credential.secret).await.unwrap();
    let body = request(principal.user_id, principal.owner_account_id);

    let first = owner_import(&pool, &body).await;
    assert_eq!(first["status"], "imported");
    assert_eq!(first["spendable"], false);
    assert_eq!(first["spendable_units"], 0);

    let replay = owner_import(&pool, &body).await;
    assert_eq!(replay, first);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.legacy_pending_liabilities")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);

    let mut conflict = body.clone();
    conflict["source_row_sha256"] =
        Value::String("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into());
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = sqlx::query_scalar::<_, Json<Value>>(
        "SELECT core_billing.import_legacy_pending_liability($1)",
    )
    .bind(Json(conflict))
    .fetch_one(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("P1001")
    );
    tx.rollback().await.unwrap();

    sqlx::query(
        "INSERT INTO core_billing.legacy_pending_liability_receipts
         (migration_id,source_table,source_row_key,evidence_sha256,observed_at,provider,provider_status,
          settlement_state,provider_payload_json,spendable_units,spendable,request,result)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,FALSE,$10,$11)",
    )
    .bind("pending-liability-test")
    .bind("legacy.top_ups")
    .bind("topup-11")
    .bind("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    .bind(1_700_000_001_i64)
    .bind("epay")
    .bind("pending")
    .bind("unknown")
    .bind(Json(json!({"status": "pending"})))
    .bind(Json(json!({"source": "synthetic"})))
    .bind(Json(json!({"status": "recorded", "spendable": false})))
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE core_billing.legacy_pending_liabilities SET status='failed'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM core_billing.legacy_pending_liabilities")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("TRUNCATE core_billing.legacy_pending_liabilities")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "UPDATE core_billing.legacy_pending_liability_receipts SET provider_status='failed'"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM core_billing.legacy_pending_liability_receipts")
            .execute(&pool)
            .await
            .is_err()
    );

    sqlx::query("DROP ROLE IF EXISTS liability_runtime_test")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE ROLE liability_runtime_test NOLOGIN")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("GRANT USAGE ON SCHEMA core_billing TO liability_runtime_test")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "GRANT SELECT ON core_billing.legacy_pending_liabilities TO liability_runtime_test",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    sqlx::query("SET ROLE liability_runtime_test")
        .execute(&mut *connection)
        .await
        .unwrap();
    let runtime_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.legacy_pending_liabilities")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
    assert_eq!(runtime_count, 1);
    let privileges = sqlx::query(
        "SELECT has_table_privilege(current_user,'core_billing.legacy_pending_liabilities','INSERT') AS insert_allowed,
                has_table_privilege(current_user,'core_billing.legacy_pending_liabilities','UPDATE') AS update_allowed,
                has_table_privilege(current_user,'core_billing.legacy_pending_liabilities','DELETE') AS delete_allowed,
                has_function_privilege(current_user,'core_billing.import_legacy_pending_liability(jsonb)','EXECUTE') AS function_allowed",
    )
    .fetch_one(&mut *connection)
    .await
    .unwrap();
    assert!(!privileges.get::<bool, _>("insert_allowed"));
    assert!(!privileges.get::<bool, _>("update_allowed"));
    assert!(!privileges.get::<bool, _>("delete_allowed"));
    assert!(!privileges.get::<bool, _>("function_allowed"));
}
