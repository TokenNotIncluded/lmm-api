use sqlx::{PgPool, Row};
use std::collections::BTreeMap;

/// Complete rows and sequence state in this SQLx-owned synthetic database.
/// Counts alone miss updates and nextval is not rolled back with table writes.
pub async fn snapshot(pool: &PgPool) -> BTreeMap<String, Vec<String>> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    let relations = sqlx::query(
        "SELECT n.nspname,c.relname,c.relkind::text AS kind FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public') AND c.relkind IN ('r','p','S') ORDER BY n.nspname,c.relname",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    let mut result = BTreeMap::new();
    for relation in relations {
        let schema: String = relation.get("nspname");
        let name: String = relation.get("relname");
        let kind: String = relation.get("kind");
        let quoted = format!(
            "\"{}\".\"{}\"",
            schema.replace('"', "\"\""),
            name.replace('"', "\"\"")
        );
        let sql = if kind == "S" {
            format!("SELECT json_build_array(last_value,log_cnt,is_called)::text FROM {quoted}")
        } else {
            format!(
                "SELECT to_jsonb(t)::text FROM {quoted} t ORDER BY to_jsonb(t)::text COLLATE \"C\""
            )
        };
        let rows: Vec<String> = sqlx::query_scalar(&sql).fetch_all(&mut *tx).await.unwrap();
        result.insert(format!("{kind}:{schema}.{name}"), rows);
    }
    let sequences: Vec<String> = sqlx::query_scalar(
        "SELECT to_jsonb(s)::text FROM pg_sequence s JOIN pg_class c ON c.oid=s.seqrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public' ORDER BY n.nspname,c.relname",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    result.insert("sequence-definitions".into(), sequences);
    tx.commit().await.unwrap();
    result
}
