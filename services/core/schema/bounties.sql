-- Native bounty authority. No Go role receives access to this schema.
CREATE SCHEMA core_bounties;
CREATE TABLE core_bounties.policy (id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(id), fee_bps BIGINT NOT NULL CHECK(fee_bps BETWEEN 0 AND 9999), fee_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id));
CREATE TABLE core_bounties.projects (id TEXT PRIMARY KEY, body JSONB NOT NULL);
CREATE TABLE core_bounties.confirmations (id TEXT PRIMARY KEY, actor BIGINT NOT NULL, credential BIGINT NOT NULL, request JSONB NOT NULL, expires BIGINT NOT NULL, policy JSONB NOT NULL, approved BOOLEAN NOT NULL DEFAULT FALSE, csrf BYTEA, browser_sid TEXT);
CREATE TABLE core_bounties.operations (id TEXT PRIMARY KEY, actor BIGINT NOT NULL, request JSONB NOT NULL, receipt JSONB NOT NULL);
REVOKE ALL ON SCHEMA core_bounties FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_bounties FROM PUBLIC;
CREATE FUNCTION core_bounties.immutable() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'bounty financial evidence is immutable' USING ERRCODE='23514'; END $$;
CREATE TRIGGER operations_immutable BEFORE UPDATE OR DELETE ON core_bounties.operations FOR EACH ROW EXECUTE FUNCTION core_bounties.immutable();
CREATE TRIGGER operations_no_truncate BEFORE TRUNCATE ON core_bounties.operations FOR EACH STATEMENT EXECUTE FUNCTION core_bounties.immutable();
CREATE TRIGGER projects_no_delete BEFORE DELETE ON core_bounties.projects FOR EACH ROW EXECUTE FUNCTION core_bounties.immutable();
CREATE TRIGGER projects_no_truncate BEFORE TRUNCATE ON core_bounties.projects FOR EACH STATEMENT EXECUTE FUNCTION core_bounties.immutable();
CREATE FUNCTION core_bounties.guard_project() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE old_slot JSONB; new_slot JSONB; old_claim JSONB; new_claim JSONB;
BEGIN
 IF NEW.id<>OLD.id OR (NEW.body-'status'-'slots'-'claims'-'revision') IS DISTINCT FROM (OLD.body-'status'-'slots'-'claims'-'revision')
    OR jsonb_typeof(NEW.body->'revision') IS DISTINCT FROM 'number'
    OR (NEW.body->>'revision')::numeric IS DISTINCT FROM coalesce((OLD.body->>'revision')::numeric,0)+1
    OR jsonb_array_length(NEW.body->'slots')<>jsonb_array_length(OLD.body->'slots') THEN
  RAISE EXCEPTION 'bounty funding basis is immutable' USING ERRCODE='23514';
 END IF;
 FOR old_slot IN SELECT value FROM jsonb_array_elements(OLD.body->'slots') LOOP
  SELECT value INTO new_slot FROM jsonb_array_elements(NEW.body->'slots') WHERE value->'hold'=old_slot->'hold';
  IF new_slot IS NULL OR (new_slot-'paid') IS DISTINCT FROM (old_slot-'paid') OR jsonb_typeof(new_slot->'paid') IS DISTINCT FROM 'boolean' OR (old_slot->>'paid'='true' AND new_slot->>'paid' IS DISTINCT FROM 'true') THEN
   RAISE EXCEPTION 'bounty hold basis is immutable' USING ERRCODE='23514';
  END IF;
 END LOOP;
 FOR old_claim IN SELECT value FROM jsonb_array_elements(OLD.body->'claims') LOOP
  SELECT value INTO new_claim FROM jsonb_array_elements(NEW.body->'claims') WHERE value->'id'=old_claim->'id';
  IF new_claim IS NULL OR new_claim->'user' IS DISTINCT FROM old_claim->'user' OR new_claim->'account' IS DISTINCT FROM old_claim->'account' THEN
   RAISE EXCEPTION 'bounty claim recipient is immutable' USING ERRCODE='23514';
  END IF;
 END LOOP;
 RETURN NEW;
END $$;
CREATE TRIGGER project_basis BEFORE UPDATE ON core_bounties.projects FOR EACH ROW EXECUTE FUNCTION core_bounties.guard_project();
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_bounties FROM PUBLIC;
CREATE FUNCTION core_bounties.guard_confirmation() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF (to_jsonb(NEW)-'approved'-'csrf'-'browser_sid') IS DISTINCT FROM (to_jsonb(OLD)-'approved'-'csrf'-'browser_sid') OR (OLD.approved AND NOT NEW.approved) THEN
  RAISE EXCEPTION 'bounty confirmation basis is immutable' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER confirmation_basis BEFORE UPDATE ON core_bounties.confirmations FOR EACH ROW EXECUTE FUNCTION core_bounties.guard_confirmation();
REVOKE ALL ON FUNCTION core_bounties.guard_confirmation() FROM PUBLIC;

CREATE INDEX confirmations_operation_actor ON core_bounties.confirmations(actor,(request->>'operation_id'),expires DESC);

-- Freeze a native publication only around its own newly posted reservations.
CREATE FUNCTION core_bounties.guard_initial_project() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE op TEXT; owner_id BIGINT; payer BIGINT; net BIGINT; gross BIGINT; fee BIGINT;
        n INTEGER; i INTEGER; hold_id BIGINT; bps BIGINT; fee_account BIGINT; slot JSONB;
        expected_key TEXT; found BOOLEAN;
