-- Second-batch legacy CREDIT v2. Not registered until split-ledger validation.
CREATE TABLE core_tool_market.credit_policy(
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton), version BIGINT NOT NULL CHECK(version>0),
 platform_account_id BIGINT REFERENCES core_identity.accounts(id), platform_user_id BIGINT REFERENCES core_identity.users(id), enabled BOOLEAN NOT NULL DEFAULT FALSE,updated_by BIGINT REFERENCES core_identity.users(id),updated_at BIGINT NOT NULL DEFAULT 0, fee_bps BIGINT NOT NULL CHECK(fee_bps BETWEEN 0 AND 10000));
INSERT INTO core_tool_market.credit_policy(singleton,version,fee_bps) VALUES(TRUE,1,0);
CREATE TABLE core_tool_market.credit_quotes(
 id TEXT PRIMARY KEY, seller_user_id BIGINT REFERENCES core_identity.users(id), seller_account_id BIGINT REFERENCES core_identity.accounts(id),
 platform_account_id BIGINT REFERENCES core_identity.accounts(id), fee_bps BIGINT NOT NULL CHECK(fee_bps BETWEEN 0 AND 10000), policy_version BIGINT NOT NULL,
 snapshot JSONB NOT NULL, approved BOOLEAN NOT NULL DEFAULT FALSE, suspended BOOLEAN NOT NULL DEFAULT FALSE, visibility TEXT NOT NULL DEFAULT 'public' CHECK(visibility IN ('public','private')),approved_by BIGINT REFERENCES core_identity.users(id),approved_at BIGINT NOT NULL DEFAULT 0, ceiling_credits BIGINT NOT NULL CHECK(ceiling_credits BETWEEN 0 AND 9007199254740991));
CREATE UNIQUE INDEX credit_quote_tool_version ON core_tool_market.credit_quotes((snapshot->>'tool_id'),(snapshot->>'version_id'));
-- Core code-owned builtins, installed explicitly from the pinned catalog.
-- Runtime SELECT only: no publisher or Go host may mark an arbitrary tool builtin.
CREATE TABLE core_tool_market.credit_builtin_bindings(quote_id TEXT PRIMARY KEY REFERENCES core_tool_market.credit_quotes(id),kind TEXT NOT NULL,input_schema_digest TEXT NOT NULL CHECK(input_schema_digest ~ '^[a-f0-9]{64}$'));
CREATE TABLE core_tool_market.credit_consents(
 id TEXT PRIMARY KEY, actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id), payer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 quote_id TEXT NOT NULL REFERENCES core_tool_market.credit_quotes(id), request JSONB NOT NULL, created_at BIGINT NOT NULL, revoked_at BIGINT NOT NULL DEFAULT 0,
 reserved_calls BIGINT NOT NULL DEFAULT 0 CHECK(reserved_calls>=0), successful_calls BIGINT NOT NULL DEFAULT 0 CHECK(successful_calls>=0),
 reserved_credits BIGINT NOT NULL DEFAULT 0 CHECK(reserved_credits>=0), spent_credits BIGINT NOT NULL DEFAULT 0 CHECK(spent_credits>=0),
 CHECK(reserved_calls+successful_calls<=(request->>'max_calls')::BIGINT),CHECK(reserved_credits+spent_credits<=(request->>'max_total_quota')::BIGINT));
CREATE TABLE core_tool_market.credit_budgets(
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id), scope TEXT NOT NULL CHECK(scope IN ('account','client','tool')),scope_id TEXT NOT NULL,
 request JSONB NOT NULL,limit_credits BIGINT NOT NULL CHECK(limit_credits BETWEEN 0 AND 9007199254740991),reserved_credits BIGINT NOT NULL DEFAULT 0 CHECK(reserved_credits>=0),spent_credits BIGINT NOT NULL DEFAULT 0 CHECK(spent_credits>=0),
 PRIMARY KEY(actor_user_id,scope,scope_id),CHECK(reserved_credits+spent_credits<=limit_credits));
CREATE TABLE core_tool_market.credit_tokens(
 id TEXT PRIMARY KEY, actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),payer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 request JSONB NOT NULL, token_hash BYTEA NOT NULL UNIQUE CHECK(octet_length(token_hash)=32),auth_version BIGINT NOT NULL,
 created_at BIGINT NOT NULL,revoked_at BIGINT NOT NULL DEFAULT 0);
