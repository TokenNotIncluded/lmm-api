-- Additive agent component request v2; root review/install required before activation.
-- Existing personal_text/gift source and ledger are retained. Each round links
-- one actual Charging request and its existing assistant grant.
CREATE TABLE core_assistant.agent_turns (
 id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
 conversation_id TEXT NOT NULL REFERENCES core_assistant.conversations(id),
 client_turn_id TEXT NOT NULL CHECK(octet_length(client_turn_id) BETWEEN 1 AND 128),
 session_id BIGINT NOT NULL REFERENCES core_identity.credentials(id),
 source_web_session_sid TEXT REFERENCES core_identity.web_sessions(sid),
 auth_version BIGINT NOT NULL CHECK(auth_version>0),
 owner_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 owner_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 original_text TEXT NOT NULL CHECK(octet_length(original_text) BETWEEN 1 AND 8192),
 text_sha256 BYTEA NOT NULL CHECK(octet_length(text_sha256)=32),
 history_position BIGINT NOT NULL UNIQUE DEFAULT nextval('core_assistant.operations_position_seq'),
 price JSONB NOT NULL CHECK(jsonb_typeof(price)='object'),
 loaded_tools JSONB NOT NULL CHECK(jsonb_typeof(loaded_tools)='array'),
 registry_sha256 BYTEA NOT NULL CHECK(octet_length(registry_sha256)=32),
 host_policy JSONB NOT NULL CHECK(jsonb_typeof(host_policy)='object'),
 max_rounds INTEGER NOT NULL CHECK(max_rounds BETWEEN 1 AND 32),
 budget_credits BIGINT NOT NULL CHECK(budget_credits BETWEEN 1 AND 9007199254740991),
 created_at BIGINT NOT NULL DEFAULT extract(epoch FROM clock_timestamp())::bigint,
 expires_at BIGINT NOT NULL CHECK(expires_at>created_at AND expires_at-created_at<=900),
 phase TEXT NOT NULL CHECK(phase IN ('new','model_running','tools_pending','completed','unknown','rejected','expired')),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 final_text TEXT CHECK(octet_length(final_text) BETWEEN 1 AND 32768),
 UNIQUE(conversation_id,client_turn_id),
 CHECK((phase='completed')=(final_text IS NOT NULL))
);
CREATE UNIQUE INDEX agent_one_active_turn ON core_assistant.agent_turns(conversation_id)
 WHERE phase IN ('new','model_running','tools_pending','unknown');
