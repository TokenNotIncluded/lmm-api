//! Exactly one explicit offline code-schema upgrade. Never called by startup.
//! A dedicated connection owns the session fence before any SERIALIZABLE snapshot.
use super::{COMPONENTS, UPGRADE_METADATA, definitions, fingerprint, schema_manifest};
use crate::identity::IdentityStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Row};
use std::{collections::BTreeMap, fmt, io::Read};
mod additive;
mod ledger;

const PROCEDURE: &str = "core-24-to-25-trust-pricing-v1";
const PREDECESSOR: &str = include_str!("predecessor24.json");
const PRE_CATALOG: &str = "e7de5a37f768dc20582b317673cf685f17ad315e1b3e2349977bb17023b82cfe";
const PRE_MANIFEST: &str = "9d7b93584b646b0d7f26a0a67e8570a213a22f06c9001838c29b21beb06722df";
const COMPONENT_25_DDL: &str = "93e0e16b04c312d9e1f27a0ae8dc25f16da4e7c044571cc14310b81de8cf1915";

// PG18 records NOT NULL constraints in pg_constraint, including the implicit
// primary-key nullability. These exact canonical pairs were observed on the
// reviewed receipt DDL; neither a name prefix nor another definition is allowed.
const UPGRADE_NOT_NULL: &[(&str, &str)] = &[
    ("schema_upgrades_upgrade_id_not_null", "NOT NULL upgrade_id"),
    ("schema_upgrades_procedure_not_null", "NOT NULL procedure"),
    (
        "schema_upgrades_request_sha256_not_null",
        "NOT NULL request_sha256",
    ),
    (
        "schema_upgrades_predecessor_manifest_sha256_not_null",
        "NOT NULL predecessor_manifest_sha256",
    ),
    (
        "schema_upgrades_predecessor_catalog_sha256_not_null",
        "NOT NULL predecessor_catalog_sha256",
    ),
    (
        "schema_upgrades_successor_manifest_sha256_not_null",
        "NOT NULL successor_manifest_sha256",
    ),
    (
        "schema_upgrades_successor_catalog_sha256_not_null",
        "NOT NULL successor_catalog_sha256",
    ),
    (
        "schema_upgrades_image_definition_changes_not_null",
        "NOT NULL image_definition_changes",
    ),
    (
        "schema_upgrades_database_system_identifier_not_null",
        "NOT NULL database_system_identifier",
    ),
    (
        "schema_upgrades_database_oid_not_null",
        "NOT NULL database_oid",
    ),
    (
        "schema_upgrades_database_name_not_null",
        "NOT NULL database_name",
    ),
    (
        "schema_upgrades_installer_role_not_null",
        "NOT NULL installer_role",
    ),
    ("schema_upgrades_owner_role_not_null", "NOT NULL owner_role"),
    (
        "schema_upgrades_runtime_role_not_null",
        "NOT NULL runtime_role",
    ),
    (
        "schema_upgrades_extensions_role_not_null",
        "NOT NULL extensions_role",
    ),
    (
        "schema_upgrades_recorded_at_not_null",
        "NOT NULL recorded_at",
    ),
];

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeDatabase {
    pub name: String,
    pub oid: u32,
    pub system_identifier: String,
    pub server_version_num: u32,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeRoles {
    pub installer: String,
    pub owner: String,
    pub runtime: String,
    pub extensions: String,
}
/// No secrets, source-archive claims, grants, or arbitrary SQL are accepted.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeRequest {
    pub version: u32,
    pub procedure: String,
    pub upgrade_id: String,
    pub database: UpgradeDatabase,
    pub roles: UpgradeRoles,
    pub predecessor_manifest_sha256: String,
    pub predecessor_catalog_sha256: String,
    pub successor_manifest_sha256: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImageDefinitionChange {
    before: String,
    after: String,
}

#[derive(Serialize)]
pub struct UpgradeReport {
    pub version: u32,
    pub protocol: &'static str,
    pub status: &'static str,
    pub upgrade_id: String,
    pub request_sha256: String,
    pub predecessor_catalog_sha256: &'static str,
    pub successor_catalog_sha256: String,
    pub components: usize,
}

#[derive(Debug)]
pub enum UpgradeError {
    Rejected(&'static str),
    Storage,
    /// The original connection is closed, never pooled or automatically retried.
    /// Reinvoke the same ID/request under the same fence: a committed receipt
    /// confirms success; only a proven intact predecessor can continue apply.
    CommitUnknown {
        upgrade_id: String,
        backend_pid: i32,
    },
}
impl fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rejected(code) => code,
            Self::Storage => "UPGRADE_DB_OBSERVATION_OR_SQL_FAILED",
            Self::CommitUnknown { .. } => "UPGRADE_DB_COMMIT_UNKNOWN_REUSE_ORIGINAL_ID",
        })
    }
}
impl std::error::Error for UpgradeError {}
impl From<sqlx::Error> for UpgradeError {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
impl From<crate::identity::IdentityError> for UpgradeError {
    fn from(_: crate::identity::IdentityError) -> Self {
        Self::Storage
    }
}
fn reject(code: &'static str) -> UpgradeError {
    UpgradeError::Rejected(code)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn digest_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.contains('\0')
        && !value.chars().any(char::is_control)
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn manifest_sha256() -> Result<String, UpgradeError> {
    let mut bytes = serde_json::to_vec(&schema_manifest()).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    Ok(hex(&Sha256::digest(bytes)))
}
impl UpgradeRequest {
    pub fn read(input: impl Read) -> Result<Self, UpgradeError> {
        let mut bytes = Vec::new();
        input
            .take(16_385)
            .read_to_end(&mut bytes)
            .map_err(|_| reject("UPGRADE_DB_INPUT_INVALID"))?;
        if bytes.len() > 16_384 {
            return Err(reject("UPGRADE_DB_INPUT_TOO_LARGE"));
        }
        let request: Self =
            serde_json::from_slice(&bytes).map_err(|_| reject("UPGRADE_DB_INPUT_INVALID"))?;
        request.validate()?;
        Ok(request)
    }
    fn validate(&self) -> Result<(), UpgradeError> {
        if self.procedure == ledger::PROCEDURE {
            return ledger::validate(self);
        }
        if self.procedure == additive::PROCEDURE {
            return additive::validate(self);
        }
        if self.version != 1
            || self.procedure != PROCEDURE
            || !(16..=96).contains(&self.upgrade_id.len())
            || !self
                .upgrade_id
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
            || !identifier(&self.database.name)
            || self.database.oid == 0
            || !(180_000..190_000).contains(&self.database.server_version_num)
            || self.database.system_identifier.is_empty()
            || self.database.system_identifier.len() > 20
            || !self
                .database
                .system_identifier
                .bytes()
                .all(|b| b.is_ascii_digit())
            || self
                .database
                .system_identifier
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0)
                .is_none()
            || self.predecessor_manifest_sha256 != PRE_MANIFEST
            || self.predecessor_catalog_sha256 != PRE_CATALOG
            || !digest_hex(&self.successor_manifest_sha256)
            || self.successor_manifest_sha256 != manifest_sha256()?
        {
            return Err(reject("UPGRADE_DB_TARGET_OR_CONTRACT_INVALID"));
        }
        let roles = [
            &self.roles.installer,
            &self.roles.owner,
            &self.roles.runtime,
            &self.roles.extensions,
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
        Ok(())
    }
    fn digest(&self) -> Result<Vec<u8>, UpgradeError> {
        // serde field order is fixed by the typed request; whitespace/key order is not authority.
        Ok(Sha256::digest(serde_json::to_vec(self).map_err(|_| UpgradeError::Storage)?).to_vec())
    }
}

fn compiled_predecessor() -> Result<BTreeMap<String, String>, UpgradeError> {
    // Reconstruct the original producer's struct/field order, not a JSON-map
    // reserialization: its exact stdout (including LF) was observed on e8b77.
    let mut predecessor_manifest = schema_manifest();
    predecessor_manifest.components.truncate(24);
    predecessor_manifest.installer_contract_sql_sha256 = hex(&Sha256::digest(super::CONTRACT));
    let mut producer_bytes =
        serde_json::to_vec(&predecessor_manifest).map_err(|_| UpgradeError::Storage)?;
    producer_bytes.push(b'\n');
    if hex(&Sha256::digest(producer_bytes)) != PRE_MANIFEST {
        return Err(reject("UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"));
    }
    let frozen: serde_json::Value =
        serde_json::from_str(PREDECESSOR).map_err(|_| UpgradeError::Storage)?;
    let items = frozen["manifest"]["components"]
        .as_array()
        .ok_or(UpgradeError::Storage)?;
    if items.len() != 24
        || COMPONENTS.len() != 25
        || frozen["schema_manifest_stdout_sha256"] != PRE_MANIFEST
        || frozen["catalog_sha256"] != PRE_CATALOG
        || frozen["manifest"]["catalog"]["query_sha256"] != hex(&Sha256::digest(super::CATALOG))
        || frozen["manifest"]["installer_contract_sql_sha256"]
            != hex(&Sha256::digest(super::CONTRACT))
    {
        return Err(reject("UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"));
    }
    let mut expected = BTreeMap::new();
    for (item, (name, ddl)) in items.iter().zip(COMPONENTS) {
        let digest = hex(&Sha256::digest(ddl));
        if item["name"] != *name || item["raw_schema_sha256"] != digest {
            return Err(reject("UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT"));
        }
        expected.insert((*name).to_owned(), digest);
    }
    if COMPONENTS[24].0 != "trust-pricing"
        || hex(&Sha256::digest(COMPONENTS[24].1)) != COMPONENT_25_DDL
    {
        return Err(reject("UPGRADE_DB_COMPONENT_ORDER_INVALID"));
    }
    Ok(expected)
}

/// Connect exactly once. Dropping this dedicated connection closes its session
/// and releases the advisory fence on every rejection/timeout/COMMIT outcome.
pub async fn upgrade_database(
    url: &str,
    request: UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    request.validate()?;
    let mut connection = PgConnection::connect(url).await?;
    let result = if request.procedure == ledger::PROCEDURE {
        ledger::connected(&mut connection, &request).await
    } else if request.procedure == additive::PROCEDURE {
        additive::connected(&mut connection, &request).await
    } else {
        upgrade_connected(&mut connection, &request).await
    };
    // No pool, reconnect, background retry, ALTER ROLE, or automatic privilege repair.
    let _ = connection.close().await;
    result
}
async fn begin_offline_upgrade<'a>(
    c: &'a mut PgConnection,
    r: &UpgradeRequest,
) -> Result<(sqlx::Transaction<'a, sqlx::Postgres>, i32, Vec<String>), UpgradeError> {
    sqlx::raw_sql("SET statement_timeout='15s'; SET lock_timeout='3s'; SET idle_in_transaction_session_timeout='20s'; SET search_path=pg_catalog")
        .execute(&mut *c).await?;
    // This SELECT is outside the SERIALIZABLE transaction. Waiting for another
    // maintenance owner can never freeze a stale transaction snapshot.
    sqlx::query("SELECT pg_catalog.pg_advisory_lock(741803120)")
        .execute(&mut *c)
        .await?;
    // The authenticated installer is NOINHERIT. Only its explicit SET path may
    // exercise the NOLOGIN owner, including the preconfigured control-system read.
    let session: String = sqlx::query_scalar("SELECT session_user::text")
        .fetch_one(&mut *c)
        .await?;
    if session != r.roles.installer {
        return Err(reject("UPGRADE_DB_INSTALLER_MISMATCH"));
    }
    sqlx::query(&format!("SET ROLE {}", quote(&r.roles.owner)))
        .execute(&mut *c)
        .await?;
    preflight(c, r).await?; // NOLOGIN is already committed before BEGIN.
    let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *c)
        .await?;
    let mut tx = c.begin().await?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SET LOCAL work_mem='4MB'; SET LOCAL max_parallel_workers_per_gather=0; SET LOCAL timezone='UTC'")
        .execute(&mut *tx)
        .await?;
    preflight(&mut tx, r).await?;
    let tables: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c
        JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
          AND c.relkind IN ('r','p') ORDER BY n.nspname COLLATE "C",c.relname COLLATE "C"
    "#,
    )
    .fetch_all(&mut *tx)
    .await?;
    for table in &tables {
        sqlx::query(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"))
            .execute(&mut *tx)
            .await?;
    }
    preflight(&mut tx, r).await?;
    Ok((tx, backend_pid, tables))
}

