-- Code schema upgrades are independent from historical source-import receipts.
-- The same bytes run on a fresh installation and on the explicit 24->25 path.
CREATE TABLE core_meta.schema_upgrades (
    upgrade_id TEXT CONSTRAINT schema_upgrades_pkey PRIMARY KEY,
    procedure TEXT NOT NULL CONSTRAINT schema_upgrades_procedure_check
        CHECK (procedure = 'core-24-to-25-trust-pricing-v1'),
    request_sha256 BYTEA NOT NULL CONSTRAINT schema_upgrades_request_check
        CHECK (octet_length(request_sha256) = 32),
    predecessor_manifest_sha256 BYTEA NOT NULL CONSTRAINT schema_upgrades_pre_manifest_check
        CHECK (octet_length(predecessor_manifest_sha256) = 32),
    predecessor_catalog_sha256 BYTEA NOT NULL CONSTRAINT schema_upgrades_pre_catalog_check
        CHECK (octet_length(predecessor_catalog_sha256) = 32),
    successor_manifest_sha256 BYTEA NOT NULL CONSTRAINT schema_upgrades_post_manifest_check
        CHECK (octet_length(successor_manifest_sha256) = 32),
    successor_catalog_sha256 BYTEA NOT NULL CONSTRAINT schema_upgrades_post_catalog_check
        CHECK (octet_length(successor_catalog_sha256) = 32),
    -- Exact PG18 CATALOG entries; SQL source hashes are not deparser hashes.
    -- Only the two reviewed image constraint/function changes are allowed.
    image_definition_changes JSONB NOT NULL CONSTRAINT schema_upgrades_image_changes_check
        CHECK (jsonb_typeof(image_definition_changes) = 'array'
            AND jsonb_array_length(image_definition_changes) = 2),
    database_system_identifier TEXT NOT NULL,
    database_oid OID NOT NULL,
    database_name TEXT NOT NULL,
    installer_role TEXT NOT NULL,
    owner_role TEXT NOT NULL,
    runtime_role TEXT NOT NULL,
    extensions_role TEXT NOT NULL,
    -- This is when the row was constructed, not a claim of a known COMMIT time.
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
REVOKE ALL ON core_meta.schema_upgrades FROM PUBLIC;
