//! Read-only startup verification of the runtime role's database authority.
//! Install/import owners must never be used by the network-facing process.
use sqlx::{PgConnection, PgPool};
use std::fmt;
#[path = "runtime_tool_market_credit.rs"]
mod tool_market_credit;

#[derive(Clone, Copy, Default)]
pub struct RequiredCapabilities {
    pub models: bool,
    pub images: bool,
    pub events: bool,
    pub payments: bool,
    pub console: bool,
    pub commerce: bool,
    pub assistant: bool,
    pub assistant_tools: bool,
    pub tool_market: bool,
    pub web_identity: bool,
    pub oauth2: bool,
    pub market_oauth: bool,
    pub wallet: bool,
    pub bounties: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDatabaseError {
    UnsafePrivileges,
    MissingPrivileges,
    Unavailable,
}
impl fmt::Display for RuntimeDatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsafePrivileges => "core runtime database authority is unsafe",
            Self::MissingPrivileges => "core runtime database authority is incomplete",
            Self::Unavailable => "core runtime database authority could not be verified",
        })
    }
}
impl std::error::Error for RuntimeDatabaseError {}
impl From<sqlx::Error> for RuntimeDatabaseError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}

/// No DDL or probe writes: inspect effective privileges, including inherited and
/// SET ROLE paths. PostgreSQL 18 pg_has_role(...,'SET') accounts for membership
/// SET options rather than assuming every membership allows impersonation.
// @mark deployment.database-authority role=authority
pub async fn check(
    pool: &PgPool,
    required: RequiredCapabilities,
) -> Result<(), RuntimeDatabaseError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='1500ms'")
        .execute(&mut *transaction)
        .await?;
    check_connection(&mut transaction, required).await?;
    transaction.commit().await?;
    Ok(())
}

