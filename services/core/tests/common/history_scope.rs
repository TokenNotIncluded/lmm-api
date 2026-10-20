//! Explicit complete synthetic mapping receipts, including known zero-row scopes.
//! Production receipts come only from the reviewed offline history importer.
use sqlx::PgPool;

pub async fn install(pool: &PgPool) {
    sqlx::query(r#"INSERT INTO core_history.scope_receipts(source_id,user_id,owner_account_id,token_id,source_archive_sha256,mapping_sha256,source_scope_sha256,complete)
      SELECT s.source_id,m.user_id,m.owner_account_id,m.token_id,s.source_archive_sha256,s.mapping_sha256,
        encode(sha256(convert_to(jsonb_build_object('synthetic_source',s.source_id,'user_id',m.user_id,'owner_account_id',m.owner_account_id,'token_id',m.token_id)::text,'UTF8')),'hex'),true
      FROM core_history.sources s CROSS JOIN (
        SELECT id AS user_id,personal_account_id AS owner_account_id,NULL::bigint AS token_id FROM core_identity.users
        UNION SELECT user_id,owner_account_id,NULL::bigint FROM core_identity.credentials WHERE kind='api_key'
        UNION SELECT user_id,owner_account_id,id FROM core_identity.credentials WHERE kind='api_key'
      ) m ON CONFLICT DO NOTHING"#).execute(pool).await.unwrap();
}
