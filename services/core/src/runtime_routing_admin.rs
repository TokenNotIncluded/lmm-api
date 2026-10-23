//! The online publisher can call only five authority-checking entry points.
//! Preview and operation rows, validation helpers and ACL delegation stay private.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    // Resolve through catalog OIDs so a missing optional component is harmless.
    // Effective privileges include inherited grants and privileges from PUBLIC.
    let (unsafe_privileges, complete): (bool, bool) = sqlx::query_as(
        r#"
WITH namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='core_routing_admin'
), expected_functions(name,arguments,result) AS (
    VALUES ('read_active','','jsonb'),
           ('store_preview','bytea, text, text, text','bigint'),
           ('read_preview','bytea, text','text'),
           ('read_operation','bytea, text','text'),
           ('publish_preview','bytea, text, text, text','boolean')
), functions AS (
    SELECT p.oid,e.name AS expected_name
    FROM pg_catalog.pg_proc p JOIN namespace n ON n.oid=p.pronamespace
    LEFT JOIN expected_functions e ON e.name=p.proname
        AND e.arguments=pg_catalog.oidvectortypes(p.proargtypes)
        AND e.result=pg_catalog.format_type(p.prorettype,NULL)
        AND p.prokind='f' AND p.prosecdef AND NOT p.proretset
        AND p.provariadic=0 AND p.pronargdefaults=0
        AND 'search_path=pg_catalog'=ANY(COALESCE(p.proconfig,'{}'::text[]))
)
SELECT (
    EXISTS(SELECT FROM namespace n WHERE
        pg_catalog.has_schema_privilege(n.oid,'CREATE,USAGE WITH GRANT OPTION'))
    OR EXISTS(
        SELECT FROM pg_catalog.pg_class c JOIN namespace n ON n.oid=c.relnamespace
        WHERE c.relkind IN ('r','p','v','m','f') AND (
            pg_catalog.has_table_privilege(c.oid,
                'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR pg_catalog.has_any_column_privilege(c.oid,'SELECT,INSERT,UPDATE,REFERENCES')))
    OR EXISTS(
        SELECT FROM pg_catalog.pg_class c JOIN namespace n ON n.oid=c.relnamespace
        WHERE CASE WHEN c.relkind='S'
            THEN pg_catalog.has_sequence_privilege(c.oid,'USAGE,SELECT,UPDATE')
            ELSE FALSE END)
    OR EXISTS(SELECT FROM functions f WHERE
        pg_catalog.has_function_privilege(f.oid,'EXECUTE WITH GRANT OPTION')
        OR (f.expected_name IS NULL AND pg_catalog.has_function_privilege(f.oid,'EXECUTE')))
) AS unsafe_privileges,
(
    COALESCE((SELECT pg_catalog.has_schema_privilege(n.oid,'USAGE')
        FROM namespace n),false)
    AND NOT EXISTS(
        SELECT FROM expected_functions e
        LEFT JOIN functions f ON f.expected_name=e.name
        WHERE NOT COALESCE(pg_catalog.has_function_privilege(f.oid,'EXECUTE'),false))
) AS complete
"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required && !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn checked(pool: &PgPool, required: bool) -> Result<(), RuntimeDatabaseError> {
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await
            .unwrap();
        let result = check(&mut transaction, required).await;
        transaction.rollback().await.unwrap();
        result
    }

    #[sqlx::test]
    async fn every_installer_entry_point_is_required_without_writable_probes(owner: PgPool) {
        crate::core_schema::init(&owner).await.unwrap();
        let runtime = crate::test_runtime::RuntimeRole::create(&owner).await;
        assert_eq!(checked(&runtime.pool, true).await, Ok(()));
        let mut read = runtime.pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *read)
            .await
            .unwrap();
        let active: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT core_routing_admin.read_active()")
                .fetch_one(&mut *read)
                .await
                .unwrap();
        assert!(active.is_none());
        read.rollback().await.unwrap();

        for (privilege, object) in [
            ("USAGE", "SCHEMA core_routing_admin"),
            ("EXECUTE", "FUNCTION core_routing_admin.read_active()"),
            (
                "EXECUTE",
                "FUNCTION core_routing_admin.store_preview(bytea,text,text,text)",
            ),
            (
                "EXECUTE",
                "FUNCTION core_routing_admin.read_preview(bytea,text)",
            ),
            (
                "EXECUTE",
                "FUNCTION core_routing_admin.read_operation(bytea,text)",
            ),
            (
                "EXECUTE",
                "FUNCTION core_routing_admin.publish_preview(bytea,text,text,text)",
            ),
        ] {
            sqlx::raw_sql(&format!(
                "REVOKE {privilege} ON {object} FROM {}",
                runtime.name
            ))
            .execute(&owner)
            .await
            .unwrap();
            assert_eq!(
                checked(&runtime.pool, true).await,
                Err(RuntimeDatabaseError::MissingPrivileges),
                "{privilege} ON {object}"
            );
            assert_eq!(checked(&runtime.pool, false).await, Ok(()));
            sqlx::raw_sql(&format!(
                "GRANT {privilege} ON {object} TO {}",
                runtime.name
            ))
            .execute(&owner)
            .await
            .unwrap();
        }
        runtime.cleanup().await;
    }

    #[sqlx::test]
    async fn direct_rows_helpers_and_delegation_are_rejected_even_when_disabled(owner: PgPool) {
        crate::core_schema::init(&owner).await.unwrap();
        let runtime = crate::test_runtime::RuntimeRole::create(&owner).await;
        for (privilege, object) in [
            ("SELECT", "TABLE core_routing_admin.previews"),
            ("INSERT", "TABLE core_routing_admin.previews"),
            ("UPDATE", "TABLE core_routing_admin.operations"),
            ("DELETE", "TABLE core_routing_admin.operations"),
            ("SELECT(payload)", "TABLE core_routing_admin.previews"),
            ("UPDATE(payload)", "TABLE core_routing_admin.previews"),
            ("EXECUTE", "FUNCTION core_routing_admin.actor(bytea,text)"),
            ("EXECUTE", "FUNCTION core_routing_admin.check_plan(jsonb)"),
            ("CREATE", "SCHEMA core_routing_admin"),
        ] {
            sqlx::raw_sql(&format!(
                "GRANT {privilege} ON {object} TO {}",
                runtime.name
            ))
            .execute(&owner)
            .await
            .unwrap();
            assert_eq!(
                checked(&runtime.pool, false).await,
                Err(RuntimeDatabaseError::UnsafePrivileges),
                "{privilege} ON {object}"
            );
            sqlx::raw_sql(&format!(
                "REVOKE {privilege} ON {object} FROM {}",
                runtime.name
            ))
            .execute(&owner)
            .await
            .unwrap();
        }
        for (privilege, object) in [
            ("USAGE", "SCHEMA core_routing_admin"),
            ("EXECUTE", "FUNCTION core_routing_admin.read_active()"),
        ] {
            sqlx::raw_sql(&format!(
                "GRANT {privilege} ON {object} TO {} WITH GRANT OPTION",
                runtime.name
            ))
            .execute(&owner)
            .await
            .unwrap();
            assert_eq!(
                checked(&runtime.pool, false).await,
                Err(RuntimeDatabaseError::UnsafePrivileges)
            );
            sqlx::raw_sql(&format!(
                "REVOKE GRANT OPTION FOR {privilege} ON {object} FROM {}",
                runtime.name
            ))
            .execute(&owner)
            .await
            .unwrap();
        }
        sqlx::raw_sql("GRANT EXECUTE ON FUNCTION core_routing_admin.check_plan(jsonb) TO PUBLIC")
            .execute(&owner)
            .await
            .unwrap();
        assert_eq!(
            checked(&runtime.pool, false).await,
            Err(RuntimeDatabaseError::UnsafePrivileges)
        );
        sqlx::raw_sql(
            "REVOKE EXECUTE ON FUNCTION core_routing_admin.check_plan(jsonb) FROM PUBLIC",
        )
        .execute(&owner)
        .await
        .unwrap();
        assert_eq!(checked(&runtime.pool, true).await, Ok(()));
        runtime.cleanup().await;
    }
}
