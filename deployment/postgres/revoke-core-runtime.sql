-- Explicit offline revocation of optional lmm-core runtime grants.
--
-- This is a reviewed ACL reset, never an automatic startup mutation. It removes
-- only grants covering optional capability schemas (plus the optional assistant
-- tables in core_gift and the Web routing projections in public). Baseline
-- identity, ledger, history, abuse, gift qualification and billing grants stay
-- intact. Re-run grant-core-runtime.sql with the final feature flags afterwards.
--
-- The complete fresh core schema installs these schemas before this script is
-- run. No role passwords, ownership changes, wildcard grants or default ACLs.
-- @mark deployment.database-authority role=permissions
\set ON_ERROR_STOP on
\if :{?runtime_role}
\else
\set runtime_role lmm_core_runtime
\endif
\if :{?confirm_revoke}
\else
\set confirm_revoke false
\endif

\if :confirm_revoke
\else
\echo 'Refusing optional runtime ACL reset; pass -v confirm_revoke=true explicitly.'
DO $$ BEGIN
    RAISE EXCEPTION 'Refusing optional runtime ACL reset; pass -v confirm_revoke=true explicitly.';
END $$;
\endif

BEGIN;
SET LOCAL statement_timeout = '5s';
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

-- Optional capability schemas have no baseline runtime authority. Resetting all
-- ACLs for this one runtime role also removes column-level grants from current
-- objects in those schemas without touching their owner or PUBLIC.
REVOKE ALL PRIVILEGES ON SCHEMA core_oauth, core_events, core_console,
    core_charging, core_telegram_auth, core_model, core_image, core_tool_market,
    core_commerce, core_payments, core_humangrant, core_assistant_tools,
    core_assistant, core_market_oauth, core_wallet, core_bounties,
    core_trust_pricing FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA core_oauth, core_events,
    core_console, core_charging, core_telegram_auth, core_model, core_image,
    core_tool_market, core_commerce, core_payments, core_humangrant,
    core_assistant_tools, core_assistant, core_market_oauth, core_wallet,
    core_bounties, core_trust_pricing FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA core_oauth, core_events,
    core_console, core_charging, core_telegram_auth, core_model, core_image,
    core_tool_market, core_commerce, core_payments, core_humangrant,
    core_assistant_tools, core_assistant, core_market_oauth, core_wallet,
    core_bounties, core_trust_pricing FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL FUNCTIONS IN SCHEMA core_oauth, core_events,
    core_console, core_charging, core_telegram_auth, core_model, core_image,
    core_tool_market, core_commerce, core_payments, core_humangrant,
    core_assistant_tools, core_assistant, core_market_oauth, core_wallet,
    core_bounties, core_trust_pricing FROM :"runtime_role";

-- Web identity is the only optional feature that uses public. Keep its
-- projections scoped; never reset unrelated public objects or baseline schema
-- usage that an operator may have reviewed separately.
REVOKE USAGE ON SCHEMA public FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON public.routing_active_release,
    public.routing_groups, public.routing_routes FROM :"runtime_role";

-- Assistant execution extends the baseline core_gift schema. Its tables and
-- sequence are optional; risk_key, policy, qualifications and lock_eligibility
-- remain baseline grants and are deliberately untouched.
REVOKE ALL PRIVILEGES ON core_gift.decisions, core_gift.registration_profiles,
    core_gift.fingerprints FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON SEQUENCE core_gift.fingerprints_id_seq
    FROM :"runtime_role";

COMMIT;
