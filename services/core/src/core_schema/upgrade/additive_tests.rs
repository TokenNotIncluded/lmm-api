use super::*;
use std::collections::BTreeSet;

const RECEIPT_FIELDS: &[(&str, &str)] = &[
    ("upgrade_id", "text"),
    ("procedure", "text"),
    ("request_sha256", "bytea"),
    ("predecessor_manifest_sha256", "bytea"),
    ("predecessor_catalog_sha256", "bytea"),
    ("successor_manifest_sha256", "bytea"),
    ("successor_catalog_sha256", "bytea"),
    ("predecessor_components", "integer"),
    ("successor_components", "integer"),
    ("delta_components", "jsonb"),
    ("added_catalog_entries", "jsonb"),
    ("preserved_data_sha256", "bytea"),
    ("preserved_permissions_sha256", "bytea"),
    ("database_system_identifier", "text"),
    ("database_oid", "oid"),
    ("database_name", "text"),
    ("installer_role", "text"),
    ("owner_role", "text"),
    ("runtime_role", "text"),
    ("extensions_role", "text"),
    ("recorded_at", "timestamp with time zone"),
];

const NAMED_CONSTRAINTS: &[(&str, &str)] = &[
    ("component_upgrades_pkey", "PRIMARY KEY (upgrade_id)"),
    (
        "component_upgrades_procedure_check",
        "CHECK (procedure = 'core-additive-components-v1')",
    ),
    (
        "component_upgrades_request_check",
        "CHECK (octet_length(request_sha256) = 32)",
    ),
    (
        "component_upgrades_pre_manifest_check",
        "CHECK (octet_length(predecessor_manifest_sha256) = 32)",
    ),
    (
        "component_upgrades_pre_catalog_check",
        "CHECK (octet_length(predecessor_catalog_sha256) = 32)",
    ),
    (
        "component_upgrades_post_manifest_check",
        "CHECK (octet_length(successor_manifest_sha256) = 32)",
    ),
    (
        "component_upgrades_post_catalog_check",
        "CHECK (octet_length(successor_catalog_sha256) = 32)",
    ),
    (
        "component_upgrades_delta_check",
        "CHECK (jsonb_typeof(delta_components) = 'array' AND jsonb_array_length(delta_components) BETWEEN 1 AND 16)",
    ),
    (
        "component_upgrades_catalog_check",
        "CHECK (jsonb_typeof(added_catalog_entries) = 'array' AND jsonb_array_length(added_catalog_entries) > 0)",
    ),
    (
        "component_upgrades_data_check",
        "CHECK (octet_length(preserved_data_sha256) = 32)",
    ),
    (
        "component_upgrades_permissions_check",
        "CHECK (octet_length(preserved_permissions_sha256) = 32)",
    ),
    (
        "component_upgrades_counts_check",
        "CHECK (predecessor_components >= 25 AND successor_components > predecessor_components)",
    ),
];

const TELEGRAM_RELATIONS: &[(&str, &str)] = &[
    ("bindings", "r"),
    ("bindings_pkey", "i"),
    ("telegram_active_user", "i"),
    ("flows", "r"),
    ("flows_pkey", "i"),
    ("telegram_flow_expiry", "i"),
    ("assertions", "r"),
    ("assertions_pkey", "i"),
    ("telegram_assertion_expiry", "i"),
];

// Observed PostgreSQL 18 index-column framing; independent of table nullability.
const EXPECTED_INDEX_COLUMNS: &[(&str, &str, &str, &str)] = &[
    ("core_meta", "component_upgrades_pkey", "upgrade_id", "text"),
    ("core_telegram_auth", "bindings_pkey", "subject", "text"),
    (
        "core_telegram_auth",
        "telegram_active_user",
        "user_id",
        "bigint",
    ),
    ("core_telegram_auth", "flows_pkey", "token_digest", "bytea"),
    (
        "core_telegram_auth",
        "telegram_flow_expiry",
        "expires_at",
        "timestamp with time zone",
    ),
    ("core_telegram_auth", "assertions_pkey", "digest", "bytea"),
    (
        "core_telegram_auth",
        "telegram_assertion_expiry",
        "expires_at",
        "timestamp with time zone",
    ),
];