CREATE TABLE core_tool_market.credit_access_operations(
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),operation_id TEXT NOT NULL,
 kind TEXT NOT NULL,request_hash TEXT NOT NULL,target_id TEXT NOT NULL,created_at BIGINT NOT NULL,details JSONB,
 PRIMARY KEY(actor_user_id,operation_id));
CREATE TABLE core_tool_market.credit_calls(
 id TEXT PRIMARY KEY, actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),session_id BIGINT REFERENCES core_identity.credentials(id),market_token_id TEXT REFERENCES core_tool_market.credit_tokens(id),market_oauth_family_id TEXT REFERENCES core_market_oauth.families(id),auth_version BIGINT NOT NULL,
 payer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),consent_id TEXT NOT NULL REFERENCES core_tool_market.credit_consents(id),
 quote_id TEXT NOT NULL REFERENCES core_tool_market.credit_quotes(id),snapshot JSONB NOT NULL,
 held_credits BIGINT NOT NULL CHECK(held_credits BETWEEN 0 AND 9007199254740991), reserve_journal_id BIGINT REFERENCES core_billing.ledger_journals(id),
 state TEXT NOT NULL CHECK(state IN ('reserved','running','unknown','settled','released')),
 dispatch_id TEXT UNIQUE, result_digest TEXT, usage JSONB, receipt JSONB NOT NULL,
 settlement_journal_id BIGINT REFERENCES core_billing.ledger_journals(id),consumption_id TEXT UNIQUE REFERENCES core_charging.external_consumption(id),
 created_at BIGINT NOT NULL,resolve_by BIGINT NOT NULL,started_at BIGINT NOT NULL DEFAULT 0,finished_at BIGINT NOT NULL DEFAULT 0,CHECK(started_at=0 OR started_at>=created_at),CHECK(finished_at=0 OR finished_at>=GREATEST(created_at,started_at)),CHECK(resolve_by>created_at),CHECK(num_nonnulls(session_id,market_token_id,market_oauth_family_id)=1));
