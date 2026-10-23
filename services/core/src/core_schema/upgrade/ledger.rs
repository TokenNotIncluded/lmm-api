//! Exact old Core26 -> escrow submission ledger revision. Offline only.
use super::*;
use crate::core_schema::LEDGER_REVISION_METADATA;
use serde_json::Value;

pub(super) const PROCEDURE: &str = "core-26-ledger-escrow-submit-v1";
const PREDECESSOR: &str = include_str!("../predecessor26.json");
const PRE_MANIFEST: &str = "310937ea692be55a4b5070e3d6bc2ddf86071945b717e47359d68d3571fad351";
const PRE_CATALOG: &str = "05dcb5fadfc11269de196e3730f9f65ff791bd44d499a6d062fc2b3f9c6c0061";
pub(super) const PRE_LEDGER: &str =
    "50a7290ce860b676c38a2cf291f90e73a6d4422e3c7830ca904c6fbc6713e8ce";
// Bound to independently observed PG18 fresh-install definitions, not requests.
const POST_CATALOG: &str = "ca87de9c089ffcdf4c2f738aee2fbf6c94bebe74cf48ac09074d7ca51b36c044";
const INDEXES: &[&str] = &["legacy_escrow_submit_once", "legacy_escrow_submit_pr_once"];

pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    if r.version != 1
        || r.procedure != PROCEDURE
        || !(16..=96).contains(&r.upgrade_id.len())
        || !r
            .upgrade_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || !identifier(&r.database.name)
        || r.database.oid == 0
        || !(180_000..190_000).contains(&r.database.server_version_num)
        || r.database
            .system_identifier
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
            .is_none()
        || !r
            .database
            .system_identifier
            .bytes()
            .all(|b| b.is_ascii_digit())
        || r.predecessor_manifest_sha256 != PRE_MANIFEST
        || r.predecessor_catalog_sha256 != PRE_CATALOG
        || r.successor_manifest_sha256 != manifest_sha256()?
    {
        return Err(reject("UPGRADE_DB_TARGET_OR_CONTRACT_INVALID"));
    }
    let roles = [
        &r.roles.installer,
        &r.roles.owner,
        &r.roles.runtime,
        &r.roles.extensions,
    ];
    if roles.iter().any(|v| !identifier(v))
        || roles
            .iter()
            .enumerate()
            .any(|(i, v)| roles[..i].contains(v))
    {
        return Err(reject("UPGRADE_DB_ROLES_INVALID"));
    }
    predecessor_components()?;
    Ok(())
}

fn predecessor_components() -> Result<BTreeMap<String, String>, UpgradeError> {
    let frozen: Value = serde_json::from_str(PREDECESSOR).map_err(|_| UpgradeError::Storage)?;
    let mut producer = schema_manifest();
    if producer.components.len() != 26 {
        return Err(reject("UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"));
    }
    for component in &mut producer.components {
        if component.name == "ledger" {
            component.raw_schema_sha256 = PRE_LEDGER.into();
        }
    }
    let mut installer = Sha256::new();
    installer.update(crate::core_schema::CONTRACT);
    installer.update(crate::core_schema::UPGRADE_METADATA);
    installer.update(crate::core_schema::ADDITIVE_UPGRADE_METADATA);
    producer.installer_contract_sql_sha256 = hex(&installer.finalize());
    let mut bytes = serde_json::to_vec(&producer).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    if hex(&Sha256::digest(bytes)) != PRE_MANIFEST
        || frozen["manifest"]
            != serde_json::to_value(&producer).map_err(|_| UpgradeError::Storage)?
        || frozen["catalog_sha256"] != PRE_CATALOG
        || frozen["table_count"] != 157
        || frozen["sequence_count"] != 21
    {
        return Err(reject("UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"));
    }
    Ok(producer
        .components
        .into_iter()
        .map(|c| (c.name.to_owned(), c.raw_schema_sha256))
        .collect())
}

