//! Newly constructed SQLx-owned current-schema fixtures with only the charging
//! index removed. These do not establish historical identity-schema upgrades.
use super::*;

#[path = "../../tests/common/database_snapshot.rs"]
mod database_snapshot;

// Exact historical SQL bytes, authenticated by the unchanged frozen contracts.
// Keeping all predecessor components independent of current sources prevents a
// future fresh-install change from rewriting historical fixture inputs.
const PRE_HISTORY_COMPONENTS: &[(&str, &str)] = &[
    (
        "identity",
        include_str!("../../tests/core_schema/identity_pre_history.sql"),
    ),
    (
        "ledger",
        include_str!("../../tests/core_schema/ledger_pre_history.sql"),
    ),
    (
        "payments",
        include_str!("../../tests/core_schema/payments_before_seventeen.sql"),
    ),
    (
        "charging",
        include_str!("../../tests/core_schema/charging_before_seventeen.sql"),
    ),
    (
        "routing",
        include_str!("../../tests/core_schema/routing_pre_history.sql"),
    ),
    (
        "events",
        include_str!("../../tests/core_schema/events_pre_history.sql"),
    ),
    (
        "charging-ledger",
        include_str!("../../tests/core_schema/charging_ledger_pre_history.sql"),
    ),
    (
        "migration",
        include_str!("../../tests/core_schema/migration_pre_history.sql"),
    ),
    (
        "console",
        include_str!("../../tests/core_schema/console_pre_history.sql"),
    ),
    (
        "model-gateway",
        include_str!("../../tests/core_schema/model_gateway_before_paging.sql"),
    ),
    (
        "commerce",
        include_str!("../../tests/core_schema/commerce_pre_history.sql"),
    ),
    (
        "abuse",
        include_str!("../../tests/core_schema/abuse_pre_history.sql"),
    ),
    (
        "oauth-grants",
        include_str!("../../tests/core_schema/oauth_grants_pre_history.sql"),
    ),
];
const PRE_MODEL_PAGING_HISTORY: &str =
    include_str!("../../tests/core_schema/history_pre_model_paging.sql");
const POST_MODEL_PAGING_MODEL: &str =
    include_str!("../../tests/core_schema/model_gateway_after_paging.sql");
const HISTORICAL_IDENTITY_RECEIPT: &str =
    include_str!("../../tests/core_schema/identity_receipt_pre_history.rs");
const HISTORICAL_OAUTH_RECEIPT: &str =
    include_str!("../../tests/core_schema/oauth_receipt_pre_history.rs");
const HISTORICAL_CONTRACT: &str =
    include_str!("../../tests/core_schema/installer_contract_pre_history.sql");
const HISTORICAL_CATALOG: &str = include_str!("../../tests/core_schema/catalog_pre_history.sql");
// Both frozen historical identity writers record version 4; OAuth records 1.
const HISTORICAL_IDENTITY_VERSION: i32 = 4;
const HISTORICAL_OAUTH_VERSION: i32 = 1;

fn historical_oauth_catalog() -> &'static str {
    HISTORICAL_OAUTH_RECEIPT
        .split_once("const CATALOG: &str = r#\"")
        .unwrap()
        .1
        .split_once("\"#;")
        .unwrap()
        .0
}

fn assert_historical_receipt_sources(frozen: &serde_json::Value) {
    assert_eq!(frozen["identity_version"], HISTORICAL_IDENTITY_VERSION);
    assert_eq!(frozen["oauth_receipt_version"], HISTORICAL_OAUTH_VERSION);
    assert!(HISTORICAL_IDENTITY_RECEIPT.contains("const VERSION: i32 = 4;"));
    assert_eq!(
        frozen["catalog_fingerprint_encoding"],
        "sha256-u64be-byte-length-prefixed-utf8-v1"
    );
    assert_eq!(
        frozen["oauth_catalog_query_sha256"],
        format!(
            "{:x}",
            Sha256::digest(historical_oauth_catalog().as_bytes())
        )
    );
    for (field, bytes) in [
        (
            "installer_contract_sql_sha256",
            HISTORICAL_CONTRACT.as_bytes(),
        ),
        ("catalog_query_sha256", HISTORICAL_CATALOG.as_bytes()),
        (
            "identity_receipt_source_sha256",
            HISTORICAL_IDENTITY_RECEIPT.as_bytes(),
        ),
        (
            "oauth_receipt_source_sha256",
            HISTORICAL_OAUTH_RECEIPT.as_bytes(),
        ),
    ] {
        assert_eq!(
            frozen[field],
            format!("{:x}", Sha256::digest(bytes)),
            "{field}"
        );
    }
}