CREATE FUNCTION core_tool_market.guard_credit_quote() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (to_jsonb(NEW)-ARRAY['approved','suspended','visibility','approved_by','approved_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['approved','suspended','visibility','approved_by','approved_at']) THEN RAISE EXCEPTION 'invalid credit quote update' USING ERRCODE='23514'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER credit_quotes_immutable BEFORE UPDATE OR DELETE ON core_tool_market.credit_quotes FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_credit_quote();
CREATE TRIGGER credit_quotes_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_quotes FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE TRIGGER credit_access_ops_immutable BEFORE UPDATE OR DELETE ON core_tool_market.credit_access_operations FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE TRIGGER credit_access_ops_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_access_operations FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE FUNCTION core_tool_market.guard_credit_consent() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (to_jsonb(NEW)-ARRAY['revoked_at','reserved_calls','successful_calls','reserved_credits','spent_credits']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['revoked_at','reserved_calls','successful_calls','reserved_credits','spent_credits']) OR (OLD.revoked_at<>0 AND NEW.revoked_at<>OLD.revoked_at) OR NEW.successful_calls<OLD.successful_calls OR NEW.spent_credits<OLD.spent_credits THEN
 RAISE EXCEPTION 'invalid credit consent transition' USING ERRCODE='23514'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER credit_consents_guard BEFORE UPDATE OR DELETE ON core_tool_market.credit_consents FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_credit_consent();
CREATE TRIGGER credit_consents_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_consents FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE FUNCTION core_tool_market.guard_credit_token() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM (to_jsonb(OLD)-'revoked_at') OR (OLD.revoked_at<>0 AND NEW.revoked_at<>OLD.revoked_at) THEN RAISE EXCEPTION 'invalid credit token transition' USING ERRCODE='23514'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER credit_tokens_guard BEFORE UPDATE OR DELETE ON core_tool_market.credit_tokens FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_credit_token();
CREATE TRIGGER credit_tokens_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_tokens FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE FUNCTION core_tool_market.guard_credit_budget() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (NEW.actor_user_id,NEW.scope,NEW.scope_id) IS DISTINCT FROM (OLD.actor_user_id,OLD.scope,OLD.scope_id) OR NEW.spent_credits<OLD.spent_credits THEN RAISE EXCEPTION 'invalid credit budget transition' USING ERRCODE='23514'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER credit_budgets_guard BEFORE UPDATE OR DELETE ON core_tool_market.credit_budgets FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_credit_budget();
CREATE TRIGGER credit_budgets_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_budgets FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE FUNCTION core_tool_market.guard_credit_call() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR (to_jsonb(NEW)-ARRAY['state','dispatch_id','result_digest','usage','receipt','settlement_journal_id','started_at','finished_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','dispatch_id','result_digest','usage','receipt','settlement_journal_id','started_at','finished_at'])
 OR NOT ((OLD.state='reserved' AND NEW.state IN ('running','released')) OR (OLD.state='running' AND NEW.state IN ('unknown','settled')) OR (OLD.state='unknown' AND NEW.state='settled') OR (OLD.state IN ('running','unknown') AND NEW.state='released' AND (EXISTS(SELECT FROM core_tool_market.credit_supplier_receipts p WHERE p.call_id=OLD.id AND p.outcome='failure') OR EXISTS(SELECT FROM core_tool_market.credit_fixed_witness_receipts p WHERE p.call_id=OLD.id AND p.outcome='failure'))))
 OR (OLD.started_at<>0 AND NEW.started_at<>OLD.started_at) OR (OLD.finished_at<>0 AND NEW.finished_at<>OLD.finished_at) OR (NEW.state IN ('settled','released') AND NEW.finished_at=0) OR (NEW.state IN ('running','unknown','settled') AND NEW.started_at=0)
 OR (OLD.dispatch_id IS NOT NULL AND NEW.dispatch_id IS DISTINCT FROM OLD.dispatch_id)
 OR (NEW.receipt->>'state') IS DISTINCT FROM NEW.state
 THEN RAISE EXCEPTION 'invalid credit call transition' USING ERRCODE='23514'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER credit_calls_guard BEFORE UPDATE OR DELETE ON core_tool_market.credit_calls FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_credit_call();
CREATE TRIGGER credit_calls_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_calls FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();

-- One original supplier receipt can authorize exactly one financial call.
CREATE TABLE core_tool_market.credit_supplier_receipts(
 receipt_hash TEXT PRIMARY KEY CHECK(receipt_hash ~ '^[a-f0-9]{64}$'),
 call_id TEXT NOT NULL UNIQUE REFERENCES core_tool_market.credit_calls(id),
 raw_receipt_hash TEXT NOT NULL CHECK(raw_receipt_hash ~ '^[a-f0-9]{64}$'),
 result_digest TEXT NOT NULL CHECK(result_digest ~ '^[a-f0-9]{64}$'),
 raw_result BYTEA NOT NULL CHECK(octet_length(raw_result)<=2097152),outcome TEXT NOT NULL CHECK(outcome IN ('success','failure')));
CREATE TRIGGER credit_supplier_receipts_immutable BEFORE UPDATE OR DELETE ON core_tool_market.credit_supplier_receipts FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE TRIGGER credit_supplier_receipts_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_supplier_receipts FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();

CREATE TABLE core_tool_market.credit_services(
 service_id TEXT NOT NULL,owner_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 active_version_id TEXT NOT NULL DEFAULT '',paused BOOLEAN NOT NULL DEFAULT FALSE,deleted BOOLEAN NOT NULL DEFAULT FALSE,suspended BOOLEAN NOT NULL DEFAULT FALSE,
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),updated_by BIGINT NOT NULL REFERENCES core_identity.users(id),updated_at BIGINT NOT NULL,PRIMARY KEY(owner_user_id,service_id));
CREATE FUNCTION core_tool_market.guard_credit_service() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') OR NEW.service_id<>OLD.service_id OR NEW.owner_user_id<>OLD.owner_user_id OR NEW.revision<>OLD.revision+1 OR (OLD.deleted AND NOT NEW.deleted) THEN RAISE EXCEPTION 'invalid service lifecycle' USING ERRCODE='23514'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER credit_services_guard BEFORE UPDATE OR DELETE ON core_tool_market.credit_services FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_credit_service();
CREATE TRIGGER credit_services_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_services FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();

-- Installer-only immutable review. Runtime can neither select its own proof mode
-- nor choose a result schema/witness role from an extension request.
CREATE TABLE core_tool_market.credit_result_contracts(
 quote_id TEXT PRIMARY KEY REFERENCES core_tool_market.credit_quotes(id),
 proof_mode TEXT NOT NULL CHECK(proof_mode IN ('supplier_signed','core_host_fixed_success')),
 witness_role TEXT NOT NULL DEFAULT '',
 output_schema JSONB NOT NULL CHECK(octet_length(output_schema::TEXT)<=65536),
 output_schema_digest TEXT NOT NULL CHECK(output_schema_digest ~ '^[a-f0-9]{64}$'),
 CHECK((proof_mode='supplier_signed' AND witness_role='') OR (proof_mode='core_host_fixed_success' AND length(witness_role) BETWEEN 1 AND 128)));
CREATE TRIGGER credit_result_contracts_immutable BEFORE UPDATE OR DELETE ON core_tool_market.credit_result_contracts FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE TRIGGER credit_result_contracts_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_result_contracts FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();

CREATE TABLE core_tool_market.credit_fixed_witness_receipts(
 call_id TEXT PRIMARY KEY REFERENCES core_tool_market.credit_calls(id),witness_role TEXT NOT NULL,
 raw_result BYTEA NOT NULL CHECK(octet_length(raw_result)<=2097152),
 raw_digest TEXT NOT NULL CHECK(raw_digest ~ '^[a-f0-9]{64}$'),result_digest TEXT NOT NULL CHECK(result_digest ~ '^[a-f0-9]{64}$'),
 outcome TEXT NOT NULL CHECK(outcome IN ('success','failure')));
CREATE TRIGGER credit_fixed_witness_receipts_immutable BEFORE UPDATE OR DELETE ON core_tool_market.credit_fixed_witness_receipts FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE TRIGGER credit_fixed_witness_receipts_no_truncate BEFORE TRUNCATE ON core_tool_market.credit_fixed_witness_receipts FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();

CREATE UNIQUE INDEX credit_reports_once ON core_tool_market.credit_access_operations(kind,target_id) WHERE kind IN ('call-report','call-report-review');

ALTER TABLE core_charging.external_consumption ALTER COLUMN key_id DROP NOT NULL;
ALTER TABLE core_charging.external_consumption ADD COLUMN market_token_id TEXT REFERENCES core_tool_market.credit_tokens(id),ADD COLUMN market_oauth_family_id TEXT REFERENCES core_market_oauth.families(id),ADD CONSTRAINT external_credential_origin CHECK(num_nonnulls(key_id,market_token_id,market_oauth_family_id)=1 AND (domain='toolmarket' OR (market_token_id IS NULL AND market_oauth_family_id IS NULL)));

-- Narrow installer-owned lock bridge for a SELECT-only drawing consumer.
-- PostgreSQL 18 SELECT ... FOR SHARE otherwise requires UPDATE on each table.
-- No data, authority receipt, payer, mutation or model dispatch is returned.
CREATE FUNCTION core_tool_market.lock_builtin_context(p_call_id TEXT) RETURNS VOID
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path=pg_catalog SET lock_timeout='250ms' AS $$
BEGIN
 IF p_call_id IS NULL OR length(p_call_id) NOT BETWEEN 1 AND 128
    OR current_setting('transaction_isolation')<>'serializable' THEN
   RAISE EXCEPTION 'invalid builtin lock context' USING ERRCODE='22023';
 END IF;
 PERFORM 1 FROM core_tool_market.credit_calls c
 JOIN core_tool_market.credit_consents g ON g.id=c.consent_id
 JOIN core_tool_market.credit_quotes q ON q.id=c.quote_id
 JOIN core_tool_market.credit_builtin_bindings b ON b.quote_id=q.id
 WHERE c.id=p_call_id FOR SHARE OF c,g,q,b;
 IF NOT FOUND THEN
   RAISE EXCEPTION 'builtin lock context unavailable' USING ERRCODE='42501';
 END IF;
END $$;
REVOKE ALL ON FUNCTION core_tool_market.lock_builtin_context(TEXT) FROM PUBLIC;

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
