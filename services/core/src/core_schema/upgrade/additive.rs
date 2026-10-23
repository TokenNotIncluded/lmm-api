//! The compiled additive-component procedure. Requests select no SQL, grants,
//! arbitrary predecessor, or version. The first supported delta is 25 -> 26.
//! Legacy 24 -> 25 keeps its separate frozen engine and receipt contract.
use super::*;
use crate::core_schema::ADDITIVE_UPGRADE_METADATA;
use serde_json::{Value, json};

pub(super) const PROCEDURE: &str = "core-additive-components-v1";
const PREDECESSOR: &str = include_str!("../predecessor25.json");
const PRE_MANIFEST: &str = "a9c69e01a631fd9e1b005b892ca855fb19a12041c279d04f19b2e9189d4ea301";
const PRE_CATALOG: &str = "0d3ceaaf5a3a25d2c372d3ede66890f69073dcb327100e293070ff47bce534ec";
const TELEGRAM_DDL: &str = "d4734912cf68cdc8083356dc4301670cec27ec522e2c5ecaf2653a6a7d292510";
const METADATA_DDL: &str = "776e470c0dbf5727891656a843a4de9d95db06430bbf4ea2d9255f60e5d96238";
const DELTA: &[(&str, &str)] = &[("telegram-auth", TELEGRAM_DDL)];
const PRE_COMPONENTS: usize = 25;
const TARGET_COMPONENTS: usize = 26;
const PRE_TABLES: usize = 153;
const RECEIPT_COLUMNS: &[&str] = &[
    "upgrade_id",
    "procedure",
    "request_sha256",
    "predecessor_manifest_sha256",
    "predecessor_catalog_sha256",
    "successor_manifest_sha256",
    "successor_catalog_sha256",
    "predecessor_components",
    "successor_components",
    "delta_components",
    "added_catalog_entries",
    "preserved_data_sha256",
    "preserved_permissions_sha256",
    "database_system_identifier",
    "database_oid",
    "database_name",
    "installer_role",
    "owner_role",
    "runtime_role",
    "extensions_role",
    "recorded_at",
];
const RECEIPT_CONSTRAINTS: &[&str] = &[
    "component_upgrades_pkey",
    "component_upgrades_procedure_check",
    "component_upgrades_request_check",
    "component_upgrades_pre_manifest_check",
    "component_upgrades_pre_catalog_check",
    "component_upgrades_post_manifest_check",
    "component_upgrades_post_catalog_check",
    "component_upgrades_delta_check",
    "component_upgrades_catalog_check",
    "component_upgrades_data_check",
    "component_upgrades_permissions_check",
    "component_upgrades_counts_check",
];
const NEW_RELATIONS: &[&str] = &[
    "bindings",
    "bindings_pkey",
    "telegram_active_user",
    "flows",
    "flows_pkey",
    "telegram_flow_expiry",
    "assertions",
    "assertions_pkey",
    "telegram_assertion_expiry",
];
// CATALOG includes pg_attribute entries for indexes as well as tables.
// These are the exact single-column indexes produced by the compiled delta.
const INDEX_COLUMNS: &[(&str, &str, &str, &str)] = &[
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

fn delta_components() -> Value {
    Value::Array(
        DELTA
            .iter()
            .map(|(name, digest)| json!({"name":name,"raw_schema_sha256":digest}))
            .collect(),
    )
}

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
        || r.database.system_identifier.is_empty()
        || r.database.system_identifier.len() > 20
        || !r
            .database
            .system_identifier
            .bytes()
            .all(|b| b.is_ascii_digit())
        || r.database
            .system_identifier
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
            .is_none()
        || r.predecessor_manifest_sha256 != PRE_MANIFEST
        || r.predecessor_catalog_sha256 != PRE_CATALOG
        || !digest_hex(&r.successor_manifest_sha256)
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
    compiled_predecessor()?;
    if COMPONENTS.iter().any(|(name, ddl)| {
        *name == "ledger" && hex(&Sha256::digest(ddl)) != super::ledger::PRE_LEDGER
    }) {
        return Err(reject("UPGRADE_DB_USE_PRESERVED_26_INSTALLER"));
    }
    Ok(())
}