CREATE TABLE core_assistant.agent_rounds (
 id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
 turn_id TEXT NOT NULL REFERENCES core_assistant.agent_turns(id),
 round INTEGER NOT NULL CHECK(round BETWEEN 0 AND 31),
 model_request_id TEXT NOT NULL UNIQUE REFERENCES core_charging.requests(id),
 context_json BYTEA NOT NULL CHECK(octet_length(context_json) BETWEEN 2 AND 1048576),
 context_sha256 BYTEA NOT NULL CHECK(octet_length(context_sha256)=32),
 provider_response_json BYTEA CHECK(octet_length(provider_response_json) BETWEEN 2 AND 131072),
 provider_response_sha256 BYTEA CHECK(octet_length(provider_response_sha256)=32),
 state TEXT NOT NULL CHECK(state IN ('prepared','streaming','settled','unknown','released')),
 UNIQUE(turn_id,round), UNIQUE(id,turn_id),
 CHECK((provider_response_json IS NULL)=(provider_response_sha256 IS NULL)),
 CHECK(state<>'settled' OR provider_response_sha256 IS NOT NULL)
);
CREATE TABLE core_assistant.agent_tool_operations (
 id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
 turn_id TEXT NOT NULL REFERENCES core_assistant.agent_turns(id),
 round_id TEXT NOT NULL,
 ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 7),
 provider_call_id TEXT NOT NULL CHECK(provider_call_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
 tool_name TEXT NOT NULL CHECK(octet_length(tool_name) BETWEEN 1 AND 128),
 tool_version TEXT NOT NULL CHECK(octet_length(tool_version) BETWEEN 1 AND 128),
 schema_sha256 BYTEA NOT NULL CHECK(octet_length(schema_sha256)=32),
 price_version TEXT NOT NULL CHECK(octet_length(price_version) BETWEEN 1 AND 128),
 price_sha256 BYTEA NOT NULL CHECK(octet_length(price_sha256)=32),
 resource_kind TEXT NOT NULL CHECK(octet_length(resource_kind)<=128),
 resource_id TEXT NOT NULL CHECK(octet_length(resource_id)<=256),
 resource_version TEXT NOT NULL CHECK(octet_length(resource_version)<=128),
 resource_schema_sha256 BYTEA CHECK(octet_length(resource_schema_sha256)=32),
 resource_price_sha256 BYTEA CHECK(octet_length(resource_price_sha256)=32),
 scope_grant_id TEXT NOT NULL CHECK(octet_length(scope_grant_id)<=256),
 domain TEXT NOT NULL CHECK(domain IN ('registered_core','extension')),
 arguments_json BYTEA NOT NULL CHECK(octet_length(arguments_json) BETWEEN 2 AND 65536),
 arguments_sha256 BYTEA NOT NULL CHECK(octet_length(arguments_sha256)=32),
 state TEXT NOT NULL CHECK(state IN ('pending','claimed','completed','unknown','cancelled')),
 claim_key TEXT CHECK(octet_length(claim_key) BETWEEN 1 AND 128),
 result_json BYTEA CHECK(octet_length(result_json) BETWEEN 2 AND 65536),
 result_sha256 BYTEA CHECK(octet_length(result_sha256)=32),
 domain_receipt_id TEXT CHECK(octet_length(domain_receipt_id) BETWEEN 1 AND 256),
 FOREIGN KEY(round_id,turn_id) REFERENCES core_assistant.agent_rounds(id,turn_id),
 UNIQUE(round_id,ordinal), UNIQUE(round_id,provider_call_id),
 CHECK(state<>'pending' OR claim_key IS NULL),
 CHECK(state NOT IN ('claimed','completed','unknown') OR claim_key IS NOT NULL),
 CHECK((result_json IS NULL)=(result_sha256 IS NULL)),
 CHECK(state<>'completed' OR result_json IS NOT NULL)
);
CREATE TABLE core_assistant.agent_tool_observations (
 position BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 operation_id TEXT NOT NULL REFERENCES core_assistant.agent_tool_operations(id),
 claim_key TEXT NOT NULL CHECK(octet_length(claim_key) BETWEEN 1 AND 128),
 state TEXT NOT NULL CHECK(state IN ('unknown','completed')),
 result_json BYTEA NOT NULL CHECK(octet_length(result_json) BETWEEN 2 AND 65536),
 result_sha256 BYTEA NOT NULL CHECK(result_sha256=sha256(result_json)),
 domain_receipt_id TEXT CHECK(octet_length(domain_receipt_id) BETWEEN 1 AND 256),
 UNIQUE(operation_id,state,result_sha256)
);
CREATE FUNCTION core_assistant.guard_agent_turn() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR
 (to_jsonb(NEW)-ARRAY['phase','revision','final_text','loaded_tools']) IS DISTINCT FROM
 (to_jsonb(OLD)-ARRAY['phase','revision','final_text','loaded_tools']) OR
 NEW.revision<>OLD.revision+1 OR OLD.phase IN ('completed','unknown','rejected','expired') OR
 (NEW.loaded_tools @> OLD.loaded_tools AND (NEW.host_policy->'allowed_tools') @> NEW.loaded_tools) IS NOT TRUE OR
 NOT (NEW.phase=OLD.phase OR
  (OLD.phase='new' AND NEW.phase IN ('model_running','completed','rejected','expired')) OR
  (OLD.phase='model_running' AND NEW.phase IN ('tools_pending','completed','unknown')) OR
  (OLD.phase='tools_pending' AND NEW.phase IN ('model_running','completed','rejected','expired'))) THEN
  RAISE EXCEPTION 'immutable or invalid agent turn transition' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER agent_turn_immutable BEFORE UPDATE OR DELETE ON core_assistant.agent_turns
 FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_agent_turn();
CREATE FUNCTION core_assistant.guard_agent_round() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR
 (to_jsonb(NEW)-ARRAY['state','provider_response_json','provider_response_sha256']) IS DISTINCT FROM
 (to_jsonb(OLD)-ARRAY['state','provider_response_json','provider_response_sha256']) OR
 OLD.state IN ('settled','unknown','released') OR
 NOT ((OLD.state='prepared' AND NEW.state IN ('streaming','released')) OR
      (OLD.state='streaming' AND NEW.state IN ('settled','unknown'))) THEN
  RAISE EXCEPTION 'immutable or invalid agent round transition' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER agent_round_immutable BEFORE UPDATE OR DELETE ON core_assistant.agent_rounds
 FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_agent_round();
