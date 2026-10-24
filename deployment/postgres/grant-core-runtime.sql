-- Explicit offline GRANTs after the complete fresh lmm-core init-db transaction.
-- This script is additive by default. To remove old optional ACLs, run the
-- reviewed deployment/postgres/revoke-core-runtime.sql reset with
-- confirm_revoke=true, then re-run this script with the final feature flags.
-- No role passwords, automatic startup hook, wildcard grants or default ACLs.
\set ON_ERROR_STOP on
\if :{?runtime_role}
\else
\set runtime_role lmm_core_runtime
\endif
\if :{?enable_events}
\else
\set enable_events false
\endif
\if :{?enable_console}
\else
\set enable_console false
\endif
\if :{?enable_models}
\else
\set enable_models false
\endif
\if :{?enable_images}
\else
\set enable_images false
\endif
\if :{?enable_web_identity}
\else
\set enable_web_identity false
\endif
\if :{?enable_commerce}
\else
\set enable_commerce false
\endif
\if :{?enable_payments}
\else
\set enable_payments false
\endif

\if :{?enable_oauth2}
\else
\set enable_oauth2 false
\endif
\if :{?enable_market_oauth}
\else
\set enable_market_oauth false
\endif
\if :{?enable_humangrant}
\else
\set enable_humangrant false
\endif

\if :{?enable_assistant_tools}
\else
\set enable_assistant_tools false
\endif
\if :{?enable_assistant}
\else
\set enable_assistant false
\endif
\if :{?enable_tool_market}
\else
\set enable_tool_market false
\endif
\if :{?enable_support_projection}
\else
\set enable_support_projection false
\endif
-- Assistant model execution uses the existing charging/model authority.
\if :enable_assistant
\set enable_models true
\endif
\if :enable_images
\set enable_models true
\endif

-- @mark deployment.database-authority role=permissions
\if :{?enable_wallet}
\else
\set enable_wallet false
\endif
\if :{?enable_bounties}
\else
\set enable_bounties false
\endif
\if :{?enable_trust_pricing}
\else
\set enable_trust_pricing false
\endif
-- Native production model/image/assistant entrypoints require Core-owned trust.
\if :enable_models
\set enable_trust_pricing true
\endif
\if :enable_assistant
\set enable_trust_pricing true
\endif
BEGIN;
SET LOCAL statement_timeout = '5s';
SELECT current_database() AS core_database \gset
SELECT EXISTS (
    SELECT FROM pg_roles r WHERE r.rolname = :'runtime_role' AND r.rolcanlogin
      AND NOT (r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls)
) AND EXISTS (
    SELECT FROM pg_roles r WHERE r.rolname = 'lmm_core_owner' AND NOT r.rolcanlogin
) AND NOT EXISTS (
    SELECT FROM pg_roles r WHERE r.rolname <> :'runtime_role' AND (
      pg_has_role(:'runtime_role', r.oid, 'SET')
      OR pg_has_role(:'runtime_role', r.oid, 'MEMBER WITH ADMIN OPTION')
      OR pg_has_role(:'runtime_role', r.oid, 'USAGE'))
) AS runtime_role_safe \gset
\if :runtime_role_safe
\else
\echo 'Refusing unsafe runtime role or login-enabled installation owner.'
ROLLBACK;
-- psql \quit ignores an exit status, so a refused script would otherwise
-- leave the process successful after this rollback. ON_ERROR_STOP is set above.
DO $$ BEGIN
    RAISE EXCEPTION 'Refusing unsafe runtime role or login-enabled installation owner.';
END $$;
\endif

REVOKE CREATE, TEMPORARY ON DATABASE :"core_database" FROM PUBLIC;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT CONNECT ON DATABASE :"core_database" TO :"runtime_role";
GRANT USAGE ON SCHEMA core_meta, core_identity, core_billing TO :"runtime_role";
GRANT SELECT ON core_meta.schema_contract, core_meta.component_contract,
    core_meta.runtime_contract, core_meta.schema_upgrades, core_meta.component_upgrades,
    core_meta.ledger_revisions TO :"runtime_role";