fn delta_ddl() -> Result<String, UpgradeError> {
    let ledger = COMPONENTS
        .iter()
        .find(|(name, _)| *name == "ledger")
        .ok_or(UpgradeError::Storage)?
        .1;
    let prefix = "CREATE FUNCTION core_billing.legacy_escrow_finalize(";
    if ledger.matches(prefix).count() != 1 {
        return Err(reject("UPGRADE_DB_COMPILED_DELTA_INVALID"));
    }
    let function = ledger
        .split_once(prefix)
        .ok_or(UpgradeError::Storage)?
        .1
        .split_once("$$;")
        .ok_or(UpgradeError::Storage)?
        .0;
    let mut delta = String::from(
        "ALTER TABLE core_billing.legacy_escrow_operations DROP CONSTRAINT legacy_escrow_operations_kind_check; ALTER TABLE core_billing.legacy_escrow_operations ADD CONSTRAINT legacy_escrow_operations_kind_check CHECK(kind IN ('submit','payout','close')); ALTER TABLE core_billing.legacy_escrow_operations DROP CONSTRAINT legacy_escrow_operations_check; ALTER TABLE core_billing.legacy_escrow_operations ADD CONSTRAINT legacy_escrow_operations_check CHECK((kind IN ('submit','payout'))=(source_challenge_id IS NOT NULL));\n",
    );
    for name in INDEXES {
        let prefix = format!("CREATE UNIQUE INDEX {name} ");
        if ledger.matches(&prefix).count() != 1 {
            return Err(reject("UPGRADE_DB_COMPILED_DELTA_INVALID"));
        }
        let definition = ledger
            .split_once(&prefix)
            .ok_or(UpgradeError::Storage)?
            .1
            .split_once(';')
            .ok_or(UpgradeError::Storage)?
            .0;
        delta.push_str(&prefix);
        delta.push_str(definition);
        delta.push_str(";\n");
    }
    delta.push_str("CREATE OR REPLACE FUNCTION core_billing.legacy_escrow_finalize(");
    delta.push_str(function);
    delta.push_str("$$;\n");
    delta.push_str(LEDGER_REVISION_METADATA);
    Ok(delta)
}

