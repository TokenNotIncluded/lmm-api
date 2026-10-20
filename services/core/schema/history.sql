-- Immutable historical source receipts and logs, installed by fresh init-db.
-- Import is offline owner work; ordinary startup and runtime never write here.
CREATE SCHEMA core_history;
CREATE TABLE core_history.sources (
  source_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  contract_version text NOT NULL CHECK(contract_version='core.history-source.v1'),
  source_archive_sha256 text NOT NULL CHECK(source_archive_sha256 ~ '^[0-9a-f]{64}$'),
  mapping_sha256 text NOT NULL CHECK(mapping_sha256 ~ '^[0-9a-f]{64}$'),
  source_namespace text NOT NULL CHECK(length(source_namespace) BETWEEN 1 AND 128),
  observed_at bigint NOT NULL CHECK(observed_at>0),
  complete boolean NOT NULL,
  UNIQUE(source_archive_sha256,mapping_sha256,source_namespace)
);
CREATE TABLE core_history.log_records (
  source_id bigint NOT NULL REFERENCES core_history.sources(source_id),
  source_log_id bigint NOT NULL CHECK(source_log_id BETWEEN 1 AND 9007199254740991),
  canonical_event_key text NOT NULL UNIQUE CHECK(length(canonical_event_key) BETWEEN 1 AND 256),
  source_row_sha256 text NOT NULL CHECK(source_row_sha256 ~ '^[0-9a-f]{64}$'),
  user_id bigint NOT NULL REFERENCES core_identity.users(id),
  owner_account_id bigint NOT NULL REFERENCES core_identity.accounts(id),
  token_id bigint REFERENCES core_identity.credentials(id),
  created_at bigint NOT NULL CHECK(created_at BETWEEN 0 AND 253402300799),
  log_type smallint NOT NULL CHECK(log_type BETWEEN 0 AND 7),
  quota bigint,
  pricing_credits_per_usd bigint CHECK(pricing_credits_per_usd BETWEEN 1 AND 9007199254740991),
  prompt_tokens bigint,
  completion_tokens bigint,
  username text NOT NULL DEFAULT '' CHECK(octet_length(username)<=256),
  token_name text NOT NULL DEFAULT '' CHECK(octet_length(token_name)<=256),
  model_name text NOT NULL DEFAULT '' CHECK(octet_length(model_name)<=256),
  channel_id bigint NOT NULL DEFAULT 0 CHECK(channel_id>=0),
  group_name text NOT NULL DEFAULT '' CHECK(octet_length(group_name)<=256),
  request_id text NOT NULL DEFAULT '' CHECK(octet_length(request_id)<=256),
  upstream_request_id text NOT NULL DEFAULT '' CHECK(octet_length(upstream_request_id)<=256),
  PRIMARY KEY(source_id,source_log_id)
);
CREATE INDEX history_user_time ON core_history.log_records(user_id,owner_account_id,created_at DESC,source_id DESC,source_log_id DESC);
CREATE INDEX history_token_time ON core_history.log_records(token_id,created_at DESC,source_id DESC,source_log_id DESC);
CREATE INDEX history_time ON core_history.log_records(created_at DESC,source_id DESC,source_log_id DESC);
-- Offline reviewed coverage includes mapped entities with no historical rows.
-- NULL token_id is user/account coverage; key reads additionally require their
-- exact credential mapping. Runtime receives SELECT only, never INSERT rights.
CREATE TABLE core_history.scope_receipts (
  source_id bigint NOT NULL REFERENCES core_history.sources(source_id),
  user_id bigint NOT NULL REFERENCES core_identity.users(id),
  owner_account_id bigint NOT NULL REFERENCES core_identity.accounts(id),
  token_id bigint REFERENCES core_identity.credentials(id),
  source_archive_sha256 text NOT NULL CHECK(source_archive_sha256 ~ '^[0-9a-f]{64}$'),
  mapping_sha256 text NOT NULL CHECK(mapping_sha256 ~ '^[0-9a-f]{64}$'),
  source_scope_sha256 text NOT NULL CHECK(source_scope_sha256 ~ '^[0-9a-f]{64}$'),
  complete boolean NOT NULL,
  UNIQUE NULLS NOT DISTINCT(source_id,user_id,owner_account_id,token_id)
);
CREATE INDEX history_scope_lookup ON core_history.scope_receipts(user_id,owner_account_id,token_id,source_id);
REVOKE ALL ON SCHEMA core_history FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_history FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_history FROM PUBLIC;
