-- Additive component 25. Explicit owner installation/upgrade only; never startup.
-- These are price-entitlement facts, NOT wallet balances or another money ledger.
CREATE SCHEMA core_trust_pricing;
CREATE TABLE core_trust_pricing.policies (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
    revision BIGINT NOT NULL CHECK(revision>0),
    spec JSONB NOT NULL CHECK(jsonb_typeof(spec)='object'),
    source_digest BYTEA NOT NULL CHECK(octet_length(source_digest)=32),
    source_cutoff BIGINT NOT NULL CHECK(source_cutoff BETWEEN 0 AND 253402300799),
    -- The owner sets this only after the complete original-user import. Missing
    -- legacy rows fail closed; future, genuinely native identities can start L0.
    native_user_floor BIGINT NOT NULL CHECK(native_user_floor>0)
);
CREATE TABLE core_trust_pricing.user_bases (
    user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
    original_created_at BIGINT NOT NULL CHECK(original_created_at BETWEEN 0 AND 253402300799),
    console_activated_at BIGINT NOT NULL CHECK(console_activated_at BETWEEN 0 AND 253402300799),
    last_api_activity_at BIGINT NOT NULL CHECK(last_api_activity_at BETWEEN 0 AND 253402300799),
    override_level SMALLINT CHECK(override_level BETWEEN 0 AND 4),
    projection_known BOOLEAN NOT NULL,
    order_count BIGINT NOT NULL CHECK(order_count BETWEEN 0 AND 10000),
    revision BIGINT NOT NULL CHECK(revision>0),
    source_digest BYTEA NOT NULL CHECK(octet_length(source_digest)=32)
);
CREATE TABLE core_trust_pricing.legacy_paid_orders (
    source_order_id BIGINT PRIMARY KEY CHECK(source_order_id>0),
    user_id BIGINT NOT NULL REFERENCES core_trust_pricing.user_bases(user_id),
    -- Independently verified remaining NET credits, including an explicit zero
    -- for a fully refunded order. Owner corrections must carry new evidence.
    net_credits BIGINT NOT NULL CHECK(net_credits>=0),
    paid_at BIGINT NOT NULL CHECK(paid_at BETWEEN 1 AND 253402300799),
    revision BIGINT NOT NULL CHECK(revision>0),
    source_digest BYTEA NOT NULL CHECK(octet_length(source_digest)=32)
);
CREATE INDEX trust_paid_orders_user ON core_trust_pricing.legacy_paid_orders(user_id,source_order_id);
-- Runtime/Go/client cannot grant price rights. Revoke PUBLIC, including future
-- table privileges; the explicit runtime grant file adds only SELECT below.
REVOKE ALL ON SCHEMA core_trust_pricing FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_trust_pricing FROM PUBLIC;
-- Owner-installed native-origin fence: existing identities require explicit
-- imported bases, while post-install native registration needs no owner hook.
ALTER TABLE core_trust_pricing.policies ADD COLUMN native_since TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp();
INSERT INTO core_trust_pricing.policies(singleton,revision,spec,source_digest,source_cutoff,native_user_floor)
SELECT TRUE,1,'{"version":1,"paid_activation_enabled":true,"decay_period_days":90,"tiers":[{"level":0,"min_paid_credits":0,"discount":{"numerator":1,"denominator":1}},{"level":1,"min_paid_credits":500000,"discount":{"numerator":1,"denominator":1}},{"level":2,"min_paid_credits":5000000,"discount":{"numerator":97,"denominator":100}},{"level":3,"min_paid_credits":25000000,"discount":{"numerator":94,"denominator":100}},{"level":4,"min_paid_credits":100000000,"discount":{"numerator":9,"denominator":10}}],"admin_discount":{"numerator":9,"denominator":10},"superadmin_discount":{"numerator":9,"denominator":10}}'::jsonb,
    sha256(convert_to('core-native-trust-policy-v1','UTF8')),
    floor(extract(epoch FROM statement_timestamp()))::bigint,
    COALESCE((SELECT max(id) FROM core_identity.users),0)+1;
-- Serialize owner policy/basis changes against admitted price snapshots without
-- giving the runtime UPDATE privilege merely to use SELECT FOR SHARE.
CREATE FUNCTION core_trust_pricing.lock_owner_change() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(741803219);
    RETURN NULL;
END $$;
CREATE TRIGGER trust_policy_owner_lock BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON core_trust_pricing.policies
    FOR EACH STATEMENT EXECUTE FUNCTION core_trust_pricing.lock_owner_change();
CREATE TRIGGER trust_basis_owner_lock BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON core_trust_pricing.user_bases
    FOR EACH STATEMENT EXECUTE FUNCTION core_trust_pricing.lock_owner_change();
CREATE TRIGGER trust_order_owner_lock BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON core_trust_pricing.legacy_paid_orders
    FOR EACH STATEMENT EXECUTE FUNCTION core_trust_pricing.lock_owner_change();
REVOKE ALL ON FUNCTION core_trust_pricing.lock_owner_change() FROM PUBLIC;
