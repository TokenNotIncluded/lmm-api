-- Exact owner-only Core26 ledger revision. Independent of financial imports.
CREATE TABLE core_meta.ledger_revisions (
 upgrade_id TEXT PRIMARY KEY,
 procedure TEXT NOT NULL CHECK(procedure='core-26-ledger-escrow-submit-v1'),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 predecessor_manifest_sha256 BYTEA NOT NULL CHECK(octet_length(predecessor_manifest_sha256)=32),
 predecessor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(predecessor_catalog_sha256)=32),
 successor_manifest_sha256 BYTEA NOT NULL CHECK(octet_length(successor_manifest_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 removed_catalog_entries JSONB NOT NULL CHECK(jsonb_typeof(removed_catalog_entries)='array'),
 added_catalog_entries JSONB NOT NULL CHECK(jsonb_typeof(added_catalog_entries)='array'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 database_system_identifier TEXT NOT NULL,
 database_oid OID NOT NULL,
 database_name TEXT NOT NULL,
 installer_role TEXT NOT NULL,
 owner_role TEXT NOT NULL,
 runtime_role TEXT NOT NULL,
 extensions_role TEXT NOT NULL,
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER ledger_revisions_immutable BEFORE UPDATE OR DELETE ON core_meta.ledger_revisions
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER ledger_revisions_no_truncate BEFORE TRUNCATE ON core_meta.ledger_revisions
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.ledger_revisions FROM PUBLIC;
