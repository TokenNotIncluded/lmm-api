-- Fresh installation only. Distinct resource and token namespace from model OAuth.
CREATE SCHEMA core_market_oauth;
CREATE TABLE core_market_oauth.clients(
 id TEXT PRIMARY KEY CHECK(id ~ '^lmm-mcp-[a-f0-9]{64}$'),issuer TEXT NOT NULL,
 metadata JSONB NOT NULL CHECK(jsonb_typeof(metadata)='object'),redirect_templates TEXT[] NOT NULL,
 scopes TEXT[] NOT NULL,refresh_enabled BOOLEAN NOT NULL,created_at BIGINT NOT NULL,
 revoked_at TIMESTAMPTZ
);
CREATE TABLE core_market_oauth.requests(
 id BYTEA PRIMARY KEY CHECK(octet_length(id)=32),csrf BYTEA NOT NULL UNIQUE CHECK(octet_length(csrf)=32),
 client_id TEXT NOT NULL REFERENCES core_market_oauth.clients(id),issuer TEXT NOT NULL,resource TEXT NOT NULL,
 redirect_uri TEXT NOT NULL,state TEXT NOT NULL,challenge TEXT NOT NULL,scopes TEXT[] NOT NULL,
 user_id BIGINT REFERENCES core_identity.users(id),account_id BIGINT REFERENCES core_identity.accounts(id),
 user_version BIGINT,sid TEXT REFERENCES core_identity.web_sessions(sid),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()+interval '300 seconds',consumed_at TIMESTAMPTZ,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '300 seconds'),
 CHECK((user_id IS NULL AND account_id IS NULL AND user_version IS NULL AND sid IS NULL) OR (user_id IS NOT NULL AND account_id IS NOT NULL AND user_version>0 AND sid IS NOT NULL))
);
CREATE TABLE core_market_oauth.families(
 id TEXT PRIMARY KEY CHECK(id ~ '^lmm_mg_[a-f0-9]{64}$'),client_id TEXT NOT NULL REFERENCES core_market_oauth.clients(id),
 issuer TEXT NOT NULL,resource TEXT NOT NULL,user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),user_version BIGINT NOT NULL CHECK(user_version>0),
 scopes TEXT[] NOT NULL,budget_credits BIGINT NOT NULL CHECK(budget_credits BETWEEN 0 AND 9007199254740991),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),expires_at TIMESTAMPTZ NOT NULL,revoked_at TIMESTAMPTZ,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '30 days')
);
CREATE TABLE core_market_oauth.codes(
 digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),family_id TEXT NOT NULL REFERENCES core_market_oauth.families(id),
 redirect_uri TEXT NOT NULL,challenge TEXT NOT NULL,created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),expires_at TIMESTAMPTZ NOT NULL,consumed_at TIMESTAMPTZ,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '121 seconds')
);
CREATE TABLE core_market_oauth.tokens(
 digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),family_id TEXT NOT NULL REFERENCES core_market_oauth.families(id),
 kind TEXT NOT NULL CHECK(kind IN ('access','refresh')),scopes TEXT[] NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),expires_at TIMESTAMPTZ NOT NULL,consumed_at TIMESTAMPTZ,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '30 days')
);
CREATE FUNCTION core_market_oauth.scope_set(s TEXT[]) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE STRICT SET search_path=pg_catalog AS $$
 SELECT cardinality(s) BETWEEN 1 AND 3 AND 'market:discover'=ANY(s)
 AND s<@ARRAY['market:discover','market:invoke','market:manage']::text[]
 AND cardinality(s)=(SELECT count(DISTINCT v) FROM unnest(s) v)
$$;
ALTER TABLE core_market_oauth.clients ADD CHECK(core_market_oauth.scope_set(scopes));
ALTER TABLE core_market_oauth.requests ADD CHECK(core_market_oauth.scope_set(scopes));
ALTER TABLE core_market_oauth.families ADD CHECK(core_market_oauth.scope_set(scopes));
ALTER TABLE core_market_oauth.tokens ADD CHECK(core_market_oauth.scope_set(scopes));
CREATE FUNCTION core_market_oauth.guard_record() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE field_ TEXT:=CASE WHEN TG_TABLE_NAME='clients' THEN 'revoked_at' ELSE 'consumed_at' END;
BEGIN
 IF TG_OP='DELETE' OR (to_jsonb(NEW)-field_) IS DISTINCT FROM (to_jsonb(OLD)-field_)
 OR (to_jsonb(OLD)->field_<>'null'::jsonb AND to_jsonb(NEW)->field_ IS DISTINCT FROM to_jsonb(OLD)->field_)
 OR (to_jsonb(NEW)->field_='null'::jsonb AND to_jsonb(OLD)->field_<>'null'::jsonb) THEN
 RAISE EXCEPTION 'immutable market OAuth binding' USING ERRCODE='23514';END IF;RETURN NEW;
END $$;
CREATE TRIGGER market_client_immutable BEFORE UPDATE OR DELETE ON core_market_oauth.clients FOR EACH ROW EXECUTE FUNCTION core_market_oauth.guard_record();
CREATE TRIGGER market_request_immutable BEFORE UPDATE OR DELETE ON core_market_oauth.requests FOR EACH ROW EXECUTE FUNCTION core_market_oauth.guard_record();
CREATE TRIGGER market_code_immutable BEFORE UPDATE OR DELETE ON core_market_oauth.codes FOR EACH ROW EXECUTE FUNCTION core_market_oauth.guard_record();
CREATE TRIGGER market_token_immutable BEFORE UPDATE OR DELETE ON core_market_oauth.tokens FOR EACH ROW EXECUTE FUNCTION core_market_oauth.guard_record();
CREATE INDEX market_oauth_tokens_family ON core_market_oauth.tokens(family_id);
CREATE FUNCTION core_market_oauth.guard_family() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM (to_jsonb(OLD)-'revoked_at') OR (OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at) THEN RAISE EXCEPTION 'immutable market OAuth grant' USING ERRCODE='23514'; END IF;RETURN NEW;
END $$;
CREATE TRIGGER market_oauth_family_immutable BEFORE UPDATE OR DELETE ON core_market_oauth.families FOR EACH ROW EXECUTE FUNCTION core_market_oauth.guard_family();
REVOKE ALL ON SCHEMA core_market_oauth FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_market_oauth FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_market_oauth FROM PUBLIC;
