use super::*;

fn request() -> UpgradeRequest {
    let frozen: serde_json::Value =
        serde_json::from_str(include_str!("../predecessor25.json")).unwrap();
    UpgradeRequest {
        version: 1,
        procedure: additive::PROCEDURE.into(),
        upgrade_id: "schema-upgrade-fixture-0001".into(),
        database: UpgradeDatabase {
            name: "isolated_core".into(),
            oid: 12345,
            system_identifier: "1234567890123456789".into(),
            server_version_num: 180000,
        },
        roles: UpgradeRoles {
            installer: "isolated_installer".into(),
            owner: "isolated_owner".into(),
            runtime: "isolated_runtime".into(),
            extensions: "isolated_extensions".into(),
        },
        predecessor_manifest_sha256: frozen["schema_manifest_stdout_sha256"]
            .as_str()
            .unwrap()
            .into(),
        predecessor_catalog_sha256: frozen["catalog_sha256"].as_str().unwrap().into(),
        successor_manifest_sha256: manifest_sha256().unwrap(),
    }
}
#[test]
fn legacy_predecessor_bytes_are_preserved_but_26_binary_rejects_24_to_25() {
    let frozen: serde_json::Value = serde_json::from_str(PREDECESSOR).unwrap();
    let old_components = frozen["manifest"]["components"].as_array().unwrap();
    assert_eq!(old_components.len(), 24);
    assert_eq!(frozen["tables"], 149);
    assert_eq!(frozen["sequences"], 21);
    assert_eq!(frozen["schema_manifest_stdout_sha256"], PRE_MANIFEST);
    assert_eq!(frozen["catalog_sha256"], PRE_CATALOG);
    for (old, (name, ddl)) in old_components.iter().zip(COMPONENTS.iter().take(24)) {
        assert_eq!(old["name"], *name);
        assert_eq!(old["raw_schema_sha256"], hex(&Sha256::digest(ddl)));
    }
    assert_eq!(
        old_components[1]["raw_schema_sha256"],
        "c637586f484386a8357e1b9e9dd39023d0d53316f17d0de403c864a3b49ebec9"
    );
    let mut producer = schema_manifest();
    producer.components.truncate(24);
    producer.installer_contract_sql_sha256 = hex(&Sha256::digest(super::super::CONTRACT));
    assert_eq!(serde_json::to_value(&producer).unwrap(), frozen["manifest"]);
    let mut stdout = serde_json::to_vec(&producer).unwrap();
    stdout.push(b'\n');
    assert_eq!(hex(&Sha256::digest(stdout)), PRE_MANIFEST);
    assert_eq!(COMPONENTS.len(), 26);
    assert_eq!(COMPONENTS[24].0, "trust-pricing");
    assert_eq!(hex(&Sha256::digest(COMPONENTS[24].1)), COMPONENT_25_DDL);
    assert_eq!(COMPONENTS[25].0, "telegram-auth");
    assert!(matches!(
        compiled_predecessor(),
        Err(UpgradeError::Rejected(
            "UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"
        ))
    ));
    let mut legacy = request();
    legacy.procedure = PROCEDURE.into();
    legacy.predecessor_manifest_sha256 = PRE_MANIFEST.into();
    legacy.predecessor_catalog_sha256 = PRE_CATALOG.into();
    assert!(matches!(
        legacy.validate(),
        Err(UpgradeError::Rejected(
            "UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"
        ))
    ));
    assert!(!UPGRADE_METADATA.contains("core_migration"));
    assert!(!UPGRADE_METADATA.contains("source_archive"));
}
#[test]
fn upgrade_request_is_bounded_strict_and_binds_both_compiled_contracts() {
    let good = request();
    let bytes = serde_json::to_vec(&good).unwrap();
    let parsed = UpgradeRequest::read(&bytes[..]).unwrap();
    assert_eq!(parsed.digest().unwrap(), good.digest().unwrap());
    let mut unknown = serde_json::to_value(&good).unwrap();
    unknown["grant_everything"] = true.into();
    assert!(UpgradeRequest::read(&serde_json::to_vec(&unknown).unwrap()[..]).is_err());
    assert!(UpgradeRequest::read(&vec![b' '; 16_385][..]).is_err());
    let mut wrong = good.clone();
    wrong.predecessor_catalog_sha256 = "0".repeat(64);
    assert!(wrong.validate().is_err());
    let mut wrong = good.clone();
    wrong.predecessor_manifest_sha256 = "0".repeat(64);
    assert!(wrong.validate().is_err());
    let mut wrong = good.clone();
    wrong.successor_manifest_sha256 = "0".repeat(64);
    assert!(wrong.validate().is_err());
    let mut wrong = good.clone();
    wrong.database.server_version_num = 170009;
    assert!(wrong.validate().is_err());
    let mut wrong = good.clone();
    wrong.roles.installer = good.roles.owner.clone();
    assert!(wrong.validate().is_err());
    let mut wrong = good.clone();
    wrong.upgrade_id = "untrusted\ntext".into();
    assert!(wrong.validate().is_err());
    assert_ne!(good.digest().unwrap(), {
        let mut another = good;
        another.database.oid += 1;
        another.digest().unwrap()
    });
}
#[test]
fn upgrade_request_whitespace_and_object_order_do_not_change_identity() {
    let good = request();
    let reordered = serde_json::to_vec_pretty(&serde_json::to_value(&good).unwrap()).unwrap();
    assert_eq!(
        UpgradeRequest::read(&reordered[..])
            .unwrap()
            .digest()
            .unwrap(),
        good.digest().unwrap()
    );
    assert_eq!(quote("a\"b"), "\"a\"\"b\"");
}
fn trigger(function: &str, kind: i32) -> String {
    serde_json::json!([
        "internal-trigger",
        "core_identity",
        "users",
        "user_bases_user_id_fkey",
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
fn image_changes() -> Vec<ImageDefinitionChange> {
    vec![
        ImageDefinitionChange {
            before: serde_json::json!([
                "constraint",
                "core_image",
                "operations_price_check",
                "CHECK (legacy price)"
            ])
            .to_string(),
            after: serde_json::json!([
                "constraint",
                "core_image",
                "operations_price_check",
                "CHECK (legacy or strict user_trust price)"
            ])
            .to_string(),
        },
        ImageDefinitionChange {
            before: serde_json::json!([
                "function",
                "core_image",
                "check_charge_atomic",
                "original complete binding body"
            ])
            .to_string(),
            after: serde_json::json!([
                "function",
                "core_image",
                "check_charge_atomic",
                "original binding plus exact rational reserve body"
            ])
            .to_string(),
        },
    ]
}
// Real PG18 canonical entries from the independent rollback-only diagnostic,
// root-cause receipt SHA 4e6b4a67df209938b669e08b402402301790a97de97c66958c2f2149f809c63d.
const OBSERVED_PG18_NOT_NULL: &str = r#"[
    ["constraint","core_meta","schema_upgrades_recorded_at_not_null","NOT NULL recorded_at"],
    ["constraint","core_meta","schema_upgrades_predecessor_catalog_sha256_not_null","NOT NULL predecessor_catalog_sha256"],
    ["constraint","core_meta","schema_upgrades_owner_role_not_null","NOT NULL owner_role"],
    ["constraint","core_meta","schema_upgrades_database_name_not_null","NOT NULL database_name"],
    ["constraint","core_meta","schema_upgrades_database_oid_not_null","NOT NULL database_oid"],
    ["constraint","core_meta","schema_upgrades_predecessor_manifest_sha256_not_null","NOT NULL predecessor_manifest_sha256"],
    ["constraint","core_meta","schema_upgrades_runtime_role_not_null","NOT NULL runtime_role"],
    ["constraint","core_meta","schema_upgrades_image_definition_changes_not_null","NOT NULL image_definition_changes"],
    ["constraint","core_meta","schema_upgrades_successor_manifest_sha256_not_null","NOT NULL successor_manifest_sha256"],
    ["constraint","core_meta","schema_upgrades_database_system_identifier_not_null","NOT NULL database_system_identifier"],
    ["constraint","core_meta","schema_upgrades_procedure_not_null","NOT NULL procedure"],
    ["constraint","core_meta","schema_upgrades_successor_catalog_sha256_not_null","NOT NULL successor_catalog_sha256"],
    ["constraint","core_meta","schema_upgrades_request_sha256_not_null","NOT NULL request_sha256"],
    ["constraint","core_meta","schema_upgrades_upgrade_id_not_null","NOT NULL upgrade_id"],
    ["constraint","core_meta","schema_upgrades_extensions_role_not_null","NOT NULL extensions_role"],
    ["constraint","core_meta","schema_upgrades_installer_role_not_null","NOT NULL installer_role"]
]"#;
fn metadata_not_null_definitions() -> Vec<String> {
    serde_json::from_str::<Vec<Vec<String>>>(OBSERVED_PG18_NOT_NULL)
        .unwrap()
        .iter()
        .map(|entry| serde_json::to_string(entry).unwrap())
        .collect()
}
#[test]
fn addition_projection_keeps_every_other_old_definition_and_exact_image_changes() {
    let changes = image_changes();
    let mut before = vec![r#"["relation", "core_identity", "users", "r"]"#.to_owned()];
    before.extend(changes.iter().map(|change| change.before.clone()));
    before.sort_unstable();
    let mut after = vec![r#"["relation", "core_identity", "users", "r"]"#.to_owned()];
    after.extend(changes.iter().map(|change| change.after.clone()));
    after.push(r#"["relation", "core_trust_pricing", "policies", "r"]"#.into());
    after.push(r#"["relation", "core_meta", "schema_upgrades", "r"]"#.into());
    after.extend(metadata_not_null_definitions());
    after.push(trigger("RI_FKey_noaction_del", 9));
    after.push(trigger("RI_FKey_noaction_upd", 17));
    assert_eq!(without_additions(&after, &changes).unwrap(), before);
    let mut changed = after.clone();
    *changed.last_mut().unwrap() = trigger("RI_FKey_noaction_upd", 99);
    assert!(without_additions(&changed, &changes).is_err());
    let mut missing = after.clone();
    missing.pop();
    assert!(without_additions(&missing, &changes).is_err());
    let mut arbitrary = after.clone();
    arbitrary.push(r#"["function", "core_meta", "schema_upgrades", "changed body"]"#.into());
    assert_ne!(without_additions(&arbitrary, &changes).unwrap(), before);
    let mut old_changed = after.clone();
    old_changed[0] = r#"["relation", "core_identity", "users", "p"]"#.into();
    assert_ne!(without_additions(&old_changed, &changes).unwrap(), before);
    let mut image_changed = after;
    image_changed[1] = serde_json::json!([
        "constraint",
        "core_image",
        "operations_price_check",
        "unexpected relaxed CHECK"
    ])
    .to_string();
    assert!(without_additions(&image_changed, &changes).is_err());
}
#[test]
fn image_receipt_is_two_exact_named_replacements_not_a_namespace_exception() {
    let changes = image_changes();
    let before: Vec<String> = changes.iter().map(|c| c.before.clone()).collect();
    let after: Vec<String> = changes.iter().map(|c| c.after.clone()).collect();
    let captured = image_definition_changes(&before, &after).unwrap();
    assert_eq!(captured[0].before, before[0]);
    assert_eq!(captured[1].after, after[1]);
    assert!(image_definition_changes(&before, &before).is_err());
    let mut overloaded = after.clone();
    overloaded.push(after[1].clone());
    assert!(image_definition_changes(&before, &overloaded).is_err());
    let mut foreign = changes.clone();
    foreign[1].after =
        serde_json::json!(["function", "core_image", "guard_operation", "unreviewed"]).to_string();
    assert!(validate_image_changes(&foreign).is_err());
    let mut reversed = changes.clone();
    reversed.reverse();
    assert!(validate_image_changes(&reversed).is_err());
    assert!(validate_image_changes(&changes[..1]).is_err());
}

#[test]
fn pg18_metadata_not_null_projection_accepts_only_observed_exact_pairs() {
    let observed: Vec<Vec<String>> = serde_json::from_str(OBSERVED_PG18_NOT_NULL).unwrap();
    assert_eq!(observed.len(), 16);
    let actual: std::collections::BTreeSet<_> = observed
        .iter()
        .map(|entry| (entry[2].as_str(), entry[3].as_str()))
        .collect();
    let allowed: std::collections::BTreeSet<_> = UPGRADE_NOT_NULL.iter().copied().collect();
    assert_eq!(actual, allowed);
    let changes = image_changes();
    let mut before: Vec<_> = changes.iter().map(|change| change.before.clone()).collect();
    before.sort_unstable();
    let mut after: Vec<_> = changes.iter().map(|change| change.after.clone()).collect();
    after.push(trigger("RI_FKey_noaction_del", 9));
    after.push(trigger("RI_FKey_noaction_upd", 17));
    let metadata_start = after.len();
    after.extend(metadata_not_null_definitions());
    assert_eq!(without_additions(&after, &changes).unwrap(), before);
    for (index, entry) in observed.iter().enumerate() {
        let mut changed = after.clone();
        let mut wrong = entry.clone();
        wrong[3] = "NOT NULL unrelated_column".into();
        changed[metadata_start + index] = serde_json::to_string(&wrong).unwrap();
        assert!(matches!(
            without_additions(&changed, &changes),
            Err(UpgradeError::Rejected(
                "UPGRADE_DB_METADATA_NOT_NULL_CONSTRAINT_INVALID"
            ))
        ));
    }
    let mut duplicate = after.clone();
    duplicate.push(after[metadata_start].clone());
    assert!(without_additions(&duplicate, &changes).is_err());
    let mut missing = after.clone();
    missing.remove(metadata_start);
    assert!(without_additions(&missing, &changes).is_err());
    let mut unknown = after.clone();
    unknown.push(
        serde_json::json!([
            "constraint",
            "core_meta",
            "schema_upgrades_unreviewed_not_null",
            "NOT NULL unreviewed"
        ])
        .to_string(),
    );
    assert_ne!(without_additions(&unknown, &changes).unwrap(), before);
    let mut foreign = after;
    let mut other_namespace = observed[0].clone();
    other_namespace[1] = "core_identity".into();
    foreign.push(serde_json::to_string(&other_namespace).unwrap());
    assert_ne!(without_additions(&foreign, &changes).unwrap(), before);
}
