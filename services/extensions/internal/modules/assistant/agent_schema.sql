-- Explicit candidate module migration. Never run from an ordinary startup.
CREATE TABLE lmm_assistant.agent_contract (
  singleton boolean PRIMARY KEY CHECK(singleton),
  version integer NOT NULL CHECK(version=1)
);
INSERT INTO lmm_assistant.agent_contract VALUES(true,1);
CREATE TABLE lmm_assistant.agent_turns (
  user_id bigint NOT NULL CHECK(user_id>0),
  conversation_id bigint NOT NULL,
  turn_id text NOT NULL CHECK(length(turn_id) BETWEEN 1 AND 64),
  revision bigint NOT NULL DEFAULT 1 CHECK(revision>0),
  body jsonb NOT NULL CHECK(jsonb_typeof(body)='object' AND octet_length(body::text)<=33554432),
  body_bytes bytea NOT NULL CHECK(octet_length(body_bytes)<=33554432),
  PRIMARY KEY(user_id,turn_id),
  FOREIGN KEY(user_id,conversation_id) REFERENCES lmm_assistant.conversations(user_id,id)
);
