-- Candidate only. Explicit module migration; never execute on runtime startup.
-- Root must review/assemble owner/runtime grants and the existing-site upgrade.
CREATE TABLE lmm_assistant.delegation_contract (
    singleton boolean PRIMARY KEY CHECK (singleton),
    version integer NOT NULL CHECK (version=1)
);
INSERT INTO lmm_assistant.delegation_contract VALUES (true,1);
CREATE UNIQUE INDEX assistant_conversation_owner_id ON lmm_assistant.conversations(user_id,id);
CREATE TABLE lmm_assistant.delegation_conversations (
    user_id bigint NOT NULL CHECK (user_id>0),
    conversation_id bigint NOT NULL,
    creation_key text NOT NULL CHECK (length(creation_key) BETWEEN 1 AND 128),
    core_conversation_id text UNIQUE CHECK (core_conversation_id ~ '^lmm_cv_[0-9a-f]{64}$'),
    phase text NOT NULL CHECK (phase IN ('creation_inflight','registered')),
    PRIMARY KEY(user_id,conversation_id),
    UNIQUE(user_id,creation_key),
    FOREIGN KEY(user_id,conversation_id) REFERENCES lmm_assistant.conversations(user_id,id),
    CHECK ((phase='registered')=(core_conversation_id IS NOT NULL))
);
CREATE TABLE lmm_assistant.delegation_turns (
    user_id bigint NOT NULL CHECK (user_id>0),
    conversation_id bigint NOT NULL,
    turn_id text NOT NULL CHECK (length(turn_id) BETWEEN 1 AND 124),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision>0),
    body jsonb NOT NULL CHECK (jsonb_typeof(body)='object' AND octet_length(body::text)<=1048576),
    PRIMARY KEY(user_id,turn_id),
    FOREIGN KEY(user_id,conversation_id) REFERENCES lmm_assistant.conversations(user_id,id)
);