fn legacy_components() -> Vec<(&'static str, &'static str)> {
    let frozen: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/core_schema/pre_history_contract.json"
    ))
    .unwrap();
    assert_eq!(
        frozen["source_commit"],
        "a436ad4a73cb640babce61fe5e9fc85aead610c1"
    );
    assert_eq!(frozen["fixture_kind"], "pre-history-13-index-predecessor");
    assert_historical_receipt_sources(&frozen);
    let components = frozen["components"].as_array().unwrap();
    assert_eq!(components.len(), 13);
    let mut names = std::collections::BTreeSet::new();
    // Validate every original compiled byte before executing any of them.
    components
        .iter()
        .map(|expected| {
            let name = expected["name"].as_str().unwrap();
            assert!(names.insert(name));
            let &(name, source) = PRE_HISTORY_COMPONENTS
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap();
            assert_eq!(
                expected["sha256"],
                format!("{:x}", Sha256::digest(source.as_bytes())),
                "{name}"
            );
            (name, source)
        })
        .collect()
}

fn pre_model_paging_components() -> Vec<(&'static str, &'static str)> {
    let frozen: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/core_schema/pre_model_paging_contract.json"
    ))
    .unwrap();
    assert_eq!(
        frozen["source_commit"],
        "7cc3be8b5c65be911b22494d7a6c724d4cbf3938"
    );
    assert_eq!(
        frozen["fixture_kind"],
        "pre-model-paging-14-index-predecessor"
    );
    assert_historical_receipt_sources(&frozen);
    let mut components = legacy_components();
    components.push(("history", PRE_MODEL_PAGING_HISTORY));
    let expected = frozen["components"].as_array().unwrap();
    assert_eq!(components.len(), 14);
    assert_eq!(expected.len(), components.len());
    for ((name, source), expected) in components.iter().zip(expected) {
        assert_eq!(expected["name"], *name);
        assert_eq!(
            expected["sha256"],
            format!("{:x}", Sha256::digest(source.as_bytes()))
        );
    }
    components
}

async fn prior(pool: &PgPool) -> [u8; 32] {
    init(pool).await.unwrap();
    remove_usage_index(pool).await
}

async fn remove_usage_index(pool: &PgPool) -> [u8; 32] {
    let mut tx = pool.begin().await.unwrap();
    check_usage_index(&mut tx).await.unwrap();
    sqlx::query("DROP INDEX core_charging.ledger_commands_request_operation")
        .execute(&mut *tx)
        .await
        .unwrap();
    let changed = sqlx::query(
        "UPDATE core_meta.component_contract SET fingerprint=$1 WHERE name='charging-ledger'",
    )
    .bind(PRIOR_CHARGING_LEDGER.to_vec())
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(changed.rows_affected(), 1);
    let hash = catalog(&mut tx).await.unwrap();
    let changed =
        sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
            .bind(&hash)
            .execute(&mut *tx)
            .await
            .unwrap();
    assert_eq!(changed.rows_affected(), 1);
    tx.commit().await.unwrap();
    hash.try_into().unwrap()
}

