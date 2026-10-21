-- Native Telegram Widget identities and short-lived, browser-bound ceremonies.
-- Existing identity and trust-pricing component bytes remain unchanged.
CREATE SCHEMA core_telegram_auth;

-- A revoked subject remains owned by its original account. Unbinding must not
-- make that stable external identity available for another user's claim.
CREATE TABLE core_telegram_auth.bindings (
 subject TEXT PRIMARY KEY CHECK(subject ~ '^[1-9][0-9]{0,18}$' AND subject::numeric<=9223372036854775807),
 user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 revoked_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX telegram_active_user ON core_telegram_auth.bindings(user_id)
 WHERE revoked_at IS NULL;

CREATE TABLE core_telegram_auth.flows (
 token_digest BYTEA PRIMARY KEY CHECK(octet_length(token_digest)=32),
 purpose TEXT NOT NULL CHECK(purpose IN ('login','bind')),
 binding_digest BYTEA NOT NULL CHECK(octet_length(binding_digest)=32),
 user_id BIGINT REFERENCES core_identity.users(id),
 user_version BIGINT CHECK(user_version>0),
 sid TEXT REFERENCES core_identity.web_sessions(sid),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()+interval '300 seconds',
 consumed_at TIMESTAMPTZ,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '300 seconds'),
 CHECK((purpose='login' AND user_id IS NULL AND user_version IS NULL AND sid IS NULL)
    OR (purpose='bind' AND user_id IS NOT NULL AND user_version IS NOT NULL AND sid IS NOT NULL))
);
CREATE INDEX telegram_flow_expiry ON core_telegram_auth.flows(expires_at);

-- The legacy Widget does not sign an application nonce. One assertion claim
-- namespace is therefore shared by login and binding, beyond the flow cookie.
CREATE TABLE core_telegram_auth.assertions (
 digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()+interval '600 seconds',
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '600 seconds')
);
CREATE INDEX telegram_assertion_expiry ON core_telegram_auth.assertions(expires_at);
REVOKE ALL ON SCHEMA core_telegram_auth FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_telegram_auth FROM PUBLIC;