CREATE FUNCTION core_assistant.guard_agent_tool() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR
 (to_jsonb(NEW)-ARRAY['state','claim_key','result_json','result_sha256','domain_receipt_id']) IS DISTINCT FROM
 (to_jsonb(OLD)-ARRAY['state','claim_key','result_json','result_sha256','domain_receipt_id']) OR
 OLD.state IN ('completed','cancelled') OR
 (OLD.claim_key IS NOT NULL AND NEW.claim_key IS DISTINCT FROM OLD.claim_key) OR
 NOT ((OLD.state='pending' AND NEW.state IN ('claimed','completed','cancelled')) OR
      (OLD.state='claimed' AND NEW.state IN ('completed','unknown')) OR
      (OLD.state='unknown' AND NEW.state='completed')) THEN
  RAISE EXCEPTION 'immutable or invalid agent tool transition' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER agent_tool_immutable BEFORE UPDATE OR DELETE ON core_assistant.agent_tool_operations
 FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_agent_tool();
CREATE TRIGGER agent_observation_immutable BEFORE UPDATE OR DELETE ON core_assistant.agent_tool_observations
 FOR EACH ROW EXECUTE FUNCTION core_assistant.guard_conversation();
-- Invoker-rights, deferred: see complete settlement/answer/round/tool writes in
-- one transaction. No SECURITY DEFINER and no separate spending ledger.
CREATE FUNCTION core_assistant.check_agent_binding() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE turn_key TEXT; conversation TEXT; t RECORD; spent NUMERIC; round_count BIGINT;
BEGIN
 IF TG_TABLE_NAME='agent_turns' THEN turn_key:=NEW.id;
 ELSIF TG_TABLE_NAME='requests' THEN
  IF NEW.assistant_grant_id IS NULL THEN RETURN NULL; END IF;
  SELECT turn_id INTO turn_key FROM core_assistant.agent_rounds WHERE model_request_id=NEW.id;
 ELSIF TG_TABLE_NAME='agent_tool_observations' THEN
  SELECT turn_id INTO turn_key FROM core_assistant.agent_tool_operations WHERE id=NEW.operation_id;
 ELSE turn_key:=NEW.turn_id; END IF;
 IF turn_key IS NULL THEN RETURN NULL; END IF;
 IF current_setting('transaction_isolation') NOT IN ('read committed','serializable') THEN
  RAISE EXCEPTION 'agent writes require read committed or serializable isolation' USING ERRCODE='23514';
 END IF;
 SELECT conversation_id INTO STRICT conversation FROM core_assistant.agent_turns WHERE id=turn_key;
 PERFORM pg_advisory_xact_lock(hashtextextended(conversation,908534117));
 SELECT * INTO STRICT t FROM core_assistant.agent_turns WHERE id=turn_key FOR NO KEY UPDATE;
 IF t.phase IN ('new','model_running','tools_pending','unknown') AND EXISTS(
  SELECT FROM core_charging.requests q JOIN core_assistant.grants g ON g.id=q.assistant_grant_id
  WHERE g.conversation_id=t.conversation_id AND q.state NOT IN ('settled','released')
   AND NOT EXISTS(SELECT FROM core_assistant.agent_rounds r WHERE r.model_request_id=q.id)) THEN
  RAISE EXCEPTION 'agent and legacy admission conflict' USING ERRCODE='23514';
 END IF;
 IF jsonb_typeof(t.host_policy->'allowed_tools') IS DISTINCT FROM 'array' OR
 jsonb_typeof(t.host_policy->'model') IS DISTINCT FROM 'object' OR
 jsonb_typeof(t.host_policy->'max_rounds') IS DISTINCT FROM 'number' OR
 jsonb_typeof(t.host_policy->'budget_credits') IS DISTINCT FROM 'number' OR
 jsonb_typeof(t.host_policy->'max_tool_calls') IS DISTINCT FROM 'number' OR
 (t.host_policy->>'max_rounds')::integer IS DISTINCT FROM t.max_rounds OR
 (t.host_policy->>'budget_credits')::bigint IS DISTINCT FROM t.budget_credits OR
 (t.host_policy->>'max_tool_calls')::integer NOT BETWEEN 1 AND 256 OR
 t.text_sha256<>sha256(convert_to(t.original_text,'UTF8')) OR
 ((t.host_policy->'allowed_tools') @> t.loaded_tools) IS NOT TRUE OR
 (t.host_policy->>'max_rounds')::integer<>t.max_rounds OR
 (t.host_policy->>'budget_credits')::bigint<>t.budget_credits OR
 NOT EXISTS(SELECT FROM core_assistant.conversations v JOIN core_identity.credentials c ON c.id=t.session_id
  WHERE v.id=t.conversation_id AND v.owner_user_id=t.owner_user_id AND v.owner_account_id=t.owner_account_id
   AND c.kind='session' AND c.user_id=t.owner_user_id AND c.owner_account_id=t.owner_account_id AND c.user_version=t.auth_version) OR
 (SELECT sid FROM core_identity.web_session_credentials WHERE credential_id=t.session_id) IS DISTINCT FROM t.source_web_session_sid OR
 (t.source_web_session_sid IS NOT NULL AND NOT EXISTS(SELECT FROM core_identity.web_sessions w WHERE w.sid=t.source_web_session_sid AND w.user_id=t.owner_user_id)) THEN
  RAISE EXCEPTION 'agent original authority binding' USING ERRCODE='23514';
 END IF;
 SELECT count(*),coalesce(sum(CASE WHEN q.state='settled' THEN q.settled
  WHEN q.state IN ('reserved','streaming','reconcile') THEN q.reserved ELSE 0 END),0)
 INTO round_count,spent FROM core_assistant.agent_rounds r JOIN core_charging.requests q ON q.id=r.model_request_id WHERE r.turn_id=t.id;
 IF round_count>t.max_rounds OR spent>t.budget_credits OR spent<0 OR
 (SELECT count(*) FROM core_assistant.agent_rounds WHERE turn_id=t.id AND state IN ('prepared','streaming','unknown'))>1 OR
 EXISTS(SELECT FROM core_assistant.agent_rounds later JOIN core_assistant.agent_tool_operations earlier ON earlier.turn_id=later.turn_id
  JOIN core_assistant.agent_rounds prior ON prior.id=earlier.round_id WHERE later.turn_id=t.id AND prior.round<later.round AND earlier.state IN ('pending','claimed','unknown')) OR
 EXISTS(SELECT FROM core_assistant.agent_rounds r
  JOIN core_charging.requests q ON q.id=r.model_request_id
  LEFT JOIN core_assistant.grants g ON g.id=q.assistant_grant_id
  LEFT JOIN core_assistant.operations o ON o.request_id=q.id
  WHERE r.turn_id=t.id AND (r.round>=t.max_rounds OR
   g.id IS NULL OR o.grant_id IS DISTINCT FROM g.id OR
   g.conversation_id<>t.conversation_id OR g.turn_id<>('agent-'||t.id||'-'||r.round::text) OR
   g.session_id<>t.session_id OR g.auth_version<>t.auth_version OR
   g.policy IS DISTINCT FROM t.host_policy->'model' OR g.price IS DISTINCT FROM t.price OR q.price IS DISTINCT FROM t.price OR
   q.actor_user_id<>t.owner_user_id OR q.owner_account_id<>t.owner_account_id OR
   o.text<>t.original_text OR o.text_sha256<>t.text_sha256 OR
   r.context_sha256<>sha256(r.context_json) OR
   (r.provider_response_json IS NOT NULL AND r.provider_response_sha256<>sha256(r.provider_response_json)) OR
   NOT ((r.state='streaming' AND q.state IN ('streaming','reconcile')) OR (r.state='settled' AND q.state='settled') OR
        (r.state='unknown' AND q.state='reconcile') OR (r.state='released' AND q.state='released')))) THEN
  RAISE EXCEPTION 'agent original model grant or parent budget binding' USING ERRCODE='23514';
 END IF;
 IF EXISTS(SELECT FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_rounds r ON r.id=o.round_id JOIN core_charging.requests q ON q.id=r.model_request_id
  WHERE o.turn_id=t.id AND (r.state<>'settled' OR q.state<>'settled' OR q.outcome IS DISTINCT FROM 'completed' OR r.provider_response_sha256 IS NULL OR
   o.arguments_sha256<>sha256(o.arguments_json) OR
   ((t.host_policy->'allowed_tools') ? o.tool_name) IS NOT TRUE OR
   (o.result_json IS NOT NULL AND o.result_sha256<>sha256(o.result_json)) OR
   NOT EXISTS(SELECT FROM jsonb_array_elements(convert_from(r.provider_response_json,'UTF8')::jsonb #> '{choices,0,message,tool_calls}') call
    WHERE call->>'id'=o.provider_call_id AND call#>>'{function,name}'=o.tool_name
      AND convert_to(call#>>'{function,arguments}','UTF8')=o.arguments_json))) OR
 (SELECT count(*) FROM core_assistant.agent_tool_operations WHERE turn_id=t.id)>(t.host_policy->>'max_tool_calls')::integer THEN
  RAISE EXCEPTION 'agent tool provider provenance binding' USING ERRCODE='23514';
 END IF;
 IF t.phase='completed' AND (EXISTS(SELECT FROM core_assistant.agent_rounds WHERE turn_id=t.id AND state NOT IN ('settled','released'))
  OR EXISTS(SELECT FROM core_assistant.agent_tool_operations WHERE turn_id=t.id AND state IN ('pending','claimed','unknown'))) THEN
  RAISE EXCEPTION 'agent completed with unresolved effects' USING ERRCODE='23514';
 END IF;
 IF EXISTS(SELECT FROM core_assistant.agent_tool_observations b JOIN core_assistant.agent_tool_operations o ON o.id=b.operation_id
  WHERE o.turn_id=t.id AND (b.claim_key IS DISTINCT FROM o.claim_key OR (b.state='completed' AND
   (o.state<>'completed' OR b.result_sha256 IS DISTINCT FROM o.result_sha256 OR b.domain_receipt_id IS DISTINCT FROM o.domain_receipt_id)))) THEN
  RAISE EXCEPTION 'agent original claim observation binding' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER agent_turn_binding AFTER INSERT OR UPDATE ON core_assistant.agent_turns
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_agent_binding();
CREATE CONSTRAINT TRIGGER agent_round_binding AFTER INSERT OR UPDATE ON core_assistant.agent_rounds
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_agent_binding();
CREATE CONSTRAINT TRIGGER agent_tool_binding AFTER INSERT OR UPDATE ON core_assistant.agent_tool_operations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_agent_binding();
CREATE CONSTRAINT TRIGGER agent_observation_binding AFTER INSERT ON core_assistant.agent_tool_observations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_agent_binding();
CREATE CONSTRAINT TRIGGER agent_charge_binding AFTER INSERT OR UPDATE ON core_charging.requests
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_agent_binding();
-- Prevent a legacy text reserve from committing beside an active agent turn.
-- Both services also take the same conversation advisory lock before reserve.
CREATE FUNCTION core_assistant.check_agent_legacy_exclusion() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE conversation TEXT;
BEGIN
 IF NEW.assistant_grant_id IS NULL THEN RETURN NULL; END IF;
 SELECT conversation_id INTO conversation FROM core_assistant.grants WHERE id=NEW.assistant_grant_id;
 PERFORM pg_advisory_xact_lock(hashtextextended(conversation,908534117));
 IF EXISTS(SELECT FROM core_assistant.agent_turns t WHERE t.conversation_id=conversation
   AND t.phase IN ('new','model_running','tools_pending','unknown')) AND
 EXISTS(SELECT FROM core_charging.requests q JOIN core_assistant.grants g ON g.id=q.assistant_grant_id
   WHERE g.conversation_id=conversation AND q.state NOT IN ('settled','released')
    AND NOT EXISTS(SELECT FROM core_assistant.agent_rounds r WHERE r.model_request_id=q.id)) THEN
  RAISE EXCEPTION 'legacy and agent admission conflict' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER agent_legacy_exclusion AFTER INSERT OR UPDATE ON core_charging.requests
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_assistant.check_agent_legacy_exclusion();
-- Runtime ACL request (installer supplies role and enables this component):
-- SELECT,INSERT on all four new tables; UPDATE only:
-- turns(phase,revision,final_text,loaded_tools), rounds(state,provider_response_json,provider_response_sha256),
-- tool_operations(state,claim_key,result_json,result_sha256,domain_receipt_id).
-- observations append-only. USAGE/SELECT on observations_position_seq and the
-- existing operations_position_seq. No DELETE/TRUNCATE, no immutable UPDATE.

-- Cross-component dependency: if this component is installed, a runtime with
-- enable_assistant=true but enable_assistant_agent=false requires SELECT only
-- on agent_turns and agent_rounds. Charge triggers use these reads solely for
-- mixed legacy/agent exclusion and source classification; no agent INSERT or
-- UPDATE is granted by the legacy flag. A model-only role returns before any
-- assistant schema/table access when assistant_grant_id IS NULL.