fn compiled_predecessor() -> Result<BTreeMap<String, String>, UpgradeError> {
    if COMPONENTS.len() != TARGET_COMPONENTS
        || PRE_COMPONENTS + DELTA.len() != TARGET_COMPONENTS
        || DELTA
            .iter()
            .zip(&COMPONENTS[PRE_COMPONENTS..])
            .any(|((name, digest), (actual, ddl))| {
                name != actual || *digest != hex(&Sha256::digest(ddl))
            })
        || hex(&Sha256::digest(ADDITIVE_UPGRADE_METADATA)) != METADATA_DDL
    {
        return Err(reject("UPGRADE_DB_ADDITIVE_DELTA_DRIFT"));
    }
    let mut producer = schema_manifest();
    producer.components.truncate(PRE_COMPONENTS);
    for component in &mut producer.components {
        if component.name == "ledger" {
            component.raw_schema_sha256 = super::ledger::PRE_LEDGER.into();
        }
    }
    let mut installer = Sha256::new();
    installer.update(crate::core_schema::CONTRACT);
    installer.update(crate::core_schema::UPGRADE_METADATA);
    producer.installer_contract_sql_sha256 = hex(&installer.finalize());
    let mut bytes = serde_json::to_vec(&producer).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    let frozen: Value = serde_json::from_str(PREDECESSOR).map_err(|_| UpgradeError::Storage)?;
    if hex(&Sha256::digest(bytes)) != PRE_MANIFEST
        || frozen["schema_manifest_stdout_sha256"] != PRE_MANIFEST
        || frozen["catalog_sha256"] != PRE_CATALOG
        || frozen["table_count"] != PRE_TABLES
        || frozen["sequence_count"] != 21
        || frozen["manifest"]
            != serde_json::to_value(&producer).map_err(|_| UpgradeError::Storage)?
    {
        return Err(reject("UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"));
    }
    Ok(producer
        .components
        .into_iter()
        .map(|c| (c.name.to_owned(), c.raw_schema_sha256))
        .collect())
}

async fn tables(c: &mut PgConnection) -> Result<Vec<String>, UpgradeError> {
    Ok(sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"")
        .fetch_all(c).await?)
}

fn old_tables(all: &[String]) -> Vec<String> {
    all.iter()
        .filter(|name| {
            !name.starts_with("core_telegram_auth.")
                && name.as_str() != "core_meta.component_upgrades"
        })
        .cloned()
        .collect()
}