async fn upgrade_connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
    let receipt_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.schema_upgrades') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if receipt_exists {
        // Resolve the original upgrade ID before any DDL. A fresh25 database,
        // another ID, changed request, partial catalog, or absent receipt rejects.
        let result = resolve(&mut tx, r).await?;
        tx.rollback().await?;
        return Ok(result);
    }
    let before = definitions(&mut tx).await?;
    if hex(&fingerprint(&before)) != PRE_CATALOG || tables.len() != 149 {
        return Err(reject("UPGRADE_DB_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let sequences: i64 = sqlx::query_scalar(r#"SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S'"#)
        .fetch_one(&mut *tx).await?;
    if sequences != 21 {
        return Err(reject("UPGRADE_DB_PREDECESSOR_SEQUENCE_MISMATCH"));
    }
    let saved: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if hex(&saved) != PRE_CATALOG {
        return Err(reject("UPGRADE_DB_PREDECESSOR_RECEIPT_MISMATCH"));
    }
    check_predecessor_components(&mut tx).await?;
    IdentityStore::check_schema_connection(&mut tx).await?;
    let permissions = old_permissions(&mut tx).await?;
    let data = old_data(&mut tx, &tables).await?;
    sqlx::raw_sql(COMPONENTS[24].1).execute(&mut *tx).await?;
    sqlx::raw_sql(UPGRADE_METADATA).execute(&mut *tx).await?;
    sqlx::query(&format!(
        "GRANT USAGE ON SCHEMA core_trust_pricing TO {}",
        quote(&r.roles.runtime)
    ))
    .execute(&mut *tx)
    .await?;
    sqlx::query(&format!("GRANT SELECT ON core_trust_pricing.policies,core_trust_pricing.user_bases,core_trust_pricing.legacy_paid_orders,core_meta.schema_upgrades TO {}",quote(&r.roles.runtime)))
        .execute(&mut *tx).await?;
    check_new_privileges(&mut tx, r).await?;
    let after = definitions(&mut tx).await?;
    let image_changes = image_definition_changes(&before, &after)?;
    if without_additions(&after, &image_changes)? != before
        || old_permissions(&mut tx).await? != permissions
    {
        return Err(reject("UPGRADE_DB_OLD_DEFINITIONS_OR_PERMISSIONS_CHANGED"));
    }
    sqlx::query(
        "INSERT INTO core_meta.component_contract(name,fingerprint) VALUES ('trust-pricing',$1)",
    )
    .bind(Sha256::digest(COMPONENTS[24].1).to_vec())
    .execute(&mut *tx)
    .await?;
    let successor = fingerprint(&after);
    let changed = sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2")
        .bind(&successor).bind(&saved).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(reject("UPGRADE_DB_RUNTIME_RECEIPT_CONFLICT"));
    }
    sqlx::query(r#"INSERT INTO core_meta.schema_upgrades(upgrade_id,procedure,request_sha256,
        predecessor_manifest_sha256,predecessor_catalog_sha256,successor_manifest_sha256,successor_catalog_sha256,
        database_system_identifier,database_oid,database_name,installer_role,owner_role,runtime_role,extensions_role,image_definition_changes)
        VALUES ($1,$2,$3,decode($4,'hex'),decode($5,'hex'),decode($6,'hex'),$7,$8,$9::bigint::oid,$10,$11,$12,$13,$14,$15)"#)
        .bind(&r.upgrade_id).bind(PROCEDURE).bind(r.digest()?).bind(PRE_MANIFEST).bind(PRE_CATALOG)
        .bind(&r.successor_manifest_sha256).bind(&successor).bind(&r.database.system_identifier)
        .bind(i64::from(r.database.oid)).bind(&r.database.name).bind(&r.roles.installer)
        .bind(&r.roles.owner).bind(&r.roles.runtime).bind(&r.roles.extensions)
        .bind(sqlx::types::Json(&image_changes)).execute(&mut *tx).await?;
    super::check_connection(&mut tx).await?;
    if old_data(&mut tx, &tables).await? != data {
        return Err(reject("UPGRADE_DB_OLD_ROWS_OR_SEQUENCE_STATE_CHANGED"));
    }
    let mut report = resolve(&mut tx, r).await?;
    report.status = "committed";
    preflight(&mut tx, r).await?;
    // Even a server-side COMMIT error is handled conservatively as unknown. The
    // exact original ID is always resolved before any later DDL attempt.
    commit_offline_upgrade(tx, r, backend_pid).await?;
    Ok(report)
}

#[cfg(test)]
mod tests;

async fn commit_offline_upgrade(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    r: &UpgradeRequest,
    backend_pid: i32,
) -> Result<(), UpgradeError> {
    if !matches!(
        tokio::time::timeout(std::time::Duration::from_secs(20), tx.commit()).await,
        Ok(Ok(()))
    ) {
        return Err(UpgradeError::CommitUnknown {
            upgrade_id: r.upgrade_id.clone(),
            backend_pid,
        });
    }
    Ok(())
}

async fn preflight(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let row = sqlx::query(r#"
        SELECT current_database()::text AS name,d.oid::bigint AS oid,
          current_setting('server_version_num')::integer AS version,
          current_user::text AS owner,session_user::text AS installer,
          has_function_privilege(current_user,'pg_catalog.pg_control_system()','EXECUTE') AS control_read,
          NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid()) AS alone,
          NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database()) AS no_prepared,
          NOT EXISTS(SELECT FROM pg_event_trigger WHERE evtenabled<>'D') AS no_event_triggers
        FROM pg_database d WHERE d.datname=current_database()
    "#).fetch_one(&mut *c).await?;
    if row.try_get::<String, _>("name")? != r.database.name
        || row.try_get::<i64, _>("oid")? != i64::from(r.database.oid)
        || row.try_get::<i32, _>("version")? != r.database.server_version_num as i32
        || row.try_get::<String, _>("owner")? != r.roles.owner
        || row.try_get::<String, _>("installer")? != r.roles.installer
    {
        return Err(reject("UPGRADE_DB_INCARNATION_OR_ROLE_MISMATCH"));
    }
    if !row.try_get::<bool, _>("control_read")? {
        return Err(reject("UPGRADE_DB_OWNER_CONTROL_SYSTEM_READ_REQUIRED"));
    }
    if !row.try_get::<bool, _>("alone")? || !row.try_get::<bool, _>("no_prepared")? {
        return Err(reject("UPGRADE_DB_OTHER_CLIENT_OR_PREPARED_TRANSACTION"));
    }
    if !row.try_get::<bool, _>("no_event_triggers")? {
        return Err(reject("UPGRADE_DB_ENABLED_EVENT_TRIGGER"));
    }
    let system: String =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_catalog.pg_control_system()")
            .fetch_one(&mut *c)
            .await?;
    if system != r.database.system_identifier {
        return Err(reject("UPGRADE_DB_SYSTEM_IDENTIFIER_MISMATCH"));
    }
    let roles_ok: bool = sqlx::query_scalar(r#"
        SELECT count(*)=4 AND bool_and(NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
          AND bool_and(CASE WHEN rolname=$1 THEN rolcanlogin AND NOT rolinherit
                           WHEN rolname IN ($2,$3) THEN NOT rolcanlogin ELSE TRUE END)
          AND pg_has_role($1,$2,'SET') AND NOT pg_has_role($1,$2,'USAGE')
          AND NOT EXISTS(SELECT FROM pg_auth_members m JOIN pg_roles runtime ON runtime.oid=m.member WHERE runtime.rolname=$3)
          AND NOT pg_has_role($4,$2,'MEMBER')
          AND NOT has_database_privilege($4,current_database(),'CONNECT')
          AND NOT has_function_privilege($3,'pg_catalog.pg_control_system()','EXECUTE')
        FROM pg_roles WHERE rolname IN ($1,$2,$3,$4)
    "#).bind(&r.roles.installer).bind(&r.roles.owner).bind(&r.roles.runtime).bind(&r.roles.extensions)
        .fetch_one(&mut *c).await?;
    if !roles_ok {
        return Err(reject("UPGRADE_DB_OFFLINE_ROLE_CONTRACT_REQUIRED"));
    }
    let ownership_ok: bool = sqlx::query_scalar(r#"
        SELECT NOT EXISTS(SELECT FROM pg_namespace n WHERE n.nspname NOT IN ('public','pg_catalog','information_schema')
          AND left(n.nspname,3)<>'pg_' AND pg_get_userbyid(n.nspowner)<>current_user)
        AND NOT EXISTS(SELECT FROM pg_class t JOIN pg_namespace n ON n.oid=t.relnamespace
          WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
            AND pg_get_userbyid(t.relowner)<>current_user)
        AND NOT EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
          WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
            AND pg_get_userbyid(p.proowner)<>current_user)
        AND NOT EXISTS(SELECT FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace
          WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
            AND pg_get_userbyid(t.typowner)<>current_user)
    "#).fetch_one(&mut *c).await?;
    if !ownership_ok {
        return Err(reject("UPGRADE_DB_FOREIGN_OWNED_OBJECT"));
    }
    Ok(())
}
async fn check_predecessor_components(c: &mut PgConnection) -> Result<(), UpgradeError> {
    let expected = compiled_predecessor()?;
    let actual: BTreeMap<String, String> = sqlx::query(
        "SELECT name,encode(fingerprint,'hex') AS digest FROM core_meta.component_contract",
    )
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get("name")?, row.try_get("digest")?)))
    .collect::<Result<_, sqlx::Error>>()?;
    if actual != expected {
        return Err(reject("UPGRADE_DB_PREDECESSOR_COMPONENT_MISMATCH"));
    }
    Ok(())
}

