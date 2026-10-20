-- Explicit fresh-install increment for the existing dedicated assistant DB.
-- Never run by ordinary service startup. No Core database objects or grants.
BEGIN;
CREATE TABLE lmm_assistant.services_contract(singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),version integer NOT NULL CHECK(version=1));
INSERT INTO lmm_assistant.services_contract VALUES(true,1);
CREATE TABLE lmm_assistant.memories(
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,user_id bigint NOT NULL CHECK(user_id>0),
 title text NOT NULL CHECK(length(title) BETWEEN 1 AND 80),content text NOT NULL CHECK(length(content) BETWEEN 1 AND 800),
 tags jsonb NOT NULL CHECK(jsonb_typeof(tags)='array' AND jsonb_array_length(tags)<=8),
 source text NOT NULL CHECK(source IN('assistant','administrator')),enabled boolean NOT NULL,
 updated_by bigint NOT NULL CHECK(updated_by>0),created_at bigint NOT NULL,updated_at bigint NOT NULL,
 UNIQUE(user_id,title)
);
CREATE INDEX assistant_memory_owner_updated ON lmm_assistant.memories(user_id,updated_at DESC,id DESC);
CREATE TABLE lmm_assistant.profiles(
 user_id bigint PRIMARY KEY CHECK(user_id>0),profile_key text NOT NULL,tags jsonb NOT NULL,
 strategy text NOT NULL CHECK(length(strategy)<=4000),source text NOT NULL CHECK(source IN('assistant','administrator')),
 enabled boolean NOT NULL,updated_by bigint NOT NULL CHECK(updated_by>0),created_at bigint NOT NULL,updated_at bigint NOT NULL
);
CREATE TABLE lmm_assistant.service_operations(
 operation_id text PRIMARY KEY CHECK(length(operation_id) BETWEEN 16 AND 128),user_id bigint NOT NULL CHECK(user_id>0),
 session_sha256 bytea NOT NULL CHECK(octet_length(session_sha256)=32),call_sha256 bytea NOT NULL CHECK(octet_length(call_sha256)=32),
 tool_name text NOT NULL,result jsonb NOT NULL CHECK(jsonb_typeof(result)='object'),result_bytes bytea NOT NULL,created_at bigint NOT NULL
);
-- Committed before external search I/O; an unresolved dispatch is never replayed.
CREATE TABLE lmm_assistant.service_network_dispatch(
 operation_id text PRIMARY KEY,user_id bigint NOT NULL,session_sha256 bytea NOT NULL CHECK(octet_length(session_sha256)=32),
 call_sha256 bytea NOT NULL CHECK(octet_length(call_sha256)=32),created_at bigint NOT NULL
);
CREATE TABLE lmm_assistant.service_confirmations(
 id text PRIMARY KEY CHECK(id ~ '^[a-f0-9]{64}$'),operation_id text NOT NULL UNIQUE REFERENCES lmm_assistant.service_operations(operation_id) DEFERRABLE INITIALLY DEFERRED,
 user_id bigint NOT NULL CHECK(user_id>0),session_sha256 bytea NOT NULL CHECK(octet_length(session_sha256)=32),
 tool_name text NOT NULL,minimum_level smallint NOT NULL CHECK(minimum_level BETWEEN 0 AND 6),
 tool_version text NOT NULL,challenge_id text NOT NULL,challenge_url text NOT NULL,payload_sha256 text NOT NULL CHECK(length(payload_sha256)=64),core_draft_sha256 text NOT NULL CHECK(core_draft_sha256 ~ '^[a-f0-9]{64}$'),
 payload jsonb NOT NULL CHECK(jsonb_typeof(payload)='object'),payload_bytes bytea NOT NULL CHECK(octet_length(payload_bytes) BETWEEN 2 AND 16384),expires_at bigint NOT NULL,
 consumed_at bigint,result jsonb,CHECK((consumed_at IS NULL)=(result IS NULL))
);
CREATE TABLE lmm_assistant.site_issues(
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,reporter_id bigint NOT NULL CHECK(reporter_id>0),
 kind text NOT NULL CHECK(kind IN('bug','security','experience','feature')),
 title text NOT NULL CHECK(length(title) BETWEEN 2 AND 128),body text NOT NULL CHECK(length(body) BETWEEN 5 AND 4000),
 visibility text NOT NULL CHECK(visibility IN('user','admin')),status text NOT NULL CHECK(status IN('open','triaged','in_progress','resolved','declined')),
 revision bigint NOT NULL CHECK(revision>0),created_at bigint NOT NULL,updated_at bigint NOT NULL,
 CHECK(kind<>'security' OR visibility='admin')
);
CREATE INDEX assistant_issue_owner_cursor ON lmm_assistant.site_issues(reporter_id,visibility,id DESC);
CREATE TABLE lmm_assistant.site_issue_events(
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,issue_id bigint NOT NULL REFERENCES lmm_assistant.site_issues(id),actor_id bigint NOT NULL,
 visibility text NOT NULL CHECK(visibility IN('user','admin')),status text NOT NULL,note text NOT NULL CHECK(length(note)<=1500),created_at bigint NOT NULL
);
CREATE TABLE lmm_assistant.platform_support(
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,user_id bigint NOT NULL CHECK(user_id>0),core_conversation_id text NOT NULL,
 kind text NOT NULL CHECK(kind IN('handoff','appointment')),status text NOT NULL CHECK(status IN('pending','accepted','completed','cancelled')),
 topic text NOT NULL CHECK(length(topic)<=2000),preferred_time text NOT NULL CHECK(length(preferred_time)<=200),scheduled_at bigint NOT NULL,
 assigned_admin_id bigint,assigned_admin_name text NOT NULL DEFAULT '',created_at bigint NOT NULL,updated_at bigint NOT NULL,
 accepted_at bigint,closed_at bigint,closed_message_id bigint,revision bigint NOT NULL DEFAULT 1 CHECK(revision>0),
 eligibility_receipt text NOT NULL DEFAULT '',CHECK((status IN('completed','cancelled'))=(closed_at IS NOT NULL))
);
CREATE UNIQUE INDEX assistant_one_active_platform_support ON lmm_assistant.platform_support(user_id) WHERE status IN('pending','accepted');
CREATE TABLE lmm_assistant.platform_support_messages(
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,support_id bigint NOT NULL REFERENCES lmm_assistant.platform_support(id),
 author_id bigint NOT NULL,author_kind text NOT NULL CHECK(author_kind IN('user','human')),content text NOT NULL CHECK(length(content) BETWEEN 1 AND 4000),
 client_turn_id text NOT NULL,created_at bigint NOT NULL,UNIQUE(support_id,author_id,client_turn_id)
);
-- Queue entries are actual administrator inbox tasks, not a fabricated email.
CREATE TABLE lmm_assistant.platform_support_todos(
 support_id bigint PRIMARY KEY REFERENCES lmm_assistant.platform_support(id),state text NOT NULL CHECK(state IN('pending','assigned','closed')),
 assigned_admin_id bigint,updated_at bigint NOT NULL
);
CREATE FUNCTION lmm_assistant.services_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'immutable assistant service receipt' USING ERRCODE='55000'; END $$;
CREATE TRIGGER service_operations_immutable BEFORE UPDATE OR DELETE ON lmm_assistant.service_operations FOR EACH ROW EXECUTE FUNCTION lmm_assistant.services_immutable();
CREATE TRIGGER service_operations_no_truncate BEFORE TRUNCATE ON lmm_assistant.service_operations FOR EACH STATEMENT EXECUTE FUNCTION lmm_assistant.services_immutable();
CREATE TRIGGER service_network_dispatch_immutable BEFORE UPDATE OR DELETE ON lmm_assistant.service_network_dispatch FOR EACH ROW EXECUTE FUNCTION lmm_assistant.services_immutable();
CREATE TRIGGER issue_events_immutable BEFORE UPDATE OR DELETE ON lmm_assistant.site_issue_events FOR EACH ROW EXECUTE FUNCTION lmm_assistant.services_immutable();
CREATE TRIGGER support_messages_immutable BEFORE UPDATE OR DELETE ON lmm_assistant.platform_support_messages FOR EACH ROW EXECUTE FUNCTION lmm_assistant.services_immutable();
CREATE FUNCTION lmm_assistant.guard_service_confirmation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR OLD.consumed_at IS NOT NULL OR (to_jsonb(OLD)-ARRAY['consumed_at','result']) IS DISTINCT FROM (to_jsonb(NEW)-ARRAY['consumed_at','result']) THEN
  RAISE EXCEPTION 'immutable assistant confirmation' USING ERRCODE='55000';
 END IF;RETURN NEW;
END $$;
CREATE TRIGGER service_confirmation_guard BEFORE UPDATE OR DELETE ON lmm_assistant.service_confirmations FOR EACH ROW EXECUTE FUNCTION lmm_assistant.guard_service_confirmation();
REVOKE ALL ON lmm_assistant.services_contract,lmm_assistant.memories,lmm_assistant.profiles,lmm_assistant.service_operations,lmm_assistant.service_network_dispatch,lmm_assistant.service_confirmations,lmm_assistant.site_issues,lmm_assistant.site_issue_events,lmm_assistant.platform_support,lmm_assistant.platform_support_messages,lmm_assistant.platform_support_todos FROM PUBLIC;
COMMIT;
