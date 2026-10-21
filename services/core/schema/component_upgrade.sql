-- Owner-only receipts for compiled, explicitly supported additive upgrades.
-- This metadata is identical on a fresh installation and an explicit upgrade.
CREATE TABLE core_meta.component_upgrades (
 upgrade_id TEXT CONSTRAINT component_upgrades_pkey PRIMARY KEY,
 procedure TEXT NOT NULL CONSTRAINT component_upgrades_procedure_check
   CHECK(procedure='core-additive-components-v1'),
 request_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_request_check CHECK(octet_length(request_sha256)=32),
 predecessor_manifest_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_pre_manifest_check CHECK(octet_length(predecessor_manifest_sha256)=32),
 predecessor_catalog_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_pre_catalog_check CHECK(octet_length(predecessor_catalog_sha256)=32),
 successor_manifest_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_post_manifest_check CHECK(octet_length(successor_manifest_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_post_catalog_check CHECK(octet_length(successor_catalog_sha256)=32),
 predecessor_components INTEGER NOT NULL,
 successor_components INTEGER NOT NULL,
 delta_components JSONB NOT NULL CONSTRAINT component_upgrades_delta_check
   CHECK(jsonb_typeof(delta_components)='array' AND jsonb_array_length(delta_components) BETWEEN 1 AND 16),
 added_catalog_entries JSONB NOT NULL CONSTRAINT component_upgrades_catalog_check
   CHECK(jsonb_typeof(added_catalog_entries)='array' AND jsonb_array_length(added_catalog_entries)>0),
 preserved_data_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_data_check CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CONSTRAINT component_upgrades_permissions_check CHECK(octet_length(preserved_permissions_sha256)=32),
 database_system_identifier TEXT NOT NULL,
 database_oid OID NOT NULL,
 database_name TEXT NOT NULL,
 installer_role TEXT NOT NULL,
 owner_role TEXT NOT NULL,
 runtime_role TEXT NOT NULL,
 extensions_role TEXT NOT NULL,
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CONSTRAINT component_upgrades_counts_check CHECK(predecessor_components>=25 AND successor_components>predecessor_components)
);
REVOKE ALL ON core_meta.component_upgrades FROM PUBLIC;