// The complete predecessor fingerprint authenticates the original definitions.
// These identities are the only reviewed replacements, not a core_image ignore.
fn reviewed_image_key(text: &str) -> Result<Option<usize>, UpgradeError> {
    let row: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|_| UpgradeError::Storage)?;
    let field = |n: usize| row.get(n).and_then(|v| v.as_str()).unwrap_or("");
    let key = match (field(0), field(1), field(2)) {
        ("constraint", "core_image", "operations_price_check") => Some(0),
        ("function", "core_image", "check_charge_atomic") => Some(1),
        _ => None,
    };
    if key.is_some() && row.len() != 4 {
        return Err(reject("UPGRADE_DB_IMAGE_DEFINITION_SHAPE_INVALID"));
    }
    Ok(key)
}
fn image_definition_changes(
    before: &[String],
    after: &[String],
) -> Result<Vec<ImageDefinitionChange>, UpgradeError> {
    let mut changes = Vec::new();
    for key in 0..2 {
        let find = |definitions: &[String]| -> Result<String, UpgradeError> {
            let mut found = None;
            for text in definitions {
                if reviewed_image_key(text)? == Some(key) {
                    if found.is_some() {
                        return Err(reject("UPGRADE_DB_IMAGE_DEFINITION_AMBIGUOUS"));
                    }
                    found = Some(text.clone());
                }
            }
            found.ok_or_else(|| reject("UPGRADE_DB_IMAGE_DEFINITION_ABSENT"))
        };
        let old = find(before)?;
        let new = find(after)?;
        if old == new {
            return Err(reject("UPGRADE_DB_IMAGE_EXTENSION_NOT_APPLIED"));
        }
        changes.push(ImageDefinitionChange {
            before: old,
            after: new,
        });
    }
    Ok(changes)
}
fn validate_image_changes(changes: &[ImageDefinitionChange]) -> Result<(), UpgradeError> {
    if changes.len() != 2 {
        return Err(reject("UPGRADE_DB_IMAGE_RECEIPT_INVALID"));
    }
    for (key, change) in changes.iter().enumerate() {
        if reviewed_image_key(&change.before)? != Some(key)
            || reviewed_image_key(&change.after)? != Some(key)
            || change.before == change.after
        {
            return Err(reject("UPGRADE_DB_IMAGE_RECEIPT_INVALID"));
        }
    }
    Ok(())
}

