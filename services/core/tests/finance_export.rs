use lmm_core::{
    core_schema,
    finance_export::{self, ExportFormat},
    history_read::{HistoryError, HistoryQuery, HistoryStore},
    identity::IdentityStore,
};
use sqlx::PgPool;
#[sqlx::test(migrations = false)]
async fn export_uses_admin_gate_allowlisted_fields_and_unknown_instead_of_zero(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("history_read/source.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'synthetic-only',1700000000,true)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,username,model_name) SELECT 1,1,'synthetic',repeat('c',64),id,personal_account_id,1700000000,2,100,'=HYPERLINK(\"https://invalid.test\")','a,b' FROM core_identity.users WHERE id=7").execute(&pool).await.unwrap();
    let store = HistoryStore::new(pool.clone());
    assert!(matches!(
        finance_export::export(
            &store,
            &user.secret,
            &HistoryQuery::default(),
            ExportFormat::Json
        )
        .await,
        Err(HistoryError::Forbidden)
    ));
    let result = finance_export::export(
        &store,
        &admin.secret,
        &HistoryQuery::default(),
        ExportFormat::Json,
    )
    .await
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&result.bytes).unwrap();
    assert_eq!(value["complete_financial_export"], false);
    assert_eq!(value["usage"][0]["quota"], 100);
    assert_eq!(value["usage"][0]["quota_money_available"], false);
    assert!(value["usage"][0].get("quota_usd").is_none());
    for forbidden in [
        "other",
        "source_row_sha256",
        "mapping_sha256",
        "source_archive_sha256",
        "password",
        "secret",
        "ip",
        "provider_payload",
    ] {
        assert!(value["usage"][0].get(forbidden).is_none(), "{forbidden}");
    }
    let csv = finance_export::export(
        &store,
        &admin.secret,
        &HistoryQuery::default(),
        ExportFormat::Csv,
    )
    .await
    .unwrap();
    let csv = String::from_utf8(csv.bytes).unwrap();
    assert!(csv.contains("\"'=HYPERLINK(\"\"https://invalid.test\"\")\""));
    assert!(csv.contains("\"a,b\""));
    assert!(!csv.contains("source_archive_sha256"));
}
