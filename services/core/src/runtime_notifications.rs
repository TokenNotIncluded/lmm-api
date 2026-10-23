//! Every Core ledger writer can read site policy and append notification facts.
//! The optional worker still requires the separate full event-consumer grants.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(connection: &mut PgConnection) -> Result<(), RuntimeDatabaseError> {
    // Use catalogue OIDs so an absent schema/table is MissingPrivileges, never
    // a lookup exception that could be mistaken for notifications being off.
    let complete: bool = sqlx::query_scalar(
        r#"
WITH expected_tables(schema_name,table_name,privilege) AS (
    VALUES ('core_console','options','SELECT'),
           ('core_events','capacity','SELECT'),
           ('core_events','subscriptions','SELECT'),
           ('core_events','outbox','SELECT'),
           ('core_events','outbox','INSERT'),
           ('core_events','deliveries','INSERT'),
           ('core_events','inbox','SELECT'),
           ('core_events','inbox','INSERT')
), expected_columns(table_name,column_name) AS (
    VALUES ('capacity','pending_events'),('capacity','pending_bytes'),
           ('subscriptions','pending'),('inbox','response'),('inbox','completed_at')
)
SELECT NOT EXISTS (
    SELECT FROM (VALUES ('core_console'),('core_events')) expected(name)
    LEFT JOIN pg_catalog.pg_namespace n ON n.nspname=expected.name
    WHERE NOT COALESCE(pg_catalog.has_schema_privilege(n.oid,'USAGE'),false)
)
AND NOT EXISTS (
    SELECT FROM expected_tables expected
    LEFT JOIN pg_catalog.pg_namespace n ON n.nspname=expected.schema_name
    LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
        AND c.relname=expected.table_name AND c.relkind='r'
    WHERE NOT COALESCE(pg_catalog.has_table_privilege(c.oid,expected.privilege),false)
)
AND NOT EXISTS (
    SELECT FROM expected_columns expected
    LEFT JOIN pg_catalog.pg_namespace n ON n.nspname='core_events'
    LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
        AND c.relname=expected.table_name AND c.relkind='r'
    LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid
        AND a.attname=expected.column_name AND a.attnum>0 AND NOT a.attisdropped
    WHERE a.attgenerated IS DISTINCT FROM ''
        OR NOT COALESCE(pg_catalog.has_column_privilege(c.oid,a.attnum,'UPDATE'),false)
)
AND COALESCE((
    SELECT pg_catalog.has_sequence_privilege(c.oid,'USAGE')
    FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    WHERE n.nspname='core_events' AND c.relname='outbox_id_seq' AND c.relkind='S'
),false)
"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}
