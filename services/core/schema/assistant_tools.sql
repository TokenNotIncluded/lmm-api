-- Fresh installation after native identity and Core assistant. No startup DDL.
CREATE SCHEMA core_assistant_tools;
CREATE TABLE core_assistant_tools.confirmations (
 id TEXT PRIMARY KEY CHECK(id ~ '^[a-f0-9]{64}$'),
 operation_id TEXT NOT NULL UNIQUE CHECK(operation_id ~ '^[a-f0-9]{64}$'),
 turn_id TEXT NOT NULL CHECK(turn_id ~ '^[a-f0-9]{64}$'),
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 credential_id BIGINT NOT NULL REFERENCES core_identity.credentials(id),
 auth_version BIGINT NOT NULL CHECK(auth_version>0),
 web_session_sid TEXT NOT NULL REFERENCES core_identity.web_sessions(sid),
 tool_name TEXT NOT NULL CHECK(tool_name IN ('set_overview_greeting','create_site_issue','update_site_issue','request_human_support','prepare_admin_site_policy_change')),
 tool_version TEXT NOT NULL,
 arguments_sha256 BYTEA NOT NULL CHECK(octet_length(arguments_sha256)=32),
 draft_sha256 BYTEA NOT NULL CHECK(octet_length(draft_sha256)=32),
 preview JSONB NOT NULL CHECK(jsonb_typeof(preview)='object' AND octet_length(preview::text)<=16384),
 resource_kind TEXT NOT NULL,
 resource_id TEXT NOT NULL,
 resource_version TEXT NOT NULL,
 expires_at BIGINT NOT NULL,
 nonce_sha256 BYTEA CHECK(nonce_sha256 IS NULL OR octet_length(nonce_sha256)=32),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','approved','rejected')),
 confirmed_at BIGINT,
 CHECK((state='pending')=(confirmed_at IS NULL))
);
CREATE FUNCTION core_assistant_tools.guard_confirmation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR OLD.state<>'pending' OR
  (to_jsonb(OLD)-ARRAY['nonce_sha256','state','confirmed_at']) IS DISTINCT FROM
  (to_jsonb(NEW)-ARRAY['nonce_sha256','state','confirmed_at']) THEN
  RAISE EXCEPTION 'immutable assistant confirmation' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER assistant_confirmation_immutable BEFORE UPDATE OR DELETE ON core_assistant_tools.confirmations FOR EACH ROW EXECUTE FUNCTION core_assistant_tools.guard_confirmation();
CREATE TRIGGER assistant_confirmation_no_truncate BEFORE TRUNCATE ON core_assistant_tools.confirmations FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON SCHEMA core_assistant_tools FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_assistant_tools FROM PUBLIC;