fn request() -> UpgradeRequest {
    UpgradeRequest {
        version: 1,
        procedure: "core-additive-components-v1".into(),
        upgrade_id: "additive-fixture-0001".into(),
        database: UpgradeDatabase {
            name: "isolated_core".into(),
            oid: 12345,
            system_identifier: "1234567890123456789".into(),
            server_version_num: 180006,
        },
        roles: UpgradeRoles {
            installer: "isolated_installer".into(),
            owner: "isolated_owner".into(),
            runtime: "isolated_runtime".into(),
            extensions: "isolated_extensions".into(),
        },
        predecessor_manifest_sha256: PRE_MANIFEST.into(),
        predecessor_catalog_sha256: PRE_CATALOG.into(),
        successor_manifest_sha256: manifest_sha256().unwrap(),
    }
}

#[test]
fn compiled_25_predecessor_and_only_26_delta_match_frozen_bytes() {
    let frozen: Value = serde_json::from_str(PREDECESSOR).unwrap();
    assert_eq!(PRE_COMPONENTS, 25);
    assert_eq!(TARGET_COMPONENTS, 26);
    assert_eq!(PRE_TABLES, 153);
    assert_eq!(frozen["table_count"], 153);
    assert_eq!(frozen["sequence_count"], 21);
    assert_eq!(frozen["catalog_definition_count"], 5968);
    assert_eq!(frozen["schema_manifest_stdout_sha256"], PRE_MANIFEST);
    assert_eq!(frozen["catalog_sha256"], PRE_CATALOG);
    let expected = compiled_predecessor().unwrap();
    assert_eq!(expected.len(), 25);
    let components = frozen["manifest"]["components"].as_array().unwrap();
    assert_eq!(components.len(), 25);
    for (entry, (name, ddl)) in components.iter().zip(COMPONENTS.iter().take(25)) {
        let original = if *name == "ledger" {
            include_str!("../../../tests/core_schema/ledger_before_escrow_submit.sql")
        } else if *name == "routing" {
            include_str!("../../../tests/core_schema/routing_before_payload_policy.sql")
        } else {
            ddl
        };
        let digest = hex(&Sha256::digest(original));
        assert_eq!(entry["name"], *name);
        assert_eq!(entry["raw_schema_sha256"], digest);
        assert_eq!(expected[*name], digest);
    }
    let mut producer = schema_manifest();
    producer.components.truncate(25);
    for component in &mut producer.components {
        if component.name == "ledger" {
            component.raw_schema_sha256 = super::super::ledger::PRE_LEDGER.into();
        } else if component.name == "routing" {
            component.raw_schema_sha256 = PRE_ROUTING.into();
        }
    }
    let mut installer = Sha256::new();
    installer.update(crate::core_schema::CONTRACT);
    installer.update(crate::core_schema::UPGRADE_METADATA);
    producer.installer_contract_sql_sha256 = hex(&installer.finalize());
    assert_eq!(serde_json::to_value(&producer).unwrap(), frozen["manifest"]);
    let mut stdout = serde_json::to_vec(&producer).unwrap();
    stdout.push(b'\n');
    assert_eq!(hex(&Sha256::digest(stdout)), PRE_MANIFEST);
    assert_eq!(COMPONENTS.len(), 26);
    assert_eq!(COMPONENTS[25].0, "telegram-auth");
    assert_eq!(hex(&Sha256::digest(COMPONENTS[25].1)), TELEGRAM_DDL);
    assert_eq!(
        hex(&Sha256::digest(ADDITIVE_UPGRADE_METADATA)),
        METADATA_DDL
    );
    assert_eq!(
        delta_components(),
        json!([{
            "name":"telegram-auth", "raw_schema_sha256":TELEGRAM_DDL
        }])
    );
    assert!(!ADDITIVE_UPGRADE_METADATA.contains("core_migration"));
    assert!(!ADDITIVE_UPGRADE_METADATA.contains("source_archive"));
}