fn data_fingerprint(data: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut hash = Sha256::new();
    for (name, value) in data {
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    hash.finalize().to_vec()
}

async fn preserved_data(
    c: &mut PgConnection,
    all: &[String],
    upgrade_id: &str,
) -> Result<BTreeMap<String, Vec<u8>>, UpgradeError> {
    let mut data = old_data(c, &old_tables(all)).await?;
    let receipts_exist: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.component_upgrades') IS NOT NULL")
            .fetch_one(&mut *c)
            .await?;
    let receipts: Vec<String> = if receipts_exist {
        // Every previous component receipt is preserved. Only this transaction's
        // own new receipt is excluded, so the same engine supports the next
        // compiled additive step without ignoring historical metadata rows.
        sqlx::query_scalar("SELECT encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex') FROM core_meta.component_upgrades t WHERE upgrade_id<>$1 ORDER BY encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex') COLLATE \"C\"")
            .bind(upgrade_id).fetch_all(&mut *c).await?
    } else {
        Vec::new()
    };
    data.insert("component-upgrade-receipts".into(), fingerprint(&receipts));
    Ok(data)
}

// Capture every predecessor ACL, including the old trust and upgrade receipt
// tables. Only the new namespace and the exact new metadata objects are absent.
async fn permissions(c: &mut PgConnection) -> Result<Vec<String>, UpgradeError> {
    Ok(sqlx::query_scalar(r#"
      WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema','core_telegram_auth') AND left(nspname,3)<>'pg_'),
      rels AS (SELECT c.*,n.nspname FROM pg_class c JOIN ns n ON n.oid=c.relnamespace WHERE NOT($1 AND n.nspname='core_meta' AND c.relname IN ('component_upgrades','component_upgrades_pkey'))),
      facts AS (
       SELECT json_build_array('schema',nspname,nspowner,nspacl)::text AS fact FROM ns
       UNION ALL SELECT json_build_array('relation',nspname,relname,relowner,relacl)::text FROM rels
       UNION ALL SELECT json_build_array('column',r.nspname,r.relname,a.attnum,a.attacl)::text FROM pg_attribute a JOIN rels r ON r.oid=a.attrelid WHERE a.attnum>0 AND NOT a.attisdropped
       UNION ALL SELECT json_build_array('function',n.nspname,p.oid::regprocedure::text,p.proowner,p.proacl)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
       UNION ALL SELECT json_build_array('type',n.nspname,t.typname,t.typowner,t.typacl)::text FROM pg_type t JOIN ns n ON n.oid=t.typnamespace WHERE NOT($1 AND n.nspname='core_meta' AND t.typname IN ('component_upgrades','_component_upgrades'))
       UNION ALL SELECT json_build_array('default',d.defaclrole,d.defaclnamespace,d.defaclobjtype,d.defaclacl)::text FROM pg_default_acl d
       UNION ALL SELECT json_build_array('database',datname,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
       UNION ALL SELECT json_build_array('role',oid,rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit,rolvaliduntil)::text FROM pg_roles
       UNION ALL SELECT json_build_array('membership',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members
      ) SELECT fact FROM facts ORDER BY fact COLLATE "C"
    "#).bind(PRE_COMPONENTS==25).fetch_all(c).await?)
}

fn reviewed_index_column(
    row: &[Value],
    seen: &mut std::collections::BTreeSet<(String, String)>,
) -> Result<bool, UpgradeError> {
    let s = |n: usize| row.get(n).and_then(Value::as_str).unwrap_or("");
    let Some((schema, index, column, data_type)) = INDEX_COLUMNS
        .iter()
        .find(|(schema, index, _, _)| *schema == s(1) && *index == s(2))
    else {
        return Ok(false);
    };
    if row.len() != 10
        || s(0) != "column"
        || row[3].as_i64() != Some(1)
        || s(4) != *column
        || s(5) != *data_type
        || row[6] != Value::Bool(false)
        || row[7].as_str() != Some("")
        || row[8].as_str() != Some("")
        || !row[9].is_null()
        || !seen.insert(((*schema).to_owned(), (*index).to_owned()))
    {
        return Err(reject("UPGRADE_DB_ADDITIVE_INDEX_COLUMN_INVALID"));
    }
    Ok(true)
}

fn projection(live: &[String]) -> Result<(Vec<String>, Vec<String>), UpgradeError> {
    let mut old = Vec::new();
    let mut added = Vec::new();
    let mut not_null = std::collections::BTreeSet::new();
    let mut receipt_columns = std::collections::BTreeSet::new();
    let mut receipt_constraints = std::collections::BTreeSet::new();
    let mut receipt_relations = std::collections::BTreeSet::new();
    let mut receipt_indexes = std::collections::BTreeSet::new();
    let mut new_relations = std::collections::BTreeSet::new();
    let mut index_columns = std::collections::BTreeSet::new();
    let mut fk = std::collections::BTreeSet::new();
    for text in live {
        let row: Vec<Value> = serde_json::from_str(text).map_err(|_| UpgradeError::Storage)?;
        let s = |n: usize| row.get(n).and_then(Value::as_str).unwrap_or("");
        let mut addition = false;
        if s(1) == "core_telegram_auth" {
            addition = match s(0) {
                "relation" => {
                    let kind = if matches!(s(2), "bindings" | "flows" | "assertions") {
                        "r"
                    } else {
                        "i"
                    };
                    if row.len() != 4
                        || s(3) != kind
                        || !NEW_RELATIONS.contains(&s(2))
                        || !new_relations.insert(s(2).to_owned())
                    {
                        return Err(reject("UPGRADE_DB_UNEXPECTED_ADDITIVE_OBJECT"));
                    }
                    true
                }
                "column" if matches!(s(2), "bindings" | "flows" | "assertions") => true,
                "column" => reviewed_index_column(&row, &mut index_columns)?,
                "constraint" => true,
                "index" => {
                    NEW_RELATIONS.contains(&s(2))
                        && !matches!(s(2), "bindings" | "flows" | "assertions")
                }
                "internal-trigger" => {
                    matches!(s(2), "bindings" | "flows")
                        && matches!(
                            s(3),
                            "bindings_user_id_fkey" | "flows_user_id_fkey" | "flows_sid_fkey"
                        )
                }
                _ => false,
            };
            if !addition {
                return Err(reject("UPGRADE_DB_UNEXPECTED_ADDITIVE_OBJECT"));
            }
        }
        if PRE_COMPONENTS == 25 && s(1) == "core_meta" {
            addition = match s(0) {
                "relation" if matches!(s(2), "component_upgrades" | "component_upgrades_pkey") => {
                    let kind = if s(2) == "component_upgrades" {
                        "r"
                    } else {
                        "i"
                    };
                    if row.len() != 4 || s(3) != kind || !receipt_relations.insert(s(2).to_owned())
                    {
                        return Err(reject("UPGRADE_DB_ADDITIVE_METADATA_INVALID"));
                    }
                    true
                }
                "column" if s(2) == "component_upgrades" => {
                    if !RECEIPT_COLUMNS.contains(&s(4)) || !receipt_columns.insert(s(4).to_owned())
                    {
                        return Err(reject("UPGRADE_DB_ADDITIVE_METADATA_INVALID"));
                    }
                    true
                }
                "column" if s(2) == "component_upgrades_pkey" => {
                    reviewed_index_column(&row, &mut index_columns)?
                }
                "index" if s(2) == "component_upgrades_pkey" => {
                    if !receipt_indexes.insert(s(2).to_owned()) {
                        return Err(reject("UPGRADE_DB_ADDITIVE_METADATA_INVALID"));
                    }
                    true
                }
                "constraint" => {
                    if RECEIPT_CONSTRAINTS.contains(&s(2)) {
                        if !receipt_constraints.insert(s(2).to_owned()) {
                            return Err(reject("UPGRADE_DB_ADDITIVE_METADATA_INVALID"));
                        }
                        true
                    } else if let Some(column) = s(2)
                        .strip_prefix("component_upgrades_")
                        .and_then(|v| v.strip_suffix("_not_null"))
                    {
                        if !RECEIPT_COLUMNS.contains(&column)
                            || row.len() != 4
                            || s(3) != format!("NOT NULL {column}")
                            || !not_null.insert(column.to_owned())
                        {
                            return Err(reject("UPGRADE_DB_ADDITIVE_METADATA_NOT_NULL_INVALID"));
                        }
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
        }
        if s(0) == "internal-trigger" && s(1) == "core_identity" {
            let allowed = matches!(
                (s(2), s(3)),
                ("users", "bindings_user_id_fkey" | "flows_user_id_fkey")
                    | ("web_sessions", "flows_sid_fkey")
            );
            if allowed {
                let trigger_type = match s(5) {
                    "RI_FKey_noaction_del" => 9,
                    "RI_FKey_noaction_upd" => 17,
                    _ => 0,
                };
                if row.len() != 13
                    || s(4) != "pg_catalog"
                    || trigger_type == 0
                    || row[6].as_i64() != Some(trigger_type)
                    || s(7) != "O"
                    || row[8] != Value::Bool(false)
                    || row[9] != Value::Bool(false)
                    || s(10) != ""
                    || s(11) != ""
                    || !row[12].is_null()
                    || !fk.insert((s(2).to_owned(), s(3).to_owned(), s(5).to_owned()))
                {
                    return Err(reject("UPGRADE_DB_UNEXPECTED_OLD_TABLE_TRIGGER"));
                }
                addition = true;
            }
        }
        if addition {
            added.push(text.clone());
        } else {
            old.push(text.clone());
        }
    }
    if (PRE_COMPONENTS == 25
        && (not_null.len() != RECEIPT_COLUMNS.len()
            || receipt_columns.len() != RECEIPT_COLUMNS.len()
            || receipt_constraints.len() != RECEIPT_CONSTRAINTS.len()
            || receipt_relations.len() != 2
            || receipt_indexes.len() != 1))
        || new_relations.len() != NEW_RELATIONS.len()
        || index_columns.len() != INDEX_COLUMNS.len()
        || fk.len() != 6
    {
        return Err(reject("UPGRADE_DB_ADDITIVE_OBJECT_SET_INCOMPLETE"));
    }
    old.sort_unstable();
    added.sort_unstable();
    Ok((old, added))
}

async fn grant_delta(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let runtime = quote(&r.roles.runtime);
    sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_telegram_auth TO {runtime}; GRANT SELECT ON core_telegram_auth.bindings,core_telegram_auth.flows,core_telegram_auth.assertions,core_meta.component_upgrades TO {runtime}; GRANT INSERT(subject,user_id),UPDATE(revoked_at) ON core_telegram_auth.bindings TO {runtime}; GRANT INSERT(token_digest,purpose,binding_digest,user_id,user_version,sid),UPDATE(consumed_at),DELETE ON core_telegram_auth.flows TO {runtime}; GRANT INSERT(digest),DELETE ON core_telegram_auth.assertions TO {runtime}"))
        .execute(c).await?;
    Ok(())
}

async fn check_delta_privileges(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<(), UpgradeError> {
    let safe:bool=sqlx::query_scalar(r#"
     WITH owner AS(SELECT oid FROM pg_roles WHERE rolname=$1), runtime AS(SELECT oid FROM pg_roles WHERE rolname=$2),
     targets AS(SELECT c.*,n.nspname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_telegram_auth' OR (n.nspname='core_meta' AND c.relname='component_upgrades')),
     allowed(table_name,column_name,privilege) AS(VALUES ('bindings','subject','INSERT'),('bindings','user_id','INSERT'),('bindings','revoked_at','UPDATE'),('flows','token_digest','INSERT'),('flows','purpose','INSERT'),('flows','binding_digest','INSERT'),('flows','user_id','INSERT'),('flows','user_version','INSERT'),('flows','sid','INSERT'),('flows','consumed_at','UPDATE'),('assertions','digest','INSERT'))
     SELECT (SELECT count(*)=4 FROM targets WHERE relkind='r')
       AND NOT EXISTS(SELECT FROM targets t CROSS JOIN LATERAL aclexplode(COALESCE(t.relacl,acldefault('r',t.relowner))) a WHERE t.relkind='r' AND a.grantee<>(SELECT oid FROM owner) AND NOT(a.grantee=(SELECT oid FROM runtime) AND NOT a.is_grantable AND (a.privilege_type='SELECT' OR (t.nspname='core_telegram_auth' AND t.relname IN ('flows','assertions') AND a.privilege_type='DELETE'))))
       AND NOT EXISTS(SELECT FROM targets t JOIN pg_attribute att ON att.attrelid=t.oid CROSS JOIN LATERAL aclexplode(att.attacl) a WHERE a.grantee<>(SELECT oid FROM owner) AND NOT(t.nspname='core_telegram_auth' AND a.grantee=(SELECT oid FROM runtime) AND NOT a.is_grantable AND EXISTS(SELECT FROM allowed g WHERE g.table_name=t.relname AND g.column_name=att.attname AND g.privilege=a.privilege_type)))
       AND NOT EXISTS(SELECT FROM pg_namespace n CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) a WHERE n.nspname='core_telegram_auth' AND a.grantee<>(SELECT oid FROM owner) AND NOT(a.grantee=(SELECT oid FROM runtime) AND a.privilege_type='USAGE' AND NOT a.is_grantable))
       AND NOT EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='core_telegram_auth')
       AND NOT EXISTS(SELECT FROM targets WHERE relkind='S')
       AND has_schema_privilege($2,'core_telegram_auth','USAGE')
       AND NOT has_schema_privilege($2,'core_telegram_auth','CREATE')
       AND NOT EXISTS(SELECT FROM targets t WHERE t.relkind='r' AND NOT has_table_privilege($2,t.oid,'SELECT'))
       AND NOT EXISTS(SELECT FROM allowed g WHERE NOT has_column_privilege($2,'core_telegram_auth.'||g.table_name,g.column_name,g.privilege))
       AND has_table_privilege($2,'core_telegram_auth.flows','DELETE')
       AND has_table_privilege($2,'core_telegram_auth.assertions','DELETE')
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
    let (mut tx, backend_pid, all) = begin_offline_upgrade(c, r).await?;
    let receipt_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.component_upgrades') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if receipt_exists {
        let original: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_meta.component_upgrades WHERE upgrade_id=$1)",
        )
        .bind(&r.upgrade_id)
        .fetch_one(&mut *tx)
        .await?;
        if original {
            let result = resolve(&mut tx, r).await?;
            tx.rollback().await?;
            return Ok(result);
        }
    }
    let before = definitions(&mut tx).await?;
    let saved: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if all.len() != PRE_TABLES
        || hex(&fingerprint(&before)) != PRE_CATALOG
        || hex(&saved) != PRE_CATALOG
    {
        return Err(reject("UPGRADE_DB_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let actual: BTreeMap<String, String> = sqlx::query(
        "SELECT name,encode(fingerprint,'hex') AS digest FROM core_meta.component_contract",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get("name")?, row.try_get("digest")?)))
    .collect::<Result<_, sqlx::Error>>()?;
    if actual != compiled_predecessor()? {
        return Err(reject("UPGRADE_DB_PREDECESSOR_COMPONENT_MISMATCH"));
    }
    IdentityStore::check_schema_connection(&mut tx).await?;
    let old_acl = permissions(&mut tx).await?;
    let data = preserved_data(&mut tx, &all, &r.upgrade_id).await?;
    for (_, ddl) in &COMPONENTS[PRE_COMPONENTS..TARGET_COMPONENTS] {
        sqlx::raw_sql(ddl).execute(&mut *tx).await?;
    }
    if !receipt_exists {
        sqlx::raw_sql(ADDITIVE_UPGRADE_METADATA)
            .execute(&mut *tx)
            .await?;
    }
    grant_delta(&mut tx, r).await?;
    check_delta_privileges(&mut tx, r).await?;
    let after = definitions(&mut tx).await?;
    let (projected, added) = projection(&after)?;
    if projected != before || permissions(&mut tx).await? != old_acl {
        return Err(reject("UPGRADE_DB_OLD_DEFINITIONS_OR_PERMISSIONS_CHANGED"));
    }
    for (name, ddl) in &COMPONENTS[PRE_COMPONENTS..TARGET_COMPONENTS] {
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES ($1,$2)")
            .bind(name)
            .bind(Sha256::digest(ddl).to_vec())
            .execute(&mut *tx)
            .await?;
    }
    let successor = fingerprint(&after);
    let changed=sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2").bind(&successor).bind(&saved).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(reject("UPGRADE_DB_RUNTIME_RECEIPT_CONFLICT"));
    }
    sqlx::query(r#"INSERT INTO core_meta.component_upgrades(upgrade_id,procedure,request_sha256,predecessor_manifest_sha256,predecessor_catalog_sha256,successor_manifest_sha256,successor_catalog_sha256,predecessor_components,successor_components,delta_components,added_catalog_entries,preserved_data_sha256,preserved_permissions_sha256,database_system_identifier,database_oid,database_name,installer_role,owner_role,runtime_role,extensions_role)
       VALUES ($1,$2,$3,decode($4,'hex'),decode($5,'hex'),decode($6,'hex'),$7,$8,$9,$10,$11,$12,$13,$14,$15::bigint::oid,$16,$17,$18,$19,$20)"#)
       .bind(&r.upgrade_id).bind(PROCEDURE).bind(r.digest()?).bind(PRE_MANIFEST).bind(PRE_CATALOG).bind(&r.successor_manifest_sha256).bind(&successor)
       .bind(PRE_COMPONENTS as i32).bind(TARGET_COMPONENTS as i32)
       .bind(sqlx::types::Json(delta_components())).bind(sqlx::types::Json(&added)).bind(data_fingerprint(&data)).bind(fingerprint(&old_acl))
       .bind(&r.database.system_identifier).bind(i64::from(r.database.oid)).bind(&r.database.name).bind(&r.roles.installer).bind(&r.roles.owner).bind(&r.roles.runtime).bind(&r.roles.extensions)
       .execute(&mut *tx).await?;
    crate::core_schema::check_connection(&mut tx).await?;
    if old_tables(&tables(&mut tx).await?) != old_tables(&all) {
        return Err(reject("UPGRADE_DB_OLD_ROWS_OR_SEQUENCE_STATE_CHANGED"));
    }
    let mut report = resolve(&mut tx, r).await?;
    report.status = "committed";
    preflight(&mut tx, r).await?;
    commit_offline_upgrade(tx, r, backend_pid).await?;
    Ok(report)
}

async fn resolve(c: &mut PgConnection, r: &UpgradeRequest) -> Result<UpgradeReport, UpgradeError> {
    crate::core_schema::check_connection(c).await?;
    check_delta_privileges(c, r).await?;
    let live = definitions(c).await?;
    let actual = fingerprint(&live);
    let (projected, added) = projection(&live)?;
    if hex(&fingerprint(&projected)) != PRE_CATALOG {
        return Err(reject(
            "UPGRADE_DB_SUCCESSOR_PREDECESSOR_PROJECTION_MISMATCH",
        ));
    }
    let row=sqlx::query("SELECT *,database_oid::bigint AS database_oid_value FROM core_meta.component_upgrades WHERE upgrade_id=$1").bind(&r.upgrade_id).fetch_optional(&mut *c).await?
        .ok_or_else(||reject("UPGRADE_DB_ORIGINAL_RECEIPT_ABSENT"))?;
    let stored_added: sqlx::types::Json<Vec<String>> = row.try_get("added_catalog_entries")?;
    let delta: sqlx::types::Json<Value> = row.try_get("delta_components")?;
    if stored_added.0 != added
        || delta.0 != delta_components()
        || row.try_get::<String, _>("procedure")? != PROCEDURE
        || row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
        || hex(&row.try_get::<Vec<u8>, _>("predecessor_manifest_sha256")?) != PRE_MANIFEST
        || hex(&row.try_get::<Vec<u8>, _>("predecessor_catalog_sha256")?) != PRE_CATALOG
        || hex(&row.try_get::<Vec<u8>, _>("successor_manifest_sha256")?)
            != r.successor_manifest_sha256
        || row.try_get::<Vec<u8>, _>("successor_catalog_sha256")? != actual
        || row.try_get::<i32, _>("predecessor_components")? != PRE_COMPONENTS as i32
        || row.try_get::<i32, _>("successor_components")? != TARGET_COMPONENTS as i32
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
    let old = old_tables(&tables(c).await?);
    let expected_old_tables = PRE_TABLES - usize::from(PRE_COMPONENTS > 25);
    if old.len() != expected_old_tables
        || row.try_get::<Vec<u8>, _>("preserved_data_sha256")?
            != data_fingerprint(&preserved_data(c, &old, &r.upgrade_id).await?)
        || row.try_get::<Vec<u8>, _>("preserved_permissions_sha256")?
            != fingerprint(&permissions(c).await?)
    {
        return Err(reject("UPGRADE_DB_OLD_ROWS_OR_PERMISSIONS_CHANGED"));
    }
    Ok(UpgradeReport {
        version: 1,
        protocol: "core.upgrade-db.v1",
        status: "already_committed",
        upgrade_id: r.upgrade_id.clone(),
        request_sha256: hex(&r.digest()?),
        predecessor_catalog_sha256: PRE_CATALOG,
        successor_catalog_sha256: hex(&actual),
        components: TARGET_COMPONENTS,
    })
}

#[cfg(test)]
#[path = "additive_tests.rs"]
mod tests;