-- Historical source receipts and logs are imported offline. Runtime only reads.
GRANT USAGE ON SCHEMA core_history TO :"runtime_role";
GRANT SELECT ON core_history.sources, core_history.log_records,
    core_history.scope_receipts TO :"runtime_role";
-- Verified registration proof may append a qualification, never edit campaign
-- policy, the installation HMAC key, qualification holds or reward history.
GRANT USAGE ON SCHEMA core_gift TO :"runtime_role";
GRANT SELECT ON core_gift.risk_key, core_gift.policy,
    core_gift.qualifications TO :"runtime_role";
GRANT INSERT ON core_gift.qualifications TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_gift.lock_eligibility(bigint) TO :"runtime_role";
GRANT SELECT ON core_billing.ledger_accounts, core_billing.balance_state,
    core_billing.ledger_operations, core_billing.ledger_journals,
    core_billing.ledger_entries, core_billing.opening_import_runs,
    core_billing.opening_imports, core_billing.account_opening_counters,
    core_billing.legacy_escrow_batch_runs, core_billing.legacy_escrow_projects,
    core_billing.legacy_escrow_claims, core_billing.legacy_escrow_disputes,
    core_billing.legacy_escrow_state, core_billing.legacy_escrow_operations,
    core_billing.legacy_pending_liabilities,
    core_billing.legacy_pending_liability_receipts TO :"runtime_role";
-- Imported administrator approval remains owner-only. Runtime reads frozen
-- facts for guarded identity changes and the complete permission snapshot.
-- Approval receipts and the migration registry are not runtime inputs.
GRANT SELECT ON core_identity.privileged_import_facts,
    core_identity.permission_policies, core_identity.permission_policy_rows TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE ON core_billing.account_policies TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_billing.open_ledger_wallet(bigint),
    core_billing.post_ledger(jsonb),
    core_billing.legacy_escrow_finalize(jsonb,bigint,bigint) TO :"runtime_role";

-- Abuse administration is checked against canonical superadmin Sessions in Rust.
-- Secrets are host-private references; audit/operations/previews are append-only.
GRANT USAGE ON SCHEMA core_abuse TO :"runtime_role";
GRANT SELECT ON ALL TABLES IN SCHEMA core_abuse TO :"runtime_role";
GRANT UPDATE (revision,document) ON core_abuse.settings TO :"runtime_role";
GRANT INSERT,UPDATE (marked,policy_id,revision) ON core_abuse.user_marks TO :"runtime_role";
GRANT INSERT ON core_abuse.previews,core_abuse.operations,core_abuse.audit,
    core_abuse.registration_observations TO :"runtime_role";
GRANT INSERT,UPDATE (attempts),DELETE ON core_abuse.model_counters TO :"runtime_role";
GRANT INSERT,UPDATE (finished_at),DELETE ON core_abuse.model_leases TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_abuse.audit_id_seq TO :"runtime_role";

-- Explicit identity-owner matrix; no writes to offline group/developer policy.
GRANT SELECT, INSERT, UPDATE ON core_identity.accounts, core_identity.users,
    core_identity.teams, core_identity.memberships, core_identity.credentials,
    core_identity.invites, core_identity.password_logins, core_identity.user_profiles,
    core_identity.company_billing_profiles, core_identity.web_sessions,
    core_identity.api_key_settings, core_identity.mfa_factors,
    core_identity.mfa_enrollments, core_identity.mfa_backup_codes,
    core_identity.mfa_challenges TO :"runtime_role";
GRANT SELECT, INSERT ON core_identity.key_funding_rules,
    core_identity.credential_grants, core_identity.oauth_identities,
    core_identity.audit, core_identity.web_session_credentials,
    core_identity.session_authentication TO :"runtime_role";
