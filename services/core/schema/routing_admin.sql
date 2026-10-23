-- Component 27. Explicit offline installation/upgrade only.
-- Native protocols previously served only from file configuration. Durable
-- publication now uses the same finite protocol set as the Rust compiler.
ALTER TABLE public.routing_upstreams DROP CONSTRAINT routing_upstreams_protocol_check;
ALTER TABLE public.routing_upstreams ADD CONSTRAINT routing_upstreams_protocol_check
    CHECK (protocol IN ('openai_chat','openai_responses','anthropic_messages','gemini',
        'system_one','deepseek_tools','ali_multimodal_embedding'));

-- Existing invokers retain their algorithm and privileges; all table references
-- are qualified so nested definer execution never searches writable schemas.
CREATE OR REPLACE FUNCTION public.routing_guard_projection() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE
    v bigint;
    is_sealed boolean;
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.version <> OLD.version THEN
        RAISE EXCEPTION 'routing version is immutable' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN v := OLD.version; ELSE v := NEW.version; END IF;
    IF TG_ARGV[0] = 'price' THEN
        SELECT sealed INTO is_sealed FROM public.routing_price_versions WHERE version = v FOR SHARE;
    ELSE
        SELECT sealed INTO is_sealed FROM public.routing_config_versions WHERE version = v FOR SHARE;
    END IF;
    IF is_sealed IS DISTINCT FROM false THEN
        RAISE EXCEPTION 'routing snapshot is sealed' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_guard_version() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'routing snapshot is immutable' USING ERRCODE = '23514';
    END IF;
    IF OLD.sealed OR NEW.version <> OLD.version OR NEW.document IS DISTINCT FROM OLD.document THEN
        RAISE EXCEPTION 'routing snapshot is immutable' USING ERRCODE = '23514';
    END IF;
    IF TG_TABLE_NAME = 'routing_config_versions'
        AND to_jsonb(NEW)->'price_version' IS DISTINCT FROM to_jsonb(OLD)->'price_version' THEN
        RAISE EXCEPTION 'routing price reference is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_guard_active() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE next_price bigint; old_price bigint; ready boolean;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'routing active slot cannot be removed' USING ERRCODE = '23514';
    END IF;
    IF NEW.config_version IS NULL OR (OLD.config_version IS NOT NULL AND NEW.config_version <= OLD.config_version) THEN
        RAISE EXCEPTION 'routing version must increase' USING ERRCODE = '40001';
    END IF;
    SELECT c.price_version, c.sealed AND p.sealed INTO next_price, ready
        FROM public.routing_config_versions c JOIN public.routing_price_versions p ON p.version = c.price_version
        WHERE c.version = NEW.config_version;
    SELECT price_version INTO old_price FROM public.routing_config_versions WHERE version = OLD.config_version;
    IF ready IS DISTINCT FROM true OR next_price < old_price THEN
        RAISE EXCEPTION 'routing release is not ready' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_stage_prices(p_document jsonb) RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE v bigint := (p_document->>'version')::bigint; existing jsonb; inserted bigint;
BEGIN
    INSERT INTO public.routing_price_versions(version, document) VALUES (v, p_document)
        ON CONFLICT (version) DO NOTHING RETURNING version INTO inserted;
    IF inserted IS NULL THEN
        SELECT document INTO existing FROM public.routing_price_versions WHERE version = v AND sealed;
        IF existing IS DISTINCT FROM p_document THEN
            RAISE EXCEPTION 'price version already exists' USING ERRCODE = '23505';
        END IF;
        RETURN;
    END IF;
    INSERT INTO public.routing_price_rules
        SELECT v, (r->>'id')::bigint, r->>'model', r->>'group', r->>'speed',
            (r->'rates'->>'input')::bigint, (r->'rates'->>'output')::bigint,
            (r->'rates'->>'cache_read')::bigint, (r->'rates'->>'cache_write')::bigint,
            (r->'rates'->>'request')::bigint,
            (r->>'group_multiplier_ppm')::bigint, (r->>'speed_multiplier_ppm')::bigint
        FROM jsonb_array_elements(p_document->'rules') r;
    UPDATE public.routing_price_versions SET sealed = true WHERE version = v;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_publish_config(
    p_document jsonb, p_price bigint, p_expected_config bigint, p_expected_price bigint
) RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE
    v bigint := (p_document->>'version')::bigint;
    old_config bigint;
    old_price bigint;
