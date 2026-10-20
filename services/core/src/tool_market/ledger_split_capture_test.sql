-- Isolated second-batch test migration only. NOT canonical startup DDL.
-- The following replaces the EXISTING ledger function, not another engine.
-- Root must merge the exact source patch with Gift before final canonical freeze.
DO $$ DECLARE c RECORD; BEGIN
 FOR c IN SELECT conname,pg_get_constraintdef(oid) AS definition FROM pg_constraint
 WHERE conrelid='core_billing.ledger_journals'::regclass AND contype='c' LOOP
  IF c.definition LIKE '%kind = ANY%' AND c.definition NOT LIKE '%payment_reference%' AND c.definition NOT LIKE '%refund_reference%'
     OR c.definition LIKE '%payer_id <> payee_id%' OR c.definition LIKE '%amount_units%'
  THEN EXECUTE format('ALTER TABLE core_billing.ledger_journals DROP CONSTRAINT %I',c.conname); END IF;
 END LOOP;
END $$;
ALTER TABLE core_billing.ledger_journals
 ADD CONSTRAINT ledger_kind_split_v2 CHECK(kind IN ('gift','credit','transfer','charge','reserve','capture','commerce_capture','split_commerce_capture','claim_affiliate','opening','release','refund','refund_topup','refund_commerce','reverse','legacy_escrow_opening','legacy_escrow_payout','legacy_escrow_release')),
 ADD CONSTRAINT ledger_amount_split_v2 CHECK(amount_units>=0 AND (amount_units>0 OR kind IN ('capture','split_commerce_capture'))),
 ADD CONSTRAINT ledger_parties_split_v2 CHECK(payer_id<>payee_id OR kind='split_commerce_capture'),
 ADD CONSTRAINT ledger_parent_split_v2 CHECK((kind IN ('capture','commerce_capture','split_commerce_capture','release','refund','refund_topup','refund_commerce','reverse','legacy_escrow_payout','legacy_escrow_release'))=(parent_journal_id IS NOT NULL));
DROP INDEX core_billing.ledger_reservation_terminal;
CREATE UNIQUE INDEX ledger_reservation_terminal ON core_billing.ledger_journals(parent_journal_id) WHERE kind IN ('capture','commerce_capture','split_commerce_capture','release');
CREATE OR REPLACE FUNCTION core_billing.post_ledger(p_request JSONB) RETURNS JSONB
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    op_scope TEXT := p_request->>'scope';
    op_key TEXT := p_request->>'operation_key';
    action JSONB := p_request->'action';
    action_kind TEXT := p_request->'action'->>'kind';
    prior core_billing.ledger_operations%ROWTYPE;
    original core_billing.ledger_journals%ROWTYPE;
    split_platform BIGINT; split_fee BIGINT; split_bps BIGINT;
    payer BIGINT; payee BIGINT; held BIGINT; revenue BIGINT; clearing BIGINT; debt_id BIGINT;
    repayment BIGINT; debt_balance BIGINT; adjusted_legs JSONB;
    refund_ref TEXT; refund_wallet BIGINT; refund_debt BIGINT; refund_wallet_units BIGINT;
    refund_recipient_debt BIGINT; original_debt_repayment NUMERIC;
    amount BIGINT; parent_id BIGINT; posted_id BIGINT; refunded NUMERIC;
    payment_ref TEXT; legs JSONB; leg JSONB; result JSONB; err TEXT; violated TEXT;
