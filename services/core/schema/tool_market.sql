-- Fresh-install only, after identity/ledger/charging. Never run at startup.
CREATE SCHEMA core_tool_market;
-- Installer-reviewed publishers, runtime SELECT only.
CREATE TABLE core_tool_market.publishers(user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id), active BOOLEAN NOT NULL DEFAULT FALSE);
CREATE TABLE core_tool_market.quotes(
 id TEXT PRIMARY KEY, seller_user_id BIGINT NOT NULL REFERENCES core_identity.users(id), seller_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 quote JSONB NOT NULL, credits BIGINT NOT NULL CHECK(credits>=0), policy TEXT NOT NULL CHECK(policy='exact-usd-micros-500k-v1'));
CREATE UNIQUE INDEX quote_release_version ON core_tool_market.quotes((quote->>'release'),(quote->>'tool'),(quote->>'version'));
CREATE TABLE core_tool_market.approvals(
 id TEXT PRIMARY KEY, grant_id TEXT NOT NULL UNIQUE, actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id), payer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 intent JSONB NOT NULL, evidence JSONB NOT NULL, revoked BOOLEAN NOT NULL DEFAULT FALSE, used_calls BIGINT NOT NULL DEFAULT 0 CHECK(used_calls>=0), used_amount BIGINT NOT NULL DEFAULT 0 CHECK(used_amount>=0));
CREATE TABLE core_tool_market.calls(
 id TEXT PRIMARY KEY, approval_id TEXT NOT NULL REFERENCES core_tool_market.approvals(id), actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 payer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id), seller_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id), call JSONB NOT NULL,
 reserve_journal_id BIGINT REFERENCES core_billing.ledger_journals(id), receipt JSONB NOT NULL, result_hash TEXT, consumption_id TEXT UNIQUE REFERENCES core_charging.external_consumption(id), CHECK(payer_account_id<>seller_account_id OR (receipt->>'credits')::BIGINT=0));
CREATE FUNCTION core_tool_market.guard_quote() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'immutable tool quote' USING ERRCODE='55000'; END $$;
CREATE TRIGGER quotes_immutable BEFORE UPDATE OR DELETE ON core_tool_market.quotes FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE TRIGGER quotes_no_truncate BEFORE TRUNCATE ON core_tool_market.quotes FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_quote();
CREATE FUNCTION core_tool_market.guard_approval() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') THEN RAISE EXCEPTION 'immutable approval history' USING ERRCODE='55000'; END IF;
 IF (to_jsonb(NEW)-'revoked'-'used_calls'-'used_amount') IS DISTINCT FROM (to_jsonb(OLD)-'revoked'-'used_calls'-'used_amount')
 OR (OLD.revoked AND NOT NEW.revoked) OR NEW.used_calls<OLD.used_calls OR NEW.used_amount<OLD.used_amount
 OR NEW.used_calls>(NEW.intent->>'max_calls')::BIGINT OR NEW.used_amount>(NEW.intent->>'max_amount')::BIGINT
 THEN RAISE EXCEPTION 'invalid approval update' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER approvals_guard BEFORE UPDATE OR DELETE ON core_tool_market.approvals FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_approval();
CREATE TRIGGER approvals_no_truncate BEFORE TRUNCATE ON core_tool_market.approvals FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_approval();
CREATE FUNCTION core_tool_market.guard_call() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') THEN RAISE EXCEPTION 'immutable call history' USING ERRCODE='55000'; END IF;
 IF (to_jsonb(NEW)-'receipt'-'result_hash') IS DISTINCT FROM (to_jsonb(OLD)-'receipt'-'result_hash')
 OR (NEW.receipt-'state'-'journal_id') IS DISTINCT FROM (OLD.receipt-'state'-'journal_id')
 OR NOT ((OLD.receipt->>'state'='reserved' AND NEW.receipt->>'state' IN ('dispatched','released')) OR (OLD.receipt->>'state'='dispatched' AND NEW.receipt->>'state'='committed'))
 OR (NEW.receipt->>'state'='committed') IS DISTINCT FROM (NEW.result_hash IS NOT NULL)
 THEN RAISE EXCEPTION 'invalid call transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER calls_guard BEFORE UPDATE OR DELETE ON core_tool_market.calls FOR EACH ROW EXECUTE FUNCTION core_tool_market.guard_call();
CREATE TRIGGER calls_no_truncate BEFORE TRUNCATE ON core_tool_market.calls FOR EACH STATEMENT EXECUTE FUNCTION core_tool_market.guard_call();