/// The caller must provide its read-only transaction. This is the same SQL
/// authority check used by the public startup wrapper, without another pool/TX.
pub(crate) async fn check_connection(
    connection: &mut PgConnection,
    required: RequiredCapabilities,
) -> Result<(), RuntimeDatabaseError> {
    let unsafe_role: bool = sqlx::query_scalar(r#"
        SELECT current_user<>session_user
        OR has_parameter_privilege(current_user,'session_replication_role','SET')
        OR has_parameter_privilege(session_user,'session_replication_role','SET')
        OR EXISTS(SELECT FROM pg_roles r WHERE r.rolname<>current_user AND (
          pg_has_role(current_user,r.oid,'SET') OR pg_has_role(session_user,r.oid,'SET')
          OR pg_has_role(current_user,r.oid,'MEMBER WITH ADMIN OPTION') OR pg_has_role(session_user,r.oid,'MEMBER WITH ADMIN OPTION')))
        OR EXISTS(SELECT FROM pg_roles r
          WHERE (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication
            OR r.rolname IN ('pg_read_server_files','pg_write_server_files','pg_execute_server_program','pg_read_all_data','pg_write_all_data'))
          AND (r.rolname IN (current_user,session_user) OR pg_has_role(current_user,r.oid,'USAGE') OR pg_has_role(session_user,r.oid,'SET')
            OR pg_has_role(current_user,r.oid,'MEMBER WITH ADMIN OPTION') OR pg_has_role(session_user,r.oid,'MEMBER WITH ADMIN OPTION')))
        OR has_database_privilege(current_database(),'CREATE,TEMP')
        OR EXISTS(SELECT FROM pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema')
          AND left(n.nspname,3)<>'pg_' AND has_schema_privilege(n.oid,'CREATE'))
        OR EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE c.relkind IN ('r','p','v','f') AND n.nspname NOT IN ('pg_catalog','information_schema')
            AND left(n.nspname,3)<>'pg_' AND has_table_privilege(c.oid,'TRIGGER'))
        OR EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
          WHERE p.prosecdef AND n.nspname NOT IN ('pg_catalog','information_schema')
            AND left(n.nspname,3)<>'pg_' AND (
              NOT ('search_path=pg_catalog'=ANY(coalesce(p.proconfig,'{}'::text[])))
              OR EXISTS(SELECT FROM aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a
                WHERE a.grantee=0 AND a.privilege_type='EXECUTE')))
        OR EXISTS(WITH owners AS (
          SELECT datdba AS owner FROM pg_database WHERE datname=current_database()
          UNION SELECT nspowner FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema') AND left(nspname,3)<>'pg_'
          UNION SELECT c.relowner FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
          UNION SELECT p.proowner FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
            WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
          UNION SELECT t.typowner FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace
            WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
        ) SELECT FROM owners o WHERE pg_has_role(current_user,o.owner,'USAGE') OR pg_has_role(session_user,o.owner,'SET')
          OR pg_has_role(current_user,o.owner,'MEMBER WITH ADMIN OPTION') OR pg_has_role(session_user,o.owner,'MEMBER WITH ADMIN OPTION'))
    "#).fetch_one(&mut *connection).await?;
    if unsafe_role {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }

    let forbidden_write: bool = sqlx::query_scalar(r#"
        SELECT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE c.relkind IN ('r','p','v','m','f') AND (
            ((n.nspname IN ('core_meta','core_migration','core_model','core_history')
              OR (n.nspname='core_console' AND c.relname IN ('options','initialization'))
              OR (n.nspname='core_identity' AND c.relname IN ('developer_access','user_routing_groups','privileged_import_facts','permission_policies','permission_policy_rows','admin_activation_receipts'))
              OR (n.nspname='public' AND c.relname IN ('routing_active_release','routing_groups','routing_routes'))
              OR (n.nspname='core_charging' AND c.relname IN ('budget_opening_consumption','subscription_opening_consumption'))
              OR (n.nspname='core_billing' AND c.relname<>'account_policies')
              OR (n.nspname='core_tool_market' AND c.relname='publishers')
              OR (n.nspname='core_gift' AND c.relname IN ('policy','risk_key'))
              OR (n.nspname='core_bounties' AND c.relname='policy'))
             AND (has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
               OR has_any_column_privilege(c.oid,'INSERT,UPDATE,REFERENCES')))
            OR ((n.nspname='core_console' AND c.relname='announcement_reads'
               OR n.nspname='core_payments' AND c.relname IN ('receipts','provider_events','provider_refunds','blocked_refunds','accepted_proofs','refund_dispatches','refund_dispatch_bindings')
               OR n.nspname='core_charging' AND c.relname IN ('budget_checks','usage_events','refunds','policy_events','ledger_commands')
               OR n.nspname='core_identity' AND c.relname IN ('key_funding_rules','credential_grants','oauth_identities','audit','web_session_credentials','session_authentication')
               OR n.nspname='core_commerce' AND c.relname='operations'
               OR n.nspname='core_bounties' AND c.relname='operations'
               OR n.nspname='core_tool_market' AND c.relname='quotes'
               OR n.nspname='core_assistant' AND c.relname='conversations'
               OR n.nspname='core_gift' AND c.relname='qualifications'
               OR n.nspname='core_abuse' AND c.relname IN ('previews','operations','audit','registration_observations'))
              AND (has_table_privilege(c.oid,'UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
                OR has_any_column_privilege(c.oid,'UPDATE,REFERENCES')))
            OR (n.nspname IN ('core_charging','core_events','core_commerce','core_payments','core_tool_market','core_assistant','core_image','core_wallet','core_bounties')
              AND has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'))
            OR (n.nspname='core_gift' AND (has_table_privilege(c.oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
              OR has_any_column_privilege(c.oid,'REFERENCES')
              OR (c.relname<>'fingerprints' AND has_table_privilege(c.oid,'DELETE'))
              OR (c.relname='fingerprints' AND (has_table_privilege(c.oid,'UPDATE') OR has_any_column_privilege(c.oid,'UPDATE')))))
            OR (n.nspname='core_abuse' AND (has_table_privilege(c.oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
              OR (c.relname NOT IN ('model_counters','model_leases') AND has_table_privilege(c.oid,'DELETE'))
              OR (c.relname='settings' AND has_table_privilege(c.oid,'INSERT'))))
            OR (n.nspname='core_oauth' AND has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'))
            OR (n.nspname='core_assistant_tools' AND (has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege(c.oid,'REFERENCES')))
            OR (n.nspname='core_market_oauth' AND has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'))
            OR (n.nspname='core_oauth' AND c.relname IN ('contract','codes')
              AND (has_table_privilege(c.oid,'INSERT,UPDATE') OR has_any_column_privilege(c.oid,'INSERT,UPDATE')))
            OR (n.nspname='core_identity' AND c.relname='api_key_settings'
              AND has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'))
            OR (n.nspname='core_identity' AND c.relname NOT IN ('oauth_flows','auth_attempts')
              AND has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'))
            OR (n.nspname='core_billing' AND c.relname='account_policies'
              AND has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'))
            OR (n.nspname='core_events' AND c.relname='capacity'
              AND (has_table_privilege(c.oid,'INSERT') OR has_any_column_privilege(c.oid,'INSERT')))))
        OR EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid
          JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE a.attnum>0 AND NOT a.attisdropped AND (
            (n.nspname='core_charging' AND c.relname='external_consumption' AND a.attname<>'refunded')
            OR (n.nspname='core_tool_market' AND c.relname='approvals' AND a.attname NOT IN ('revoked','used_calls','used_amount'))
            OR (n.nspname='core_tool_market' AND c.relname='calls' AND a.attname NOT IN ('receipt','result_hash'))
            OR (n.nspname='core_assistant_tools' AND a.attname NOT IN ('nonce_sha256','state','confirmed_at'))
            OR (n.nspname='core_assistant' AND c.relname='grants' AND a.attname<>'revoked')
            OR (n.nspname='core_assistant' AND c.relname='operations' AND a.attname NOT IN ('answer_text','completed_at','gift_receipt','gift_tool_id','gift_response_sha256'))
            OR (n.nspname='core_image' AND (c.relname<>'operations' OR a.attname NOT IN ('state','confirmed_at','confirmation_digest','confirmation_sid','charge_id','output','output_digest','image_usage','settled_credits')))
            OR (n.nspname='core_wallet' AND (c.relname<>'transfers' OR a.attname NOT IN ('state','hold_journal','final_journal','recipient_user','confirmed_at','finished_at','confirmation_digest','confirmation_sid','confirmation_kind','confirmation_expires')))
            OR (n.nspname='core_bounties' AND NOT ((c.relname='projects' AND a.attname='body') OR (c.relname='confirmations' AND a.attname IN ('approved','csrf','browser_sid'))))
            OR (n.nspname='core_gift' AND c.relname='decisions' AND a.attname NOT IN ('claimed_at','ledger_journal_id'))
            OR (n.nspname='core_gift' AND c.relname='registration_profiles' AND a.attname<>'observed_at')
            OR (n.nspname='core_commerce' AND c.relname='orders' AND a.attname NOT IN ('state','settlement_journal_id','revision','delivery_reference'))
            OR (n.nspname='core_commerce' AND c.relname='key_scopes' AND a.attname<>'can_spend')
            OR (n.nspname='core_events' AND c.relname='capacity' AND a.attname NOT IN ('pending_events','pending_bytes'))
            OR (n.nspname='core_abuse' AND c.relname='settings' AND a.attname NOT IN ('revision','document'))
            OR (n.nspname='core_abuse' AND c.relname='user_marks' AND a.attname NOT IN ('marked','policy_id','revision'))
            OR (n.nspname='core_abuse' AND c.relname='model_counters' AND a.attname<>'attempts')
            OR (n.nspname='core_abuse' AND c.relname='model_leases' AND a.attname<>'finished_at')
            OR (n.nspname='core_oauth' AND NOT (
              (c.relname='families' AND a.attname IN ('scopes','revoked_at'))
              OR (c.relname IN ('authorization_requests','authorization_codes','refresh_tokens') AND a.attname='consumed_at')))
            OR (n.nspname='core_market_oauth' AND NOT (
              (c.relname IN ('clients','families') AND a.attname='revoked_at')
              OR (c.relname IN ('requests','codes','tokens') AND a.attname='consumed_at'))))
          AND has_column_privilege(c.oid,a.attname,'UPDATE'))
        OR has_function_privilege('core_billing.import_opening_balance(jsonb)','EXECUTE')
        OR has_function_privilege('core_billing.import_account_opening_counters(jsonb)','EXECUTE')
        OR has_function_privilege('core_billing.import_legacy_escrow_batch(jsonb)','EXECUTE')
        OR EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE (n.nspname IN ('core_billing','core_history') AND CASE WHEN c.relkind='S'
            THEN has_sequence_privilege(c.oid,'USAGE,SELECT,UPDATE') ELSE FALSE END)
            OR (n.nspname IN ('core_assistant','core_gift','core_wallet') AND CASE WHEN c.relkind='S' THEN has_sequence_privilege(c.oid,'UPDATE') ELSE FALSE END))
    "#).fetch_one(&mut *connection).await?;
    if forbidden_write {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }

    tool_market_credit::check(connection, required.tool_market).await?;

    // Identity is always mounted when the runtime has a database. Normal identity
    // operations must work; unsupported optional modules need no extra grants.
    let identity_complete: bool = sqlx::query_scalar(r#"
        SELECT has_schema_privilege('core_identity','USAGE')
          AND has_schema_privilege('core_abuse','USAGE')
          AND NOT EXISTS(SELECT FROM (VALUES ('settings'),('user_marks'),('previews'),('operations'),('audit'),('model_counters'),('model_leases'),('registration_observations')) t(name)
            WHERE NOT has_table_privilege('core_abuse.'||t.name,'SELECT'))
          AND NOT EXISTS(SELECT FROM (VALUES ('user_marks'),('previews'),('operations'),('audit'),('model_counters'),('model_leases'),('registration_observations')) t(name)
            WHERE NOT has_table_privilege('core_abuse.'||t.name,'INSERT'))
          AND has_column_privilege('core_abuse.settings','revision','UPDATE')
          AND has_column_privilege('core_abuse.settings','document','UPDATE')
          AND has_column_privilege('core_abuse.user_marks','marked','UPDATE')
          AND has_column_privilege('core_abuse.user_marks','policy_id','UPDATE')
          AND has_column_privilege('core_abuse.user_marks','revision','UPDATE')
          AND has_column_privilege('core_abuse.model_counters','attempts','UPDATE')
          AND has_column_privilege('core_abuse.model_leases','finished_at','UPDATE')
          AND has_table_privilege('core_abuse.model_counters','DELETE')
          AND has_table_privilege('core_abuse.model_leases','DELETE')
          AND has_sequence_privilege('core_abuse.audit_id_seq','USAGE')
          AND has_schema_privilege('core_meta','USAGE')
          AND has_schema_privilege('core_billing','USAGE')
          AND has_schema_privilege('core_history','USAGE')
          AND has_table_privilege('core_history.sources','SELECT')
          AND has_table_privilege('core_history.log_records','SELECT')
          AND has_table_privilege('core_history.scope_receipts','SELECT')
          AND has_schema_privilege('core_gift','USAGE')
          AND NOT EXISTS(SELECT FROM (VALUES ('risk_key'),('policy'),('qualifications')) t(name)
            WHERE NOT has_table_privilege('core_gift.'||t.name,'SELECT'))
          AND has_table_privilege('core_gift.qualifications','INSERT')
          AND has_function_privilege('core_gift.lock_eligibility(bigint)','EXECUTE')
          AND NOT EXISTS(SELECT FROM (VALUES ('accounts'),('users'),('teams'),('memberships'),('credentials'),('invites'),('password_logins'),('user_profiles'),('company_billing_profiles'),('web_sessions'),('api_key_settings'),('mfa_factors'),('mfa_enrollments'),('mfa_backup_codes'),('mfa_challenges')) t(name)
            CROSS JOIN (VALUES ('SELECT'),('INSERT'),('UPDATE')) p(privilege)
            WHERE NOT has_table_privilege('core_identity.'||t.name,p.privilege))
          AND NOT EXISTS(SELECT FROM (VALUES ('key_funding_rules'),('credential_grants'),('oauth_identities'),('audit'),('web_session_credentials'),('session_authentication')) t(name)
            CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
            WHERE NOT has_table_privilege('core_identity.'||t.name,p.privilege))
          AND NOT EXISTS(SELECT FROM (VALUES ('oauth_flows'),('auth_attempts')) t(name)
            CROSS JOIN (VALUES ('SELECT'),('INSERT'),('DELETE')) p(privilege)
            WHERE NOT has_table_privilege('core_identity.'||t.name,p.privilege))
          AND has_table_privilege('core_identity.auth_attempts','UPDATE')
          AND has_table_privilege('core_identity.developer_access','SELECT')
          AND has_table_privilege('core_identity.user_routing_groups','SELECT')
          AND has_table_privilege('core_identity.privileged_import_facts','SELECT')
          AND has_table_privilege('core_identity.permission_policies','SELECT')
          AND has_table_privilege('core_identity.permission_policy_rows','SELECT')
          AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='core_identity' AND CASE WHEN c.relkind='S' THEN NOT has_sequence_privilege(c.oid,'USAGE') ELSE FALSE END)
          AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE (n.nspname='core_meta' OR (n.nspname='core_billing' AND c.relkind IN ('r','p')))
              AND c.relkind IN ('r','p') AND NOT has_table_privilege(c.oid,'SELECT'))
          AND has_table_privilege('core_billing.balance_state','SELECT')
          AND has_table_privilege('core_billing.account_policies','SELECT')
          AND has_table_privilege('core_billing.account_policies','INSERT')
          AND has_table_privilege('core_billing.account_policies','UPDATE')
          AND has_function_privilege('core_billing.open_ledger_wallet(bigint)','EXECUTE')
          AND has_function_privilege('core_billing.post_ledger(jsonb)','EXECUTE')
          AND has_function_privilege('core_billing.legacy_escrow_finalize(jsonb,bigint,bigint)','EXECUTE')
    "#).fetch_one(&mut *connection).await?;
    if !identity_complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }

    if required.market_oauth {
        let complete:bool=sqlx::query_scalar(r#"
            SELECT has_schema_privilege('core_market_oauth','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('clients'),('requests'),('families'),('codes'),('tokens')) t(name)
              CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
              WHERE NOT has_table_privilege('core_market_oauth.'||t.name,p.privilege))
            AND has_column_privilege('core_market_oauth.clients','revoked_at','UPDATE')
            AND has_column_privilege('core_market_oauth.families','revoked_at','UPDATE')
            AND has_column_privilege('core_market_oauth.requests','consumed_at','UPDATE')
            AND has_column_privilege('core_market_oauth.codes','consumed_at','UPDATE')
            AND has_column_privilege('core_market_oauth.tokens','consumed_at','UPDATE')
            AND has_function_privilege('core_market_oauth.scope_set(text[])','EXECUTE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.oauth2 {
        let complete: bool = sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_oauth','USAGE')
          AND has_table_privilege('core_oauth.contract','SELECT')
          AND NOT EXISTS(SELECT FROM (VALUES ('families'),('authorization_requests'),('authorization_codes'),('access_tokens'),('refresh_tokens')) t(name)
            CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
            WHERE NOT has_table_privilege('core_oauth.'||t.name,p.privilege))
          AND has_column_privilege('core_oauth.families','scopes','UPDATE')
          AND has_column_privilege('core_oauth.families','revoked_at','UPDATE')
          AND NOT EXISTS(SELECT FROM (VALUES ('authorization_requests'),('authorization_codes'),('refresh_tokens')) t(name)
            WHERE NOT has_column_privilege('core_oauth.'||t.name,'consumed_at','UPDATE'))
          AND has_function_privilege('core_oauth.readonly_scope_set(text[])','EXECUTE')
          AND has_function_privilege('core_oauth.client_scope_set(text,text[],boolean)','EXECUTE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.web_identity {
        let complete: bool = sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_charging','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('budgets'),('requests'),('external_consumption'),('budget_opening_consumption')) t(name)
              WHERE NOT has_table_privilege('core_charging.'||t.name,'SELECT'))
            AND has_table_privilege('core_charging.budgets','INSERT')
            AND has_table_privilege('core_charging.budgets','UPDATE')
            AND has_table_privilege('core_charging.policy_events','INSERT')
            AND has_sequence_privilege('core_charging.budgets_id_seq','USAGE')
            AND has_sequence_privilege('core_charging.policy_events_id_seq','USAGE')
            AND has_function_privilege('core_charging.committed_credits(text,bigint,bigint,bigint)','EXECUTE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    // An enabled gateway supplies its own validated catalogue. Web identity
    // without that gateway reads only the published group/model projections.
    if required.web_identity && !required.models {
        let complete: bool = sqlx::query_scalar(r#"
          SELECT has_schema_privilege('public','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('routing_active_release'),('routing_groups'),('routing_routes')) t(name)
              WHERE NOT has_table_privilege('public.'||t.name,'SELECT'))
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.models || required.images || required.commerce || required.assistant {
        let complete: bool = sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_charging','USAGE')
            AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
              WHERE n.nspname='core_charging' AND c.relkind IN ('r','p') AND NOT has_table_privilege(c.oid,'SELECT'))
            AND NOT EXISTS(SELECT FROM (VALUES ('budgets'),('subscriptions'),('ledger_holds')) t(name)
              CROSS JOIN (VALUES ('INSERT'),('UPDATE')) p(privilege)
              WHERE NOT has_table_privilege('core_charging.'||t.name,p.privilege))
            AND NOT EXISTS(SELECT FROM (VALUES ('id'),('actor_user_id'),('key_id'),('oauth_source'),('assistant_grant_id'),('assistant_sponsor_account_id'),('owner_account_id'),('payer_account_id'),('fingerprint'),('worker_digest'),('spec_digest'),('price'),('source_subscription_id'),('source_start'),('source_end'),('authorized_at'),('lease_until'),('hard_deadline'),('state'),('reserved')) c(name)
              WHERE NOT has_column_privilege('core_charging.requests',c.name,'INSERT'))
            AND NOT EXISTS(SELECT FROM (VALUES ('state'),('reserved'),('input_tokens'),('cached_tokens'),('output_tokens'),('settled'),('refunded'),('lease_until'),('revision'),('outcome')) c(name)
              WHERE NOT has_column_privilege('core_charging.requests',c.name,'UPDATE'))
            AND NOT EXISTS(SELECT FROM (VALUES ('budget_checks'),('usage_events'),('refunds'),('policy_events'),('ledger_commands'),('external_consumption')) t(name)
              WHERE NOT has_table_privilege('core_charging.'||t.name,'INSERT'))
            AND has_column_privilege('core_charging.external_consumption','refunded','UPDATE')
            AND has_sequence_privilege('core_charging.budgets_id_seq','USAGE')
            AND has_sequence_privilege('core_charging.subscriptions_id_seq','USAGE')
            AND has_sequence_privilege('core_charging.policy_events_id_seq','USAGE')
            AND has_function_privilege('core_charging.period_window(text,bigint,bigint,bigint)','EXECUTE')
            AND has_function_privilege('core_charging.subscription_window(bigint,bigint)','EXECUTE')
            AND has_function_privilege('core_charging.committed_credits(text,bigint,bigint,bigint)','EXECUTE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.events || required.commerce {
        let complete: bool = sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_events','USAGE')
            AND has_table_privilege('core_events.capacity','SELECT')
            AND has_column_privilege('core_events.capacity','pending_events','UPDATE')
            AND has_column_privilege('core_events.capacity','pending_bytes','UPDATE')
            AND NOT EXISTS(SELECT FROM (VALUES ('subscriptions'),('outbox'),('deliveries'),('inbox')) t(name)
              CROSS JOIN (VALUES ('SELECT'),('INSERT'),('UPDATE')) p(privilege)
              WHERE NOT has_table_privilege('core_events.'||t.name,p.privilege))
            AND has_sequence_privilege('core_events.outbox_id_seq','USAGE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.console {
        let complete: bool = sqlx::query_scalar(
            r#"
          SELECT has_schema_privilege('core_console','USAGE')
            AND has_table_privilege('core_console.options','SELECT')
            AND has_table_privilege('core_console.initialization','SELECT')
            AND has_table_privilege('core_console.announcement_reads','SELECT')
            AND has_table_privilege('core_console.announcement_reads','INSERT')
        "#,
        )
        .fetch_one(&mut *connection)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.models {
        let complete: bool = sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_model','USAGE')
            AND has_function_privilege('core_model.checkpoint_output(text,bytea,bigint,bytea,bigint,jsonb,boolean)','EXECUTE')
            AND has_function_privilege('core_model.completed_streams_after(integer,text)','EXECUTE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.wallet {
        let complete:bool=sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_wallet','USAGE')
            AND has_table_privilege('core_wallet.transfers','SELECT')
            AND has_table_privilege('core_wallet.transfers','INSERT')
            AND has_sequence_privilege('core_wallet.transfers_id_seq','USAGE')
            AND has_sequence_privilege('core_wallet.transfers_id_seq','SELECT')
            AND NOT EXISTS(SELECT FROM (VALUES ('state'),('hold_journal'),('final_journal'),('recipient_user'),('confirmed_at'),('finished_at'),('confirmation_digest'),('confirmation_sid'),('confirmation_kind'),('confirmation_expires')) c(name)
              WHERE NOT has_column_privilege('core_wallet.transfers',c.name,'UPDATE'))
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.bounties {
        let complete:bool=sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_bounties','USAGE')
            AND has_schema_privilege('core_charging','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('budgets'),('requests'),('external_consumption'),('budget_opening_consumption')) t(name)
              WHERE NOT has_table_privilege('core_charging.'||t.name,'SELECT'))
            AND has_column_privilege('core_charging.budgets','revision','UPDATE')
            AND has_table_privilege('core_charging.external_consumption','INSERT')
            AND has_column_privilege('core_charging.external_consumption','refunded','UPDATE')
            AND has_function_privilege('core_charging.period_window(text,bigint,bigint,bigint)','EXECUTE')
            AND has_function_privilege('core_charging.committed_credits(text,bigint,bigint,bigint)','EXECUTE')
            AND NOT EXISTS(SELECT FROM (VALUES ('policy'),('projects'),('confirmations'),('operations')) t(name)
              WHERE NOT has_table_privilege('core_bounties.'||t.name,'SELECT'))
            AND NOT EXISTS(SELECT FROM (VALUES ('projects'),('operations')) t(name)
              WHERE NOT has_table_privilege('core_bounties.'||t.name,'INSERT'))
            AND NOT EXISTS(SELECT FROM (VALUES ('id'),('actor'),('credential'),('request'),('expires'),('policy')) c(name)
              WHERE NOT has_column_privilege('core_bounties.confirmations',c.name,'INSERT'))
            AND has_column_privilege('core_bounties.projects','body','UPDATE')
            AND NOT EXISTS(SELECT FROM (VALUES ('approved'),('csrf'),('browser_sid')) c(name)
              WHERE NOT has_column_privilege('core_bounties.confirmations',c.name,'UPDATE'))
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.images {
        // Resolve names inside optional schemas only after their USAGE grant.
        // Otherwise privilege probes themselves raise 42501 instead of reporting
        // the missing capability to installation/readiness callers.
        let visible: bool = sqlx::query_scalar("SELECT has_schema_privilege('core_image','USAGE') AND has_schema_privilege('core_tool_market','USAGE')")
            .fetch_one(&mut *connection).await?;
        if !visible {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
        let complete:bool=sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_image','USAGE')
            AND has_table_privilege('core_image.operations','SELECT') AND has_table_privilege('core_image.operations','INSERT')
            AND NOT EXISTS(SELECT FROM (VALUES ('state'),('confirmed_at'),('confirmation_digest'),('confirmation_sid'),('charge_id'),('output'),('output_digest'),('image_usage'),('settled_credits')) c(name)
              WHERE NOT has_column_privilege('core_image.operations',c.name,'UPDATE'))
            AND has_column_privilege('core_charging.requests','image_operation_id','INSERT')
            AND has_column_privilege('core_charging.requests','image_usage','UPDATE')
            AND has_function_privilege('core_image.valid_output(jsonb)','EXECUTE')
            AND has_function_privilege('core_tool_market.lock_builtin_context(text)','EXECUTE')
            AND has_schema_privilege('core_tool_market','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('credit_calls'),('credit_consents'),('credit_quotes'),('credit_builtin_bindings')) t(name)
              WHERE NOT has_table_privilege('core_tool_market.'||t.name,'SELECT'))
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.commerce {
        let complete: bool = sqlx::query_scalar(
            r#"
          SELECT has_schema_privilege('core_commerce','USAGE')
            AND has_table_privilege('core_commerce.orders','SELECT')
            AND has_table_privilege('core_commerce.orders','INSERT')
            AND has_column_privilege('core_commerce.orders','state','UPDATE')
            AND has_column_privilege('core_commerce.orders','settlement_journal_id','UPDATE')
            AND has_column_privilege('core_commerce.orders','revision','UPDATE')
            AND has_column_privilege('core_commerce.orders','delivery_reference','UPDATE')
            AND has_table_privilege('core_commerce.operations','SELECT')
            AND has_table_privilege('core_commerce.operations','INSERT')
            AND has_table_privilege('core_commerce.key_scopes','SELECT')
            AND has_table_privilege('core_commerce.key_scopes','INSERT')
            AND has_column_privilege('core_commerce.key_scopes','can_spend','UPDATE')
            AND has_sequence_privilege('core_commerce.orders_id_seq','USAGE')
        "#,
        )
        .fetch_one(&mut *connection)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.payments {
        let complete: bool = sqlx::query_scalar(
            r#"
          SELECT has_schema_privilege('core_payments','USAGE')
            AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
              CROSS JOIN (VALUES ('SELECT'),('INSERT')) AS p(privilege)
              WHERE n.nspname='core_payments' AND c.relkind IN ('r','p')
                AND NOT has_table_privilege(c.oid,p.privilege))
            AND has_table_privilege('core_payments.intents','UPDATE')
            AND has_table_privilege('core_payments.refund_holds','UPDATE')
            AND has_sequence_privilege('core_payments.intents_id_seq','USAGE')
            AND has_sequence_privilege('core_payments.intents_id_seq','SELECT')
        "#,
        )
        .fetch_one(&mut *connection)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.assistant_tools {
        let complete:bool=sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_assistant_tools','USAGE')
            AND has_schema_privilege('core_humangrant','USAGE')
            AND has_table_privilege('core_humangrant.human_grant_gate','SELECT')
            AND has_column_privilege('core_humangrant.human_grant_gate','revision','UPDATE')
            AND has_table_privilege('core_assistant_tools.confirmations','SELECT')
            AND has_table_privilege('core_assistant_tools.confirmations','INSERT')
            AND has_column_privilege('core_assistant_tools.confirmations','nonce_sha256','UPDATE')
            AND has_column_privilege('core_assistant_tools.confirmations','state','UPDATE')
            AND has_column_privilege('core_assistant_tools.confirmations','confirmed_at','UPDATE')
            AND has_schema_privilege('core_payments','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('intents'),('accepted_proofs'),('refund_holds'),('blocked_refunds')) t(name)
              WHERE NOT has_table_privilege('core_payments.'||t.name,'SELECT'))
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.assistant {
        let complete: bool = sqlx::query_scalar(
            r#"
          SELECT has_schema_privilege('core_assistant','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('conversations'),('grants'),('operations')) t(name)
              CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
              WHERE NOT has_table_privilege('core_assistant.'||t.name,p.privilege))
            AND has_column_privilege('core_assistant.grants','revoked','UPDATE')
            AND has_column_privilege('core_assistant.operations','answer_text','UPDATE')
            AND has_column_privilege('core_assistant.operations','completed_at','UPDATE')
            AND has_column_privilege('core_assistant.operations','gift_receipt','UPDATE')
            AND has_column_privilege('core_assistant.operations','gift_tool_id','UPDATE')
            AND has_column_privilege('core_assistant.operations','gift_response_sha256','UPDATE')
            AND has_sequence_privilege('core_assistant.operations_position_seq','USAGE')
            AND has_sequence_privilege('core_assistant.operations_position_seq','SELECT')
            AND NOT EXISTS(SELECT FROM (VALUES ('decisions'),('registration_profiles'),('fingerprints')) t(name)
              CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
              WHERE NOT has_table_privilege('core_gift.'||t.name,p.privilege))
            AND has_column_privilege('core_gift.decisions','claimed_at','UPDATE')
            AND has_column_privilege('core_gift.decisions','ledger_journal_id','UPDATE')
            AND has_column_privilege('core_gift.registration_profiles','observed_at','UPDATE')
            AND has_table_privilege('core_gift.fingerprints','DELETE')
            AND has_sequence_privilege('core_gift.fingerprints_id_seq','USAGE')
            AND has_sequence_privilege('core_gift.fingerprints_id_seq','SELECT')
        "#,
        )
        .fetch_one(&mut *connection)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    if required.tool_market {
        let complete:bool=sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_tool_market','USAGE')
            AND has_table_privilege('core_tool_market.publishers','SELECT')
            AND NOT EXISTS(SELECT FROM (VALUES ('quotes'),('approvals'),('calls')) t(name)
              CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
              WHERE NOT has_table_privilege('core_tool_market.'||t.name,p.privilege))
            AND has_column_privilege('core_tool_market.approvals','revoked','UPDATE')
            AND has_column_privilege('core_tool_market.approvals','used_calls','UPDATE')
            AND has_column_privilege('core_tool_market.approvals','used_amount','UPDATE')
            AND has_column_privilege('core_tool_market.calls','receipt','UPDATE')
            AND has_column_privilege('core_tool_market.calls','result_hash','UPDATE')
            AND has_schema_privilege('core_charging','USAGE')
            AND NOT EXISTS(SELECT FROM (VALUES ('budgets'),('subscriptions'),('requests'),('external_consumption'),('budget_opening_consumption'),('subscription_opening_consumption')) t(name)
              WHERE NOT has_table_privilege('core_charging.'||t.name,'SELECT'))
            AND has_table_privilege('core_charging.external_consumption','INSERT')
            AND has_column_privilege('core_charging.external_consumption','refunded','UPDATE')
            AND has_table_privilege('core_charging.budget_checks','INSERT')
            AND has_function_privilege('core_charging.period_window(text,bigint,bigint,bigint)','EXECUTE')
            AND has_function_privilege('core_charging.subscription_window(bigint,bigint)','EXECUTE')
            AND has_function_privilege('core_charging.committed_credits(text,bigint,bigint,bigint)','EXECUTE')
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}
