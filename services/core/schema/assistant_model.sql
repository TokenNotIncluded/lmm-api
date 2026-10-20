-- Fresh installation only, after identity and charging. Core runtime owns all writes.
CREATE SCHEMA core_assistant;
CREATE TABLE core_assistant.conversations (
 id TEXT PRIMARY KEY CHECK (id ~ '^lmm_cv_[a-f0-9]{64}$'),
 owner_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 owner_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 creation_key TEXT NOT NULL CHECK (octet_length(creation_key) BETWEEN 1 AND 128),
 created_at BIGINT NOT NULL,
 UNIQUE(owner_user_id,creation_key)
);
CREATE TABLE core_assistant.grants (
 id TEXT PRIMARY KEY CHECK(id ~ '^lmm_ad_[a-f0-9]{64}$'),
 conversation_id TEXT NOT NULL REFERENCES core_assistant.conversations(id),
 turn_id TEXT NOT NULL CHECK (turn_id ~ '^[A-Za-z0-9_.-]{1,124}$'),
 session_id BIGINT NOT NULL REFERENCES core_identity.credentials(id),
 auth_version BIGINT NOT NULL CHECK(auth_version>0),
 policy JSONB NOT NULL CHECK(jsonb_typeof(policy)='object'),
 price JSONB NOT NULL CHECK(jsonb_typeof(price)='object'),
 issued_at BIGINT NOT NULL, expires_at BIGINT NOT NULL,
 revoked BOOLEAN NOT NULL DEFAULT FALSE,
 CHECK(expires_at>issued_at AND expires_at-issued_at<=900),
 UNIQUE(conversation_id,turn_id)
);
ALTER TABLE core_charging.requests ADD CONSTRAINT assistant_grant_source FOREIGN KEY(assistant_grant_id) REFERENCES core_assistant.grants(id);
CREATE UNIQUE INDEX assistant_one_charge_per_grant ON core_charging.requests(assistant_grant_id) WHERE assistant_grant_id IS NOT NULL;
CREATE TABLE core_assistant.operations (
 position BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
 grant_id TEXT PRIMARY KEY REFERENCES core_assistant.grants(id),
 request_id TEXT NOT NULL UNIQUE REFERENCES core_charging.requests(id),
 text_sha256 BYTEA NOT NULL CHECK(octet_length(text_sha256)=32),
 operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 1 AND 128),
 text TEXT NOT NULL CHECK(octet_length(text) BETWEEN 1 AND 8192),
 gift_receipt JSONB CHECK(gift_receipt IS NULL OR jsonb_typeof(gift_receipt)='object'),
 gift_tool_id TEXT CHECK(gift_tool_id IS NULL OR octet_length(gift_tool_id) BETWEEN 1 AND 128),
 gift_response_sha256 BYTEA CHECK(gift_response_sha256 IS NULL OR octet_length(gift_response_sha256)=32),
 answer_text TEXT CHECK(octet_length(answer_text) BETWEEN 1 AND 32768),
 completed_at BIGINT,
 CHECK((answer_text IS NULL)=(completed_at IS NULL)),
 CHECK((gift_tool_id IS NULL)=(gift_response_sha256 IS NULL)),
 CHECK(gift_receipt IS NULL OR gift_response_sha256 IS NOT NULL)
);
-- At most one unsettled turn per conversation. Checked under conversation lock
-- by the service; no process-local lock grants authority.
CREATE INDEX assistant_grant_conversation ON core_assistant.grants(conversation_id,issued_at,id);
CREATE FUNCTION core_assistant.guard_conversation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'immutable assistant conversation' USING ERRCODE='23514'; END $$;
CREATE TRIGGER assistant_conversation_immutable BEFORE UPDATE OR DELETE ON core_assistant.conversations FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_conversation();
CREATE FUNCTION core_assistant.guard_grant() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR (to_jsonb(NEW)-'revoked') IS DISTINCT FROM (to_jsonb(OLD)-'revoked') OR (OLD.revoked AND NOT NEW.revoked) THEN
  RAISE EXCEPTION 'immutable assistant grant' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER assistant_grant_immutable BEFORE UPDATE OR DELETE ON core_assistant.grants FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_grant();