BEGIN
    SELECT config_version INTO old_config FROM public.routing_active_release WHERE singleton FOR UPDATE;
    SELECT price_version INTO old_price FROM public.routing_config_versions WHERE version = old_config;
    IF old_config IS DISTINCT FROM p_expected_config OR old_price IS DISTINCT FROM p_expected_price
        OR (old_config IS NOT NULL AND v <= old_config) THEN
        RAISE EXCEPTION 'routing publication conflict' USING ERRCODE = '40001';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM public.routing_price_versions WHERE version = p_price AND sealed)
        OR (old_price IS NOT NULL AND p_price < old_price) THEN
        RAISE EXCEPTION 'routing price version is unavailable' USING ERRCODE = '23514';
    END IF;
    INSERT INTO public.routing_config_versions(version, price_version, document) VALUES (v, p_price, p_document);
    INSERT INTO public.routing_groups
        SELECT v, g->>'name', (g->>'enabled')::boolean FROM jsonb_array_elements(p_document->'groups') g;
    INSERT INTO public.routing_upstreams
        SELECT v, (u->>'id')::bigint, u->>'endpoint', u->>'protocol',
            (u->'credential'->>'id')::bigint, (u->'credential'->>'revision')::bigint,
            (u->>'enabled')::boolean FROM jsonb_array_elements(p_document->'upstreams') u;
    INSERT INTO public.routing_routes
        SELECT v, (r->>'id')::bigint, r->>'group', r->>'model', (r->>'enabled')::boolean,
            ARRAY(SELECT jsonb_array_elements_text(r->'speeds')),
            (r->'failover'->>'max_attempts')::smallint,
            ARRAY(SELECT jsonb_array_elements_text(r->'failover'->'retry_on')),
            (r->'failover'->>'allow_replay_after_send')::boolean
        FROM jsonb_array_elements(p_document->'routes') r;
    INSERT INTO public.routing_targets
        SELECT v, (t->>'id')::bigint, (r->>'id')::bigint, (t->>'upstream_id')::bigint,
            t->>'upstream_model', (t->>'enabled')::boolean, (t->>'priority')::integer,
            (t->>'weight')::bigint, ARRAY(SELECT jsonb_array_elements_text(t->'speeds')),
            t->>'payload', (t->>'timeout_ms')::integer
        FROM jsonb_array_elements(p_document->'routes') r,
            LATERAL jsonb_array_elements(r->'targets') t;
    INSERT INTO public.routing_aliases
        SELECT v, a->>'group', a->>'alias', a->>'model'
        FROM jsonb_array_elements(p_document->'aliases') a;
    UPDATE public.routing_config_versions SET sealed = true WHERE version = v;
    UPDATE public.routing_active_release SET config_version = v WHERE singleton;
END;
$$;

