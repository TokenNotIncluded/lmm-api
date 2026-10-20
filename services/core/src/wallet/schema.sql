-- Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
-- Fresh installation only; ordinary startup must never apply this definition.
CREATE SCHEMA IF NOT EXISTS core_wallet;
CREATE TABLE core_wallet.transfers (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 owner_user bigint NOT NULL REFERENCES core_identity.users(id),
 payer_account bigint NOT NULL REFERENCES core_identity.accounts(id),
 request_key text NOT NULL CHECK(length(request_key) BETWEEN 16 AND 64),
 amount bigint NOT NULL CHECK(amount>0),
 token_digest bytea NOT NULL UNIQUE CHECK(octet_length(token_digest)=32),
 state text NOT NULL DEFAULT 'awaiting_confirmation' CHECK(state IN ('awaiting_confirmation','pending','claimed','cancelled')),
 hold_journal bigint UNIQUE REFERENCES core_billing.ledger_journals(id),
 final_journal bigint UNIQUE REFERENCES core_billing.ledger_journals(id),
 recipient_user bigint REFERENCES core_identity.users(id),
 created_at bigint NOT NULL DEFAULT extract(epoch FROM clock_timestamp())::bigint,
 confirmed_at bigint,
 finished_at bigint,
 confirmation_digest bytea,
 confirmation_sid text,
 confirmation_kind text CHECK(confirmation_kind IN ('create','cancel')),
 confirmation_expires bigint,
 UNIQUE(owner_user,request_key),
 CHECK((state='awaiting_confirmation')=(hold_journal IS NULL)),
 CHECK((state='claimed')=(recipient_user IS NOT NULL)),
 CHECK((state IN ('claimed','cancelled'))=(final_journal IS NOT NULL)),
 CHECK((state IN ('claimed','cancelled'))=(finished_at IS NOT NULL)),
 CHECK((state<>'awaiting_confirmation')=(confirmed_at IS NOT NULL))
);
REVOKE ALL ON SCHEMA core_wallet FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_wallet FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_wallet FROM PUBLIC;