#[test]
fn additive_request_is_strict_and_binds_every_target_field() {
    let good = request();
    validate(&good).unwrap();
    // The older ledger boundary remains independently enforced.
    assert!(matches!(
        authorize_target(),
        Err(UpgradeError::Rejected(
            "UPGRADE_DB_USE_PRESERVED_26_INSTALLER"
        ))
    ));
    let encoded = serde_json::to_vec(&good).unwrap();
    UpgradeRequest::read_fields(&encoded[..]).unwrap();
    assert!(matches!(
        UpgradeRequest::read(&encoded[..]),
        Err(UpgradeError::Rejected(
            "UPGRADE_DB_USE_PRESERVED_ROUTING_INSTALLER"
        ))
    ));
    for field in [
        "sql",
        "grant_everything",
        "predecessor_components",
        "delta_components",
    ] {
        let mut unknown = serde_json::to_value(&good).unwrap();
        unknown[field] = json!("untrusted");
        assert!(UpgradeRequest::read_fields(&serde_json::to_vec(&unknown).unwrap()[..]).is_err());
    }
    let mut unknown = serde_json::to_value(&good).unwrap();
    unknown["roles"]["password"] = json!("not-accepted");
    assert!(UpgradeRequest::read_fields(&serde_json::to_vec(&unknown).unwrap()[..]).is_err());
    let invalid: &[fn(&mut UpgradeRequest)] = &[
        |r| r.version = 0,
        |r| r.procedure = "core-unreviewed-v1".into(),
        |r| r.upgrade_id = "x".repeat(15),
        |r| r.upgrade_id = "x".repeat(97),
        |r| r.upgrade_id = "invalid/upgrade/identifier".into(),
        |r| r.database.name.clear(),
        |r| r.database.name = "x".repeat(64),
        |r| r.database.name = "invalid\0database".into(),
        |r| r.database.oid = 0,
        |r| r.database.server_version_num = 179999,
        |r| r.database.server_version_num = 190000,
        |r| r.database.system_identifier.clear(),
        |r| r.database.system_identifier = "0".into(),
        |r| r.database.system_identifier = "18446744073709551616".into(),
        |r| r.database.system_identifier = "123notnumeric".into(),
        |r| r.predecessor_manifest_sha256 = "0".repeat(64),
        |r| r.predecessor_catalog_sha256 = "0".repeat(64),
        |r| r.successor_manifest_sha256 = "0".repeat(64),
        |r| r.roles.owner.clear(),
        |r| r.roles.runtime = "invalid\nrole".into(),
        |r| r.roles.extensions = "x".repeat(64),
        |r| r.roles.installer = r.roles.owner.clone(),
        |r| r.roles.runtime = r.roles.extensions.clone(),
    ];
    for mutate in invalid {
        let mut wrong = good.clone();
        mutate(&mut wrong);
        assert!(wrong.validate().is_err());
    }
}

fn fk(table: &str, constraint: &str, function: &str, kind: i32) -> String {
    json!([
        "internal-trigger",
        "core_identity",
        table,
        constraint,
        "pg_catalog",
        function,
        kind,
        "O",
        false,
        false,
        "",
        "",
        null
    ])
    .to_string()
}