CREATE SCHEMA core_routing_admin;
CREATE TABLE core_routing_admin.previews (
    preview_id text PRIMARY KEY CHECK (preview_id ~ '^[a-f0-9]{64}$'),
    actor_user_id bigint NOT NULL REFERENCES core_identity.users(id),
    actor_credential_id bigint NOT NULL REFERENCES core_identity.credentials(id),
    kind text NOT NULL CHECK (kind IN ('prices','priorities')),
    payload text NOT NULL CHECK (octet_length(payload) BETWEEN 1 AND 33554432),
    digest text NOT NULL CHECK (digest ~ '^[a-f0-9]{64}$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL DEFAULT clock_timestamp()+interval '5 minutes',
    CHECK (expires_at > created_at AND expires_at <= created_at+interval '6 minutes')
);
CREATE TABLE core_routing_admin.operations (
    operation_id text PRIMARY KEY CHECK (operation_id ~ '^[a-f0-9]{64}$'),
    preview_id text NOT NULL UNIQUE REFERENCES core_routing_admin.previews(preview_id),
    actor_user_id bigint NOT NULL REFERENCES core_identity.users(id),
    confirmation_sha256 text NOT NULL CHECK (confirmation_sha256 ~ '^[a-f0-9]{64}$'),
    routing_version bigint NOT NULL UNIQUE REFERENCES public.routing_config_versions(version),
    pricing_version bigint NOT NULL REFERENCES public.routing_price_versions(version),
    published_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
REVOKE ALL ON SCHEMA core_routing_admin FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_routing_admin FROM PUBLIC;

-- Same complete permission-policy byte contract as identity::permissions.
-- This is an additional database check; Rust still calls the existing locked
-- Session/MFA admission and shared permission evaluator in this transaction.
CREATE FUNCTION core_routing_admin.actor(p_secret_digest bytea,p_kind text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE c record; p record; f record; proof record; policy record; imported boolean;
    canonical bytea; action text; effect text; policy_revision text; policy_hash text;
BEGIN
    IF octet_length(p_secret_digest)<>32 OR p_kind NOT IN ('prices','priorities') THEN
        RAISE EXCEPTION 'routing actor refused' USING ERRCODE='42501';
    END IF;
    SELECT cr.id,cr.user_id,u.auth_version,u.platform_role INTO c
      FROM core_identity.credentials cr
      JOIN core_identity.users u ON u.id=cr.user_id
      JOIN core_identity.accounts a ON a.id=u.personal_account_id
      WHERE cr.digest=p_secret_digest AND cr.kind='session'
        AND cr.owner_account_id=u.personal_account_id AND a.kind='personal'
        AND cr.revoked_at IS NULL AND cr.expires_at>clock_timestamp()
        AND cr.user_version=u.auth_version AND u.active AND a.active
        AND u.platform_role IN ('admin','superadmin')
      FOR SHARE OF cr,u,a;
    IF c IS NULL OR (p_kind='prices' AND c.platform_role<>'superadmin') THEN
        RAISE EXCEPTION 'routing actor refused' USING ERRCODE='42501';
    END IF;
    PERFORM 1 FROM core_identity.web_sessions w WHERE w.current_credential_id=c.id FOR SHARE;
    IF EXISTS(SELECT FROM core_identity.web_sessions w WHERE w.current_credential_id=c.id
        AND (w.revoked_at IS NOT NULL OR w.expires_at<=clock_timestamp() OR w.user_version<>c.auth_version
          OR (w.weekly_logout AND w.created_at<=clock_timestamp()-interval '7 days'))) THEN
        RAISE EXCEPTION 'routing session refused' USING ERRCODE='42501';
    END IF;
    SELECT second_factor_required INTO p FROM core_identity.user_profiles WHERE user_id=c.user_id FOR SHARE;
    SELECT revision,enabled INTO f FROM core_identity.mfa_factors WHERE user_id=c.user_id FOR SHARE;
    SELECT * INTO proof FROM core_identity.session_authentication WHERE credential_id=c.id;
    IF p IS NULL OR proof IS NULL OR proof.user_id<>c.user_id OR proof.user_version<>c.auth_version
       OR p.second_factor_required IS DISTINCT FROM coalesce(f.enabled,false)
       OR (p.second_factor_required AND (proof.factor_level<>2 OR proof.factor_revision<>f.revision OR proof.mfa_verified_at IS NULL)) THEN
        RAISE EXCEPTION 'routing assurance refused' USING ERRCODE='42501';
    END IF;
    SELECT EXISTS(SELECT FROM core_identity.privileged_import_facts WHERE user_id=c.user_id) INTO imported;
    SELECT * INTO policy FROM core_identity.permission_policies WHERE user_id=c.user_id FOR SHARE;
    IF imported OR policy IS NOT NULL THEN
        IF policy IS NULL OR NOT policy.verified_complete OR policy.contract<>'core.permissions.v1'
           OR (SELECT count(*) FROM core_identity.permission_policy_rows WHERE user_id=c.user_id)<>9 THEN
            RAISE EXCEPTION 'routing policy refused' USING ERRCODE='42501';
        END IF;
        canonical:=convert_to('core.permissions.v1','UTF8')||decode('00','hex')||int8send(policy.revision)||decode('00','hex')||int8send(c.user_id)||decode('00','hex');
        FOREACH action IN ARRAY ARRAY['ChannelRead','ChannelOperate','ChannelWrite','ChannelSensitiveWrite','ChannelSecretView','AcquisitionRead','AcquisitionWrite','AcquisitionDetails','AcquisitionExport'] LOOP
            SELECT r.effect INTO effect FROM core_identity.permission_policy_rows r WHERE r.user_id=c.user_id AND r.action=action FOR SHARE;
            IF effect IS NULL OR effect NOT IN ('allow','deny') THEN RAISE EXCEPTION 'routing policy refused' USING ERRCODE='42501'; END IF;
            canonical:=canonical||convert_to(action,'UTF8')||decode('00','hex')||convert_to(effect,'UTF8')||decode('00','hex');
            IF action='ChannelWrite' AND p_kind='priorities' AND c.platform_role<>'superadmin' AND effect='deny' THEN
                RAISE EXCEPTION 'routing policy denied' USING ERRCODE='42501';
            END IF;
        END LOOP;
        IF policy.canonical_payload<>canonical OR policy.sha256<>sha256(canonical) THEN
            RAISE EXCEPTION 'routing policy refused' USING ERRCODE='42501';
        END IF;
        policy_revision:=policy.revision::text; policy_hash:=encode(policy.sha256,'hex');
    END IF;
    RETURN jsonb_build_object('user_id',c.user_id::text,'credential_id',c.id::text,'auth_version',c.auth_version::text,'policy_revision',policy_revision,'policy_sha256',policy_hash);
END $$;

CREATE FUNCTION core_routing_admin.read_active() RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE result jsonb; active bigint;
BEGIN
    SELECT config_version INTO active FROM public.routing_active_release WHERE singleton;
    IF NOT FOUND THEN RAISE EXCEPTION 'routing active slot missing' USING ERRCODE='55000'; END IF;
    IF active IS NULL THEN
        IF EXISTS(SELECT FROM core_routing_admin.operations) OR EXISTS(SELECT FROM public.routing_config_versions) THEN
            RAISE EXCEPTION 'durable routing active release missing' USING ERRCODE='55000';
        END IF;
        RETURN NULL;
    END IF;
    SELECT jsonb_build_object('contract_version',1,'routing',r.document,'pricing',p.document) INTO result
      FROM public.routing_config_versions r JOIN public.routing_price_versions p ON p.version=r.price_version
      WHERE r.version=active AND r.sealed AND p.sealed;
    IF result IS NULL THEN RAISE EXCEPTION 'durable routing release unavailable' USING ERRCODE='55000'; END IF;
    RETURN result;
END $$;

-- Normalize only finite, nonnegative monetary coefficients. Unknown metadata,
-- optional metering evidence and tier bounds remain part of the comparison.
CREATE FUNCTION core_routing_admin.rate_contract(p_rate jsonb) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
BEGIN
    IF jsonb_typeof(p_rate) IS DISTINCT FROM 'object' OR p_rate-ARRAY['numerator','denominator']<>'{}'::jsonb
       OR jsonb_typeof(p_rate->'numerator') IS DISTINCT FROM 'number' OR jsonb_typeof(p_rate->'denominator') IS DISTINCT FROM 'number'
       OR (p_rate->>'numerator')!~'^[0-9]+$' OR (p_rate->>'denominator')!~'^[0-9]+$'
       OR (p_rate->>'numerator')::numeric>9223372036854775807
       OR (p_rate->>'denominator')::numeric NOT BETWEEN 1 AND 1000000000000000000 THEN
        RAISE EXCEPTION 'invalid monetary fraction' USING ERRCODE='23514';
    END IF;
    RETURN '{"numerator":0,"denominator":1}'::jsonb;
END $$;
CREATE FUNCTION core_routing_admin.price_contract(p_rule jsonb) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE result jsonb:=p_rule; rates jsonb; tier jsonb; tiers jsonb:='[]'::jsonb; name text;
BEGIN
    rates:=result->'rates';
    FOREACH name IN ARRAY ARRAY['input','output','cache_read','cache_write','request'] LOOP
        IF jsonb_typeof(rates->name) IS DISTINCT FROM 'number' OR (rates->>name)!~'^[0-9]+$'
           OR (rates->>name)::numeric>9223372036854775807 THEN RAISE EXCEPTION 'invalid monetary rate' USING ERRCODE='23514'; END IF;
        rates:=jsonb_set(rates,ARRAY[name],'0'::jsonb,false);
    END LOOP;
    result:=jsonb_set(result,'{rates}',rates,false);
    IF result->'tiered' IS NOT NULL AND result->'tiered'<>'null'::jsonb THEN
        IF coalesce(result#>>'{tiered,source_digest}','')!~'^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'invalid tariff provenance' USING ERRCODE='23514'; END IF;
        FOR tier IN SELECT value FROM jsonb_array_elements(result#>'{tiered,tiers}') LOOP
            FOREACH name IN ARRAY ARRAY['input','output','cache_read','cache_write','cache_write_1h','cache_write_12h'] LOOP
                IF tier#>ARRAY['rates',name] IS NOT NULL AND tier#>ARRAY['rates',name]<>'null'::jsonb THEN
                    tier:=jsonb_set(tier,ARRAY['rates',name],core_routing_admin.rate_contract(tier#>ARRAY['rates',name]),false);
                END IF;
            END LOOP;
            tiers:=tiers||jsonb_build_array(tier);
        END LOOP;
        result:=jsonb_set(result,'{tiered,tiers}',tiers,false);
        result:=jsonb_set(result,'{tiered,source_digest}',to_jsonb(repeat('0',64)),false);
    END IF;
    FOREACH name IN ARRAY ARRAY['usd_per_unit','x_posts_fetched_usd_per_unit','x_users_fetched_usd_per_unit'] LOOP
        IF result#>ARRAY['search',name] IS NOT NULL AND result#>ARRAY['search',name]<>'null'::jsonb THEN
            result:=jsonb_set(result,ARRAY['search',name],core_routing_admin.rate_contract(result#>ARRAY['search',name]),false);
        END IF;
    END LOOP;
    FOREACH name IN ARRAY ARRAY['text_usd_per_million','image_usd_per_million'] LOOP
        IF result#>ARRAY['embedding',name] IS NOT NULL AND result#>ARRAY['embedding',name]<>'null'::jsonb THEN
            result:=jsonb_set(result,ARRAY['embedding',name],core_routing_admin.rate_contract(result#>ARRAY['embedding',name]),false);
        END IF;
    END LOOP;
    RETURN result;
END $$;
CREATE FUNCTION core_routing_admin.routing_contract(p_routing jsonb) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE result jsonb:=p_routing-'version'; route jsonb; target jsonb; routes jsonb:='[]'::jsonb; targets jsonb;
BEGIN
    FOR route IN SELECT value FROM jsonb_array_elements(result->'routes') LOOP
        targets:='[]'::jsonb;
        FOR target IN SELECT value FROM jsonb_array_elements(route->'targets') LOOP
            IF (target->>'priority')!~'^[0-9]+$' OR (target->>'priority')::numeric NOT BETWEEN 0 AND 65535 THEN
                RAISE EXCEPTION 'invalid priority' USING ERRCODE='23514';
            END IF;
            targets:=targets||jsonb_build_array(jsonb_set(target,'{priority}','0'::jsonb,false));
        END LOOP;
        routes:=routes||jsonb_build_array(jsonb_set(route,'{targets}',targets,false));
    END LOOP;
    RETURN jsonb_set(result,'{routes}',routes,false);
END $$;
CREATE FUNCTION core_routing_admin.check_plan(p_plan jsonb) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE old jsonb:=p_plan->'before'; next jsonb:=p_plan->'after'; kind text:=p_plan->>'kind';
    before_rule jsonb; after_rule jsonb; before_route jsonb; after_route jsonb; before_target jsonb; after_target jsonb;
    declared jsonb; changed_routes integer:=0; changed_targets integer:=0;
BEGIN
    IF old->>'contract_version' IS DISTINCT FROM '1' OR next->>'contract_version' IS DISTINCT FROM '1'
       OR (next#>>'{routing,version}')::bigint<>(old#>>'{routing,version}')::bigint+1
       OR p_plan->>'source_sha256'!~'^[a-f0-9]{64}$'
       OR jsonb_typeof(p_plan->'changes') IS DISTINCT FROM 'array' OR jsonb_array_length(p_plan->'changes') NOT BETWEEN 1 AND 2000 THEN
        RAISE EXCEPTION 'invalid routing plan' USING ERRCODE='23514';
    END IF;
    IF kind='prices' THEN
        IF old->'routing'-'version' IS DISTINCT FROM next->'routing'-'version'
           OR (next#>>'{pricing,version}')::bigint<>(old#>>'{pricing,version}')::bigint+1
           OR old->'pricing'-ARRAY['version','rules'] IS DISTINCT FROM next->'pricing'-ARRAY['version','rules']
           OR jsonb_array_length(old#>'{pricing,rules}')<>jsonb_array_length(next#>'{pricing,rules}') THEN
            RAISE EXCEPTION 'price-only scope refused' USING ERRCODE='23514';
        END IF;
        FOR before_rule IN SELECT value FROM jsonb_array_elements(old#>'{pricing,rules}') LOOP
            SELECT value INTO after_rule FROM jsonb_array_elements(next#>'{pricing,rules}') WHERE value->'id'=before_rule->'id';
            IF after_rule IS NULL OR core_routing_admin.price_contract(before_rule) IS DISTINCT FROM core_routing_admin.price_contract(after_rule) THEN
                RAISE EXCEPTION 'price contract changed' USING ERRCODE='23514';
            END IF;
            SELECT value INTO declared FROM jsonb_array_elements(p_plan->'changes') WHERE value->>'rule_id'=before_rule->>'id';
            IF before_rule IS DISTINCT FROM after_rule AND (declared IS NULL OR declared->>'model' IS DISTINCT FROM before_rule->>'model'
                OR before_rule->>'billing_mode'='upstream_cost') THEN
                RAISE EXCEPTION 'unselected price changed' USING ERRCODE='23514';
            END IF;
        END LOOP;
    ELSIF kind='priorities' THEN
        IF old->'pricing' IS DISTINCT FROM next->'pricing'
           OR core_routing_admin.routing_contract(old->'routing') IS DISTINCT FROM core_routing_admin.routing_contract(next->'routing') THEN
            RAISE EXCEPTION 'priority-only scope refused' USING ERRCODE='23514';
        END IF;
        FOR before_route IN SELECT value FROM jsonb_array_elements(old#>'{routing,routes}') LOOP
            SELECT value INTO after_route FROM jsonb_array_elements(next#>'{routing,routes}') WHERE value->'id'=before_route->'id';
            IF before_route IS DISTINCT FROM after_route THEN changed_routes:=changed_routes+1; END IF;
            FOR before_target IN SELECT value FROM jsonb_array_elements(before_route->'targets') LOOP
                SELECT value INTO after_target FROM jsonb_array_elements(after_route->'targets') WHERE value->'id'=before_target->'id';
                IF before_target IS DISTINCT FROM after_target THEN
                    changed_targets:=changed_targets+1;
                    SELECT value INTO declared FROM jsonb_array_elements(p_plan->'changes') WHERE value->>'target_id'=before_target->>'id';
                    IF declared IS NULL OR declared->>'route_id' IS DISTINCT FROM before_route->>'id'
                       OR declared->'before' IS DISTINCT FROM before_target->'priority' OR declared->'after' IS DISTINCT FROM after_target->'priority' THEN
                        RAISE EXCEPTION 'priority preview mismatch' USING ERRCODE='23514';
                    END IF;
                END IF;
            END LOOP;
        END LOOP;
        IF changed_routes<>1 OR changed_targets NOT BETWEEN 1 AND 64 OR changed_targets<>jsonb_array_length(p_plan->'changes')
           OR p_plan->'fx'<>'null'::jsonb THEN RAISE EXCEPTION 'priority-only scope refused' USING ERRCODE='23514'; END IF;
    ELSE RAISE EXCEPTION 'unknown routing operation' USING ERRCODE='23514'; END IF;
END $$;

CREATE FUNCTION core_routing_admin.check_fx(p_plan jsonb) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE fx jsonb:=p_plan->'fx'; current record;
BEGIN
    IF fx IS NOT NULL AND fx<>'null'::jsonb THEN
        IF fx->>'key' IS DISTINCT FROM 'USDExchangeRate' THEN RAISE EXCEPTION 'exchange snapshot refused' USING ERRCODE='23514'; END IF;
        SELECT value,revision INTO current FROM core_console.options WHERE key='USDExchangeRate' FOR SHARE;
        IF current IS NULL OR current.value IS DISTINCT FROM fx->>'raw' OR current.revision::text IS DISTINCT FROM fx->>'revision' THEN
            RAISE EXCEPTION 'exchange snapshot changed' USING ERRCODE='40001';
        END IF;
    END IF;
END $$;
CREATE FUNCTION core_routing_admin.store_preview(p_secret_digest bytea,p_id text,p_payload text,p_digest text) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE payload jsonb:=p_payload::jsonb; plan jsonb:=payload->'plan'; actor jsonb; active jsonb; expiry timestamptz;
BEGIN
    IF octet_length(p_payload)>33554432 OR p_id!~'^[a-f0-9]{64}$' OR p_digest!~'^[a-f0-9]{64}$'
       OR p_digest<>encode(sha256(convert_to(p_payload,'UTF8')),'hex') OR payload->>'contract' IS DISTINCT FROM 'core.routing-admin.preview.v1'
       OR payload->>'preview_id' IS DISTINCT FROM p_id THEN RAISE EXCEPTION 'invalid routing preview' USING ERRCODE='23514'; END IF;
    actor:=core_routing_admin.actor(p_secret_digest,plan->>'kind');
    IF actor IS DISTINCT FROM payload->'authority' THEN RAISE EXCEPTION 'routing authority changed' USING ERRCODE='42501'; END IF;
    PERFORM pg_advisory_xact_lock(741803127);
    DELETE FROM core_routing_admin.previews p WHERE p.expires_at<clock_timestamp()
       AND NOT EXISTS(SELECT FROM core_routing_admin.operations o WHERE o.preview_id=p.preview_id);
    IF (SELECT count(*) FROM core_routing_admin.previews p WHERE p.expires_at>clock_timestamp() AND NOT EXISTS(SELECT FROM core_routing_admin.operations o WHERE o.preview_id=p.preview_id))>=64
       OR (SELECT count(*) FROM core_routing_admin.previews p WHERE p.actor_user_id=(actor->>'user_id')::bigint AND p.expires_at>clock_timestamp()
           AND NOT EXISTS(SELECT FROM core_routing_admin.operations o WHERE o.preview_id=p.preview_id))>=4 THEN
        RAISE EXCEPTION 'routing previews busy' USING ERRCODE='P2702';
    END IF;
    PERFORM core_routing_admin.check_plan(plan);
    PERFORM core_routing_admin.check_fx(plan);
    active:=core_routing_admin.read_active();
    IF active IS NULL THEN
        RAISE EXCEPTION 'offline routing baseline required' USING ERRCODE='P2703';
    END IF;
    IF active IS DISTINCT FROM plan->'before' THEN RAISE EXCEPTION 'routing changed' USING ERRCODE='40001'; END IF;
    INSERT INTO core_routing_admin.previews(preview_id,actor_user_id,actor_credential_id,kind,payload,digest)
      VALUES(p_id,(actor->>'user_id')::bigint,(actor->>'credential_id')::bigint,plan->>'kind',p_payload,p_digest) RETURNING expires_at INTO expiry;
    RETURN floor(extract(epoch FROM expiry))::bigint;
END $$;
CREATE FUNCTION core_routing_admin.read_preview(p_secret_digest bytea,p_id text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE preview core_routing_admin.previews; actor jsonb;
BEGIN
    SELECT * INTO preview FROM core_routing_admin.previews WHERE preview_id=p_id;
    IF preview IS NULL THEN RAISE EXCEPTION 'preview unavailable' USING ERRCODE='P2701'; END IF;
    actor:=core_routing_admin.actor(p_secret_digest,preview.kind);
    IF actor IS DISTINCT FROM preview.payload::jsonb->'authority' THEN RAISE EXCEPTION 'preview actor changed' USING ERRCODE='42501'; END IF;
    RETURN preview.payload;
END $$;
CREATE FUNCTION core_routing_admin.read_operation(p_secret_digest bytea,p_operation text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE preview_id text;
BEGIN
    SELECT o.preview_id INTO preview_id FROM core_routing_admin.operations o WHERE operation_id=p_operation;
    IF preview_id IS NULL THEN RAISE EXCEPTION 'operation unavailable' USING ERRCODE='P2701'; END IF;
    RETURN core_routing_admin.read_preview(p_secret_digest,preview_id);
END $$;
CREATE FUNCTION core_routing_admin.publish_preview(p_secret_digest bytea,p_id text,p_digest text,p_operation text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE preview core_routing_admin.previews; existing core_routing_admin.operations; actor jsonb; plan jsonb; before jsonb; next jsonb; active jsonb;
BEGIN
    IF p_operation!~'^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'invalid routing operation' USING ERRCODE='23514'; END IF;
    SELECT * INTO preview FROM core_routing_admin.previews WHERE preview_id=p_id FOR UPDATE;
    IF preview IS NULL THEN RAISE EXCEPTION 'preview unavailable' USING ERRCODE='P2701'; END IF;
    actor:=core_routing_admin.actor(p_secret_digest,preview.kind);
    IF actor IS DISTINCT FROM preview.payload::jsonb->'authority' THEN RAISE EXCEPTION 'preview actor changed' USING ERRCODE='42501'; END IF;
    IF preview.digest IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'confirmation mismatch' USING ERRCODE='40001'; END IF;
    SELECT * INTO existing FROM core_routing_admin.operations WHERE preview_id=p_id OR operation_id=p_operation;
    IF existing IS NOT NULL THEN
        IF existing.preview_id<>p_id OR existing.operation_id<>p_operation OR existing.confirmation_sha256<>p_digest THEN
            RAISE EXCEPTION 'operation reused' USING ERRCODE='40001';
        END IF;
        RETURN false;
    END IF;
    IF preview.expires_at<=clock_timestamp() THEN RAISE EXCEPTION 'preview expired' USING ERRCODE='P2701'; END IF;
    plan:=preview.payload::jsonb->'plan'; before:=plan->'before'; next:=plan->'after';
    PERFORM core_routing_admin.check_plan(plan);
    PERFORM core_routing_admin.check_fx(plan);
    PERFORM 1 FROM public.routing_active_release WHERE singleton FOR UPDATE;
    active:=core_routing_admin.read_active();
    IF active IS NULL THEN
        -- A narrow runtime wrapper cannot establish the immutable host baseline.
        -- Only an explicit offline owner operation may register the first release.
        RAISE EXCEPTION 'offline routing baseline required' USING ERRCODE='P2703';
    ELSIF active IS DISTINCT FROM before THEN RAISE EXCEPTION 'routing changed' USING ERRCODE='40001'; END IF;
    PERFORM public.routing_stage_prices(next->'pricing');
    PERFORM public.routing_publish_config(next->'routing',(next#>>'{pricing,version}')::bigint,(before#>>'{routing,version}')::bigint,(before#>>'{pricing,version}')::bigint);
    INSERT INTO core_routing_admin.operations(operation_id,preview_id,actor_user_id,confirmation_sha256,routing_version,pricing_version)
      VALUES(p_operation,p_id,(actor->>'user_id')::bigint,p_digest,(next#>>'{routing,version}')::bigint,(next#>>'{pricing,version}')::bigint);
    RETURN true;
END $$;

-- Only these wrappers are granted EXECUTE to the dedicated Core runtime role
-- by an explicit offline installer. Helpers and direct table writes stay denied.
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_routing_admin FROM PUBLIC;