BEGIN
    IF current_setting('transaction_isolation') NOT IN ('read committed', 'serializable') THEN
        RAISE EXCEPTION 'ledger requires read committed or serializable' USING ERRCODE = 'P1000';
    END IF;
    IF NOT COALESCE(jsonb_typeof(p_request) = 'object'
        AND octet_length(p_request::TEXT) <= 8192
        AND jsonb_typeof(p_request->'scope') = 'string'
        AND op_scope ~ '^[A-Za-z0-9._:/-]{1,64}$'
        AND jsonb_typeof(p_request->'operation_key') = 'string'
        AND op_key ~ '^[A-Za-z0-9._:/-]{1,128}$'
        AND jsonb_typeof(p_request->'reason') = 'string'
        AND length(p_request->>'reason') BETWEEN 1 AND 512
        AND action_kind IN ('gift', 'credit', 'transfer', 'charge', 'reserve', 'capture', 'commerce_capture', 'split_commerce_capture', 'claim_affiliate', 'release', 'refund', 'refund_topup', 'refund_commerce', 'reverse'), FALSE)
    THEN
        RAISE EXCEPTION 'invalid ledger request' USING ERRCODE = 'P1000';
    END IF;
    -- JSON array encoding makes the key unambiguous. Hash collisions only serialize work.
    PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array('lmm-ledger-v1', op_scope, op_key)::TEXT, 0));
    SELECT * INTO prior FROM core_billing.ledger_operations WHERE scope = op_scope AND operation_key = op_key;
    IF FOUND THEN
        IF prior.request IS DISTINCT FROM p_request THEN
            RAISE EXCEPTION 'operation key has different parameters' USING ERRCODE = 'P1001';
        END IF;
        RETURN prior.result;
    END IF;

    -- Expected business failures roll back every posting in this subtransaction,
    -- then store a rejected result outside it. Unexpected failures abort everything.
    BEGIN
        IF action_kind IN ('gift', 'credit', 'transfer', 'charge', 'reserve', 'capture', 'commerce_capture', 'split_commerce_capture', 'claim_affiliate', 'refund', 'refund_topup') THEN
            amount := (action->>'amount_units')::BIGINT;
            IF amount IS NULL OR amount < 0 OR (amount = 0 AND action_kind NOT IN ('capture','split_commerce_capture')) THEN
                RAISE EXCEPTION 'invalid amount' USING ERRCODE = 'P1000';
            END IF;
        END IF;
        SELECT id INTO STRICT revenue FROM core_billing.ledger_accounts WHERE bucket = 'revenue';
        SELECT id INTO STRICT clearing FROM core_billing.ledger_accounts WHERE bucket = 'clearing';

        IF action_kind IN ('capture', 'commerce_capture', 'split_commerce_capture', 'release', 'refund', 'refund_topup', 'refund_commerce', 'reverse') THEN
            parent_id := CASE WHEN action_kind IN ('capture', 'commerce_capture', 'split_commerce_capture', 'release')
                THEN (action->>'reservation_journal_id')::BIGINT ELSE (action->>'journal_id')::BIGINT END;
            SELECT * INTO original FROM core_billing.ledger_journals WHERE id = parent_id FOR NO KEY UPDATE;
            IF NOT FOUND THEN RAISE EXCEPTION 'original not found' USING ERRCODE = 'P1002'; END IF;
        END IF;

        IF action_kind = 'gift' THEN
            payer := clearing;
            SELECT id INTO payee FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'account_id')::BIGINT AND bucket = 'wallet';
        ELSIF action_kind = 'credit' THEN
            payer := clearing;
            SELECT id INTO payee FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'account_id')::BIGINT AND bucket = 'wallet';
            payment_ref := action->>'payment_reference';
            IF payment_ref IS NULL OR length(payment_ref) NOT BETWEEN 1 AND 256 THEN
                RAISE EXCEPTION 'invalid payment reference' USING ERRCODE = 'P1000';
            END IF;
        ELSIF action_kind = 'transfer' THEN
            SELECT id INTO payer FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'from_account_id')::BIGINT AND bucket = 'wallet';
            SELECT id INTO payee FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'to_account_id')::BIGINT AND bucket = 'wallet';
        ELSIF action_kind = 'claim_affiliate' THEN
            SELECT id INTO payer FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'account_id')::BIGINT AND bucket = 'affiliate';
            SELECT id INTO payee FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'account_id')::BIGINT AND bucket = 'wallet';
        ELSIF action_kind IN ('charge', 'reserve') THEN
            SELECT id INTO payer FROM core_billing.ledger_accounts
                WHERE owner_account_id = (action->>'account_id')::BIGINT AND bucket = 'wallet';
            IF action_kind = 'charge' THEN payee := revenue;
            ELSE
                SELECT id INTO payee FROM core_billing.ledger_accounts
                    WHERE owner_account_id = (action->>'account_id')::BIGINT AND bucket = 'reserved';
            END IF;
        ELSIF action_kind IN ('capture', 'commerce_capture', 'split_commerce_capture', 'release') THEN
            IF original.kind <> 'reserve' THEN
                RAISE EXCEPTION 'not a reservation' USING ERRCODE = 'P1004';
            END IF;
            IF EXISTS (SELECT 1 FROM core_billing.ledger_journals
                WHERE parent_journal_id = parent_id AND kind IN ('capture', 'commerce_capture', 'split_commerce_capture', 'release')) THEN
                RAISE EXCEPTION 'reservation finalized' USING ERRCODE = 'P1006';
            END IF;
            IF action_kind = 'release' THEN
                payer := original.payee_id; payee := original.payer_id; amount := original.amount_units;
            ELSE
                IF amount > original.amount_units THEN
                    RAISE EXCEPTION 'capture exceeds reservation' USING ERRCODE = 'P1005';
                END IF;
                -- Economic payer stays the ORIGINAL wallet, so refunds never go to a held bucket.
                payer := original.payer_id; payee := revenue; held := original.payee_id;
                IF action_kind = 'split_commerce_capture' THEN
                    -- Only the reviewed tool-market business leaf can construct
                    -- this closed action. No generic caller-selected entry list.
                    IF op_scope <> 'core-tool-market-credit' THEN
                        RAISE EXCEPTION 'invalid split capture scope' USING ERRCODE='P1000';
                    END IF;
                    split_bps := (action->>'fee_basis_points')::BIGINT;
                    IF split_bps IS NULL OR split_bps NOT BETWEEN 0 AND 10000 THEN
                        RAISE EXCEPTION 'invalid split capture fee' USING ERRCODE='P1000';
                    END IF;
                    SELECT id INTO payee FROM core_billing.ledger_accounts
                        WHERE owner_account_id=(action->>'seller_account_id')::BIGINT AND bucket='wallet';
                    SELECT id INTO split_platform FROM core_billing.ledger_accounts
                        WHERE owner_account_id=(action->>'platform_account_id')::BIGINT AND bucket='wallet';
                    IF payee IS NULL OR split_platform IS NULL THEN
                        RAISE EXCEPTION 'split capture wallet not found' USING ERRCODE='P1002';
                    END IF;
                    split_fee := floor(amount::NUMERIC*split_bps/10000)::BIGINT;
                    -- Merge shared parties before debt allocation. A seller may
                    -- be the payer or platform; no duplicate entry or debt repay.
                    SELECT jsonb_agg(jsonb_build_object('id',id,'units',units) ORDER BY id) INTO legs
                    FROM (SELECT (v->>'id')::BIGINT AS id,sum((v->>'units')::BIGINT)::BIGINT AS units
                        FROM jsonb_array_elements(jsonb_build_array(
                            jsonb_build_object('id',held,'units',-original.amount_units),
                            jsonb_build_object('id',payee,'units',amount-split_fee),
                            jsonb_build_object('id',split_platform,'units',split_fee),
                            jsonb_build_object('id',payer,'units',original.amount_units-amount))) v
                        GROUP BY (v->>'id')::BIGINT) grouped;
                ELSIF action_kind = 'commerce_capture' THEN
                    SELECT id INTO payee FROM core_billing.ledger_accounts
                        WHERE owner_account_id = (action->>'destination_account_id')::BIGINT AND bucket = 'wallet';
                    IF payee IS NULL OR payee = payer THEN
                        RAISE EXCEPTION 'invalid commerce destination' USING ERRCODE = 'P1000';
                    END IF;
                END IF;
                IF action_kind <> 'split_commerce_capture' THEN
                    legs := jsonb_build_array(
                        jsonb_build_object('id', held, 'units', -original.amount_units),
                        jsonb_build_object('id', payee, 'units', amount),
                        jsonb_build_object('id', payer, 'units', original.amount_units - amount));
                END IF;
            END IF;
        ELSE
            IF original.kind NOT IN ('credit', 'transfer', 'charge', 'capture', 'commerce_capture')
                OR (action_kind = 'reverse' AND original.kind IN ('capture', 'commerce_capture')) THEN
                RAISE EXCEPTION 'original cannot be compensated this way' USING ERRCODE = 'P1004';
            END IF;
            -- Restoring debt repaid by an incoming payment requires the original
            -- provider/refund basis. Fail visibly until that bridge is supplied;
            -- never replace a debt repayment with newly spendable wallet funds.
            IF action_kind NOT IN ('refund_topup','refund_commerce') AND original.kind IN ('credit', 'transfer', 'commerce_capture') AND EXISTS (
                SELECT 1 FROM core_billing.ledger_entries e JOIN core_billing.ledger_accounts a
                    ON a.id=e.ledger_account_id WHERE e.journal_id=original.id
                    AND a.bucket='debt' AND e.delta_units>0
            ) THEN RAISE EXCEPTION 'debt repayment refund requires original basis' USING ERRCODE = 'P1004'; END IF;
            SELECT COALESCE(sum(amount_units), 0) INTO refunded FROM core_billing.ledger_journals
                WHERE parent_journal_id = parent_id AND kind IN ('refund', 'refund_topup', 'refund_commerce', 'reverse');
            IF action_kind IN ('reverse','refund_commerce') THEN
                IF action_kind = 'refund_commerce' AND original.kind <> 'commerce_capture' THEN
                    RAISE EXCEPTION 'not a commerce capture' USING ERRCODE='P1004';
                END IF;
                IF refunded <> 0 THEN RAISE EXCEPTION 'already compensated' USING ERRCODE = 'P1007'; END IF;
                amount := original.amount_units;
            END IF;
            IF amount > original.amount_units - refunded THEN
                RAISE EXCEPTION 'refund exceeds remaining amount' USING ERRCODE = 'P1005';
            END IF;
            payer := original.payee_id; payee := original.payer_id;
            IF action_kind IN ('refund_topup','refund_commerce') THEN
                IF action_kind = 'refund_topup' THEN
                    refund_ref := action->>'refund_reference';
                    IF original.kind <> 'credit' OR refund_ref IS NULL OR length(refund_ref) NOT BETWEEN 1 AND 256 THEN
                        RAISE EXCEPTION 'invalid verified topup refund' USING ERRCODE='P1000';
                    END IF;
                END IF;
                refund_wallet := original.payee_id;
                SELECT d.id INTO STRICT refund_debt FROM core_billing.ledger_accounts w
                    JOIN core_billing.ledger_accounts d ON d.owner_account_id=w.owner_account_id AND d.bucket='debt'
                    WHERE w.id=refund_wallet AND w.bucket='wallet';
                SELECT d.id INTO refund_recipient_debt FROM core_billing.ledger_accounts w
                    JOIN core_billing.ledger_accounts d ON d.owner_account_id=w.owner_account_id AND d.bucket='debt'
                    WHERE w.id=payee AND w.bucket='wallet';
                -- Lock these in the same order as every other ledger posting.
                PERFORM b.ledger_account_id FROM core_billing.balance_state b
                    WHERE b.ledger_account_id IN (refund_wallet,refund_debt,payee,refund_recipient_debt)
                    ORDER BY b.ledger_account_id FOR NO KEY UPDATE;
                SELECT least(balance_units,amount) INTO STRICT refund_wallet_units
                    FROM core_billing.balance_state WHERE ledger_account_id=refund_wallet;
                IF action_kind = 'refund_commerce' THEN
                    SELECT COALESCE(sum(delta_units),0) INTO original_debt_repayment
                        FROM core_billing.ledger_entries WHERE journal_id=parent_id
                            AND ledger_account_id=refund_debt AND delta_units>0;
                    IF amount-refund_wallet_units > original_debt_repayment THEN
                        RAISE EXCEPTION 'spent seller funds are not new debt' USING ERRCODE='P1003';
                    END IF;
                END IF;
                legs := jsonb_build_array(jsonb_build_object('id',refund_wallet,'units',-refund_wallet_units),
                    jsonb_build_object('id',refund_debt,'units',-(amount-refund_wallet_units)),
                    jsonb_build_object('id',payee,'units',amount));
            END IF;
        END IF;
        IF payer IS NULL OR payee IS NULL THEN
            RAISE EXCEPTION 'wallet not found' USING ERRCODE = 'P1002';
        END IF;
        IF payer = payee AND action_kind <> 'split_commerce_capture' THEN RAISE EXCEPTION 'self transfer' USING ERRCODE = 'P1000'; END IF;
        IF legs IS NULL THEN
            legs := jsonb_build_array(jsonb_build_object('id', payer, 'units', -amount),
                jsonb_build_object('id', payee, 'units', amount));
        END IF;

        -- Lock incoming wallets AND their debt buckets before choosing the
        -- repayment split. Every writer uses the same ascending balance order.
        PERFORM b.ledger_account_id FROM core_billing.balance_state b
            WHERE b.ledger_account_id IN (SELECT (value->>'id')::BIGINT FROM jsonb_array_elements(legs)
                UNION SELECT d.id FROM jsonb_array_elements(legs) l
                    JOIN core_billing.ledger_accounts w ON w.id=(l.value->>'id')::BIGINT
                    JOIN core_billing.ledger_accounts d ON d.owner_account_id=w.owner_account_id AND d.bucket='debt'
                    WHERE w.bucket='wallet' AND (l.value->>'units')::BIGINT>0)
            ORDER BY b.ledger_account_id FOR NO KEY UPDATE;
        adjusted_legs := '[]'::JSONB;
        FOR leg IN SELECT value FROM jsonb_array_elements(legs) LOOP
            debt_id := NULL;
            IF (leg->>'units')::BIGINT>0 THEN
                SELECT d.id,b.balance_units INTO debt_id,debt_balance FROM core_billing.ledger_accounts w
                    JOIN core_billing.ledger_accounts d ON d.owner_account_id=w.owner_account_id AND d.bucket='debt'
                    JOIN core_billing.balance_state b ON b.ledger_account_id=d.id
                    WHERE w.id=(leg->>'id')::BIGINT AND w.bucket='wallet';
            END IF;
            IF debt_id IS NOT NULL AND debt_balance<0 THEN
                repayment := least((leg->>'units')::BIGINT, -debt_balance);
                adjusted_legs := adjusted_legs || jsonb_build_array(jsonb_build_object('id',debt_id,'units',repayment));
                leg := jsonb_set(leg,'{units}',to_jsonb((leg->>'units')::BIGINT-repayment));
            END IF;
            adjusted_legs := adjusted_legs || jsonb_build_array(leg);
        END LOOP;
        legs := adjusted_legs;
        INSERT INTO core_billing.ledger_journals(scope, operation_key, kind, payer_id, payee_id,
            amount_units, parent_journal_id, payment_reference, refund_reference, actor_user_id, reason)
            VALUES (op_scope, op_key, action_kind, payer, payee, amount, parent_id, payment_ref, refund_ref,
                (p_request->>'actor_user_id')::BIGINT, p_request->>'reason') RETURNING id INTO posted_id;
        FOR leg IN SELECT value FROM jsonb_array_elements(legs)
            WHERE (value->>'units')::BIGINT <> 0 ORDER BY (value->>'id')::BIGINT
        LOOP
            INSERT INTO core_billing.ledger_entries(journal_id, ledger_account_id, delta_units)
                VALUES (posted_id, (leg->>'id')::BIGINT, (leg->>'units')::BIGINT);
        END LOOP;
        SELECT jsonb_build_object('status', 'posted', 'journal_id', posted_id, 'entries',
            jsonb_agg(jsonb_build_object('ledger_account_id', e.ledger_account_id, 'delta_units', e.delta_units,
                'balance_after_units', e.balance_after_units, 'balance_revision', e.balance_revision)
                ORDER BY e.ledger_account_id)) INTO result
            FROM core_billing.ledger_entries e WHERE e.journal_id = posted_id;
    EXCEPTION
        WHEN SQLSTATE 'P1002' OR SQLSTATE 'P1003' OR SQLSTATE 'P1004'
            OR SQLSTATE 'P1005' OR SQLSTATE 'P1006' OR SQLSTATE 'P1007' THEN
            GET STACKED DIAGNOSTICS err = RETURNED_SQLSTATE;
            result := jsonb_build_object('status', 'rejected', 'code', CASE err
                WHEN 'P1002' THEN 'not_found' WHEN 'P1003' THEN 'insufficient_funds'
                WHEN 'P1004' THEN 'invalid_original' WHEN 'P1005' THEN 'exceeds_remaining'
                WHEN 'P1006' THEN 'already_finalized' WHEN 'P1007' THEN 'already_compensated' END);
        WHEN numeric_value_out_of_range THEN
            result := jsonb_build_object('status', 'rejected', 'code', 'amount_overflow');
        WHEN unique_violation THEN
            GET STACKED DIAGNOSTICS violated = CONSTRAINT_NAME;
            IF violated NOT IN ('ledger_payment_once','ledger_refund_once') THEN RAISE; END IF;
            result := jsonb_build_object('status', 'rejected', 'code', CASE violated
                WHEN 'ledger_refund_once' THEN 'duplicate_refund' ELSE 'duplicate_payment' END);
    END;
    INSERT INTO core_billing.ledger_operations(scope, operation_key, request, result)
        VALUES (op_scope, op_key, p_request, result);
    RETURN result;
EXCEPTION WHEN unique_violation THEN
    GET STACKED DIAGNOSTICS violated = CONSTRAINT_NAME;
    -- An advisory-lock wait does not refresh a SERIALIZABLE snapshot. If the
    -- permanent operation receipt was committed after that snapshot, its two
    -- identity constraints can conflict before SSI reports 40001. Retry the
    -- whole transaction so the normal replay/payload check sees the receipt.
    -- Payment, reservation-terminal and every other uniqueness error retain
    -- their existing business/error semantics.
    IF current_setting('transaction_isolation') = 'serializable'
        AND violated IN ('ledger_operations_pkey', 'ledger_journals_scope_operation_key_key') THEN
        RAISE EXCEPTION 'retry complete ledger transaction' USING ERRCODE = '40001';
    END IF;
    RAISE;
END $$;