BEGIN
 op:=NEW.body->>'publication_operation'; owner_id:=(NEW.body->>'owner')::bigint;
 payer:=(NEW.body->>'payer')::bigint; net:=(NEW.body->>'net')::bigint;
 gross:=(NEW.body->>'gross')::bigint; fee:=(NEW.body->>'fee')::bigint;
 IF op IS NULL OR op !~ '^[A-Za-z0-9_.:-]{1,128}$' OR NEW.id !~ '^[A-Za-z0-9_.:-]{1,128}$'
    OR jsonb_typeof(NEW.body->'slots') IS DISTINCT FROM 'array'
    OR NEW.body->'claims' IS DISTINCT FROM '[]'::jsonb
    OR NEW.body->>'status' IS DISTINCT FROM 'published'
    OR NEW.body->'revision' IS DISTINCT FROM '1'::jsonb
    OR net IS NULL OR net<=0 OR gross IS NULL OR gross<=0 OR fee IS NULL OR fee<0
    OR jsonb_typeof(NEW.body->'published_at') IS DISTINCT FROM 'number'
    OR (NEW.body->>'published_at')::numeric NOT BETWEEN floor(extract(epoch FROM transaction_timestamp()))-1 AND ceil(extract(epoch FROM clock_timestamp()))
    OR NEW.body->>'consumption' IS DISTINCT FROM 'bounty:'||encode(sha256(convert_to(to_json(op)::text,'UTF8')),'hex')
    OR NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
                  WHERE u.id=owner_id AND a.id=payer AND a.kind='personal' AND u.active AND a.active) THEN
  RAISE EXCEPTION 'invalid initial bounty funding basis' USING ERRCODE='23514';
 END IF;
 n:=jsonb_array_length(NEW.body->'slots');
 IF n NOT BETWEEN 1 AND 100 OR (SELECT count(DISTINCT value->'hold') FROM jsonb_array_elements(NEW.body->'slots'))<>n THEN
  RAISE EXCEPTION 'invalid initial bounty slots' USING ERRCODE='23514';
 END IF;
 SELECT (c.policy->>0)::bigint,(c.policy->>1)::bigint INTO bps,fee_account
 FROM core_bounties.confirmations c
 WHERE c.actor=owner_id AND c.approved AND c.expires>=floor(extract(epoch FROM transaction_timestamp()))
   AND c.request->>'operation_id'=op AND c.request->>'action'='publish' AND c.request->>'project_id'=NEW.id
   AND (c.request->>'reward_credit')::bigint=gross AND (c.request->>'reward_slots')::bigint=n
   AND regexp_replace(rtrim(c.request->>'repository_url','/'),'(\.git)+$','')=NEW.body->>'repository'
 ORDER BY c.expires DESC,c.id DESC LIMIT 1;
 IF bps IS NULL OR bps NOT BETWEEN 0 AND 9999 OR fee_account IS NULL
    OR (NEW.body->>'fee_bps')::bigint IS DISTINCT FROM bps
    OR net::numeric<>gross::numeric-ceil(gross::numeric*bps/10000)
    OR fee::numeric<>(gross::numeric-net)*n THEN
  RAISE EXCEPTION 'bounty publication lacks exact confirmed economics' USING ERRCODE='23514';
 END IF;
 FOR i IN 0..n-1 LOOP
  slot:=NEW.body->'slots'->i; hold_id:=(slot->>'hold')::bigint;
  expected_key:=encode(sha256(convert_to('["'||op||'","slot-'||i||'"]','UTF8')),'hex');
  SELECT EXISTS(SELECT FROM core_billing.ledger_journals j
     JOIN core_billing.ledger_accounts w ON w.id=j.payer_id
     JOIN core_billing.ledger_accounts r ON r.id=j.payee_id
     WHERE j.id=hold_id AND j.scope='core-bounty' AND j.operation_key=expected_key
       AND j.actor_user_id=owner_id AND j.kind='reserve' AND j.amount_units=net
       AND j.created_xid=pg_current_xact_id() AND j.reason=format('Bounty %s slot-%s',NEW.id,i)
       AND w.owner_account_id=payer AND w.bucket='wallet' AND r.owner_account_id=payer AND r.bucket='reserved'
       AND NOT EXISTS(SELECT FROM core_billing.ledger_journals f WHERE f.parent_journal_id=j.id)) INTO found;
  IF slot->'paid' IS DISTINCT FROM 'false'::jsonb OR NOT found THEN
   RAISE EXCEPTION 'bounty slot is not its original newly posted hold' USING ERRCODE='23514';
  END IF;
 END LOOP;
 IF fee>0 AND fee_account<>payer AND NOT EXISTS(
   SELECT FROM core_billing.ledger_journals j
   JOIN core_billing.ledger_accounts w ON w.id=j.payer_id JOIN core_billing.ledger_accounts r ON r.id=j.payee_id
   WHERE j.scope='core-bounty' AND j.operation_key=encode(sha256(convert_to('["'||op||'","fee"]','UTF8')),'hex')
     AND j.actor_user_id=owner_id AND j.kind='transfer' AND j.amount_units=fee
     AND j.created_xid=pg_current_xact_id() AND j.reason=format('Bounty %s fee',NEW.id)
     AND w.owner_account_id=payer AND w.bucket='wallet' AND r.owner_account_id=fee_account AND r.bucket='wallet') THEN
  RAISE EXCEPTION 'bounty publication fee lacks original payer journal' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER initial_project_basis BEFORE INSERT ON core_bounties.projects FOR EACH ROW EXECUTE FUNCTION core_bounties.guard_initial_project();
REVOKE ALL ON FUNCTION core_bounties.guard_initial_project() FROM PUBLIC;
