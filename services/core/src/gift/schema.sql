-- Fresh installation after assistant-model and ledger. Never applied at startup.
CREATE SCHEMA core_gift;
CREATE TABLE core_gift.risk_key(singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),secret BYTEA NOT NULL CHECK(octet_length(secret)=32));
INSERT INTO core_gift.risk_key(singleton,secret) VALUES(TRUE,decode(replace(gen_random_uuid()::text||gen_random_uuid()::text,'-',''),'hex'));
CREATE TABLE core_gift.policy (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 enabled BOOLEAN NOT NULL DEFAULT FALSE,
 exclusive_cap BIGINT NOT NULL DEFAULT 0 CHECK(exclusive_cap>=0)
);
INSERT INTO core_gift.policy(singleton) VALUES(TRUE);
-- Only cryptographically verified identity registration can establish eligibility.
-- Runtime SELECT only; owner-reviewed imports require the same evidence contract.
CREATE TABLE core_gift.qualifications (
 user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
 email_hash BYTEA NOT NULL CHECK(octet_length(email_hash)=32),
 network_hash BYTEA NOT NULL CHECK(octet_length(network_hash)=32),
 evidence_digest BYTEA NOT NULL CHECK(octet_length(evidence_digest)=32),
 evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('google_verified_email_v1','owner_reviewed_registration_v1')),
 verified_at TIMESTAMPTZ NOT NULL,
 registration_hold BOOLEAN NOT NULL,
 reward_used BOOLEAN NOT NULL
);
CREATE TABLE core_gift.decisions (
 id TEXT PRIMARY KEY CHECK(id ~ '^[a-f0-9]{64}$'),
 user_id BIGINT NOT NULL UNIQUE REFERENCES core_identity.users(id),
 conversation_id TEXT NOT NULL REFERENCES core_assistant.conversations(id),
 model_request_id TEXT NOT NULL UNIQUE REFERENCES core_charging.requests(id),
 amount_credits BIGINT NOT NULL CHECK(amount_credits>0),
 reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 240),
 email_hash BYTEA NOT NULL UNIQUE CHECK(octet_length(email_hash)=32),
 network_hash BYTEA NOT NULL CHECK(octet_length(network_hash)=32),
 created_at BIGINT NOT NULL,
 claimed_at BIGINT,
 ledger_journal_id BIGINT UNIQUE REFERENCES core_billing.ledger_journals(id),
 CHECK((claimed_at IS NULL)=(ledger_journal_id IS NULL))
);
CREATE TABLE core_gift.registration_profiles(
 user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
 observed_at BIGINT NOT NULL
);
CREATE TABLE core_gift.fingerprints(
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 message_digest BYTEA NOT NULL CHECK(octet_length(message_digest)=32),
 token BYTEA NOT NULL CHECK(octet_length(token)=32),
 created_at BIGINT NOT NULL,
 UNIQUE(user_id,message_digest,token)
);
CREATE INDEX gift_fingerprint_token ON core_gift.fingerprints(token,created_at,user_id);
CREATE INDEX gift_network_window ON core_gift.decisions(network_hash,created_at);
CREATE FUNCTION core_gift.guard_decision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR OLD.claimed_at IS NOT NULL OR
 (to_jsonb(OLD)-ARRAY['claimed_at','ledger_journal_id']) IS DISTINCT FROM
 (to_jsonb(NEW)-ARRAY['claimed_at','ledger_journal_id']) THEN
  RAISE EXCEPTION 'immutable gift decision' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER gift_decision_immutable BEFORE UPDATE OR DELETE ON core_gift.decisions FOR EACH ROW EXECUTE FUNCTION core_gift.guard_decision();
CREATE TRIGGER gift_decision_no_truncate BEFORE TRUNCATE ON core_gift.decisions FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE FUNCTION core_gift.check_journal() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.ledger_journal_id IS NOT NULL AND NOT EXISTS(
  SELECT 1 FROM core_billing.ledger_journals j
  JOIN core_billing.ledger_accounts a ON a.id=j.payee_id
  JOIN core_identity.users u ON u.personal_account_id=a.owner_account_id
  WHERE j.id=NEW.ledger_journal_id AND j.kind='gift' AND j.scope='core-assistant-gift'
   AND j.operation_key=NEW.id AND j.actor_user_id=NEW.user_id AND u.id=NEW.user_id
   AND j.amount_units=NEW.amount_credits AND j.payment_reference IS NULL
 ) THEN RAISE EXCEPTION 'gift journal binding mismatch' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER gift_journal_bound AFTER INSERT OR UPDATE ON core_gift.decisions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_gift.check_journal();
REVOKE ALL ON SCHEMA core_gift FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_gift FROM PUBLIC;
