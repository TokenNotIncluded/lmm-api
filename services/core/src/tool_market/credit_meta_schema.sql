-- Fresh installer asset. Runtime never creates or upgrades this schema.
CREATE TABLE core_tool_market.credit_meta_delegations(
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 credential_kind TEXT NOT NULL CHECK(credential_kind IN ('personal','oauth')),
 credential_id TEXT NOT NULL,client_id TEXT NOT NULL,auth_version BIGINT NOT NULL,
 personal_token_id TEXT REFERENCES core_tool_market.credit_tokens(id),
 oauth_family_id TEXT REFERENCES core_market_oauth.families(id),
 enabled BOOLEAN NOT NULL,max_total_quota BIGINT NOT NULL CHECK(max_total_quota BETWEEN 0 AND 9007199254740991),
 expires_at BIGINT NOT NULL,updated_at BIGINT NOT NULL,revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 PRIMARY KEY(actor_user_id,credential_kind,credential_id),
 CHECK((credential_kind='personal' AND personal_token_id IS NOT NULL AND personal_token_id=credential_id AND oauth_family_id IS NULL) OR (credential_kind='oauth' AND oauth_family_id IS NOT NULL AND oauth_family_id=credential_id AND personal_token_id IS NULL)),
 CHECK((enabled AND expires_at>updated_at) OR (NOT enabled AND expires_at=0 AND max_total_quota=0))
);
CREATE FUNCTION core_tool_market.guard_meta_delegation() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (to_jsonb(NEW)-ARRAY['enabled','max_total_quota','expires_at','updated_at','revision']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['enabled','max_total_quota','expires_at','updated_at','revision']) OR NEW.updated_at<OLD.updated_at OR NEW.revision<>OLD.revision+1 THEN
 RAISE EXCEPTION 'invalid market delegation transition' USING ERRCODE='23514';END IF;RETURN NEW;
END $$;
CREATE TRIGGER credit_meta_delegation_guard BEFORE UPDATE OR DELETE ON core_tool_market.credit_meta_delegations FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_meta_delegation();
CREATE TRIGGER credit_meta_delegation_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_meta_delegations FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
-- Metadata delegation may only tighten existing immutable grant identities.
-- Held/spent counters and identity bindings retain the original constraints.
CREATE OR REPLACE FUNCTION core_tool_market.guard_credit_consent() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (to_jsonb(NEW)-ARRAY['request','revoked_at','reserved_calls','successful_calls','reserved_credits','spent_credits']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['request','revoked_at','reserved_calls','successful_calls','reserved_credits','spent_credits'])
 OR (NEW.request-ARRAY['max_price_quota','max_total_quota','max_calls','expires_at']) IS DISTINCT FROM (OLD.request-ARRAY['max_price_quota','max_total_quota','max_calls','expires_at'])
 OR NEW.request->>'max_price_quota' IS NULL OR NEW.request->>'max_total_quota' IS NULL OR NEW.request->>'max_calls' IS NULL OR NEW.request->>'expires_at' IS NULL
 OR (NEW.request->>'max_price_quota')::BIGINT NOT BETWEEN 0 AND (OLD.request->>'max_price_quota')::BIGINT
 OR (NEW.request->>'max_total_quota')::BIGINT NOT BETWEEN 0 AND (OLD.request->>'max_total_quota')::BIGINT
 OR (NEW.request->>'max_calls')::BIGINT NOT BETWEEN 1 AND (OLD.request->>'max_calls')::BIGINT
 OR (NEW.request->>'expires_at')::BIGINT NOT BETWEEN 1 AND (OLD.request->>'expires_at')::BIGINT
 OR (OLD.revoked_at<>0 AND NEW.revoked_at<>OLD.revoked_at) OR NEW.successful_calls<OLD.successful_calls OR NEW.spent_credits<OLD.spent_credits THEN
 RAISE EXCEPTION 'invalid credit consent transition' USING ERRCODE='23514';END IF;RETURN NEW;
END $$;