async fn prior_components(pool: &PgPool, components: &[(&str, &str)]) -> [u8; 32] {
    let mut tx = pool.begin().await.unwrap();
    for (name, source) in components {
        let source = if *name == "charging-ledger" {
            let index = "CREATE INDEX ledger_commands_request_operation\n    ON core_charging.ledger_commands USING btree (request_id, operation_id);\n";
            assert_eq!(source.matches(index).count(), 1);
            let old = source.replace(index, "");
            assert_eq!(
                Sha256::digest(old.as_bytes()).as_slice(),
                PRIOR_CHARGING_LEDGER
            );
            old
        } else {
            source.to_string()
        };
        sqlx::raw_sql(&source).execute(&mut *tx).await.unwrap();
        // Receipt tables are created below, after all component sources.
    }
    sqlx::raw_sql(HISTORICAL_CONTRACT)
        .execute(&mut *tx)
        .await
        .unwrap();
    let source = |name| components.iter().find(|(n, _)| *n == name).unwrap().1;
    sqlx::query("INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES ($1,$2)")
        .bind(HISTORICAL_IDENTITY_VERSION)
        .bind(Sha256::digest(source("identity").as_bytes()).to_vec())
        .execute(&mut *tx)
        .await
        .unwrap();
    let oauth_definitions: Vec<String> = sqlx::query_scalar(historical_oauth_catalog())
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint) VALUES ($1,$2,$3)")
        .bind(HISTORICAL_OAUTH_VERSION)
        .bind(Sha256::digest(source("oauth-grants").as_bytes()).to_vec())
        .bind(fingerprint(&oauth_definitions))
        .execute(&mut *tx)
        .await
        .unwrap();
    for (name, source) in components {
        let hash = if *name == "charging-ledger" {
            PRIOR_CHARGING_LEDGER.to_vec()
        } else {
            Sha256::digest(source.as_bytes()).to_vec()
        };
        sqlx::query("INSERT INTO core_meta.component_contract VALUES ($1,$2)")
            .bind(name)
            .bind(hash)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let definitions: Vec<String> = sqlx::query_scalar(HISTORICAL_CATALOG)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    let hash = fingerprint(&definitions);
    sqlx::query("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES ($1)")
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    hash.try_into().unwrap()
}

async fn snapshot(pool: &PgPool) -> Vec<String> {
    database_snapshot::snapshot(pool)
        .await
        .into_iter()
        .map(|(table, rows)| {
            let kind = match table.as_str() {
                "r:core_meta.component_contract" => "component",
                "r:core_meta.runtime_contract" => "runtime",
                _ => "data",
            };
            serde_json::to_string(&(kind, table, rows)).unwrap()
        })
        .collect()
}

async fn receipts(pool: &PgPool) -> std::collections::BTreeMap<String, Vec<serde_json::Value>> {
    database_snapshot::snapshot(pool)
        .await
        .into_iter()
        .filter(|(table, _)| table.starts_with("r:core_meta.") || table == "r:core_oauth.contract")
        .map(|(table, rows)| {
            (
                table,
                rows.into_iter()
                    .map(|row| serde_json::from_str(&row).unwrap())
                    .collect(),
            )
        })
        .collect()
}

