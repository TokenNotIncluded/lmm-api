-- Additive component 25 image price support; explicit owner install/upgrade.
-- Replace only the existing price CHECK and deferred charge-binding function.
-- Existing no-trust price JSON and immutable operation/charge bindings remain.
ALTER TABLE core_image.operations DROP CONSTRAINT operations_price_check;
ALTER TABLE core_image.operations ADD CONSTRAINT operations_price_check CHECK((
 (jsonb_typeof((price-'user_trust'))='object' AND jsonb_typeof((price-'user_trust')->'image')='object'
   AND ((price-'user_trust')->'image') ?& ARRAY['revision','rule','request','reservation_credits']
   AND ((price-'user_trust')->'image')-ARRAY['revision','rule','request','reservation_credits']='{}'::jsonb
   AND jsonb_typeof((price-'user_trust')#>'{image,reservation_credits}')='number'
   AND (price-'user_trust')#>>'{image,reservation_credits}' ~ '^(0|[1-9][0-9]{0,18})$'
   AND ((price-'user_trust')#>>'{image,reservation_credits}')::numeric<=9223372036854775807
   AND (price-'user_trust') ?& ARRAY['image','version','model','group','input_per_million','cached_per_million','output_per_million']
   AND (price-'user_trust')-ARRAY['image','upstream_cost_group_ppm','version','model','group','input_per_million','cached_per_million','output_per_million']='{}'::jsonb
   AND coalesce((price-'user_trust')->'upstream_cost_group_ppm','null'::jsonb)='null'::jsonb
   AND (price-'user_trust')->'input_per_million'='0'::jsonb AND (price-'user_trust')->'cached_per_million'='0'::jsonb AND (price-'user_trust')->'output_per_million'='0'::jsonb
   AND jsonb_typeof((price-'user_trust')->'version')='string' AND jsonb_typeof((price-'user_trust')->'model')='string' AND jsonb_typeof((price-'user_trust')->'group')='string') IS TRUE
 AND CASE
 WHEN NOT (price ? 'user_trust') THEN TRUE
 WHEN jsonb_typeof(price->'user_trust') IS DISTINCT FROM 'object' THEN FALSE
 ELSE (
  (price->'user_trust') ?& ARRAY['user_id','policy_revision','basis_revision','effective_level','level_source','discount','evaluated_at','evidence_digest']
  AND (price->'user_trust')-ARRAY['user_id','policy_revision','basis_revision','effective_level','level_source','discount','evaluated_at','next_decay_at','evidence_digest']='{}'::jsonb
  AND CASE WHEN
   jsonb_typeof(price#>'{user_trust,user_id}')='number'
   AND price#>>'{user_trust,user_id}' ~ '^[1-9][0-9]{0,18}$'
   AND jsonb_typeof(price#>'{user_trust,policy_revision}')='number'
   AND price#>>'{user_trust,policy_revision}' ~ '^[1-9][0-9]{0,18}$'
   AND jsonb_typeof(price#>'{user_trust,basis_revision}')='number'
   AND price#>>'{user_trust,basis_revision}' ~ '^(0|[1-9][0-9]{0,18})$'
   AND jsonb_typeof(price#>'{user_trust,effective_level}')='number'
   AND price#>>'{user_trust,effective_level}' ~ '^[0-6]$'
   AND jsonb_typeof(price#>'{user_trust,level_source}')='string'
   AND price#>>'{user_trust,level_source}' IN ('role','override','automatic')
   AND jsonb_typeof(price#>'{user_trust,evaluated_at}')='number'
   AND price#>>'{user_trust,evaluated_at}' ~ '^(0|[1-9][0-9]{0,11})$'
  THEN
   (price#>>'{user_trust,user_id}')::numeric<=9223372036854775807
   AND (price#>>'{user_trust,user_id}')::numeric=actor_user_id
   AND (price#>>'{user_trust,policy_revision}')::numeric<=9223372036854775807
   AND (price#>>'{user_trust,basis_revision}')::numeric<=9223372036854775807
   AND (price#>>'{user_trust,evaluated_at}')::numeric<=253402300799
   AND (((price#>>'{user_trust,effective_level}')::integer>=5)
        = (price#>>'{user_trust,level_source}'='role'))
   AND CASE
    WHEN NOT ((price->'user_trust') ? 'next_decay_at')
         OR price#>'{user_trust,next_decay_at}'='null'::jsonb THEN TRUE
    WHEN jsonb_typeof(price#>'{user_trust,next_decay_at}')='number'
         AND price#>>'{user_trust,next_decay_at}' ~ '^(0|[1-9][0-9]{0,11})$'
    THEN (price#>>'{user_trust,next_decay_at}')::numeric<=253402300799
         AND (price#>>'{user_trust,next_decay_at}')::numeric>(price#>>'{user_trust,evaluated_at}')::numeric
         AND price#>>'{user_trust,level_source}'='automatic'
    ELSE FALSE END
  ELSE FALSE END
  AND CASE WHEN jsonb_typeof(price#>'{user_trust,discount}')='object' THEN (
   (price#>'{user_trust,discount}') ?& ARRAY['numerator','denominator']
   AND (price#>'{user_trust,discount}')-ARRAY['numerator','denominator']='{}'::jsonb
   AND CASE WHEN
    jsonb_typeof(price#>'{user_trust,discount,numerator}')='number'
    AND price#>>'{user_trust,discount,numerator}' ~ '^[1-9][0-9]{0,18}$'
    AND jsonb_typeof(price#>'{user_trust,discount,denominator}')='number'
    AND price#>>'{user_trust,discount,denominator}' ~ '^[1-9][0-9]{0,18}$'
   THEN (price#>>'{user_trust,discount,numerator}')::numeric
          <=(price#>>'{user_trust,discount,denominator}')::numeric
        AND (price#>>'{user_trust,discount,denominator}')::numeric<=1000000000000000000
   ELSE FALSE END
  ) IS TRUE ELSE FALSE END
  AND CASE WHEN jsonb_typeof(price#>'{user_trust,evidence_digest}')='array'
  THEN jsonb_array_length(price#>'{user_trust,evidence_digest}')=32
       AND NOT jsonb_path_exists(price#>'{user_trust,evidence_digest}','$[*] ? (@.type() != "number" || @ < 0 || @ > 255 || @ % 1 != 0)')
       AND jsonb_path_exists(price#>'{user_trust,evidence_digest}','$[*] ? (@ > 0)')
  ELSE FALSE END
 ) IS TRUE
 END
) IS TRUE);

CREATE OR REPLACE FUNCTION core_image.check_charge_atomic() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE operation_id TEXT; operation RECORD; charge RECORD;
 image_price JSONB; ratio JSONB; ratio_name TEXT; part TEXT; raw_value TEXT;
 requested_n NUMERIC; maximum_n NUMERIC; max_budget NUMERIC;
 usd_num NUMERIC; usd_den NUMERIC; group_num NUMERIC; group_den NUMERIC;
 numerators NUMERIC[]; denominators NUMERIC[];
 gcd_a NUMERIC; gcd_b NUMERIC; remainder NUMERIC;
 base_numerator NUMERIC; base_denominator NUMERIC; base_credits NUMERIC;
 discounted_numerator NUMERIC; discounted_denominator NUMERIC;
 discounted_credits NUMERIC; trusted_reservation BIGINT; factor NUMERIC;
BEGIN
 IF TG_TABLE_SCHEMA='core_charging' THEN
  IF NEW.image_operation_id IS NULL THEN RETURN NULL; END IF;
  operation_id:=NEW.image_operation_id;
 ELSE operation_id:=NEW.id;
 END IF;
 SELECT o.* INTO operation FROM core_image.operations o WHERE o.id=operation_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'missing image operation source' USING ERRCODE='23514'; END IF;
 -- The old no-trust path is unchanged. New frozen evidence must also preserve
 -- the original Rust image quote's count, budget and base-overflow checks.
 -- Run before the no-charge return: previews must not persist an invalid base.
 IF operation.price ? 'user_trust' THEN
  image_price:=operation.price->'image';
  IF jsonb_typeof(image_price->'rule') IS DISTINCT FROM 'object'
     OR jsonb_typeof(image_price->'request') IS DISTINCT FROM 'object' THEN
   RAISE EXCEPTION 'invalid trusted image tariff shape' USING ERRCODE='23514';
  END IF;
  FOREACH ratio_name IN ARRAY ARRAY['usd_micros_per_image','group_multiplier'] LOOP
   ratio:=image_price#>ARRAY['rule',ratio_name];
   IF jsonb_typeof(ratio) IS DISTINCT FROM 'object' THEN
    RAISE EXCEPTION 'invalid trusted image ratio shape' USING ERRCODE='23514';
   END IF;
   IF NOT (ratio ?& ARRAY['numerator','denominator'])
      OR ratio-ARRAY['numerator','denominator']<>'{}'::jsonb THEN
    RAISE EXCEPTION 'invalid trusted image ratio fields' USING ERRCODE='23514';
   END IF;
   FOREACH part IN ARRAY ARRAY['numerator','denominator'] LOOP
    raw_value:=ratio->>part;
    IF jsonb_typeof(ratio->part) IS DISTINCT FROM 'number'
       OR raw_value !~ '^(0|[1-9][0-9]{0,19})$' THEN
     RAISE EXCEPTION 'invalid trusted image ratio integer' USING ERRCODE='23514';
    END IF;
    IF raw_value::numeric>18446744073709551615
       OR (part='denominator' AND raw_value::numeric=0) THEN
     RAISE EXCEPTION 'trusted image ratio outside u64 domain' USING ERRCODE='23514';
    END IF;
   END LOOP;
  END LOOP;
  IF jsonb_typeof(image_price#>'{request,n}') IS DISTINCT FROM 'number'
     OR image_price#>>'{request,n}' !~ '^[1-9][0-9]{0,2}$'
     OR jsonb_typeof(image_price#>'{rule,max_n}') IS DISTINCT FROM 'number'
     OR image_price#>>'{rule,max_n}' !~ '^[1-9][0-9]{0,2}$'
     OR jsonb_typeof(image_price#>'{rule,max_budget}') IS DISTINCT FROM 'number'
     OR image_price#>>'{rule,max_budget}' !~ '^(0|[1-9][0-9]{0,18})$' THEN
   RAISE EXCEPTION 'invalid trusted image count or budget integer' USING ERRCODE='23514';
  END IF;
  requested_n:=(image_price#>>'{request,n}')::numeric;
  maximum_n:=(image_price#>>'{rule,max_n}')::numeric;
  max_budget:=(image_price#>>'{rule,max_budget}')::numeric;
  IF requested_n>maximum_n OR maximum_n>128 OR max_budget>9223372036854775807 THEN
   RAISE EXCEPTION 'trusted image count or budget outside domain' USING ERRCODE='23514';
  END IF;
  usd_num:=(image_price#>>'{rule,usd_micros_per_image,numerator}')::numeric;
  usd_den:=(image_price#>>'{rule,usd_micros_per_image,denominator}')::numeric;
  group_num:=(image_price#>>'{rule,group_multiplier,numerator}')::numeric;
  group_den:=(image_price#>>'{rule,group_multiplier,denominator}')::numeric;
  -- Rust validates both the requested quote and the maximum configured count.
  -- Euclidean cancellation precedes each checked u128 base multiplication.
  FOR count_index IN 1..2 LOOP
   numerators:=ARRAY[usd_num,group_num,
       CASE WHEN count_index=1 THEN requested_n ELSE maximum_n END,500000::numeric];
   denominators:=ARRAY[usd_den,group_den,1000000::numeric];
   FOR numerator_index IN 1..4 LOOP
    FOR denominator_index IN 1..3 LOOP
     gcd_a:=numerators[numerator_index];
     gcd_b:=denominators[denominator_index];
     WHILE gcd_b<>0 LOOP
      remainder:=mod(gcd_a,gcd_b); gcd_a:=gcd_b; gcd_b:=remainder;
     END LOOP;
     numerators[numerator_index]:=div(numerators[numerator_index],gcd_a);
     denominators[denominator_index]:=div(denominators[denominator_index],gcd_a);
    END LOOP;
   END LOOP;
   base_numerator:=1; base_denominator:=1;
   FOREACH factor IN ARRAY numerators LOOP
    base_numerator:=base_numerator*factor;
    IF base_numerator>340282366920938463463374607431768211455 THEN
     RAISE EXCEPTION 'trusted image original numerator overflow' USING ERRCODE='23514';
    END IF;
   END LOOP;
   FOREACH factor IN ARRAY denominators LOOP
    base_denominator:=base_denominator*factor;
    IF base_denominator>340282366920938463463374607431768211455 THEN
     RAISE EXCEPTION 'trusted image original denominator overflow' USING ERRCODE='23514';
    END IF;
   END LOOP;
   base_credits:=div(base_numerator,base_denominator)
       + CASE WHEN mod(base_numerator,base_denominator)<>0 THEN 1 ELSE 0 END;
   IF base_credits>9223372036854775807 OR base_credits>max_budget
      OR (count_index=1 AND base_credits IS DISTINCT FROM
          (image_price->>'reservation_credits')::numeric) THEN
    RAISE EXCEPTION 'trusted image original reservation or budget mismatch' USING ERRCODE='23514';
   END IF;
  END LOOP;
  -- Multiply the trust factor into the complete exact amount. Never discount
  -- the already-rounded persisted base reservation, and never use numeric ceil
  -- on division whose scale may round away a nonzero remainder.
  discounted_numerator:=usd_num*group_num*requested_n*500000
      *(operation.price#>>'{user_trust,discount,numerator}')::numeric;
  discounted_denominator:=usd_den*group_den*1000000
      *(operation.price#>>'{user_trust,discount,denominator}')::numeric;
  discounted_credits:=div(discounted_numerator,discounted_denominator)
      + CASE WHEN mod(discounted_numerator,discounted_denominator)<>0 THEN 1 ELSE 0 END;
  IF discounted_credits>9223372036854775807 THEN
   RAISE EXCEPTION 'trusted image discounted reservation overflow' USING ERRCODE='23514';
  END IF;
  trusted_reservation:=discounted_credits::bigint;
 END IF;

 SELECT q.* INTO charge FROM core_charging.requests q WHERE q.image_operation_id=operation_id;
 IF NOT FOUND THEN
  IF operation.state NOT IN ('previewed','confirmed') OR operation.charge_id IS NOT NULL THEN
   RAISE EXCEPTION 'missing image charge' USING ERRCODE='23514';
  END IF;
  RETURN NULL;
 END IF;
 IF charge.id<>operation.id OR charge.image_operation_id<>operation.id
    OR charge.actor_user_id<>operation.actor_user_id
    OR charge.owner_account_id<>operation.owner_account_id
    OR (operation.source_kind='drawing' AND (charge.key_id IS NOT NULL OR charge.payer_account_id<>operation.owner_account_id))
    OR (operation.source_kind='api_key' AND charge.key_id IS DISTINCT FROM operation.key_id)
    OR charge.oauth_source IS NOT NULL
    OR charge.assistant_grant_id IS NOT NULL OR charge.assistant_sponsor_account_id IS NOT NULL
    OR charge.fingerprint IS DISTINCT FROM operation.fingerprint OR charge.price IS DISTINCT FROM operation.price
    OR charge.reserved IS DISTINCT FROM (CASE WHEN operation.price ? 'user_trust' THEN trusted_reservation
       ELSE (operation.price#>>'{image,reservation_credits}')::bigint END)
    OR charge.input_tokens<>0 OR charge.cached_tokens<>0 OR charge.output_tokens<>0 THEN
  RAISE EXCEPTION 'image charge source mismatch' USING ERRCODE='23514';
 END IF;
 IF (operation.state='confirmed' AND (charge.state NOT IN ('reserved','released') OR operation.charge_id IS NOT NULL))
    OR (operation.state='dispatching' AND (charge.state NOT IN ('streaming','reconcile') OR operation.charge_id IS DISTINCT FROM charge.id))
    OR (operation.state='billing_pending' AND (charge.state NOT IN ('streaming','reconcile','settled') OR operation.charge_id IS DISTINCT FROM charge.id))
    OR operation.state='previewed'
    OR (operation.state='completed' AND charge.state<>'settled') THEN
  RAISE EXCEPTION 'image operation and charge states must commit together' USING ERRCODE='23514';
 END IF;
 IF charge.image_usage IS NOT NULL AND charge.image_usage IS DISTINCT FROM operation.image_usage THEN
  RAISE EXCEPTION 'image usage evidence mismatch' USING ERRCODE='23514';
 END IF;
 IF charge.state='settled' AND (operation.state NOT IN ('billing_pending','completed')
    OR operation.output IS NULL OR operation.image_usage IS DISTINCT FROM charge.image_usage
    OR charge.outcome IS DISTINCT FROM 'completed') THEN
  RAISE EXCEPTION 'image settlement requires durable original output receipt' USING ERRCODE='23514';
 END IF;
 IF operation.state='completed' AND (operation.charge_id IS DISTINCT FROM charge.id
    OR operation.settled_credits IS DISTINCT FROM charge.settled
    OR operation.image_usage IS DISTINCT FROM charge.image_usage
    OR operation.output IS NULL OR charge.outcome IS DISTINCT FROM 'completed') THEN
  RAISE EXCEPTION 'image output and completed settlement must commit together' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