CREATE FUNCTION core_assistant.guard_operation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR (to_jsonb(NEW)-ARRAY['answer_text','completed_at','gift_receipt','gift_tool_id','gift_response_sha256']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['answer_text','completed_at','gift_receipt','gift_tool_id','gift_response_sha256']) OR OLD.answer_text IS NOT NULL THEN
  RAISE EXCEPTION 'immutable assistant operation' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER assistant_operation_immutable BEFORE UPDATE OR DELETE ON core_assistant.operations FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_operation();
-- Deferred checks see the completed transaction: an answer cannot commit before
-- its charge, nor can a model settlement commit without its immutable answer.
CREATE FUNCTION core_assistant.check_terminal_answer() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE request TEXT; r RECORD;
BEGIN
 IF TG_TABLE_SCHEMA='core_charging' THEN
  IF NEW.assistant_grant_id IS NULL THEN RETURN NULL; END IF;
  request:=NEW.id;
 ELSE request:=NEW.request_id;
 END IF;
 SELECT q.state,q.assistant_grant_id,q.assistant_sponsor_account_id,q.payer_account_id,q.owner_account_id,q.actor_user_id,g.policy,o.grant_id,o.answer_text,o.gift_response_sha256,o.gift_receipt INTO r FROM core_charging.requests q JOIN core_assistant.grants g ON g.id=q.assistant_grant_id LEFT JOIN core_assistant.operations o ON o.request_id=q.id WHERE q.id=request;
 IF r.grant_id IS NULL OR r.assistant_grant_id IS DISTINCT FROM r.grant_id OR ((r.state='settled') IS DISTINCT FROM (r.answer_text IS NOT NULL)) THEN
  RAISE EXCEPTION 'assistant answer and settlement must commit together' USING ERRCODE='23514';
 END IF;
 IF (r.assistant_sponsor_account_id IS NULL AND (r.payer_account_id<>r.owner_account_id OR coalesce(r.policy->'sponsor','null'::jsonb)<>'null'::jsonb)) OR (r.assistant_sponsor_account_id IS NOT NULL AND (r.payer_account_id IS DISTINCT FROM (r.policy#>>'{sponsor,treasury_account_id}')::bigint OR (r.policy->>'gift_tools')::boolean IS DISTINCT FROM TRUE)) THEN
  RAISE EXCEPTION 'invalid assistant sponsor source' USING ERRCODE='23514';
 END IF;
 IF r.gift_receipt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM core_gift.decisions d WHERE d.id=r.gift_receipt->>'gift_id' AND d.user_id=r.actor_user_id AND d.user_id=(r.gift_receipt->>'owner_user_id')::bigint AND d.amount_credits=(r.gift_receipt->>'credit_amount')::bigint AND d.claimed_at IS NOT DISTINCT FROM (r.gift_receipt->>'claimed_at_unix')::bigint AND d.ledger_journal_id IS NOT DISTINCT FROM (r.gift_receipt->>'ledger_transaction_id')::bigint) THEN
  RAISE EXCEPTION 'gift receipt must match the original Core ledger decision' USING ERRCODE='23514';
 END IF;
 IF r.gift_response_sha256 IS NOT NULL AND (r.state<>'settled' OR (r.policy->>'gift_tools')::boolean IS DISTINCT FROM TRUE) THEN
  RAISE EXCEPTION 'gift result requires settled enabled model operation' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER assistant_answer_atomic AFTER INSERT OR UPDATE ON core_assistant.operations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_terminal_answer();
CREATE CONSTRAINT TRIGGER assistant_charge_atomic AFTER INSERT OR UPDATE ON core_charging.requests DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_terminal_answer();