// Synthetic canonical-shaped entries exercise classification. CHECK bodies are
// valid illustrative SQL, not claims about an observed PostgreSQL deparser.
// The procedure pins executed DDL and records actual full catalog entries.
fn fixture() -> (Vec<String>, Vec<String>) {
    let mut before = vec![
        json!(["relation", "core_identity", "users", "r"]).to_string(),
        json!(["relation", "core_identity", "web_sessions", "r"]).to_string(),
        json!(["relation", "core_meta", "schema_upgrades", "r"]).to_string(),
        json!([
            "function",
            "core_identity",
            "unchanged_guard",
            "original body"
        ])
        .to_string(),
    ];
    before.sort_unstable();
    let mut live = before.clone();
    for (name, kind) in TELEGRAM_RELATIONS {
        live.push(json!(["relation", "core_telegram_auth", name, kind]).to_string());
    }
    live.push(json!(["relation", "core_meta", "component_upgrades", "r"]).to_string());
    live.push(json!(["relation", "core_meta", "component_upgrades_pkey", "i"]).to_string());
    live.push(json!(["index","core_meta","component_upgrades_pkey",
        "CREATE UNIQUE INDEX component_upgrades_pkey ON core_meta.component_upgrades USING btree (upgrade_id)"]).to_string());
    for (schema, index, column, data_type) in EXPECTED_INDEX_COLUMNS {
        live.push(
            json!([
                "column", schema, index, 1, column, data_type, false, "", "", null
            ])
            .to_string(),
        );
    }
    for (index, (name, column_type)) in RECEIPT_FIELDS.iter().enumerate() {
        live.push(
            json!([
                "column",
                "core_meta",
                "component_upgrades",
                index + 1,
                name,
                column_type,
                true,
                "",
                "",
                null
            ])
            .to_string(),
        );
        live.push(
            json!([
                "constraint",
                "core_meta",
                format!("component_upgrades_{name}_not_null"),
                format!("NOT NULL {name}")
            ])
            .to_string(),
        );
    }
    for (name, definition) in NAMED_CONSTRAINTS {
        live.push(json!(["constraint", "core_meta", name, definition]).to_string());
    }
    for (table, constraint) in [
        ("users", "bindings_user_id_fkey"),
        ("users", "flows_user_id_fkey"),
        ("web_sessions", "flows_sid_fkey"),
    ] {
        live.push(fk(table, constraint, "RI_FKey_noaction_del", 9));
        live.push(fk(table, constraint, "RI_FKey_noaction_upd", 17));
    }
    (before, live)
}

fn entries(rows: &[String], kind: &str, schema: &str) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, text)| {
            let row: Vec<Value> = serde_json::from_str(text).unwrap();
            (row[0] == kind && row[1] == schema).then_some(index)
        })
        .collect()
}