GRANT SELECT, INSERT, DELETE ON core_identity.oauth_flows TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE, DELETE ON core_identity.auth_attempts TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_identity.mfa_enrollments_id_seq,
    core_identity.mfa_backup_codes_id_seq TO :"runtime_role";
GRANT SELECT ON core_identity.developer_access,
    core_identity.user_routing_groups TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_identity.accounts_id_seq, core_identity.users_id_seq,
    core_identity.teams_id_seq, core_identity.credentials_id_seq,
    core_identity.invites_id_seq, core_identity.audit_id_seq TO :"runtime_role";

\if :enable_oauth2
-- Native OAuth issues no core credential or spending authority. Only the
-- immutable snapshot's consumption and family scope/revocation are mutable.
GRANT USAGE ON SCHEMA core_oauth TO :"runtime_role";
GRANT SELECT ON core_oauth.contract TO :"runtime_role";
GRANT SELECT, INSERT ON core_oauth.families, core_oauth.authorization_requests,
    core_oauth.authorization_codes, core_oauth.access_tokens,
    core_oauth.refresh_tokens TO :"runtime_role";
GRANT UPDATE (scopes, revoked_at) ON core_oauth.families TO :"runtime_role";
GRANT UPDATE (consumed_at) ON core_oauth.authorization_requests,
    core_oauth.authorization_codes, core_oauth.refresh_tokens TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_oauth.readonly_scope_set(text[]) TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_oauth.client_scope_set(text,text[],boolean) TO :"runtime_role";
\endif

\if :enable_commerce
-- Commerce performs charging and publishes events in the same core transaction.
\set enable_events true
\endif

\if :enable_events
GRANT USAGE ON SCHEMA core_events TO :"runtime_role";
GRANT SELECT, UPDATE (pending_events, pending_bytes) ON core_events.capacity TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE ON core_events.subscriptions, core_events.outbox,
    core_events.deliveries, core_events.inbox TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_events.outbox_id_seq TO :"runtime_role";
\endif

\if :enable_console
GRANT USAGE ON SCHEMA core_console TO :"runtime_role";
GRANT SELECT ON core_console.options, core_console.initialization,
    core_console.announcement_reads TO :"runtime_role";
GRANT INSERT ON core_console.announcement_reads TO :"runtime_role";
\endif

\if :enable_models
\set enable_charging true
\else
\set enable_charging false
\endif
\if :enable_commerce
\set enable_charging true
\endif
\if :enable_images
\set enable_charging true
\endif
\if :enable_charging
GRANT USAGE ON SCHEMA core_charging TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE ON core_charging.budgets, core_charging.subscriptions,
    core_charging.ledger_holds TO :"runtime_role";
GRANT SELECT ON core_charging.requests TO :"runtime_role";
GRANT INSERT(id,actor_user_id,key_id,oauth_source,assistant_grant_id,assistant_sponsor_account_id,
    owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,
    source_subscription_id,source_start,source_end,authorized_at,lease_until,hard_deadline,state,reserved),
    UPDATE(state,reserved,input_tokens,cached_tokens,output_tokens,settled,refunded,lease_until,revision,outcome)
    ON core_charging.requests TO :"runtime_role";
GRANT SELECT, INSERT ON core_charging.budget_checks, core_charging.usage_events,
    core_charging.refunds, core_charging.policy_events,
    core_charging.ledger_commands TO :"runtime_role";
GRANT SELECT ON core_charging.budget_opening_consumption,
    core_charging.subscription_opening_consumption TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (refunded) ON core_charging.external_consumption TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_charging.budgets_id_seq,
    core_charging.subscriptions_id_seq, core_charging.policy_events_id_seq TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_charging.period_window(text,bigint,bigint,bigint),
    core_charging.subscription_window(bigint,bigint),
    core_charging.committed_credits(text,bigint,bigint,bigint) TO :"runtime_role";
\endif