-- Runtime may attach a receipt only for THIS transfer's exact economic action.
-- These constraints also protect direct SQL callers holding narrow column UPDATE.
CREATE FUNCTION core_wallet.guard_transfer() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE h core_billing.ledger_journals%ROWTYPE; f core_billing.ledger_journals%ROWTYPE;
        wallet_id bigint; held_id bigint; receiver_wallet bigint;
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'wallet transfer immutable' USING ERRCODE='23514'; END IF;
 IF TG_OP='INSERT' THEN
  IF NEW.state<>'awaiting_confirmation' OR NEW.hold_journal IS NOT NULL OR NEW.final_journal IS NOT NULL OR NEW.recipient_user IS NOT NULL THEN
   RAISE EXCEPTION 'wallet transfer must start as draft' USING ERRCODE='23514';
  END IF;
 ELSE
  IF (NEW.id,NEW.owner_user,NEW.payer_account,NEW.request_key,NEW.amount,NEW.token_digest,NEW.created_at)
      IS DISTINCT FROM (OLD.id,OLD.owner_user,OLD.payer_account,OLD.request_key,OLD.amount,OLD.token_digest,OLD.created_at)
      OR (OLD.hold_journal IS NOT NULL AND NEW.hold_journal IS DISTINCT FROM OLD.hold_journal)
      OR (OLD.final_journal IS NOT NULL AND NEW.final_journal IS DISTINCT FROM OLD.final_journal)
      OR (OLD.confirmed_at IS NOT NULL AND NEW.confirmed_at IS DISTINCT FROM OLD.confirmed_at)
      OR (OLD.finished_at IS NOT NULL AND NEW.finished_at IS DISTINCT FROM OLD.finished_at)
      OR (OLD.recipient_user IS NOT NULL AND NEW.recipient_user IS DISTINCT FROM OLD.recipient_user) THEN
   RAISE EXCEPTION 'wallet original economics immutable' USING ERRCODE='23514';
  END IF;
  IF NOT (NEW.state=OLD.state OR (OLD.state='awaiting_confirmation' AND NEW.state='pending') OR (OLD.state='pending' AND NEW.state IN ('claimed','cancelled'))) THEN
   RAISE EXCEPTION 'wallet invalid transition' USING ERRCODE='23514';
  END IF;
 END IF;
 IF NOT EXISTS(SELECT 1 FROM core_identity.users WHERE id=NEW.owner_user AND personal_account_id=NEW.payer_account) THEN
  RAISE EXCEPTION 'wallet payer is not original owner' USING ERRCODE='23514';
 END IF;
 IF NEW.hold_journal IS NOT NULL THEN
  SELECT * INTO h FROM core_billing.ledger_journals WHERE id=NEW.hold_journal;
  SELECT id INTO wallet_id FROM core_billing.ledger_accounts WHERE owner_account_id=NEW.payer_account AND bucket='wallet';
  SELECT id INTO held_id FROM core_billing.ledger_accounts WHERE owner_account_id=NEW.payer_account AND bucket='reserved';
  IF h.id IS NULL OR h.scope<>'core-wallet-transfer' OR h.operation_key<>('hold-'||NEW.id::text) OR h.kind<>'reserve'
      OR h.actor_user_id IS DISTINCT FROM NEW.owner_user OR h.amount_units<>NEW.amount
      OR h.payer_id IS DISTINCT FROM wallet_id OR h.payee_id IS DISTINCT FROM held_id THEN
   RAISE EXCEPTION 'wallet hold receipt mismatch' USING ERRCODE='23514';
  END IF;
 END IF;
 IF NEW.final_journal IS NOT NULL THEN
  SELECT * INTO f FROM core_billing.ledger_journals WHERE id=NEW.final_journal;
  IF f.id IS NULL OR f.scope<>'core-wallet-transfer' OR f.parent_journal_id IS DISTINCT FROM NEW.hold_journal OR f.amount_units<>NEW.amount THEN
   RAISE EXCEPTION 'wallet final receipt mismatch' USING ERRCODE='23514';
  END IF;
  IF NEW.state='claimed' THEN
   SELECT a.id INTO receiver_wallet FROM core_identity.users u JOIN core_billing.ledger_accounts a ON a.owner_account_id=u.personal_account_id AND a.bucket='wallet' WHERE u.id=NEW.recipient_user;
   IF NEW.recipient_user=NEW.owner_user OR f.kind<>'commerce_capture' OR f.operation_key<>('claim-'||NEW.id::text)
     OR f.actor_user_id IS DISTINCT FROM NEW.recipient_user OR f.payer_id IS DISTINCT FROM wallet_id OR f.payee_id IS DISTINCT FROM receiver_wallet THEN
    RAISE EXCEPTION 'wallet claim receipt mismatch' USING ERRCODE='23514';
   END IF;
  ELSIF NEW.state='cancelled' THEN
   IF f.kind<>'release' OR f.operation_key<>('cancel-'||NEW.id::text) OR f.actor_user_id IS DISTINCT FROM NEW.owner_user
     OR f.payer_id IS DISTINCT FROM held_id OR f.payee_id IS DISTINCT FROM wallet_id THEN
    RAISE EXCEPTION 'wallet cancel receipt mismatch' USING ERRCODE='23514';
   END IF;
  ELSE RAISE EXCEPTION 'wallet nonterminal final receipt' USING ERRCODE='23514';
  END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION core_wallet.guard_transfer() FROM PUBLIC;
CREATE TRIGGER wallet_transfer_receipts BEFORE INSERT OR UPDATE OR DELETE ON core_wallet.transfers
 FOR EACH ROW EXECUTE FUNCTION core_wallet.guard_transfer();
CREATE TRIGGER wallet_transfers_no_truncate BEFORE TRUNCATE ON core_wallet.transfers
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
