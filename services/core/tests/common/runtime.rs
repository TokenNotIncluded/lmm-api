use sqlx::{PgPool, postgres::PgPoolOptions};

/// Synthetic role scoped to this test's SQLx-owned database. Never used by a
/// developer/production database, and removed explicitly after closing children.
pub struct RuntimeRole {
    pub pool: PgPool,
    pub url: String,
    pub name: String,
    owner: PgPool,
}
impl RuntimeRole {
    pub async fn create(owner: &PgPool) -> Self {
        let name = format!(
            "lmm_runtime_test_{}_{:016x}",
            std::process::id(),
            rand::random::<u64>()
        );
        let password = format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        );
        let database = owner.connect_options().get_database().unwrap().to_owned();
        assert!(
            database
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        sqlx::raw_sql(&format!(r#"
            CREATE ROLE {name} LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
            REVOKE CREATE,TEMPORARY ON DATABASE "{database}" FROM PUBLIC;
            REVOKE CREATE ON SCHEMA public FROM PUBLIC;
            GRANT CONNECT ON DATABASE "{database}" TO {name};
            GRANT USAGE ON SCHEMA core_meta,core_identity,core_billing TO {name};
            GRANT SELECT ON ALL TABLES IN SCHEMA core_meta,core_billing TO {name};
            GRANT SELECT,INSERT,UPDATE ON core_identity.accounts,core_identity.users,core_identity.teams,core_identity.memberships,core_identity.credentials,core_identity.invites,core_identity.password_logins,core_identity.user_profiles,core_identity.company_billing_profiles,core_identity.web_sessions,core_identity.api_key_settings,core_identity.mfa_factors,core_identity.mfa_enrollments,core_identity.mfa_backup_codes,core_identity.mfa_challenges TO {name};
            GRANT SELECT,INSERT ON core_identity.key_funding_rules,core_identity.credential_grants,core_identity.oauth_identities,core_identity.audit,core_identity.web_session_credentials,core_identity.session_authentication TO {name};
            GRANT SELECT,INSERT,DELETE ON core_identity.oauth_flows TO {name};
            GRANT SELECT,INSERT,UPDATE,DELETE ON core_identity.auth_attempts TO {name};
            GRANT SELECT ON core_identity.developer_access,core_identity.user_routing_groups,core_identity.privileged_import_facts,core_identity.permission_policies,core_identity.permission_policy_rows TO {name};
            GRANT USAGE ON ALL SEQUENCES IN SCHEMA core_identity TO {name};
            GRANT SELECT,INSERT,UPDATE ON core_billing.account_policies TO {name};
            GRANT EXECUTE ON FUNCTION core_billing.open_ledger_wallet(bigint),core_billing.post_ledger(jsonb),core_billing.legacy_escrow_finalize(jsonb,bigint,bigint) TO {name};
        "#)).execute(owner).await.unwrap();
        // Identity/ledger-only fixtures intentionally omit optional components.
        // Complete runtime provisioning/checking remains strict in production.
        let abuse_installed: bool =
            sqlx::query_scalar("SELECT to_regnamespace('core_abuse') IS NOT NULL")
                .fetch_one(owner)
                .await
                .unwrap();
        if abuse_installed {
            sqlx::raw_sql(&format!(
                r#"
-- Abuse administration is checked against canonical superadmin Sessions in Rust.
-- Secrets are host-private references; audit/operations/previews are append-only.
GRANT USAGE ON SCHEMA core_abuse TO {name};
GRANT SELECT ON ALL TABLES IN SCHEMA core_abuse TO {name};
GRANT UPDATE (revision,document) ON core_abuse.settings TO {name};
GRANT INSERT,UPDATE (marked,policy_id,revision) ON core_abuse.user_marks TO {name};
GRANT INSERT ON core_abuse.previews,core_abuse.operations,core_abuse.audit,core_abuse.registration_observations TO {name};
GRANT INSERT,UPDATE (attempts),DELETE ON core_abuse.model_counters TO {name};
GRANT INSERT,UPDATE (finished_at),DELETE ON core_abuse.model_leases TO {name};
GRANT USAGE ON SEQUENCE core_abuse.audit_id_seq TO {name};
            "#
            ))
            .execute(owner)
            .await
            .unwrap();
        }
        let history_installed: bool =
            sqlx::query_scalar("SELECT to_regnamespace('core_history') IS NOT NULL")
                .fetch_one(owner)
                .await
                .unwrap();
        if history_installed {
            sqlx::raw_sql(&format!(
                "GRANT USAGE ON SCHEMA core_history TO {0}; GRANT SELECT ON core_history.sources,core_history.log_records,core_history.scope_receipts TO {0}",
                name
            ))
            .execute(owner)
            .await
            .unwrap();
        }
        let gift_installed: bool =
            sqlx::query_scalar("SELECT to_regnamespace('core_gift') IS NOT NULL")
                .fetch_one(owner)
                .await
                .unwrap();
        if gift_installed {
            sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_gift TO {0}; GRANT SELECT ON core_gift.risk_key,core_gift.policy,core_gift.qualifications TO {0}; GRANT INSERT ON core_gift.qualifications TO {0}; GRANT EXECUTE ON FUNCTION core_gift.lock_eligibility(bigint) TO {0}",name))
                .execute(owner).await.unwrap();
        }
        let telegram_installed: bool = sqlx::query_scalar(
            "SELECT count(*)=3 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_telegram_auth' AND c.relkind='r' AND c.relname IN ('bindings','flows','assertions')",
        ).fetch_one(owner).await.unwrap();
        if telegram_installed {
            sqlx::raw_sql(&format!(
                "GRANT USAGE ON SCHEMA core_telegram_auth TO {name}; GRANT SELECT ON core_telegram_auth.bindings,core_telegram_auth.flows,core_telegram_auth.assertions TO {name}; GRANT INSERT(subject,user_id),UPDATE(revoked_at) ON core_telegram_auth.bindings TO {name}; GRANT INSERT(token_digest,purpose,binding_digest,user_id,user_version,sid),UPDATE(consumed_at),DELETE ON core_telegram_auth.flows TO {name}; GRANT INSERT(digest),DELETE ON core_telegram_auth.assertions TO {name}",
            )).execute(owner).await.unwrap();
        }
        // Match the production enable_trust_pricing read grants, without adding
        // optional components or authority to older partial-schema fixtures.
        let trust_installed: bool = sqlx::query_scalar(
            "SELECT count(*)=3 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_trust_pricing' AND c.relkind='r' AND c.relname IN ('policies','user_bases','legacy_paid_orders')",
        )
        .fetch_one(owner)
        .await
        .unwrap();
        if trust_installed {
            sqlx::raw_sql(&format!(
                "GRANT USAGE ON SCHEMA core_trust_pricing TO {name}; GRANT SELECT ON core_trust_pricing.policies,core_trust_pricing.user_bases,core_trust_pricing.legacy_paid_orders TO {name}",
            ))
            .execute(owner)
            .await
            .unwrap();
            let payments_installed: bool =
                sqlx::query_scalar("SELECT to_regclass('core_payments.intents') IS NOT NULL")
                    .fetch_one(owner)
                    .await
                    .unwrap();
            if payments_installed {
                sqlx::raw_sql(&format!(
                    "GRANT USAGE ON SCHEMA core_payments TO {name}; GRANT SELECT ON core_payments.intents TO {name}",
                ))
                .execute(owner)
                .await
                .unwrap();
            }
            let charging_installed: bool = sqlx::query_scalar(
                "SELECT to_regclass('core_charging.requests') IS NOT NULL AND to_regclass('core_charging.usage_events') IS NOT NULL",
            )
            .fetch_one(owner)
            .await
            .unwrap();
            if charging_installed {
                sqlx::raw_sql(&format!(
                    "GRANT USAGE ON SCHEMA core_charging TO {name}; GRANT SELECT ON core_charging.requests,core_charging.usage_events TO {name}",
                ))
                .execute(owner)
                .await
                .unwrap();
            }
        }
        let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
        assert!(matches!(url.host_str(), Some("localhost" | "127.0.0.1")));
        url.set_path(&database);
        url.set_username(&name).unwrap();
        url.set_password(Some(&password)).unwrap();
        let url = url.to_string();
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap();
        Self {
            pool,
            url,
            name,
            owner: owner.clone(),
        }
    }
    pub async fn optional(&self, schema: &str) {
        if schema == "core_market_oauth" {
            sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_market_oauth TO {0}; GRANT SELECT,INSERT ON core_market_oauth.clients,core_market_oauth.requests,core_market_oauth.families,core_market_oauth.codes,core_market_oauth.tokens TO {0}; GRANT UPDATE(revoked_at) ON core_market_oauth.clients,core_market_oauth.families TO {0}; GRANT UPDATE(consumed_at) ON core_market_oauth.requests,core_market_oauth.codes,core_market_oauth.tokens TO {0}; GRANT EXECUTE ON FUNCTION core_market_oauth.scope_set(text[]) TO {0}",self.name)).execute(&self.owner).await.unwrap();
            return;
        }
        if schema == "core_console" {
            sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_console TO {0}; GRANT SELECT ON core_console.options,core_console.initialization,core_console.announcement_reads TO {0}; GRANT INSERT ON core_console.announcement_reads TO {0}",self.name))
                .execute(&self.owner).await.unwrap();
            return;
        }
        if schema == "core_model" {
            sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_model TO {0}; GRANT EXECUTE ON FUNCTION core_model.checkpoint_output(text,bytea,bigint,bytea,bigint,jsonb,boolean),core_model.completed_streams_after(integer,text) TO {0}",self.name))
                .execute(&self.owner).await.unwrap();
            return;
        }
        if schema == "core_payments" {
            sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_payments TO {0}; GRANT SELECT,INSERT ON ALL TABLES IN SCHEMA core_payments TO {0}; GRANT UPDATE ON core_payments.intents,core_payments.refund_holds TO {0}; GRANT USAGE,SELECT ON SEQUENCE core_payments.intents_id_seq TO {0}",self.name))
                .execute(&self.owner).await.unwrap();
            return;
        }
        let sql = match schema {
            "support_projection" => format!(
                "GRANT USAGE ON SCHEMA core_commerce TO {0}; GRANT SELECT ON core_commerce.orders TO {0}",
                self.name
            ),
            "core_assistant_tools" => format!(
                "GRANT USAGE ON SCHEMA core_assistant_tools,core_payments,core_humangrant TO {0}; GRANT SELECT,UPDATE(revision) ON core_humangrant.human_grant_gate TO {0}; GRANT SELECT ON core_payments.intents,core_payments.accepted_proofs,core_payments.refund_holds,core_payments.blocked_refunds TO {0}; GRANT SELECT,INSERT ON core_assistant_tools.confirmations TO {0}; GRANT UPDATE(nonce_sha256,state,confirmed_at) ON core_assistant_tools.confirmations TO {0}",
                self.name
            ),
            "core_assistant" => format!(
                r#"
              GRANT USAGE ON SCHEMA core_assistant TO {0};
              GRANT SELECT,INSERT ON core_assistant.conversations,core_assistant.grants,core_assistant.operations TO {0};
              GRANT UPDATE(revoked) ON core_assistant.grants TO {0};
              GRANT UPDATE(answer_text,completed_at,gift_receipt,gift_tool_id,gift_response_sha256) ON core_assistant.operations TO {0};
              GRANT USAGE,SELECT ON SEQUENCE core_assistant.operations_position_seq TO {0};
              GRANT SELECT,INSERT ON core_gift.decisions,core_gift.registration_profiles,core_gift.fingerprints TO {0};
              GRANT UPDATE(claimed_at,ledger_journal_id) ON core_gift.decisions TO {0};
              GRANT UPDATE(observed_at) ON core_gift.registration_profiles TO {0};
              GRANT DELETE ON core_gift.fingerprints TO {0};
              GRANT USAGE,SELECT ON SEQUENCE core_gift.fingerprints_id_seq TO {0};
            "#,
                self.name
            ),
            "core_gift" => format!(
                r#"
              GRANT SELECT,INSERT ON core_gift.decisions,core_gift.registration_profiles,core_gift.fingerprints TO {0};
              GRANT UPDATE(claimed_at,ledger_journal_id) ON core_gift.decisions TO {0};
              GRANT UPDATE(observed_at) ON core_gift.registration_profiles TO {0};
              GRANT DELETE ON core_gift.fingerprints TO {0};
              GRANT USAGE,SELECT ON SEQUENCE core_gift.fingerprints_id_seq TO {0};
            "#,
                self.name
            ),
            "web_identity" => format!(
                r#"
              GRANT USAGE ON SCHEMA public TO {0};
              GRANT SELECT ON public.routing_active_release,public.routing_groups,public.routing_routes TO {0};
              GRANT USAGE ON SCHEMA core_charging TO {0};
              GRANT SELECT ON core_charging.budgets,core_charging.requests,core_charging.external_consumption,core_charging.budget_opening_consumption TO {0};
              GRANT INSERT,UPDATE ON core_charging.budgets TO {0};
              GRANT INSERT ON core_charging.policy_events TO {0};
              GRANT USAGE ON SEQUENCE core_charging.budgets_id_seq,core_charging.policy_events_id_seq TO {0};
              GRANT EXECUTE ON FUNCTION core_charging.committed_credits(text,bigint,bigint,bigint) TO {0};
            "#,
                self.name
            ),
            "core_charging" => format!(
                r#"
              GRANT USAGE ON SCHEMA core_charging TO {0};
              GRANT SELECT ON ALL TABLES IN SCHEMA core_charging TO {0};
              GRANT INSERT,UPDATE ON core_charging.budgets,core_charging.subscriptions,core_charging.ledger_holds TO {0};
              GRANT INSERT(id,actor_user_id,key_id,oauth_source,assistant_grant_id,assistant_sponsor_account_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,source_subscription_id,source_start,source_end,authorized_at,lease_until,hard_deadline,state,reserved), UPDATE(state,reserved,input_tokens,cached_tokens,output_tokens,settled,refunded,lease_until,revision,outcome) ON core_charging.requests TO {0};
              GRANT INSERT ON core_charging.budget_checks,core_charging.usage_events,core_charging.refunds,core_charging.policy_events,core_charging.ledger_commands,core_charging.external_consumption TO {0};
              GRANT UPDATE(refunded) ON core_charging.external_consumption TO {0};
              GRANT USAGE ON SEQUENCE core_charging.budgets_id_seq,core_charging.subscriptions_id_seq,core_charging.policy_events_id_seq TO {0};
              GRANT EXECUTE ON FUNCTION core_charging.period_window(text,bigint,bigint,bigint),core_charging.subscription_window(bigint,bigint),core_charging.committed_credits(text,bigint,bigint,bigint) TO {0};
            "#,
                self.name
            ),
            "core_image" => format!(
                r#"
              GRANT USAGE ON SCHEMA core_image,core_tool_market TO {0};
              GRANT SELECT,INSERT,UPDATE(state,confirmed_at,confirmation_digest,confirmation_sid,charge_id,output,output_digest,image_usage,settled_credits) ON core_image.operations TO {0};
              GRANT INSERT(image_operation_id),UPDATE(image_usage) ON core_charging.requests TO {0};
              GRANT EXECUTE ON FUNCTION core_image.valid_output(jsonb),core_tool_market.lock_builtin_context(text) TO {0};
              GRANT SELECT ON core_tool_market.credit_calls,core_tool_market.credit_consents,core_tool_market.credit_quotes,core_tool_market.credit_builtin_bindings TO {0};
            "#,
                self.name
            ),
            "core_events" => format!(
                r#"
              GRANT USAGE ON SCHEMA core_events TO {0};
              GRANT SELECT ON core_events.capacity TO {0};
              GRANT UPDATE(pending_events,pending_bytes) ON core_events.capacity TO {0};
              GRANT SELECT,INSERT,UPDATE ON core_events.subscriptions,core_events.outbox,core_events.deliveries,core_events.inbox TO {0};
              GRANT USAGE ON SEQUENCE core_events.outbox_id_seq TO {0};
            "#,
                self.name
            ),
            "core_commerce" => format!(
                r#"
              GRANT USAGE ON SCHEMA core_commerce TO {0};
              GRANT SELECT,INSERT ON core_commerce.orders,core_commerce.operations,core_commerce.key_scopes TO {0};
              GRANT UPDATE(state,settlement_journal_id,revision,delivery_reference) ON core_commerce.orders TO {0};
              GRANT UPDATE(can_spend) ON core_commerce.key_scopes TO {0};
              GRANT USAGE ON SEQUENCE core_commerce.orders_id_seq TO {0};
            "#,
                self.name
            ),
            "core_tool_market" => {
                let credit =
                    include_str!("../../../../deployment/postgres/grant-tool-market-credit.sql")
                        .replace(":\"runtime_role\"", &self.name);
                format!(
                    r#"
 GRANT USAGE ON SCHEMA core_tool_market,core_charging TO {0};
 GRANT SELECT ON core_tool_market.publishers TO {0};
 GRANT SELECT,INSERT ON core_tool_market.quotes TO {0};
 GRANT SELECT,INSERT,UPDATE(revoked,used_calls,used_amount) ON core_tool_market.approvals TO {0};
 GRANT SELECT,INSERT,UPDATE(receipt,result_hash) ON core_tool_market.calls TO {0};
 GRANT SELECT ON core_charging.budgets,core_charging.subscriptions,core_charging.requests,core_charging.external_consumption,core_charging.budget_opening_consumption,core_charging.subscription_opening_consumption TO {0};
 GRANT INSERT,UPDATE(refunded) ON core_charging.external_consumption TO {0};GRANT INSERT ON core_charging.budget_checks TO {0};
 GRANT EXECUTE ON FUNCTION core_charging.period_window(text,bigint,bigint,bigint),core_charging.subscription_window(bigint,bigint),core_charging.committed_credits(text,bigint,bigint,bigint) TO {0};
 {credit}
 "#,
                    self.name
                )
            }
            _ => panic!("unknown synthetic optional schema"),
        };
        sqlx::raw_sql(&sql).execute(&self.owner).await.unwrap();
    }
    pub async fn cleanup(self) {
        self.pool.close().await;
        sqlx::raw_sql(&format!(
            "DROP OWNED BY {}; DROP ROLE {}",
            self.name, self.name
        ))
        .execute(&self.owner)
        .await
        .unwrap();
    }
}