\if :enable_web_identity
GRANT USAGE ON SCHEMA core_telegram_auth TO :"runtime_role";
GRANT SELECT ON core_telegram_auth.bindings, core_telegram_auth.flows,
    core_telegram_auth.assertions TO :"runtime_role";
GRANT INSERT(subject,user_id), UPDATE(revoked_at) ON core_telegram_auth.bindings TO :"runtime_role";
GRANT INSERT(token_digest,purpose,binding_digest,user_id,user_version,sid), UPDATE(consumed_at), DELETE
    ON core_telegram_auth.flows TO :"runtime_role";
GRANT INSERT(digest), DELETE ON core_telegram_auth.assertions TO :"runtime_role";
-- Legacy Web identity manages budgets, not model requests or reservation holds.
-- Without an enabled model gateway, its catalogue uses only these projections.
GRANT USAGE ON SCHEMA public TO :"runtime_role";
GRANT SELECT ON public.routing_active_release, public.routing_groups,
    public.routing_routes TO :"runtime_role";
GRANT USAGE ON SCHEMA core_charging TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE ON core_charging.budgets TO :"runtime_role";
GRANT SELECT, INSERT ON core_charging.policy_events TO :"runtime_role";
GRANT SELECT ON core_charging.requests, core_charging.external_consumption,
    core_charging.budget_opening_consumption TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_charging.budgets_id_seq,
    core_charging.policy_events_id_seq TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_charging.committed_credits(text,bigint,bigint,bigint)
    TO :"runtime_role";
\endif

\if :enable_models
GRANT USAGE ON SCHEMA core_model TO :"runtime_role";
GRANT EXECUTE ON FUNCTION
    core_model.checkpoint_output(text,bytea,bigint,bytea,bigint,jsonb,boolean),
    core_model.completed_streams_after(integer,text) TO :"runtime_role";
\endif

\if :enable_images
-- Separate opt-in: ordinary model grants cannot create image operations or receipts.
GRANT USAGE ON SCHEMA core_image, core_tool_market TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(state,confirmed_at,confirmation_digest,confirmation_sid,charge_id,output,output_digest,image_usage,settled_credits)
    ON core_image.operations TO :"runtime_role";
GRANT INSERT(image_operation_id),UPDATE(image_usage) ON core_charging.requests TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_image.valid_output(jsonb),
    core_tool_market.lock_builtin_context(text) TO :"runtime_role";
-- Only inspect the installer-bound original drawing call; no marketplace mutation.
GRANT SELECT ON core_tool_market.credit_calls,core_tool_market.credit_consents,
    core_tool_market.credit_quotes,core_tool_market.credit_builtin_bindings TO :"runtime_role";
\endif

\if :enable_commerce
GRANT USAGE ON SCHEMA core_commerce TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (can_spend) ON core_commerce.key_scopes TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (state, settlement_journal_id, revision, delivery_reference)
    ON core_commerce.orders TO :"runtime_role";
GRANT SELECT, INSERT ON core_commerce.operations TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_commerce.orders_id_seq TO :"runtime_role";
\endif
\if :enable_support_projection
-- Customer summaries need only seller-bound reads, never a commerce writer.
GRANT USAGE ON SCHEMA core_commerce TO :"runtime_role";
GRANT SELECT ON core_commerce.orders TO :"runtime_role";
\endif

\if :enable_payments
GRANT USAGE ON SCHEMA core_payments TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE ON core_payments.intents,
    core_payments.refund_holds TO :"runtime_role";
GRANT SELECT, INSERT ON core_payments.receipts, core_payments.provider_events,
    core_payments.provider_refunds, core_payments.blocked_refunds,
    core_payments.accepted_proofs, core_payments.refund_dispatches,
    core_payments.refund_dispatch_bindings TO :"runtime_role";