/// Exclude only objects added by the two reviewed DDL constants. The FK creates
/// two RI triggers on OLD users; their exact stable properties remain catalogued
/// under core_identity. Exact image replacements are restored from the receipt;
/// no other old image definition, ownership, ACL or row is exempted.
fn without_additions(
    definitions: &[String],
    image_changes: &[ImageDefinitionChange],
) -> Result<Vec<String>, UpgradeError> {
    validate_image_changes(image_changes)?;
    let mut replaced = [0_u8; 2];
    let mut seen_not_null = [false; UPGRADE_NOT_NULL.len()];
    let mut old = Vec::new();
    let constraints = [
        "schema_upgrades_pkey",
        "schema_upgrades_procedure_check",
        "schema_upgrades_request_check",
        "schema_upgrades_pre_manifest_check",
        "schema_upgrades_pre_catalog_check",
        "schema_upgrades_post_manifest_check",
        "schema_upgrades_post_catalog_check",
        "schema_upgrades_image_changes_check",
    ];
    let mut users_fk_triggers = 0;
    for text in definitions {
        if let Some(key) = reviewed_image_key(text)? {
            let change = &image_changes[key];
            if text != &change.after || replaced[key] != 0 {
                return Err(reject("UPGRADE_DB_IMAGE_AFTER_DEFINITION_MISMATCH"));
            }
            replaced[key] = 1;
            old.push(change.before.clone());
            continue;
        }
        let row: Vec<serde_json::Value> =
            serde_json::from_str(text).map_err(|_| UpgradeError::Storage)?;
        let field = |n: usize| row.get(n).and_then(|v| v.as_str()).unwrap_or("");
        if field(0) == "constraint" && field(1) == "core_meta" {
            if let Some((index, (_, expected))) = UPGRADE_NOT_NULL
                .iter()
                .enumerate()
                .find(|(_, (name, _))| *name == field(2))
            {
                if row.len() != 4 || field(3) != *expected || seen_not_null[index] {
                    return Err(reject("UPGRADE_DB_METADATA_NOT_NULL_CONSTRAINT_INVALID"));
                }
                seen_not_null[index] = true;
                continue;
            }
        }
        if field(1) == "core_trust_pricing" {
            continue;
        }
        if field(1) == "core_meta"
            && match field(0) {
                "relation" | "column" => {
                    matches!(field(2), "schema_upgrades" | "schema_upgrades_pkey")
                }
                "constraint" => constraints.contains(&field(2)),
                "index" => field(2) == "schema_upgrades_pkey",
                _ => false,
            }
        {
            continue;
        }
        if field(0) == "internal-trigger"
            && field(1) == "core_identity"
            && field(2) == "users"
            && field(3) == "user_bases_user_id_fkey"
            && field(4) == "pg_catalog"
            && matches!(field(5), "RI_FKey_noaction_del" | "RI_FKey_noaction_upd")
        {
            // PG18 default NO ACTION FK triggers: row after-delete/update,
            // origin enabled, not deferrable/deferred, empty attrs/args/qual.
            let expected_type = if field(5) == "RI_FKey_noaction_del" {
                9
            } else {
                17
            };
            if row.get(6).and_then(|v| v.as_i64()) != Some(expected_type)
                || field(7) != "O"
                || row.get(8) != Some(&serde_json::Value::Bool(false))
                || row.get(9) != Some(&serde_json::Value::Bool(false))
                || field(10) != ""
                || field(11) != ""
                || row.get(12) != Some(&serde_json::Value::Null)
            {
                return Err(reject("UPGRADE_DB_UNEXPECTED_OLD_TABLE_TRIGGER"));
            }
            users_fk_triggers += 1;
            continue;
        }
        old.push(text.clone());
    }
    if replaced != [1, 1] {
        return Err(reject("UPGRADE_DB_IMAGE_AFTER_DEFINITION_ABSENT"));
    }
    if seen_not_null.iter().any(|seen| !seen) {
        return Err(reject("UPGRADE_DB_METADATA_NOT_NULL_CONSTRAINT_MISSING"));
    }
    if users_fk_triggers != 2 {
        return Err(reject("UPGRADE_DB_FOREIGN_KEY_ADDITION_MISMATCH"));
    }
    old.sort_unstable(); // Same UTF-8 byte order as PostgreSQL C collation.
    Ok(old)
}