async fn assert_current_installation_extends_reviewed_catalog(pool: &PgPool, installed: &[String]) {
    // a7e1105d3 only appends pending-liability objects to 0b4f40996's ledger.
    // Pin both original Git inputs instead of approving a newly observed hash.
    let ledger = COMPONENTS
        .iter()
        .find(|(name, _)| *name == "ledger")
        .unwrap()
        .1;
    let marker = "\n-- Historical provider orders that were still unresolved at the source\n";
    assert_eq!(ledger.matches(marker).count(), 1);
    let (baseline, addition) = ledger.split_once(marker).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(baseline)),
        "1edf84beaebc03ddc45b52da7f266980b5873905f116e031ef21efbf5fbebe7f",
        "0b4f40996 ledger blob 75f2da806477b9bfd70c608825a50cdc6e848fb3"
    );
    let mut hash = Sha256::new();
    hash.update(marker);
    hash.update(addition);
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "a9edae2dd0b0b279459bdc0d5c121bc7c9c1657acdd5851e56ccbc6c33b3a007",
        "the exact 142-line a7e1105d3 append"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(ledger)),
        "ce973f6fee25904d4fcdc87895ec49e6c53e10c9fb3508df4d7d72ace16df77f",
        "a7e1105d3 ledger blob 759a2ca17fe682a5e4bba3a1c62e4d006bcc0ff7"
    );
    let before_receipts = receipts(pool).await;
    let before_permissions = permissions(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    // This disposable SQLx database is projected only inside a rolled-back
    // transaction. RESTRICT refuses any unreviewed dependency; PostgreSQL
    // removes the new objects' own indexes, constraints and FK triggers.
    sqlx::raw_sql(
        "DROP FUNCTION core_billing.import_legacy_pending_liability(JSONB) RESTRICT;
         DROP TABLE core_billing.legacy_pending_liability_receipts RESTRICT;
         DROP TABLE core_billing.legacy_pending_liabilities RESTRICT;",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    let baseline_definitions = definitions(&mut tx).await.unwrap();
    assert_eq!(
        fingerprint(&baseline_definitions)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "ca87de9c089ffcdf4c2f738aee2fbf6c94bebe74cf48ac09074d7ca51b36c044",
        "all definitions outside the exact a7e1105d3 append retain the reviewed 0b4f40996 catalog"
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        definitions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap(),
        installed,
        "the projection must leave the complete current catalog unchanged"
    );
    assert_eq!(receipts(pool).await, before_receipts);
    assert_eq!(
        permissions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap(),
        before_permissions
    );
}

#[sqlx::test(migrations = false)]
async fn pre_history_thirteen_component_contract_is_refused_without_changes(pool: PgPool) {
    let approved = prior_components(&pool, &legacy_components()).await;
    assert_eq!(
        approved
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "b338104a3394e575c11434d9434c077793a4090cb31c830d0b490e2ea7ec40db"
    );
    for installed_index in [false, true] {
        if installed_index {
            sqlx::query(USAGE_INDEX_DDL).execute(&pool).await.unwrap();
            let source = COMPONENTS
                .iter()
                .find(|(n, _)| *n == "charging-ledger")
                .unwrap()
                .1;
            sqlx::query("UPDATE core_meta.component_contract SET fingerprint=$1 WHERE name='charging-ledger'")
                .bind(Sha256::digest(source.as_bytes()).to_vec()).execute(&pool).await.unwrap();
            let actual = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
            sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1")
                .bind(actual)
                .execute(&pool)
                .await
                .unwrap();
        }
        let before = snapshot(&pool).await;
        let before_catalog = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
        let before_permissions = permissions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap();
        let saved: Vec<u8> =
            sqlx::query_scalar("SELECT catalog_fingerprint FROM core_meta.runtime_contract")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(check(&pool).await.is_err());
        assert!(
            install_usage_receipt_index(&pool, saved.try_into().unwrap())
                .await
                .is_err()
        );
        assert_eq!(snapshot(&pool).await, before);
        assert_eq!(
            catalog(&mut pool.acquire().await.unwrap()).await.unwrap(),
            before_catalog
        );
        assert_eq!(
            permissions(&mut pool.acquire().await.unwrap())
                .await
                .unwrap(),
            before_permissions
        );
    }
}

#[sqlx::test(migrations = false)]
async fn pre_model_paging_fourteen_component_contract_is_refused_without_changes(pool: PgPool) {
    let approved = prior_components(&pool, &pre_model_paging_components()).await;
    assert_eq!(
        approved
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "fac62641832a7dea66dd565031c31087ae067659fe2bfff2a42fa0b76c32c5bd"
    );
    let before = snapshot(&pool).await;
    let before_catalog = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
    let before_permissions = permissions(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert!(check(&pool).await.is_err());
    assert!(install_usage_receipt_index(&pool, approved).await.is_err());
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        catalog(&mut pool.acquire().await.unwrap()).await.unwrap(),
        before_catalog
    );
    assert_eq!(
        permissions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap(),
        before_permissions
    );
}

#[sqlx::test(migrations = false)]
async fn post_model_paging_fourteen_component_contract_is_refused_without_changes(pool: PgPool) {
    let mut components = pre_model_paging_components();
    assert_eq!(
        format!("{:x}", Sha256::digest(POST_MODEL_PAGING_MODEL)),
        "3550655d1318c7b33e1877d6eb5b98eb47f045a9c4cfb801d23331fcccb00be4",
        "f296ce49b model-gateway blob 08537c7fc33afed4e1882f7e93f6be5284d20c17"
    );
    components
        .iter_mut()
        .find(|(name, _)| *name == "model-gateway")
        .unwrap()
        .1 = POST_MODEL_PAGING_MODEL;
    let approved = prior_components(&pool, &components).await;
    assert_eq!(
        approved
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "dbc8ad04bf41b2ea13282db9b82947b29e33b8c4329a14d9d53497c44c360b54",
        "preserve the f296ce49b reviewed catalog as an actual historical fixture"
    );
    let before = snapshot(&pool).await;
    let before_catalog = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
    let before_permissions = permissions(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert!(check(&pool).await.is_err());
    assert!(install_usage_receipt_index(&pool, approved).await.is_err());
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        catalog(&mut pool.acquire().await.unwrap()).await.unwrap(),
        before_catalog
    );
    assert_eq!(
        permissions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap(),
        before_permissions
    );
}

async fn unchanged(pool: &PgPool, before: &[String]) {
    assert_eq!(snapshot(pool).await, before);
    let index: bool = sqlx::query_scalar(
        "SELECT to_regclass('core_charging.ledger_commands_request_operation') IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(!index);
}

#[sqlx::test(migrations = false)]
async fn current_schema_index_predecessor_upgrade_changes_only_verified_index_and_two_receipts(
    pool: PgPool,
) {
    init(&pool).await.unwrap();
    check(&pool).await.unwrap();
    let installed = definitions(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    let installed_receipts = receipts(&pool).await;
    assert_eq!(
        installed_receipts["r:core_meta.component_contract"].len(),
        26
    );
    for table in [
        "r:core_meta.schema_upgrades",
        "r:core_meta.component_upgrades",
        "r:core_meta.ledger_revisions",
    ] {
        assert!(installed_receipts.contains_key(table), "missing {table}");
    }
    assert_current_installation_extends_reviewed_catalog(&pool, &installed).await;
    let approved = remove_usage_index(&pool).await;
    let prior_hex = approved
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let prior_definitions = definitions(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert!(prior_definitions.iter().all(|row| installed.contains(row)));
    let removed: Vec<serde_json::Value> = installed
        .iter()
        .filter(|row| !prior_definitions.contains(row))
        .map(|row| serde_json::from_str(row).unwrap())
        .collect();
    assert_eq!(
        removed.len(),
        4,
        "only the complete usage index was removed"
    );
    for row in &removed {
        assert_eq!(row[1], "core_charging");
        assert_eq!(row[2], "ledger_commands_request_operation");
    }
    for (kind, count) in [("relation", 1), ("column", 2), ("index", 1)] {
        assert_eq!(removed.iter().filter(|row| row[0] == kind).count(), count);
    }
    let columns: Vec<&str> = removed
        .iter()
        .filter(|row| row[0] == "column")
        .map(|row| row[4].as_str().unwrap())
        .collect();
    assert_eq!(columns, ["request_id", "operation_id"]);
    let mut expected_prior_receipts = installed_receipts.clone();
    let component = expected_prior_receipts
        .get_mut("r:core_meta.component_contract")
        .unwrap()
        .iter_mut()
        .find(|row| row["name"] == "charging-ledger")
        .unwrap();
    component["fingerprint"] = format!(
        "\\x{}",
        PRIOR_CHARGING_LEDGER
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
    .into();
    expected_prior_receipts
        .get_mut("r:core_meta.runtime_contract")
        .unwrap()[0]["catalog_fingerprint"] = format!("\\x{prior_hex}").into();
    assert_eq!(receipts(&pool).await, expected_prior_receipts);
    println!("prior catalog SHA256={prior_hex}");
    let store = IdentityStore::from_pool(pool.clone());
    store.check_schema().await.unwrap();
    let credential = store.bootstrap_user(7, 0).await.unwrap();
    let owner = store
        .authorize(&credential.secret)
        .await
        .unwrap()
        .owner_account_id;
    let ledger = crate::billing::ledger::Ledger::from_pool(pool.clone());
    ledger.open_wallet(owner).await.unwrap();
    // Nonempty financial data makes preservation assertions meaningful.
    let request = crate::billing::ledger::Request {
        scope: "index-fixture".into(),
        operation_key: "credit".into(),
        actor_user_id: Some(7),
        reason: "Synthetic prior-schema fixture".into(),
        action: crate::billing::ledger::Action::Credit {
            account_id: owner,
            amount_units: crate::billing::Credits::try_from(1000).unwrap(),
            payment_reference: "index-fixture-credit".into(),
        },
    };
    ledger.execute(&request).await.unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES ('core.history-source.v1',repeat('a',64),repeat('b',64),'index-maintenance-synthetic',1700000000,true)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota) VALUES (1,1,'index-maintenance-history',repeat('c',64),7,$1,1700000000,2,1)")
        .bind(owner).execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    let grants = permissions(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert!(
        check(&pool).await.is_err(),
        "startup must not repair old schema"
    );
    unchanged(&pool, &before).await;
    // The retired pre-Pi thirteen-component, twelve-component and earlier catalogs
    // are not the current catalog, even when approval matches the saved receipt.
    for retired_hex in [
        "dbc8ad04bf41b2ea13282db9b82947b29e33b8c4329a14d9d53497c44c360b54",
        "b338104a3394e575c11434d9434c077793a4090cb31c830d0b490e2ea7ec40db",
        "660ddf6f75c0350c93ae2886951312997c82c4bc6ebcf5d79a561de629781bad",
        "431a4af6b4790abdb32a3583cb097b733c41b6b2cdc04537e15808260de44d57",
        "9ce6c5120645792608d4466ef9fe4b6f91add7ec082670d276bffbd94f48c14a",
        "e8fa053daca7741952caa4c60e27071652f600b4fa48f4b7631ea54ee801df7b",
        "5510e0bee0e5659ff940b798c9d6d3c69aea220734d9f73577c847b6841254b7",
    ] {
        let retired: [u8; 32] = std::array::from_fn(|i| {
            u8::from_str_radix(&retired_hex[i * 2..i * 2 + 2], 16).unwrap()
        });
        for (saved, approval) in [(approved, retired), (retired, retired), (retired, approved)] {
            sqlx::query(
                "UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton",
            )
            .bind(saved.to_vec())
            .execute(&pool)
            .await
            .unwrap();
            let before_refusal = snapshot(&pool).await;
            let before_catalog = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
            assert!(
                install_usage_receipt_index(&pool, approval).await.is_err(),
                "retired catalog receipt or approval must be rejected"
            );
            unchanged(&pool, &before_refusal).await;
            assert_eq!(
                catalog(&mut pool.acquire().await.unwrap()).await.unwrap(),
                before_catalog,
                "retired catalog refusal changed definitions"
            );
            assert_eq!(
                permissions(&mut pool.acquire().await.unwrap())
                    .await
                    .unwrap(),
                grants,
                "retired catalog refusal changed permissions"
            );
        }
    }
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(approved.to_vec())
        .execute(&pool)
        .await
        .unwrap();
    unchanged(&pool, &before).await;
    install_usage_receipt_index(&pool, approved).await.unwrap();
    check(&pool).await.unwrap();
    store.check_schema().await.unwrap();
    assert_eq!(
        definitions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap(),
        installed,
        "the upgraded catalog must equal the complete reviewed fresh installation"
    );
    assert_eq!(
        receipts(&pool).await,
        installed_receipts,
        "only the charging-ledger and runtime receipts may change"
    );
    let financial = |rows: Vec<String>| {
        rows.into_iter()
            .filter(|s| !s.starts_with("[\"component\"") && !s.starts_with("[\"runtime\""))
            .collect::<Vec<_>>()
    };
    assert_eq!(financial(before), financial(snapshot(&pool).await));
    assert_eq!(
        permissions(&mut pool.acquire().await.unwrap())
            .await
            .unwrap(),
        grants
    );
    assert!(
        install_usage_receipt_index(&pool, approved).await.is_err(),
        "no retry/reinstallation"
    );
}

#[sqlx::test(migrations = false)]
async fn upgrade_rejects_wrong_approval_component_catalog_and_native_receipts(pool: PgPool) {
    let approved = prior(&pool).await;
    let before = snapshot(&pool).await;
    assert!(install_usage_receipt_index(&pool, [0; 32]).await.is_err());
    unchanged(&pool, &before).await;
    for sql in [
        "UPDATE core_meta.component_contract SET fingerprint=decode(repeat('00',32),'hex') WHERE name='ledger'",
        "UPDATE core_meta.component_contract SET fingerprint=decode(repeat('00',32),'hex') WHERE name='charging-ledger'",
        "UPDATE core_meta.schema_contract SET version=version+1",
        "UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode(repeat('00',32),'hex')",
        "ALTER TABLE core_charging.ledger_commands ADD COLUMN unexpected TEXT",
        "INSERT INTO core_meta.component_contract VALUES ('unexpected',decode(repeat('00',32),'hex'))",
    ] {
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("BEGIN")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(sql).execute(&mut *connection).await.unwrap();
        sqlx::query("COMMIT")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        let drift = snapshot(&pool).await;
        assert!(
            install_usage_receipt_index(&pool, approved).await.is_err(),
            "{sql}"
        );
        unchanged(&pool, &drift).await;
        let undo = match sql {
            s if s.contains("name='ledger'") => Some((
                "ledger",
                COMPONENTS.iter().find(|(n, _)| *n == "ledger").unwrap().1,
            )),
            s if s.contains("name='charging-ledger'") => None,
            _ => None,
        };
        if let Some((name, source)) = undo {
            sqlx::query("UPDATE core_meta.component_contract SET fingerprint=$1 WHERE name=$2")
                .bind(Sha256::digest(source.as_bytes()).to_vec())
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        } else if sql.contains("name='charging-ledger'") {
            sqlx::query("UPDATE core_meta.component_contract SET fingerprint=$1 WHERE name='charging-ledger'")
                .bind(PRIOR_CHARGING_LEDGER.to_vec()).execute(&pool).await.unwrap();
        } else if sql.contains("version=version+1") {
            sqlx::query("UPDATE core_meta.schema_contract SET version=version-1")
                .execute(&pool)
                .await
                .unwrap();
        } else if sql.contains("runtime_contract") {
            sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1")
                .bind(approved.to_vec())
                .execute(&pool)
                .await
                .unwrap();
        } else if sql.contains("ADD COLUMN") {
            sqlx::query("ALTER TABLE core_charging.ledger_commands DROP COLUMN unexpected")
                .execute(&pool)
                .await
                .unwrap();
        } else {
            sqlx::query("DELETE FROM core_meta.component_contract WHERE name='unexpected'")
                .execute(&pool)
                .await
                .unwrap();
        }
        unchanged(&pool, &before).await;
    }
}

#[sqlx::test(migrations = false)]
async fn startup_required_shape_cannot_be_bypassed_by_re_receipting(pool: PgPool) {
    init(&pool).await.unwrap();
    for ddl in [
        "", // missing
        "CREATE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(operation_id,request_id)",
        "CREATE UNIQUE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(request_id,operation_id)",
        "CREATE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(request_id,operation_id) WHERE request_id<>''",
        "CREATE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(request_id,operation_id) INCLUDE(created_at)",
        "CREATE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(request_id,operation_id) WITH(fillfactor=90)",
        "CREATE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(request_id DESC,operation_id)",
    ] {
        sqlx::query("DROP INDEX IF EXISTS core_charging.ledger_commands_request_operation")
            .execute(&pool)
            .await
            .unwrap();
        if !ddl.is_empty() {
            sqlx::query(ddl).execute(&pool).await.unwrap();
        }
        let hash = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
        sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1")
            .bind(hash)
            .execute(&pool)
            .await
            .unwrap();
        assert!(check(&pool).await.is_err(), "{ddl}");
    }
    sqlx::query("DROP INDEX core_charging.ledger_commands_request_operation")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(USAGE_INDEX_DDL).execute(&pool).await.unwrap();
    let hash = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1")
        .bind(hash)
        .execute(&pool)
        .await
        .unwrap();
    check(&pool).await.unwrap();
    // Disposable superuser fixture only: exercise readiness/validity omitted by
    // pg_get_indexdef. Restore catalog flags before fixture teardown.
    for flag in ["indisvalid", "indisready"] {
        sqlx::query(&format!("UPDATE pg_index SET {flag}=false WHERE indexrelid='core_charging.ledger_commands_request_operation'::regclass"))
            .execute(&pool).await.unwrap();
        assert!(check(&pool).await.is_err());
        sqlx::query(&format!("UPDATE pg_index SET {flag}=true WHERE indexrelid='core_charging.ledger_commands_request_operation'::regclass"))
            .execute(&pool).await.unwrap();
    }
    check(&pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn upgrade_rejects_existing_index_even_with_approved_catalog(pool: PgPool) {
    let _ = prior(&pool).await;
    for ddl in [
        USAGE_INDEX_DDL,
        "CREATE INDEX ledger_commands_request_operation ON core_charging.ledger_commands(operation_id,request_id)",
    ] {
        sqlx::query(ddl).execute(&pool).await.unwrap();
        let hash = catalog(&mut pool.acquire().await.unwrap()).await.unwrap();
        sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1")
            .bind(&hash)
            .execute(&pool)
            .await
            .unwrap();
        let before = snapshot(&pool).await;
        assert!(
            install_usage_receipt_index(&pool, hash.try_into().unwrap())
                .await
                .is_err()
        );
        assert_eq!(snapshot(&pool).await, before);
        sqlx::query("DROP INDEX core_charging.ledger_commands_request_operation")
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[sqlx::test(migrations = false)]
async fn upgrade_lock_timeout_and_cancellation_leave_no_partial_index_or_receipt(pool: PgPool) {
    let approved = prior(&pool).await;
    let before = snapshot(&pool).await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE core_charging.ledger_commands IN ROW EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    assert!(install_usage_receipt_index(&pool, approved).await.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    blocker.rollback().await.unwrap();
    unchanged(&pool, &before).await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(741803120)")
        .execute(&mut *blocker)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(150),
            install_usage_receipt_index(&pool, approved)
        )
        .await
        .is_err()
    );
    blocker.rollback().await.unwrap();
    unchanged(&pool, &before).await;
    install_usage_receipt_index(&pool, approved).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn upgrade_rejects_runtime_role_and_unrelated_superuser(pool: PgPool) {
    let approved = prior(&pool).await;
    let before = snapshot(&pool).await;
    let name = format!(
        "index_other_{}_{:x}",
        std::process::id(),
        rand::random::<u64>()
    );
    let password = format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    );
    sqlx::query(&format!(
        "CREATE ROLE {name} LOGIN SUPERUSER PASSWORD '{password}'"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let options = pool
        .connect_options()
        .as_ref()
        .clone()
        .username(&name)
        .password(&password);
    let unrelated = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    assert!(
        install_usage_receipt_index(&unrelated, approved)
            .await
            .is_err()
    );
    unrelated.close().await;
    sqlx::query(&format!("DROP ROLE {name}"))
        .execute(&pool)
        .await
        .unwrap();
    unchanged(&pool, &before).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    assert!(
        role.url
            .contains(pool.connect_options().get_database().unwrap())
    );
    assert!(
        install_usage_receipt_index(&role.pool, approved)
            .await
            .is_err()
    );
    unchanged(&pool, &before).await;
    role.cleanup().await;
}

use crate::test_runtime as runtime;