GRANT USAGE, SELECT ON SEQUENCE core_payments.intents_id_seq TO :"runtime_role";
\endif
\if :enable_humangrant
GRANT USAGE ON SCHEMA core_humangrant TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE, DELETE ON core_humangrant.human_grant_records TO :"runtime_role";
GRANT SELECT, UPDATE (revision) ON core_humangrant.human_grant_gate TO :"runtime_role";
\endif
\if :enable_assistant_tools
GRANT USAGE ON SCHEMA core_assistant_tools,core_payments,core_humangrant TO :"runtime_role";
GRANT SELECT,UPDATE(revision) ON core_humangrant.human_grant_gate TO :"runtime_role";
GRANT SELECT ON core_payments.intents,core_payments.accepted_proofs,core_payments.refund_holds,core_payments.blocked_refunds TO :"runtime_role";
GRANT SELECT,INSERT ON core_assistant_tools.confirmations TO :"runtime_role";
GRANT UPDATE(nonce_sha256,state,confirmed_at) ON core_assistant_tools.confirmations TO :"runtime_role";
\endif
\if :enable_assistant
GRANT USAGE ON SCHEMA core_assistant TO :"runtime_role";
GRANT SELECT, INSERT ON core_assistant.conversations TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (revoked) ON core_assistant.grants TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (answer_text,completed_at,gift_receipt,gift_tool_id,gift_response_sha256) ON core_assistant.operations TO :"runtime_role";
GRANT USAGE, SELECT ON SEQUENCE core_assistant.operations_position_seq TO :"runtime_role";
GRANT SELECT, INSERT ON core_gift.decisions, core_gift.registration_profiles,
    core_gift.fingerprints TO :"runtime_role";
GRANT UPDATE (claimed_at,ledger_journal_id) ON core_gift.decisions TO :"runtime_role";
GRANT UPDATE (observed_at) ON core_gift.registration_profiles TO :"runtime_role";
GRANT DELETE ON core_gift.fingerprints TO :"runtime_role";
GRANT USAGE, SELECT ON SEQUENCE core_gift.fingerprints_id_seq TO :"runtime_role";
\endif
\if :enable_tool_market
-- Closed fixed-price escrow uses shared budgets, without model writer grants.
GRANT USAGE ON SCHEMA core_tool_market, core_charging TO :"runtime_role";
GRANT SELECT ON core_tool_market.publishers TO :"runtime_role";
GRANT SELECT, INSERT ON core_tool_market.quotes TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (revoked,used_calls,used_amount) ON core_tool_market.approvals TO :"runtime_role";
GRANT SELECT, INSERT, UPDATE (receipt,result_hash) ON core_tool_market.calls TO :"runtime_role";
-- BEGIN tool-market-credit runtime grants
-- Explicit installer-only complement to enable_tool_market. Canonical CREDIT v2
-- must already be installed. Runtime owns no schema, policy seed or code binding.
GRANT SELECT,UPDATE(version,platform_account_id,platform_user_id,enabled,fee_bps,updated_by,updated_at) ON core_tool_market.credit_policy TO :"runtime_role";
GRANT SELECT,INSERT(id,seller_user_id,seller_account_id,platform_account_id,fee_bps,policy_version,snapshot,ceiling_credits),UPDATE(approved,suspended,visibility,approved_by,approved_at) ON core_tool_market.credit_quotes TO :"runtime_role";
GRANT SELECT ON core_tool_market.credit_builtin_bindings,core_tool_market.credit_result_contracts TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(revoked_at,reserved_calls,successful_calls,reserved_credits,spent_credits) ON core_tool_market.credit_consents TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(request,limit_credits,reserved_credits,spent_credits) ON core_tool_market.credit_budgets TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(revoked_at) ON core_tool_market.credit_tokens TO :"runtime_role";
GRANT SELECT,INSERT ON core_tool_market.credit_access_operations,core_tool_market.credit_supplier_receipts,core_tool_market.credit_fixed_witness_receipts TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(state,dispatch_id,result_digest,usage,receipt,settlement_journal_id,started_at,finished_at) ON core_tool_market.credit_calls TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(active_version_id,paused,deleted,suspended,revision,updated_by,updated_at) ON core_tool_market.credit_services TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_tool_market.lock_builtin_context(TEXT) TO :"runtime_role";
-- END tool-market-credit runtime grants
GRANT SELECT ON core_charging.budgets, core_charging.subscriptions,
    core_charging.requests, core_charging.external_consumption,
    core_charging.budget_opening_consumption, core_charging.subscription_opening_consumption TO :"runtime_role";