async fn permissions(c: &mut PgConnection) -> Result<Vec<String>, UpgradeError> {
    Ok(sqlx::query_scalar(r#"
    WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema') AND left(nspname,3)<>'pg_'),
    rels AS (SELECT t.*,n.nspname FROM pg_class t JOIN ns n ON n.oid=t.relnamespace
      WHERE NOT(n.nspname='core_meta' AND t.relname IN ('ledger_revisions','ledger_revisions_pkey'))
        AND NOT(n.nspname='core_billing' AND t.relname IN ('legacy_escrow_submit_once','legacy_escrow_submit_pr_once'))),
    facts AS (
      SELECT json_build_array('schema',nspname,nspowner,nspacl)::text AS fact FROM ns
      UNION ALL SELECT json_build_array('relation',nspname,relname,relowner,relacl)::text FROM rels
      UNION ALL SELECT json_build_array('column',r.nspname,r.relname,a.attnum,a.attacl)::text FROM pg_attribute a JOIN rels r ON r.oid=a.attrelid WHERE a.attnum>0 AND NOT a.attisdropped
      UNION ALL SELECT json_build_array('function',n.nspname,p.oid::regprocedure::text,p.proowner,p.proacl)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
      UNION ALL SELECT json_build_array('type',n.nspname,t.typname,t.typowner,t.typacl)::text FROM pg_type t JOIN ns n ON n.oid=t.typnamespace WHERE NOT(n.nspname='core_meta' AND t.typname IN ('ledger_revisions','_ledger_revisions'))
      UNION ALL SELECT json_build_array('default',defaclrole,defaclnamespace,defaclobjtype,defaclacl)::text FROM pg_default_acl
      UNION ALL SELECT json_build_array('database',datname,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
      UNION ALL SELECT json_build_array('role',oid,rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit,rolvaliduntil)::text FROM pg_roles
      UNION ALL SELECT json_build_array('membership',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members
    ) SELECT fact FROM facts ORDER BY fact COLLATE "C"
    "#).fetch_all(c).await?)
}

fn data_hash(data: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut h = Sha256::new();
    for (name, value) in data {
        h.update((name.len() as u64).to_be_bytes());
        h.update(name.as_bytes());
        h.update((value.len() as u64).to_be_bytes());
        h.update(value);
    }
    h.finalize().to_vec()
}

async fn check_receipt_acl(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let safe:bool=sqlx::query_scalar(r#"
      SELECT has_table_privilege($2,'core_meta.ledger_revisions','SELECT')
      AND NOT has_table_privilege($2,'core_meta.ledger_revisions','INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
      AND NOT EXISTS(SELECT FROM pg_class t CROSS JOIN LATERAL aclexplode(coalesce(t.relacl,acldefault('r',t.relowner))) a
        WHERE t.oid='core_meta.ledger_revisions'::regclass AND a.grantee<>(SELECT oid FROM pg_roles WHERE rolname=$1)
        AND NOT(a.grantee=(SELECT oid FROM pg_roles WHERE rolname=$2)AND a.privilege_type='SELECT'AND NOT a.is_grantable))
      AND NOT EXISTS(SELECT FROM pg_attribute att CROSS JOIN LATERAL aclexplode(att.attacl) a
        WHERE att.attrelid='core_meta.ledger_revisions'::regclass AND a.grantee<>(SELECT oid FROM pg_roles WHERE rolname=$1))
    "#).bind(&r.roles.owner).bind(&r.roles.runtime).fetch_one(c).await?;
    if !safe {
        return Err(reject("UPGRADE_DB_NEW_OBJECT_PRIVILEGES_INVALID"));
    }
    Ok(())
}

pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.ledger_revisions')IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if exists {
        let report = resolve(&mut tx, r, &tables).await?;
        tx.rollback().await?;
        return Ok(report);
    }
    let before = definitions(&mut tx).await?;
    let saved: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if tables.len() != 157
        || hex(&fingerprint(&before)) != PRE_CATALOG
        || hex(&saved) != PRE_CATALOG
    {
        return Err(reject("UPGRADE_DB_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let actual: BTreeMap<String, String> = sqlx::query(
        "SELECT name,encode(fingerprint,'hex')AS digest FROM core_meta.component_contract",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get("name")?, row.try_get("digest")?)))
    .collect::<Result<_, sqlx::Error>>()?;
    if actual != predecessor_components()? {
        return Err(reject("UPGRADE_DB_PREDECESSOR_COMPONENT_MISMATCH"));
    }
    IdentityStore::check_schema_connection(&mut tx).await?;
    let data = old_data(&mut tx, &tables).await?;
    let old_acl = permissions(&mut tx).await?;
    sqlx::raw_sql(&delta_ddl()?).execute(&mut *tx).await?;
    sqlx::query(&format!(
        "GRANT SELECT ON core_meta.ledger_revisions TO {}",
        quote(&r.roles.runtime)
    ))
    .execute(&mut *tx)
    .await?;
    check_receipt_acl(&mut tx, r).await?;
    let after = definitions(&mut tx).await?;
    let successor = fingerprint(&after);
    if hex(&successor) != POST_CATALOG
        || permissions(&mut tx).await? != old_acl
        || old_data(&mut tx, &tables).await? != data
    {
        return Err(reject(
            "UPGRADE_DB_OLD_ROWS_DEFINITIONS_OR_PERMISSIONS_CHANGED",
        ));
    }
    let new_ledger = Sha256::digest(
        COMPONENTS
            .iter()
            .find(|(name, _)| *name == "ledger")
            .ok_or(UpgradeError::Storage)?
            .1
            .as_bytes(),
    )
    .to_vec();
    if sqlx::query("UPDATE core_meta.component_contract SET fingerprint=$1 WHERE name='ledger'AND fingerprint=decode($2,'hex')").bind(new_ledger).bind(PRE_LEDGER).execute(&mut *tx).await?.rows_affected()!=1 {return Err(reject("UPGRADE_DB_COMPONENT_RECEIPT_CONFLICT"));}
    if sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2").bind(&successor).bind(&saved).execute(&mut *tx).await?.rows_affected()!=1 {return Err(reject("UPGRADE_DB_RUNTIME_RECEIPT_CONFLICT"));}
    let removed: Vec<&String> = before
        .iter()
        .filter(|line| after.binary_search(line).is_err())
        .collect();
    let added: Vec<&String> = after
        .iter()
        .filter(|line| before.binary_search(line).is_err())
        .collect();
    sqlx::query(r#"INSERT INTO core_meta.ledger_revisions(upgrade_id,procedure,request_sha256,predecessor_manifest_sha256,predecessor_catalog_sha256,successor_manifest_sha256,successor_catalog_sha256,removed_catalog_entries,added_catalog_entries,preserved_data_sha256,preserved_permissions_sha256,database_system_identifier,database_oid,database_name,installer_role,owner_role,runtime_role,extensions_role)
      VALUES($1,$2,$3,decode($4,'hex'),decode($5,'hex'),decode($6,'hex'),$7,$8,$9,$10,$11,$12,$13::bigint::oid,$14,$15,$16,$17,$18)"#)
      .bind(&r.upgrade_id).bind(PROCEDURE).bind(r.digest()?).bind(PRE_MANIFEST).bind(PRE_CATALOG).bind(&r.successor_manifest_sha256).bind(&successor).bind(sqlx::types::Json(&removed)).bind(sqlx::types::Json(&added)).bind(data_hash(&data)).bind(fingerprint(&old_acl)).bind(&r.database.system_identifier).bind(i64::from(r.database.oid)).bind(&r.database.name).bind(&r.roles.installer).bind(&r.roles.owner).bind(&r.roles.runtime).bind(&r.roles.extensions).execute(&mut *tx).await?;
    super::super::check_connection(&mut tx).await?;
    let mut all = tables.clone();
    all.push("core_meta.ledger_revisions".into());
    all.sort();
    let mut report = resolve(&mut tx, r, &all).await?;
    report.status = "committed";
    preflight(&mut tx, r).await?;
    commit_offline_upgrade(tx, r, backend_pid).await?;
    Ok(report)
}

async fn resolve(
    c: &mut PgConnection,
    r: &UpgradeRequest,
    tables: &[String],
) -> Result<UpgradeReport, UpgradeError> {
    super::super::check_connection(c).await?;
    check_receipt_acl(c, r).await?;
    if tables.len() != 158 {
        return Err(reject("UPGRADE_DB_SUCCESSOR_TABLE_SET_CHANGED"));
    }
    if sqlx::query_scalar::<_, i64>("SELECT count(*)FROM core_meta.ledger_revisions")
        .fetch_one(&mut *c)
        .await?
        != 1
    {
        return Err(reject("UPGRADE_DB_ORIGINAL_RECEIPT_MISMATCH"));
    }
    let row=sqlx::query("SELECT *,database_oid::bigint AS database_oid_value FROM core_meta.ledger_revisions WHERE upgrade_id=$1").bind(&r.upgrade_id).fetch_optional(&mut *c).await?.ok_or_else(||reject("UPGRADE_DB_ORIGINAL_RECEIPT_ABSENT"))?;
    let actual = fingerprint(&definitions(c).await?);
    if hex(&actual) != POST_CATALOG
        || row.try_get::<String, _>("procedure")? != PROCEDURE
        || row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
        || hex(&row.try_get::<Vec<u8>, _>("predecessor_manifest_sha256")?) != PRE_MANIFEST
        || hex(&row.try_get::<Vec<u8>, _>("predecessor_catalog_sha256")?) != PRE_CATALOG
        || hex(&row.try_get::<Vec<u8>, _>("successor_manifest_sha256")?)
            != r.successor_manifest_sha256
        || row.try_get::<Vec<u8>, _>("successor_catalog_sha256")? != actual
        || row.try_get::<String, _>("database_system_identifier")? != r.database.system_identifier
        || row.try_get::<i64, _>("database_oid_value")? != i64::from(r.database.oid)
        || row.try_get::<String, _>("database_name")? != r.database.name
        || row.try_get::<String, _>("installer_role")? != r.roles.installer
        || row.try_get::<String, _>("owner_role")? != r.roles.owner
        || row.try_get::<String, _>("runtime_role")? != r.roles.runtime
        || row.try_get::<String, _>("extensions_role")? != r.roles.extensions
    {
        return Err(reject("UPGRADE_DB_ORIGINAL_RECEIPT_MISMATCH"));
    }
    let old: Vec<String> = tables
        .iter()
        .filter(|name| name.as_str() != "core_meta.ledger_revisions")
        .cloned()
        .collect();
    if row.try_get::<Vec<u8>, _>("preserved_data_sha256")? != data_hash(&old_data(c, &old).await?)
        || row.try_get::<Vec<u8>, _>("preserved_permissions_sha256")?
            != fingerprint(&permissions(c).await?)
    {
        return Err(reject("UPGRADE_DB_OLD_ROWS_OR_PERMISSIONS_CHANGED"));
    }
    let removed: sqlx::types::Json<Vec<String>> = row.try_get("removed_catalog_entries")?;
    let added: sqlx::types::Json<Vec<String>> = row.try_get("added_catalog_entries")?;
    let mut projected = definitions(c).await?;
    projected.retain(|line| !added.0.contains(line));
    projected.extend(removed.0);
    projected.sort();
    if hex(&fingerprint(&projected)) != PRE_CATALOG {
        return Err(reject(
            "UPGRADE_DB_SUCCESSOR_PREDECESSOR_PROJECTION_MISMATCH",
        ));
    }
    Ok(UpgradeReport {
        version: 1,
        protocol: "core.upgrade-db.v1",
        status: "already_committed",
        upgrade_id: r.upgrade_id.clone(),
        request_sha256: hex(&r.digest()?),
        predecessor_catalog_sha256: PRE_CATALOG,
        successor_catalog_sha256: hex(&actual),
        components: 26,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> UpgradeRequest {
        UpgradeRequest {
            version: 1,
            procedure: PROCEDURE.into(),
            upgrade_id: "ledger-revision-fixture-0001".into(),
            database: UpgradeDatabase {
                name: "synthetic_core".into(),
                oid: 123,
                system_identifier: "123456789".into(),
                server_version_num: 180006,
            },
            roles: UpgradeRoles {
                installer: "synthetic_installer".into(),
                owner: "synthetic_owner".into(),
                runtime: "synthetic_runtime".into(),
                extensions: "synthetic_extensions".into(),
            },
            predecessor_manifest_sha256: PRE_MANIFEST.into(),
            predecessor_catalog_sha256: PRE_CATALOG.into(),
            successor_manifest_sha256: manifest_sha256().unwrap(),
        }
    }
    #[test]
    fn exact_predecessor_manifest_preserves_all_twenty_six_components() {
        let predecessor = predecessor_components().unwrap();
        assert_eq!(predecessor.len(), 26);
        assert_eq!(
            predecessor["ledger"],
            hex(&Sha256::digest(include_str!(
                "../../../tests/core_schema/ledger_before_escrow_submit.sql"
            )))
        );
        assert_eq!(predecessor["ledger"], PRE_LEDGER);
        assert_ne!(predecessor["ledger"], hex(&Sha256::digest(COMPONENTS[2].1)));
        assert_ne!(PRE_CATALOG, POST_CATALOG);
    }
    #[test]
    fn only_the_compiled_ledger_delta_is_accepted() {
        let ddl = delta_ddl().unwrap();
        assert!(ddl.contains("CREATE OR REPLACE FUNCTION core_billing.legacy_escrow_finalize("));
        assert!(ddl.contains("CREATE UNIQUE INDEX legacy_escrow_submit_once"));
        assert!(ddl.contains("CREATE UNIQUE INDEX legacy_escrow_submit_pr_once"));
        assert!(!ddl.contains("CREATE TABLE core_billing"));
        assert!(!ddl.contains("CREATE FUNCTION core_billing.import_legacy_escrow_batch"));
        assert!(!ddl.contains("DROP TABLE"));
        assert!(!ddl.contains("ALTER ROLE"));
        assert!(ddl.ends_with(LEDGER_REVISION_METADATA));
    }
    #[test]
    fn request_binds_compiled_target_and_rejects_unknown_fields() {
        let good = request();
        validate(&good).unwrap();
        let bytes = serde_json::to_vec(&good).unwrap();
        assert_eq!(
            UpgradeRequest::read(&bytes[..]).unwrap().digest().unwrap(),
            good.digest().unwrap()
        );
        let mut bad = serde_json::to_value(&good).unwrap();
        bad["sql"] = serde_json::json!("arbitrary SQL");
        assert!(UpgradeRequest::read(&serde_json::to_vec(&bad).unwrap()[..]).is_err());
        for change in [0, 1, 2, 3, 4] {
            let mut bad = good.clone();
            match change {
                0 => bad.predecessor_catalog_sha256 = "0".repeat(64),
                1 => bad.predecessor_manifest_sha256 = "0".repeat(64),
                2 => bad.successor_manifest_sha256 = "0".repeat(64),
                3 => bad.roles.runtime = bad.roles.owner.clone(),
                _ => bad.database.server_version_num = 170000,
            };
            assert!(validate(&bad).is_err());
        }
    }
    #[test]
    fn upgrade_receipt_has_no_financial_authority_or_mutable_history() {
        assert!(!LEDGER_REVISION_METADATA.contains("core_migration"));
        assert!(!LEDGER_REVISION_METADATA.contains("source_archive"));
        assert!(LEDGER_REVISION_METADATA.contains("BEFORE UPDATE OR DELETE"));
        assert!(LEDGER_REVISION_METADATA.contains("BEFORE TRUNCATE"));
        assert!(
            LEDGER_REVISION_METADATA
                .contains("REVOKE ALL ON core_meta.ledger_revisions FROM PUBLIC")
        );
    }
}
