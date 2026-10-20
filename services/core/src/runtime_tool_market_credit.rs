//! Read-only verification of the canonical CREDIT v2 runtime authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let unsafe_:bool=sqlx::query_scalar(r#"
 SELECT EXISTS(SELECT FROM pg_class t JOIN pg_namespace n ON n.oid=t.relnamespace
 WHERE n.nspname='core_tool_market' AND t.relname LIKE 'credit\_%' ESCAPE '\' AND (
 has_table_privilege(t.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege(t.oid,'REFERENCES')
 OR (t.relname NOT IN('credit_quotes','credit_consents','credit_budgets','credit_tokens','credit_access_operations','credit_calls','credit_services','credit_supplier_receipts','credit_fixed_witness_receipts') AND (has_table_privilege(t.oid,'INSERT') OR has_any_column_privilege(t.oid,'INSERT')))
 OR (t.relname NOT IN('credit_policy','credit_quotes','credit_consents','credit_budgets','credit_tokens','credit_calls','credit_services') AND (has_table_privilege(t.oid,'UPDATE') OR has_any_column_privilege(t.oid,'UPDATE')))))
 OR EXISTS(SELECT FROM pg_attribute a JOIN pg_class t ON t.oid=a.attrelid JOIN pg_namespace n ON n.oid=t.relnamespace
 WHERE n.nspname='core_tool_market' AND a.attnum>0 AND NOT a.attisdropped AND has_column_privilege(t.oid,a.attname,'UPDATE') AND (
 (t.relname='credit_policy' AND a.attname NOT IN('version','platform_account_id','platform_user_id','enabled','fee_bps','updated_by','updated_at'))
 OR (t.relname='credit_quotes' AND a.attname NOT IN('approved','suspended','visibility','approved_by','approved_at'))
 OR (t.relname='credit_consents' AND a.attname NOT IN('revoked_at','reserved_calls','successful_calls','reserved_credits','spent_credits'))
 OR (t.relname='credit_budgets' AND a.attname NOT IN('request','limit_credits','reserved_credits','spent_credits'))
 OR (t.relname='credit_tokens' AND a.attname<>'revoked_at')
 OR (t.relname='credit_calls' AND a.attname NOT IN('state','dispatch_id','result_digest','usage','receipt','settlement_journal_id','started_at','finished_at'))
 OR (t.relname='credit_services' AND a.attname NOT IN('active_version_id','paused','deleted','suspended','revision','updated_by','updated_at'))))
 OR EXISTS(SELECT FROM pg_attribute a JOIN pg_class t ON t.oid=a.attrelid JOIN pg_namespace n ON n.oid=t.relnamespace
 WHERE n.nspname='core_tool_market' AND t.relname='credit_quotes' AND a.attnum>0 AND NOT a.attisdropped
 AND a.attname NOT IN('id','seller_user_id','seller_account_id','platform_account_id','fee_bps','policy_version','snapshot','ceiling_credits') AND has_column_privilege(t.oid,a.attname,'INSERT'))
 "#).fetch_one(&mut *c).await?;
    if unsafe_ {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if !required {
        return Ok(());
    }
    // Resolve schema privileges by catalog OID before resolving qualified names.
    // PostgreSQL rejects to_regclass on a schema without USAGE with SQLSTATE 42501.
    let usage: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT has_schema_privilege(oid,'USAGE') FROM pg_namespace WHERE nspname='core_tool_market'),FALSE)",
    )
    .fetch_one(&mut *c)
    .await?;
    if !usage {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    let complete:bool=sqlx::query_scalar(r#"
 SELECT NOT EXISTS(SELECT FROM (VALUES('credit_policy'),('credit_quotes'),('credit_builtin_bindings'),('credit_result_contracts'),('credit_consents'),('credit_budgets'),('credit_tokens'),('credit_access_operations'),('credit_calls'),('credit_services'),('credit_supplier_receipts'),('credit_fixed_witness_receipts')) t(name)
 WHERE NOT COALESCE(has_table_privilege(to_regclass('core_tool_market.'||t.name),'SELECT'),FALSE))
 AND NOT EXISTS(SELECT FROM (VALUES('credit_consents'),('credit_budgets'),('credit_tokens'),('credit_access_operations'),('credit_calls'),('credit_services'),('credit_supplier_receipts'),('credit_fixed_witness_receipts')) t(name)
 WHERE NOT COALESCE(has_table_privilege(to_regclass('core_tool_market.'||t.name),'INSERT'),FALSE))
 AND NOT EXISTS(SELECT FROM (VALUES
 ('credit_policy','version'),('credit_policy','platform_account_id'),('credit_policy','platform_user_id'),('credit_policy','enabled'),('credit_policy','fee_bps'),('credit_policy','updated_by'),('credit_policy','updated_at'),
 ('credit_quotes','approved'),('credit_quotes','suspended'),('credit_quotes','visibility'),('credit_quotes','approved_by'),('credit_quotes','approved_at'),
 ('credit_consents','revoked_at'),('credit_consents','reserved_calls'),('credit_consents','successful_calls'),('credit_consents','reserved_credits'),('credit_consents','spent_credits'),
 ('credit_budgets','request'),('credit_budgets','limit_credits'),('credit_budgets','reserved_credits'),('credit_budgets','spent_credits'),('credit_tokens','revoked_at'),
 ('credit_calls','state'),('credit_calls','dispatch_id'),('credit_calls','result_digest'),('credit_calls','usage'),('credit_calls','receipt'),('credit_calls','settlement_journal_id'),('credit_calls','started_at'),('credit_calls','finished_at'),
 ('credit_services','active_version_id'),('credit_services','paused'),('credit_services','deleted'),('credit_services','suspended'),('credit_services','revision'),('credit_services','updated_by'),('credit_services','updated_at')) a(t,c)
 WHERE NOT COALESCE(has_column_privilege(to_regclass('core_tool_market.'||a.t),a.c,'UPDATE'),FALSE))
 AND NOT EXISTS(SELECT FROM (VALUES('id'),('seller_user_id'),('seller_account_id'),('platform_account_id'),('fee_bps'),('policy_version'),('snapshot'),('ceiling_credits')) a(c)
 WHERE NOT COALESCE(has_column_privilege(to_regclass('core_tool_market.credit_quotes'),a.c,'INSERT'),FALSE))
 AND COALESCE(has_function_privilege(to_regprocedure('core_tool_market.lock_builtin_context(text)'),'EXECUTE'),FALSE)
 "#).fetch_one(&mut *c).await?;
    if !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}
