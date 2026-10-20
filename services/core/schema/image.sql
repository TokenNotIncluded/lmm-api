-- REVIEW ARTIFACT ONLY. Fresh install after identity, charging and tool-market.
-- No extensions, migrations, runtime DDL or new ledger authority.
CREATE SCHEMA core_image;

CREATE FUNCTION core_image.valid_output(value JSONB) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE item JSONB;
BEGIN
 IF jsonb_typeof(value)<>'object'
    OR value-ARRAY['created','data']<>'{}'::jsonb
    OR jsonb_typeof(value->'data') IS DISTINCT FROM 'array'
    -- JSONB inserts structural spaces when printing an admitted provider JSON.
    -- Keep 4 KiB for that bounded (at most 128 items) representation overhead;
    -- the provider still enforces its 32 MiB payload + 64 KiB raw envelope.
    OR octet_length(value::text)>33624064 THEN RETURN FALSE; END IF;
 IF jsonb_array_length(value->'data') NOT BETWEEN 1 AND 128 THEN RETURN FALSE; END IF;
 IF value ? 'created' AND (jsonb_typeof(value->'created') IS DISTINCT FROM 'number'
    OR (value->>'created') !~ '^(0|[1-9][0-9]{0,19})$') THEN RETURN FALSE; END IF;
 FOR item IN SELECT e.value FROM jsonb_array_elements(value->'data') AS e(value) LOOP
  IF jsonb_typeof(item)<>'object'
     OR item-ARRAY['b64_json','url','revised_prompt']<>'{}'::jsonb
     OR (item ? 'b64_json')=(item ? 'url') THEN RETURN FALSE; END IF;
  IF item ? 'b64_json' AND (jsonb_typeof(item->'b64_json') IS DISTINCT FROM 'string'
     OR octet_length(item->>'b64_json') NOT BETWEEN 1 AND 33554432) THEN RETURN FALSE; END IF;
  IF item ? 'url' AND (jsonb_typeof(item->'url') IS DISTINCT FROM 'string'
     OR octet_length(item->>'url') NOT BETWEEN 1 AND 8192
     OR (item->>'url') !~ '^https://[^[:space:][:cntrl:]]+$') THEN RETURN FALSE; END IF;
  IF item ? 'revised_prompt' AND (jsonb_typeof(item->'revised_prompt') IS DISTINCT FROM 'string'
     OR octet_length(item->>'revised_prompt')>65536) THEN RETURN FALSE; END IF;
 END LOOP;
 -- This is a shape/size guard. Core independently parses URL credentials,
 -- validates base64 and removes provider credentials before saving this value.
 RETURN TRUE;
END $$;