#[test]
fn complete_projection_preserves_old_bytes_and_sorts_both_partitions() {
    assert_eq!(RECEIPT_FIELDS.len(), 21);
    assert_eq!(NAMED_CONSTRAINTS.len(), 12);
    assert_eq!(TELEGRAM_RELATIONS.len(), 9);
    assert_eq!(EXPECTED_INDEX_COLUMNS.len(), 7);
    assert_eq!(INDEX_COLUMNS, EXPECTED_INDEX_COLUMNS);
    assert_eq!(
        RECEIPT_FIELDS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        RECEIPT_COLUMNS
    );
    assert_eq!(
        NAMED_CONSTRAINTS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        RECEIPT_CONSTRAINTS
    );
    assert_eq!(
        TELEGRAM_RELATIONS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        NEW_RELATIONS
    );
    let (before, mut live) = fixture();
    let (old, added) = projection(&live).unwrap();
    assert_eq!(old, before);
    assert_eq!(added.len(), 79);
    assert_eq!(added.iter().collect::<BTreeSet<_>>().len(), added.len());
    assert!(old.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(added.windows(2).all(|pair| pair[0] <= pair[1]));
    live.reverse();
    assert_eq!(projection(&live).unwrap(), (old, added));
}

#[test]
fn every_required_object_must_occur_exactly_once() {
    let (before, live) = fixture();
    for index in before.len()..live.len() {
        let mut missing = live.clone();
        missing.remove(index);
        assert!(
            projection(&missing).is_err(),
            "missing required entry {index}"
        );
        let mut duplicate = live.clone();
        duplicate.push(live[index].clone());
        assert!(
            projection(&duplicate).is_err(),
            "duplicate required entry {index}"
        );
    }
}

#[test]
fn all_seven_index_columns_require_exact_fields_and_shape() {
    let (_, live) = fixture();
    let indices: Vec<_> = live
        .iter()
        .enumerate()
        .filter_map(|(index, text)| {
            let row: Vec<Value> = serde_json::from_str(text).unwrap();
            (row[0] == "column"
                && EXPECTED_INDEX_COLUMNS
                    .iter()
                    .any(|(schema, relation, _, _)| row[1] == *schema && row[2] == *relation))
            .then_some(index)
        })
        .collect();
    assert_eq!(indices.len(), 7);
    for index in indices {
        for (field, value) in [
            (0, json!("unreviewed_kind")),
            (1, json!("foreign_namespace")),
            (2, json!("unreviewed_index")),
            (3, json!(2)),
            (3, json!("1")),
            (4, json!("unreviewed_column")),
            (5, json!("unreviewed_type")),
            (6, json!(true)),
            (7, json!("a")),
            (7, Value::Null),
            (8, json!("s")),
            (8, Value::Null),
            (9, json!("unreviewed default")),
        ] {
            let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
            row[field] = value;
            let mut wrong = live.clone();
            wrong[index] = serde_json::to_string(&row).unwrap();
            assert!(
                projection(&wrong).is_err(),
                "index column {index} field {field}"
            );
        }
        for length in [9, 11] {
            let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
            row.resize(length, Value::Null);
            let mut wrong = live.clone();
            wrong[index] = serde_json::to_string(&row).unwrap();
            assert!(
                projection(&wrong).is_err(),
                "index column {index} length {length}"
            );
        }
    }
}

#[test]
fn all_21_metadata_not_null_pairs_reject_same_name_wrong_column_and_shape() {
    let (_, live) = fixture();
    let indices: Vec<_> = entries(&live, "constraint", "core_meta")
        .into_iter()
        .filter(|index| {
            let row: Vec<Value> = serde_json::from_str(&live[*index]).unwrap();
            row[2].as_str().unwrap().ends_with("_not_null")
        })
        .collect();
    assert_eq!(indices.len(), 21);
    for index in indices {
        let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
        row[3] = json!("NOT NULL unrelated_column");
        let mut wrong = live.clone();
        wrong[index] = serde_json::to_string(&row).unwrap();
        assert!(projection(&wrong).is_err());
        let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
        row.push(Value::Null);
        wrong[index] = serde_json::to_string(&row).unwrap();
        assert!(projection(&wrong).is_err());
    }
    let mut unknown = live;
    unknown.push(
        json!([
            "constraint",
            "core_meta",
            "component_upgrades_unreviewed_not_null",
            "NOT NULL unreviewed"
        ])
        .to_string(),
    );
    assert!(projection(&unknown).is_err());
}

#[test]
fn all_six_reviewed_old_table_fk_triggers_reject_flag_or_shape_changes() {
    let (_, live) = fixture();
    let indices = entries(&live, "internal-trigger", "core_identity");
    assert_eq!(indices.len(), 6);
    for index in indices {
        for (field, value) in [
            (4, json!("untrusted_namespace")),
            (5, json!("RI_FKey_cascade_del")),
            (6, json!(99)),
            (7, json!("D")),
            (8, json!(true)),
            (9, json!(true)),
            (10, json!("1")),
            (11, json!("00")),
            (12, json!("unexpected qualification")),
        ] {
            let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
            row[field] = value;
            let mut wrong = live.clone();
            wrong[index] = serde_json::to_string(&row).unwrap();
            assert!(projection(&wrong).is_err(), "trigger {index} field {field}");
        }
        let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
        row.push(Value::Null);
        let mut wrong = live.clone();
        wrong[index] = serde_json::to_string(&row).unwrap();
        assert!(projection(&wrong).is_err());
    }
}

#[test]
fn new_relations_require_exact_kind_and_no_unreviewed_function_or_relation() {
    let (_, live) = fixture();
    let mut relation_indices = entries(&live, "relation", "core_telegram_auth");
    relation_indices.extend(
        entries(&live, "relation", "core_meta")
            .into_iter()
            .filter(|index| {
                let row: Vec<Value> = serde_json::from_str(&live[*index]).unwrap();
                matches!(
                    row[2].as_str(),
                    Some("component_upgrades" | "component_upgrades_pkey")
                )
            }),
    );
    assert_eq!(relation_indices.len(), 11);
    for index in relation_indices {
        let mut row: Vec<Value> = serde_json::from_str(&live[index]).unwrap();
        row[3] = json!("v");
        let mut wrong = live.clone();
        wrong[index] = serde_json::to_string(&row).unwrap();
        assert!(projection(&wrong).is_err());
    }
    for unexpected in [
        json!([
            "function",
            "core_telegram_auth",
            "unexpected_function",
            "unreviewed body"
        ]),
        json!(["relation", "core_telegram_auth", "unreviewed_table", "r"]),
        json!([
            "index",
            "core_telegram_auth",
            "unreviewed_index",
            "unreviewed definition"
        ]),
        json!([
            "column",
            "core_meta",
            "component_upgrades",
            22,
            "unreviewed",
            "text",
            true,
            "",
            "",
            null
        ]),
        json!([
            "column",
            "core_telegram_auth",
            "unreviewed_index",
            1,
            "digest",
            "bytea",
            false,
            "",
            "",
            null
        ]),
    ] {
        let mut wrong = live.clone();
        wrong.push(unexpected.to_string());
        assert!(projection(&wrong).is_err());
    }
}

#[test]
fn unrelated_old_objects_and_foreign_namespace_are_never_silently_stripped() {
    let (before, live) = fixture();
    for unexpected in [
        json!([
            "function",
            "core_meta",
            "component_upgrades",
            "unreviewed body"
        ]),
        json!(["relation", "core_meta", "unreviewed_relation", "r"]),
        json!([
            "constraint",
            "core_identity",
            "component_upgrades_upgrade_id_not_null",
            "NOT NULL upgrade_id"
        ]),
        json!([
            "column",
            "foreign_namespace",
            "component_upgrades_pkey",
            1,
            "upgrade_id",
            "text",
            false,
            "",
            "",
            null
        ]),
        json!([
            "column",
            "core_meta",
            "unreviewed_index",
            1,
            "upgrade_id",
            "text",
            false,
            "",
            "",
            null
        ]),
    ] {
        let text = unexpected.to_string();
        let mut changed = live.clone();
        changed.push(text.clone());
        let (old, _) = projection(&changed).unwrap();
        assert_ne!(old, before);
        assert!(old.contains(&text));
    }
    let mut malformed = live;
    malformed.push("not JSON".into());
    assert!(projection(&malformed).is_err());
}

#[test]
fn preserved_table_selection_and_data_hash_use_exact_boundaries() {
    let retained = vec![
        "core_identity.users".into(),
        "core_meta.schema_upgrades".into(),
        "core_meta.component_upgrades_history".into(),
        "core_telegram_auth_copy.bindings".into(),
    ];
    let mut all = retained.clone();
    all.extend([
        "core_telegram_auth.bindings".into(),
        "core_meta.component_upgrades".into(),
    ]);
    assert_eq!(old_tables(&all), retained);
    let first = BTreeMap::from([("ab".into(), b"c".to_vec())]);
    let second = BTreeMap::from([("a".into(), b"bc".to_vec())]);
    assert_ne!(data_fingerprint(&first), data_fingerprint(&second));
    let data = BTreeMap::from([("a".into(), vec![1, 2]), ("b".into(), vec![3])]);
    let reversed = BTreeMap::from([("b".into(), vec![3]), ("a".into(), vec![1, 2])]);
    assert_eq!(data_fingerprint(&data), data_fingerprint(&reversed));
}