/// Includes column ACL, owner/default ACL, database ACL and role membership.
/// Raw values stay in memory; output contains no role settings or row data.
async fn old_permissions(c: &mut PgConnection) -> Result<Vec<String>, UpgradeError> {
    Ok(sqlx::query_scalar(r#"
        WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema','core_trust_pricing') AND left(nspname,3)<>'pg_'),
        rels AS (SELECT c.*,n.nspname FROM pg_class c JOIN ns n ON n.oid=c.relnamespace WHERE NOT(n.nspname='core_meta' AND c.relname IN ('schema_upgrades','schema_upgrades_pkey'))),
        facts AS (
          SELECT json_build_array('schema',nspname,nspowner,nspacl)::text AS fact FROM ns
          UNION ALL SELECT json_build_array('relation',nspname,relname,relowner,relacl)::text FROM rels
          UNION ALL SELECT json_build_array('column',r.nspname,r.relname,a.attnum,a.attacl)::text FROM pg_attribute a JOIN rels r ON r.oid=a.attrelid WHERE a.attnum>0 AND NOT a.attisdropped
          UNION ALL SELECT json_build_array('function',n.nspname,p.oid::regprocedure::text,p.proowner,p.proacl)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
          UNION ALL SELECT json_build_array('type',n.nspname,t.typname,t.typowner,t.typacl)::text FROM pg_type t JOIN ns n ON n.oid=t.typnamespace WHERE NOT(n.nspname='core_meta' AND t.typname IN ('schema_upgrades','_schema_upgrades'))
          UNION ALL SELECT json_build_array('default',d.defaclrole,d.defaclnamespace,d.defaclobjtype,d.defaclacl)::text FROM pg_default_acl d
          UNION ALL SELECT json_build_array('database',datname,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
          UNION ALL SELECT json_build_array('role',oid,rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit,rolvaliduntil)::text FROM pg_roles
          UNION ALL SELECT json_build_array('membership',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members
        ) SELECT fact FROM facts ORDER BY fact COLLATE "C"
    "#).fetch_all(c).await?)
}

// Full old-row conservation. Sort fixed-size SHA256 values with multiplicity,
// not large user JSON. The 4MB work_mem bound permits a disk spill; the client
// only holds one 256-row batch of digests. No user row is emitted or retained.
async fn old_data(
    c: &mut PgConnection,
    tables: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, UpgradeError> {
    let mut result = BTreeMap::new();
    for table in tables {
        if matches!(
            table.as_str(),
            "core_meta.component_contract" | "core_meta.runtime_contract"
        ) {
            continue;
        }
        sqlx::query(&format!("DECLARE core_upgrade_preserved_rows NO SCROLL CURSOR FOR SELECT json_build_array(encode(digest,'hex'),count(*)::text)::text FROM (SELECT sha256(convert_to(to_jsonb(t)::text,'UTF8')) AS digest FROM {table} t) d GROUP BY digest ORDER BY digest"))
            .execute(&mut *c).await?;
        let mut hash = Sha256::new();
        loop {
            let rows: Vec<String> =
                sqlx::query_scalar("FETCH FORWARD 256 FROM core_upgrade_preserved_rows")
                    .fetch_all(&mut *c)
                    .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                hash.update((row.len() as u64).to_be_bytes());
                hash.update(row.as_bytes());
            }
        }
        sqlx::query("CLOSE core_upgrade_preserved_rows")
            .execute(&mut *c)
            .await?;
        result.insert(table.clone(), hash.finalize().to_vec());
    }
    let sequences:Vec<String> = sqlx::query_scalar(r#"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c
        JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema')
        AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname COLLATE "C",c.relname COLLATE "C""#)
        .fetch_all(&mut *c).await?;
    if sequences.len() != 21 {
        return Err(reject("UPGRADE_DB_SEQUENCE_SET_CHANGED"));
    }
    for sequence in sequences {
        let state: String = sqlx::query_scalar(&format!(
            "SELECT json_build_array(last_value,log_cnt,is_called)::text FROM {sequence}"
        ))
        .fetch_one(&mut *c)
        .await?;
        result.insert(
            format!("sequence:{sequence}"),
            Sha256::digest(state.as_bytes()).to_vec(),
        );
    }
    let sequence_definitions: Vec<String> = sqlx::query_scalar(
        r#"SELECT to_jsonb(s)::text FROM pg_sequence s
        JOIN pg_class c ON c.oid=s.seqrelid JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
        ORDER BY n.nspname COLLATE "C",c.relname COLLATE "C""#,
    )
    .fetch_all(&mut *c)
    .await?;
    result.insert(
        "sequence-definitions".into(),
        fingerprint(&sequence_definitions),
    );
    Ok(result)
}

async fn check_new_privileges(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<(), UpgradeError> {
    // Inspect explicit grants including default ACL fallout, PUBLIC and columns;
    // runtime has no role memberships, so its effective rights cannot hide in
    // another role. Owner remains the only principal with grant/write authority.
    let safe: bool = sqlx::query_scalar(r#"
        WITH targets AS (SELECT c.* FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname='core_trust_pricing' OR (n.nspname='core_meta' AND c.relname='schema_upgrades')),
        owner AS (SELECT oid FROM pg_roles WHERE rolname=$1), runtime AS (SELECT oid FROM pg_roles WHERE rolname=$2)
        SELECT (SELECT count(*)=4 FROM targets WHERE relkind='r')
          AND NOT EXISTS(SELECT FROM targets t CROSS JOIN LATERAL aclexplode(COALESCE(t.relacl,acldefault('r',t.relowner))) a
            WHERE a.grantee<>(SELECT oid FROM owner) AND NOT(a.grantee=(SELECT oid FROM runtime) AND a.privilege_type='SELECT' AND NOT a.is_grantable))
          AND NOT EXISTS(SELECT FROM targets t JOIN pg_attribute att ON att.attrelid=t.oid CROSS JOIN LATERAL aclexplode(att.attacl) a
            WHERE a.grantee<>(SELECT oid FROM owner))
          AND (SELECT count(*)=1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='core_trust_pricing')
          AND NOT EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
            CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a
            WHERE n.nspname='core_trust_pricing' AND a.grantee<>(SELECT oid FROM owner))
          AND NOT EXISTS(SELECT FROM pg_namespace n CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) a
            WHERE n.nspname='core_trust_pricing' AND a.grantee<>(SELECT oid FROM owner)
              AND NOT(a.grantee=(SELECT oid FROM runtime) AND a.privilege_type='USAGE' AND NOT a.is_grantable))
          AND has_schema_privilege($2,'core_trust_pricing','USAGE')
          AND NOT has_schema_privilege($2,'core_trust_pricing','CREATE')
          AND NOT EXISTS(SELECT FROM targets t WHERE t.relkind='r' AND
            (NOT has_table_privilege($2,t.oid,'SELECT') OR has_table_privilege($2,t.oid,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')))
    "#).bind(&r.roles.owner).bind(&r.roles.runtime).fetch_one(&mut *c).await?;
    if !safe {
        return Err(reject("UPGRADE_DB_NEW_OBJECT_PRIVILEGES_INVALID"));
    }
    Ok(())
}
async fn resolve(c: &mut PgConnection, r: &UpgradeRequest) -> Result<UpgradeReport, UpgradeError> {
    super::check_connection(c).await?;
    let live = definitions(c).await?;
    let actual = fingerprint(&live);
    check_new_privileges(c, r).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_meta.schema_upgrades")
        .fetch_one(&mut *c)
        .await?;
    let row = sqlx::query(r#"SELECT request_sha256,procedure,predecessor_manifest_sha256,predecessor_catalog_sha256,
        successor_manifest_sha256,successor_catalog_sha256,database_system_identifier,database_oid::bigint AS database_oid,
        database_name,installer_role,owner_role,runtime_role,extensions_role,image_definition_changes
        FROM core_meta.schema_upgrades WHERE upgrade_id=$1"#).bind(&r.upgrade_id).fetch_optional(&mut *c).await?;
    let row = row.ok_or_else(|| reject("UPGRADE_DB_ORIGINAL_RECEIPT_ABSENT"))?;
    if count != 1
        || row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
        || row.try_get::<String, _>("procedure")? != PROCEDURE
        || hex(&row.try_get::<Vec<u8>, _>("predecessor_manifest_sha256")?) != PRE_MANIFEST
        || hex(&row.try_get::<Vec<u8>, _>("predecessor_catalog_sha256")?) != PRE_CATALOG
        || hex(&row.try_get::<Vec<u8>, _>("successor_manifest_sha256")?)
            != r.successor_manifest_sha256
        || row.try_get::<Vec<u8>, _>("successor_catalog_sha256")? != actual
        || row.try_get::<String, _>("database_system_identifier")? != r.database.system_identifier
        || row.try_get::<i64, _>("database_oid")? != i64::from(r.database.oid)
        || row.try_get::<String, _>("database_name")? != r.database.name
        || row.try_get::<String, _>("installer_role")? != r.roles.installer
        || row.try_get::<String, _>("owner_role")? != r.roles.owner
        || row.try_get::<String, _>("runtime_role")? != r.roles.runtime
        || row.try_get::<String, _>("extensions_role")? != r.roles.extensions
    {
        return Err(reject("UPGRADE_DB_ORIGINAL_RECEIPT_MISMATCH"));
    }
    let changes: sqlx::types::Json<Vec<ImageDefinitionChange>> =
        row.try_get("image_definition_changes")?;
    if hex(&fingerprint(&without_additions(&live, &changes.0)?)) != PRE_CATALOG {
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
        components: 25,
    })
}