CREATE TABLE core_image.operations (
 id TEXT PRIMARY KEY CHECK(id ~ '^image:[0-9a-f]{64}$'),
 source_kind TEXT NOT NULL DEFAULT 'drawing' CHECK(source_kind IN ('drawing','api_key')),
 session_id BIGINT REFERENCES core_identity.credentials(id),
 key_id BIGINT REFERENCES core_identity.credentials(id),
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 owner_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 auth_version BIGINT NOT NULL CHECK(auth_version>0),
 context JSONB CHECK(context IS NULL OR ((jsonb_typeof(context)='object'
   AND context ?& ARRAY['call_id','dispatch_id','tool_id','version_id','grant_id','input_digest']
   AND context-ARRAY['call_id','dispatch_id','tool_id','version_id','grant_id','input_digest']='{}'::jsonb
   AND jsonb_typeof(context->'call_id')='string' AND octet_length(context->>'call_id') BETWEEN 1 AND 256
   AND jsonb_typeof(context->'dispatch_id')='string' AND octet_length(context->>'dispatch_id') BETWEEN 1 AND 256
   AND jsonb_typeof(context->'tool_id')='string' AND octet_length(context->>'tool_id') BETWEEN 1 AND 256
   AND jsonb_typeof(context->'version_id')='string' AND octet_length(context->>'version_id') BETWEEN 1 AND 256
   AND jsonb_typeof(context->'grant_id')='string' AND octet_length(context->>'grant_id') BETWEEN 1 AND 256
   AND jsonb_typeof(context->'input_digest')='string' AND context->>'input_digest' ~ '^[0-9a-f]{64}$') IS TRUE)),
 input JSONB NOT NULL CHECK(jsonb_typeof(input)='object'),
 fingerprint BYTEA NOT NULL CHECK(octet_length(fingerprint)=32 AND fingerprint<>decode(repeat('00',32),'hex')),
 price JSONB NOT NULL CHECK((jsonb_typeof(price)='object' AND jsonb_typeof(price->'image')='object'
   AND (price->'image') ?& ARRAY['revision','rule','request','reservation_credits']
   AND (price->'image')-ARRAY['revision','rule','request','reservation_credits']='{}'::jsonb
   AND jsonb_typeof(price#>'{image,reservation_credits}')='number'
   AND price#>>'{image,reservation_credits}' ~ '^(0|[1-9][0-9]{0,18})$'
   AND (price#>>'{image,reservation_credits}')::numeric<=9223372036854775807
   AND price ?& ARRAY['image','version','model','group','input_per_million','cached_per_million','output_per_million']
   AND price-ARRAY['image','upstream_cost_group_ppm','version','model','group','input_per_million','cached_per_million','output_per_million']='{}'::jsonb
   AND coalesce(price->'upstream_cost_group_ppm','null'::jsonb)='null'::jsonb
   AND price->'input_per_million'='0'::jsonb AND price->'cached_per_million'='0'::jsonb AND price->'output_per_million'='0'::jsonb
   AND jsonb_typeof(price->'version')='string' AND jsonb_typeof(price->'model')='string' AND jsonb_typeof(price->'group')='string') IS TRUE),
 state TEXT NOT NULL DEFAULT 'previewed' CHECK(state IN ('previewed','confirmed','dispatching','billing_pending','completed')),
 created_at BIGINT NOT NULL CHECK(created_at>0),
 expires_at BIGINT NOT NULL CHECK(expires_at>created_at),
 confirmed_at BIGINT CHECK(confirmed_at>=created_at AND confirmed_at<expires_at),
 confirmation_digest BYTEA CHECK(confirmation_digest IS NULL OR (octet_length(confirmation_digest)=32 AND confirmation_digest<>decode(repeat('00',32),'hex'))),
 confirmation_sid TEXT CHECK(confirmation_sid IS NULL OR (octet_length(confirmation_sid) BETWEEN 1 AND 256 AND confirmation_sid ~ '^[A-Za-z0-9_-]+$')),
 charge_id TEXT UNIQUE CHECK(charge_id IS NULL OR charge_id=id),
 output JSONB CHECK(output IS NULL OR core_image.valid_output(output)),
 output_digest BYTEA CHECK(output_digest IS NULL OR (octet_length(output_digest)=32 AND output_digest<>decode(repeat('00',32),'hex'))),
 image_usage JSONB CHECK(image_usage IS NULL OR ((jsonb_typeof(image_usage)='object'
   AND image_usage ?& ARRAY['output_images','provider_receipt_sha256']
   AND image_usage-ARRAY['output_images','provider_receipt_sha256']='{}'::jsonb
   AND jsonb_typeof(image_usage->'output_images')='number'
   AND (image_usage->>'output_images') ~ '^[1-9][0-9]{0,2}$'
   AND (image_usage->>'output_images')::integer BETWEEN 1 AND 128
   AND jsonb_typeof(image_usage->'provider_receipt_sha256')='array'
   AND jsonb_array_length(image_usage->'provider_receipt_sha256')=32
   AND NOT jsonb_path_exists(image_usage->'provider_receipt_sha256','$[*] ? (@.type() != "number" || @ < 0 || @ > 255 || @ % 1 != 0)')
   AND jsonb_path_exists(image_usage->'provider_receipt_sha256','$[*] ? (@ > 0)')) IS TRUE)),
 settled_credits BIGINT CHECK(settled_credits>=0),
 CHECK((source_kind='drawing' AND session_id IS NOT NULL AND key_id IS NULL AND context IS NOT NULL)
   OR (source_kind='api_key' AND session_id IS NULL AND key_id IS NOT NULL AND context IS NULL
       AND confirmation_digest IS NULL AND confirmation_sid IS NULL AND state<>'previewed')),
 CHECK((confirmation_digest IS NULL)=(confirmation_sid IS NULL)),
 CHECK((output IS NULL)=(output_digest IS NULL) AND (output IS NULL)=(image_usage IS NULL)),
 CHECK(output IS NULL OR ((image_usage->>'output_images')::integer=jsonb_array_length(output->'data'))),
 CHECK((state='previewed' AND confirmed_at IS NULL AND charge_id IS NULL AND output IS NULL AND settled_credits IS NULL)
   OR (state='confirmed' AND confirmed_at IS NOT NULL AND (source_kind='api_key' OR confirmation_digest IS NOT NULL) AND charge_id IS NULL AND output IS NULL AND settled_credits IS NULL)
   OR (state IN ('dispatching','billing_pending') AND confirmed_at IS NOT NULL AND (source_kind='api_key' OR confirmation_digest IS NOT NULL) AND charge_id=id AND settled_credits IS NULL)
   OR (state='completed' AND confirmed_at IS NOT NULL AND (source_kind='api_key' OR confirmation_digest IS NOT NULL) AND charge_id=id AND output IS NOT NULL AND settled_credits IS NOT NULL)),
 CONSTRAINT image_charge_source FOREIGN KEY(charge_id) REFERENCES core_charging.requests(id) DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX image_one_operation_per_dispatch ON core_image.operations((context->>'call_id'),(context->>'dispatch_id')) WHERE source_kind='drawing';
-- id is already globally unique. The trusted API service derives its stable id
-- from key_id + caller request identity + operation kind before admission.
CREATE UNIQUE INDEX image_one_operation_per_key_identity ON core_image.operations(key_id,id) WHERE source_kind='api_key';
CREATE INDEX image_session_created ON core_image.operations(session_id,created_at,id);
CREATE INDEX image_pending ON core_image.operations(state,created_at,id) WHERE state IN ('dispatching','billing_pending');
ALTER TABLE core_charging.requests ADD CONSTRAINT image_operation_source
 FOREIGN KEY(image_operation_id) REFERENCES core_image.operations(id) DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION core_image.guard_operation() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') THEN
  RAISE EXCEPTION 'immutable image operation history' USING ERRCODE='55000';
 END IF;
 IF TG_OP='INSERT' THEN
  IF (NEW.source_kind='drawing' AND NEW.state<>'previewed')
     OR (NEW.source_kind='api_key' AND (NEW.state<>'confirmed' OR NEW.confirmed_at IS DISTINCT FROM NEW.created_at
         OR NEW.created_at>floor(extract(epoch FROM clock_timestamp()))::bigint
         OR NEW.expires_at<=floor(extract(epoch FROM clock_timestamp()))::bigint)) THEN
   RAISE EXCEPTION 'invalid initial image source state' USING ERRCODE='23514';
  END IF;
  IF NOT EXISTS(SELECT 1 FROM core_identity.credentials c
      WHERE c.id=coalesce(NEW.session_id,NEW.key_id)
        AND c.kind=CASE WHEN NEW.source_kind='drawing' THEN 'session' ELSE 'api_key' END
        AND c.user_id=NEW.actor_user_id AND c.owner_account_id=NEW.owner_account_id
        AND c.user_version=NEW.auth_version) THEN
   RAISE EXCEPTION 'invalid original image credential binding' USING ERRCODE='23514';
  END IF;
  RETURN NEW;
 END IF;
 IF (OLD.state='completed' AND NEW IS DISTINCT FROM OLD)
    OR (to_jsonb(NEW)-ARRAY['state','confirmed_at','confirmation_digest','confirmation_sid','charge_id','output','output_digest','image_usage','settled_credits'])
       IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','confirmed_at','confirmation_digest','confirmation_sid','charge_id','output','output_digest','image_usage','settled_credits'])
    OR (OLD.confirmed_at IS NOT NULL AND NEW.confirmed_at IS DISTINCT FROM OLD.confirmed_at)
    OR (OLD.state<>'previewed' AND (NEW.confirmation_digest,NEW.confirmation_sid) IS DISTINCT FROM (OLD.confirmation_digest,OLD.confirmation_sid))
    OR (OLD.charge_id IS NOT NULL AND NEW.charge_id IS DISTINCT FROM OLD.charge_id)
    OR (OLD.output IS NOT NULL AND (NEW.output,NEW.output_digest,NEW.image_usage) IS DISTINCT FROM (OLD.output,OLD.output_digest,OLD.image_usage))
    OR (OLD.settled_credits IS NOT NULL AND NEW.settled_credits IS DISTINCT FROM OLD.settled_credits) THEN
  RAISE EXCEPTION 'immutable image operation facts' USING ERRCODE='23514';
 END IF;
 IF NEW.state<>OLD.state AND NOT (
    (OLD.state='previewed' AND NEW.state='confirmed')
    OR (OLD.state='confirmed' AND NEW.state='dispatching')
    OR (OLD.state='dispatching' AND NEW.state IN ('billing_pending','completed'))
    OR (OLD.state='billing_pending' AND NEW.state='completed')) THEN
  RAISE EXCEPTION 'invalid image operation transition' USING ERRCODE='23514';
 END IF;
 IF OLD.state='previewed' AND NEW.state='confirmed' AND
    (NEW.confirmed_at>floor(extract(epoch FROM clock_timestamp()))::bigint
     OR NEW.expires_at<=floor(extract(epoch FROM clock_timestamp()))::bigint) THEN
  RAISE EXCEPTION 'expired or future image confirmation' USING ERRCODE='23514';
 END IF;
 IF OLD.state='confirmed' AND NEW.state='dispatching' AND NEW.expires_at<=floor(extract(epoch FROM clock_timestamp()))::bigint THEN
  RAISE EXCEPTION 'expired image dispatch' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER image_operation_guard BEFORE INSERT OR UPDATE OR DELETE ON core_image.operations FOR EACH ROW EXECUTE FUNCTION core_image.guard_operation();
CREATE TRIGGER image_operation_no_truncate BEFORE TRUNCATE ON core_image.operations FOR EACH STATEMENT EXECUTE FUNCTION core_image.guard_operation();

-- Inspect final transaction state, not the intermediate statement ordering.
-- No output may complete before charging settles. A durable billing_pending
-- receipt is sufficient for the separate Charging.settle transaction; recovery
-- may subsequently mark it completed without another provider request.
CREATE FUNCTION core_image.check_charge_atomic() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE operation_id TEXT; operation RECORD; charge RECORD;
BEGIN
 IF TG_TABLE_SCHEMA='core_charging' THEN
  IF NEW.image_operation_id IS NULL THEN RETURN NULL; END IF;
  operation_id:=NEW.image_operation_id;
 ELSE operation_id:=NEW.id;
 END IF;
 SELECT o.* INTO operation FROM core_image.operations o WHERE o.id=operation_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'missing image operation source' USING ERRCODE='23514'; END IF;
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
    OR charge.reserved IS DISTINCT FROM (operation.price#>>'{image,reservation_credits}')::bigint
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
CREATE CONSTRAINT TRIGGER image_operation_charge_atomic AFTER INSERT OR UPDATE ON core_image.operations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_image.check_charge_atomic();
CREATE CONSTRAINT TRIGGER image_charge_operation_atomic AFTER INSERT OR UPDATE ON core_charging.requests
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_image.check_charge_atomic();

-- Installer owns schema objects. Runtime grants belong to the reviewed role
-- provisioning script; do not give runtime CREATE or table-wide UPDATE.
REVOKE ALL ON SCHEMA core_image FROM PUBLIC;
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA core_image FROM PUBLIC;