GRANT INSERT, UPDATE (refunded) ON core_charging.external_consumption TO :"runtime_role";
GRANT INSERT ON core_charging.budget_checks TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_charging.period_window(text,bigint,bigint,bigint),
    core_charging.subscription_window(bigint,bigint),
    core_charging.committed_credits(text,bigint,bigint,bigint) TO :"runtime_role";
\endif
\if :enable_market_oauth
-- Explicit opt-in; this grant authorizes only the market resource issuer.
GRANT USAGE ON SCHEMA core_market_oauth TO :"runtime_role";
GRANT SELECT,INSERT ON core_market_oauth.clients,core_market_oauth.requests,
    core_market_oauth.families,core_market_oauth.codes,core_market_oauth.tokens TO :"runtime_role";
GRANT UPDATE(revoked_at) ON core_market_oauth.clients,core_market_oauth.families TO :"runtime_role";
GRANT UPDATE(consumed_at) ON core_market_oauth.requests,core_market_oauth.codes,
    core_market_oauth.tokens TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_market_oauth.scope_set(text[]) TO :"runtime_role";
\endif
\if :enable_wallet
-- Immutable payer, original amount/key and receipt lineage stay installer-owned.
GRANT USAGE ON SCHEMA core_wallet TO :"runtime_role";
GRANT SELECT,INSERT ON core_wallet.transfers TO :"runtime_role";
GRANT UPDATE(state,hold_journal,final_journal,recipient_user,confirmed_at,finished_at,
    confirmation_digest,confirmation_sid,confirmation_kind,confirmation_expires)
    ON core_wallet.transfers TO :"runtime_role";
GRANT USAGE,SELECT ON SEQUENCE core_wallet.transfers_id_seq TO :"runtime_role";
\endif
\if :enable_bounties
-- Policy changes and original operation receipts are never runtime updates.
-- Bounty budget checks do not require model request/stream write authority.
GRANT USAGE ON SCHEMA core_charging TO :"runtime_role";
GRANT SELECT ON core_charging.budgets,core_charging.requests,
    core_charging.external_consumption,core_charging.budget_opening_consumption TO :"runtime_role";
-- PostgreSQL FOR SHARE needs one UPDATE column; no budget limit/source is writable.
GRANT UPDATE(revision) ON core_charging.budgets TO :"runtime_role";
GRANT INSERT,UPDATE(refunded) ON core_charging.external_consumption TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_charging.period_window(text,bigint,bigint,bigint),
    core_charging.committed_credits(text,bigint,bigint,bigint) TO :"runtime_role";
GRANT USAGE ON SCHEMA core_bounties TO :"runtime_role";
GRANT SELECT ON core_bounties.policy TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(body) ON core_bounties.projects TO :"runtime_role";
GRANT SELECT,INSERT(id,actor,credential,request,expires,policy),UPDATE(approved,csrf,browser_sid)
    ON core_bounties.confirmations TO :"runtime_role";
GRANT SELECT,INSERT ON core_bounties.operations TO :"runtime_role";
\endif
\if :enable_trust_pricing
GRANT USAGE ON SCHEMA core_trust_pricing,core_payments,core_charging TO :"runtime_role";
GRANT SELECT ON core_payments.intents,core_charging.requests,core_charging.usage_events TO :"runtime_role";
GRANT SELECT ON core_trust_pricing.policies, core_trust_pricing.user_bases,
    core_trust_pricing.legacy_paid_orders TO :"runtime_role";
\endif
COMMIT;
