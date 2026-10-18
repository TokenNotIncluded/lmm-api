//! Offline activation of one imported privileged account.
//!
//! Plan is read-only and never grants authority. Apply requires an explicit
//! human-reviewed confirmation equal to the candidate binding hash, an owner
//! database session, and persists a complete nine-action policy in the same
//! transaction as activation. Runtime credentials have no approval authority.

use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Row};
use std::io::Read;

use super::permissions::{
    self, CONTRACT_VERSION, PlatformRole, PolicySnapshot, PolicySnapshotError,
    complete_policy_snapshot, policy_sha256,
};

const DOMAIN: &[u8] = b"core.admin-approval.v1\0";
const MAX_STDIN_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenSource {
    pub archive_sha256: [u8; 32],
    pub a2_semantic_sha256: [u8; 32],
    pub a2_file_sha256: [u8; 32],
    /// SHA-256 of the pre-seal base import SQL body, not the sealed file.
    pub a2_sql_sha256: [u8; 32],
    pub migration_id: String,
    pub user_id: i64,
    pub personal_account_id: i64,
    pub enabled: bool,
    pub deleted: bool,
    pub password_verifier_recognized: bool,
    /// Source-permitted password flag from the archive, not the live dormant gate.
    pub password_login_enabled: bool,
    pub password_hash_sha256: [u8; 32],
    pub login_name_sha256: [u8; 32],
    pub second_factor_required: bool,
    pub role: PlatformRole,
    pub auth_version: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetInstance {
    pub system_id: [u8; 32],
    pub database_oid: i64,
    pub instance_nonce: [u8; 32],
    pub import_run: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyBinding {
    pub revision: i64,
    pub sha256: [u8; 32],
    pub verified_complete: bool,
}

/// One human-selected approval. Fields are the whole payload; there is no
/// extra map that could add a second user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approval {
    pub id: [u8; 32],
    pub approver: [u8; 32],
    pub expiry_unix: i64,
    pub source: FrozenSource,
    pub target: TargetInstance,
    pub policy: PolicyBinding,
    pub policy_snapshot: PolicySnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedAccount {
    pub source: FrozenSource,
    pub target: TargetInstance,
    pub policy: PolicyBinding,
    pub current_auth_version: i64,
    pub active: bool,
    pub now_unix: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredReceipt {
    pub approval_id: [u8; 32],
    pub payload_sha256: [u8; 32],
    pub user_id: i64,
    pub expected_auth_version: i64,
    pub applied: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Activation {
    /// First successful admission. Role and MFA stay as sourced; auth_version
    /// advances by exactly one.
    Apply {
        user_id: i64,
        personal_account_id: i64,
        role: PlatformRole,
        second_factor_required: bool,
        password_login_enabled: bool,
        expected_auth_version: i64,
        next_auth_version: i64,
        approval_id: [u8; 32],
        payload_sha256: [u8; 32],
        policy_revision: i64,
        policy_sha256: [u8; 32],
    },
    /// Same approval id and same payload already applied. Do not write again.
    Replay {
        user_id: i64,
        approval_id: [u8; 32],
        payload_sha256: [u8; 32],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalError {
    Batch,
    Incomplete,
    Expired,
    SourceBlocked,
    SourceMismatch,
    TargetMismatch,
    PolicyMismatch,
    VersionConflict,
    PayloadConflict,
    CasConflict,
    AlreadyActive,
    NotOwner,
    ConfirmationMismatch,
    InputTooLarge,
    InvalidJson,
    Storage,
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Batch => "batch activation is refused",
            Self::Incomplete => "approval binding is incomplete",
            Self::Expired => "approval expired",
            Self::SourceBlocked => "source account is blocked",
            Self::SourceMismatch => "source facts mismatch",
            Self::TargetMismatch => "target instance mismatch",
            Self::PolicyMismatch => "permission policy mismatch",
            Self::VersionConflict => "auth_version conflict",
            Self::PayloadConflict => "approval payload conflict",
            Self::CasConflict => "competing approval conflict",
            Self::AlreadyActive => "account already active",
            Self::NotOwner => "owner database session required",
            Self::ConfirmationMismatch => "confirmation does not match candidate",
            Self::InputTooLarge => "stdin JSON exceeds 64 KiB",
            Self::InvalidJson => "invalid activation JSON",
            Self::Storage => "activation storage unavailable",
        })
    }
}
impl std::error::Error for ApprovalError {}

fn nonzero(hash: [u8; 32]) -> bool {
    hash != [0; 32]
}

fn feed_hash(hasher: &mut Sha256, hash: [u8; 32]) {
    hasher.update(hash);
}

fn feed_i64(hasher: &mut Sha256, value: i64) {
    hasher.update(value.to_be_bytes());
}

fn feed_bool(hasher: &mut Sha256, value: bool) {
    hasher.update([u8::from(value)]);
}

fn feed_str(hasher: &mut Sha256, value: &str) {
    feed_i64(hasher, value.len() as i64);
    hasher.update(value.as_bytes());
}

fn role_byte(role: PlatformRole) -> u8 {
    match role {
        PlatformRole::User => 1,
        PlatformRole::Admin => 10,
        PlatformRole::Root => 100,
    }
}

/// @mark identity.admin-approval role=authority contract=core.admin-approval.v1
pub fn payload_sha256(approval: &Approval) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(CONTRACT_VERSION.as_bytes());
    feed_hash(&mut hasher, approval.id);
    feed_hash(&mut hasher, approval.approver);
    feed_i64(&mut hasher, approval.expiry_unix);
    let source = &approval.source;
    feed_hash(&mut hasher, source.archive_sha256);
    feed_hash(&mut hasher, source.a2_semantic_sha256);
    feed_hash(&mut hasher, source.a2_file_sha256);
    feed_hash(&mut hasher, source.a2_sql_sha256);
    feed_str(&mut hasher, &source.migration_id);
    feed_i64(&mut hasher, source.user_id);
    feed_i64(&mut hasher, source.personal_account_id);
    feed_bool(&mut hasher, source.enabled);
    feed_bool(&mut hasher, source.deleted);
    feed_bool(&mut hasher, source.password_verifier_recognized);
    feed_bool(&mut hasher, source.password_login_enabled);
    feed_hash(&mut hasher, source.password_hash_sha256);
    feed_hash(&mut hasher, source.login_name_sha256);
    feed_bool(&mut hasher, source.second_factor_required);
    hasher.update([role_byte(source.role)]);
    feed_i64(&mut hasher, source.auth_version);
    let target = &approval.target;
    feed_hash(&mut hasher, target.system_id);
    feed_i64(&mut hasher, target.database_oid);
    feed_hash(&mut hasher, target.instance_nonce);
    feed_hash(&mut hasher, target.import_run);
    feed_i64(&mut hasher, approval.policy.revision);
    feed_hash(&mut hasher, approval.policy.sha256);
    feed_bool(&mut hasher, approval.policy.verified_complete);
    hasher.finalize().into()
}

fn migration_id_ok(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn complete(approval: &Approval) -> bool {
    let source = &approval.source;
    nonzero(approval.id)
        && nonzero(approval.approver)
        && approval.expiry_unix > 0
        && migration_id_ok(&source.migration_id)
        && source.user_id > 0
        && source.personal_account_id > 0
        && source.auth_version > 0
        && nonzero(source.archive_sha256)
        && nonzero(source.a2_semantic_sha256)
        && nonzero(source.a2_file_sha256)
        && nonzero(source.a2_sql_sha256)
        && nonzero(source.password_hash_sha256)
        && nonzero(source.login_name_sha256)
        && approval.target.database_oid > 0
        && nonzero(approval.target.system_id)
        && nonzero(approval.target.instance_nonce)
        && nonzero(approval.target.import_run)
        && approval.policy.revision > 0
        && approval.policy.verified_complete
        && nonzero(approval.policy.sha256)
        && matches!(source.role, PlatformRole::Admin | PlatformRole::Root)
        && approval.policy_snapshot.user_id == source.user_id
        && approval.policy_snapshot.revision == approval.policy.revision
        && policy_sha256(&approval.policy_snapshot).ok() == Some(approval.policy.sha256)
}

/// Admit exactly one approval. `occupied` is a different approval that already
/// claimed this user at the same expected `auth_version`.
pub fn plan_one(
    approval: &Approval,
    observed: &ObservedAccount,
    prior: Option<&StoredReceipt>,
    occupied: Option<&StoredReceipt>,
) -> Result<Activation, ApprovalError> {
    if !complete(approval) {
        return Err(ApprovalError::Incomplete);
    }
    let payload = payload_sha256(approval);
    if let Some(prior) = prior {
        if prior.approval_id != approval.id || prior.user_id != approval.source.user_id {
            return Err(ApprovalError::PayloadConflict);
        }
        if prior.payload_sha256 != payload {
            return Err(ApprovalError::PayloadConflict);
        }
        if prior.applied && prior.expected_auth_version == approval.source.auth_version {
            return Ok(Activation::Replay {
                user_id: prior.user_id,
                approval_id: prior.approval_id,
                payload_sha256: payload,
            });
        }
        return Err(ApprovalError::PayloadConflict);
    }
    if observed.now_unix >= approval.expiry_unix {
        return Err(ApprovalError::Expired);
    }
    let mut source = observed.source.clone();
    source.auth_version = approval.source.auth_version;
    if source != approval.source {
        return Err(ApprovalError::SourceMismatch);
    }
    if !approval.source.enabled
        || approval.source.deleted
        || !approval.source.password_verifier_recognized
        || !approval.source.password_login_enabled
    {
        return Err(ApprovalError::SourceBlocked);
    }
    if observed.target != approval.target {
        return Err(ApprovalError::TargetMismatch);
    }
    if observed.policy != approval.policy {
        return Err(ApprovalError::PolicyMismatch);
    }
    if let Some(other) = occupied {
        if other.applied
            && other.user_id == approval.source.user_id
            && other.expected_auth_version == approval.source.auth_version
            && other.approval_id != approval.id
        {
            return Err(ApprovalError::CasConflict);
        }
    }
    if observed.current_auth_version != approval.source.auth_version {
        return Err(ApprovalError::VersionConflict);
    }
    if observed.active {
        return Err(ApprovalError::AlreadyActive);
    }
    let next = approval
        .source
        .auth_version
        .checked_add(1)
        .ok_or(ApprovalError::VersionConflict)?;
    Ok(Activation::Apply {
        user_id: approval.source.user_id,
        personal_account_id: approval.source.personal_account_id,
        role: approval.source.role,
        second_factor_required: approval.source.second_factor_required,
        password_login_enabled: approval.source.password_login_enabled,
        expected_auth_version: approval.source.auth_version,
        next_auth_version: next,
        approval_id: approval.id,
        payload_sha256: payload,
        policy_revision: approval.policy.revision,
        policy_sha256: approval.policy.sha256,
    })
}

pub fn plan_batch(
    approvals: &[Approval],
    observed: &ObservedAccount,
    prior: Option<&StoredReceipt>,
    occupied: Option<&StoredReceipt>,
) -> Result<Activation, ApprovalError> {
    let [approval] = approvals else {
        return Err(ApprovalError::Batch);
    };
    plan_one(approval, observed, prior, occupied)
}

fn db(error: sqlx::Error) -> ApprovalError {
    if let Some(database) = error.as_database_error()
        && matches!(
            database.code().as_deref(),
            Some("23505" | "40001" | "40P01")
        )
    {
        return ApprovalError::CasConflict;
    }
    ApprovalError::Storage
}

fn array32(value: Vec<u8>) -> Result<[u8; 32], ApprovalError> {
    value.try_into().map_err(|_| ApprovalError::Incomplete)
}

pub fn system_id(identifier: i64) -> [u8; 32] {
    let mut out = [0; 32];
    out[..8].copy_from_slice(&identifier.to_be_bytes());
    out
}

pub fn import_run_digest(migration_id: &str) -> [u8; 32] {
    Sha256::digest(migration_id.as_bytes()).into()
}

fn decode_hex_32(text: &str) -> Result<[u8; 32], ApprovalError> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApprovalError::InvalidJson);
    }
    let mut out = [0; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| ApprovalError::InvalidJson)?;
    }
    Ok(out)
}

fn encode_hex_32(value: &[u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn role_name(role: PlatformRole) -> &'static str {
    match role {
        PlatformRole::User => "user",
        PlatformRole::Admin => "admin",
        PlatformRole::Root => "superadmin",
    }
}

fn parse_role(value: &str) -> Result<PlatformRole, ApprovalError> {
    match value {
        "user" => Ok(PlatformRole::User),
        "admin" => Ok(PlatformRole::Admin),
        "superadmin" => Ok(PlatformRole::Root),
        _ => Err(ApprovalError::SourceMismatch),
    }
}

async fn require_relation_owner(connection: &mut PgConnection) -> Result<(), ApprovalError> {
    let owner: bool = sqlx::query_scalar("SELECT core_identity.is_identity_relation_owner()")
        .fetch_one(&mut *connection)
        .await
        .map_err(db)?;
    if owner {
        Ok(())
    } else {
        Err(ApprovalError::NotOwner)
    }
}

fn digest_text(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn snapshot_from_binding(
    user_id: i64,
    revision: i64,
    entries: &[(String, String)],
) -> Result<(PolicySnapshot, PolicyBinding), ApprovalError> {
    let snapshot =
        complete_policy_snapshot(user_id, revision, entries).map_err(|error| match error {
            PolicySnapshotError::UnknownAction | PolicySnapshotError::UnknownEffect => {
                ApprovalError::PolicyMismatch
            }
            PolicySnapshotError::UnknownField => ApprovalError::InvalidJson,
            _ => ApprovalError::Incomplete,
        })?;
    let sha = policy_sha256(&snapshot).map_err(|_| ApprovalError::Incomplete)?;
    Ok((
        snapshot,
        PolicyBinding {
            revision,
            sha256: sha,
            verified_complete: true,
        },
    ))
}

async fn load_frozen_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    lock: bool,
    require_dormant: bool,
) -> Result<(FrozenSource, TargetInstance, bool, i64, bool), ApprovalError> {
    let lock_clause = if lock { " FOR UPDATE OF u,a,p" } else { "" };
    let user = sqlx::query(&format!(
        "SELECT u.personal_account_id,u.platform_role,u.active,u.auth_version,a.active AS account_active,
         a.kind,p.second_factor_required,p.password_login_allowed,p.legacy_bcrypt_allowed,p.email_login_allowed,
         l.login_name,l.password_hash,
         (SELECT oid::bigint FROM pg_database WHERE datname=current_database()) AS database_oid,
         (SELECT system_identifier FROM pg_control_system()) AS system_identifier,
         current_setting('lmm.migration_instance_nonce', true) AS instance_nonce
         FROM core_identity.users u
         JOIN core_identity.accounts a ON a.id=u.personal_account_id
         JOIN core_identity.user_profiles p ON p.user_id=u.id
         LEFT JOIN core_identity.password_logins l ON l.user_id=u.id
         WHERE u.id=$1{lock_clause}",
    ))
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?
    .ok_or(ApprovalError::SourceMismatch)?;
    let account_id: i64 = user
        .try_get("personal_account_id")
        .map_err(|_| ApprovalError::Storage)?;
    let account_active: bool = user
        .try_get("account_active")
        .map_err(|_| ApprovalError::Storage)?;
    let kind: String = user.try_get("kind").map_err(|_| ApprovalError::Storage)?;
    if kind != "personal" {
        return Err(ApprovalError::SourceMismatch);
    }
    let role = parse_role(
        &user
            .try_get::<String, _>("platform_role")
            .map_err(|_| ApprovalError::Storage)?,
    )?;
    let active: bool = user.try_get("active").map_err(|_| ApprovalError::Storage)?;
    let auth_version: i64 = user
        .try_get("auth_version")
        .map_err(|_| ApprovalError::Storage)?;
    let second_factor: bool = user
        .try_get("second_factor_required")
        .map_err(|_| ApprovalError::Storage)?;
    let live_password_gate: bool = user
        .try_get("password_login_allowed")
        .map_err(|_| ApprovalError::Storage)?;
    let live_legacy_bcrypt: bool = user
        .try_get("legacy_bcrypt_allowed")
        .map_err(|_| ApprovalError::Storage)?;
    let live_email_login: bool = user
        .try_get("email_login_allowed")
        .map_err(|_| ApprovalError::Storage)?;
    let login_name: Option<String> = user
        .try_get("login_name")
        .map_err(|_| ApprovalError::Storage)?;
    let password_hash: Option<String> = user
        .try_get("password_hash")
        .map_err(|_| ApprovalError::Storage)?;
    let database_oid: i64 = user
        .try_get("database_oid")
        .map_err(|_| ApprovalError::Storage)?;
    let system_identifier: i64 = user
        .try_get("system_identifier")
        .map_err(|_| ApprovalError::Storage)?;
    let nonce_text: Option<String> = user
        .try_get("instance_nonce")
        .map_err(|_| ApprovalError::Storage)?;
    let instance_nonce = decode_hex_32(nonce_text.as_deref().unwrap_or(""))
        .map_err(|_| ApprovalError::TargetMismatch)?;
    let live_system = system_id(system_identifier);
    if lock {
        sqlx::query("SELECT user_id FROM core_identity.password_logins WHERE user_id=$1 FOR SHARE")
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    }
    let facts_lock = if lock { " FOR SHARE" } else { "" };
    let facts = sqlx::query(&format!(
        "SELECT archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,
         database_oid,instance_nonce,import_run,source_enabled,source_deleted,
         password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,
         source_role,source_second_factor_required,source_auth_version
         FROM core_identity.privileged_import_facts WHERE user_id=$1{facts_lock}",
    ))
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?
    .ok_or(ApprovalError::SourceMismatch)?;
    let stored_system = array32(
        facts
            .try_get("system_id")
            .map_err(|_| ApprovalError::Storage)?,
    )?;
    let stored_oid: i64 = facts
        .try_get("database_oid")
        .map_err(|_| ApprovalError::Storage)?;
    let stored_nonce = array32(
        facts
            .try_get("instance_nonce")
            .map_err(|_| ApprovalError::Storage)?,
    )?;
    if stored_system != live_system || stored_oid != database_oid || stored_nonce != instance_nonce
    {
        return Err(ApprovalError::TargetMismatch);
    }
    let migration_id: String = facts
        .try_get("migration_id")
        .map_err(|_| ApprovalError::Storage)?;
    let fact_role = parse_role(
        &facts
            .try_get::<String, _>("source_role")
            .map_err(|_| ApprovalError::Storage)?,
    )?;
    let fact_mfa: bool = facts
        .try_get("source_second_factor_required")
        .map_err(|_| ApprovalError::Storage)?;
    let fact_auth: i64 = facts
        .try_get("source_auth_version")
        .map_err(|_| ApprovalError::Storage)?;
    let recognized: bool = facts
        .try_get("password_verifier_recognized")
        .map_err(|_| ApprovalError::Storage)?;
    let fact_login: bool = facts
        .try_get("password_login_enabled")
        .map_err(|_| ApprovalError::Storage)?;
    let fact_password_hash = array32(
        facts
            .try_get("password_hash_sha256")
            .map_err(|_| ApprovalError::Storage)?,
    )?;
    let fact_login_name = array32(
        facts
            .try_get("login_name_sha256")
            .map_err(|_| ApprovalError::Storage)?,
    )?;
    let expected_legacy_bcrypt = password_hash.as_deref().is_some_and(|value| {
        value.starts_with("$2a$") || value.starts_with("$2b$") || value.starts_with("$2y$")
    });
    if require_dormant && (live_email_login || live_legacy_bcrypt != expected_legacy_bcrypt) {
        return Err(ApprovalError::SourceMismatch);
    }
    let live_password_hash = password_hash.as_deref().map(digest_text);
    let live_login_name = login_name.as_deref().map(digest_text);
    if require_dormant
        && (role != fact_role || second_factor != fact_mfa || auth_version != fact_auth)
    {
        return Err(ApprovalError::SourceMismatch);
    }
    if require_dormant && recognized {
        match (live_password_hash, live_login_name) {
            (Some(hash), Some(login)) if hash == fact_password_hash && login == fact_login_name => {
            }
            _ => return Err(ApprovalError::SourceMismatch),
        }
    } else if require_dormant && !recognized && live_password_hash.is_some() {
        return Err(ApprovalError::SourceMismatch);
    }
    // Live dormant gate must remain closed until apply; source flags are separate.
    if require_dormant && (active || account_active || live_password_gate) {
        return Err(ApprovalError::SourceMismatch);
    }
    let source = FrozenSource {
        archive_sha256: array32(
            facts
                .try_get("archive_sha256")
                .map_err(|_| ApprovalError::Storage)?,
        )?,
        a2_semantic_sha256: array32(
            facts
                .try_get("a2_semantic_sha256")
                .map_err(|_| ApprovalError::Storage)?,
        )?,
        a2_file_sha256: array32(
            facts
                .try_get("a2_file_sha256")
                .map_err(|_| ApprovalError::Storage)?,
        )?,
        a2_sql_sha256: array32(
            facts
                .try_get("a2_sql_sha256")
                .map_err(|_| ApprovalError::Storage)?,
        )?,
        migration_id: migration_id.clone(),
        user_id,
        personal_account_id: account_id,
        enabled: facts
            .try_get("source_enabled")
            .map_err(|_| ApprovalError::Storage)?,
        deleted: facts
            .try_get("source_deleted")
            .map_err(|_| ApprovalError::Storage)?,
        password_verifier_recognized: recognized,
        password_login_enabled: fact_login,
        password_hash_sha256: fact_password_hash,
        login_name_sha256: fact_login_name,
        second_factor_required: fact_mfa,
        role: fact_role,
        auth_version: fact_auth,
    };
    let target = TargetInstance {
        system_id: live_system,
        database_oid,
        instance_nonce,
        import_run: array32(
            facts
                .try_get("import_run")
                .map_err(|_| ApprovalError::Storage)?,
        )?,
    };
    if target.import_run != import_run_digest(&migration_id) {
        return Err(ApprovalError::TargetMismatch);
    }
    let registry = sqlx::query("SELECT source_archive_sha256,mapping_sha256,manifest FROM core_migration.import_runs WHERE migration_id=$1")
        .bind(&migration_id).fetch_optional(&mut **tx).await.map_err(db)?
        .ok_or(ApprovalError::SourceMismatch)?;
    let archive: String = registry
        .try_get("source_archive_sha256")
        .map_err(|_| ApprovalError::Storage)?;
    let mapping: String = registry
        .try_get("mapping_sha256")
        .map_err(|_| ApprovalError::Storage)?;
    let manifest: serde_json::Value = registry
        .try_get("manifest")
        .map_err(|_| ApprovalError::Storage)?;
    if archive != encode_hex_32(&source.archive_sha256)
        || mapping != encode_hex_32(&source.a2_semantic_sha256)
        || manifest
            .get("mapping_file_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(encode_hex_32(&source.a2_file_sha256).as_str())
    {
        return Err(ApprovalError::SourceMismatch);
    }
    Ok((source, target, active, auth_version, second_factor))
}

fn deserialize_policy<'de, D>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct PolicyVisitor;
    impl<'de> serde::de::Visitor<'de> for PolicyVisitor {
        type Value = std::collections::BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object with unique policy actions")
        }
        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: serde::de::MapAccess<'de>,
        {
            let mut policy = std::collections::BTreeMap::new();
            while let Some((action, effect)) = map.next_entry::<String, String>()? {
                if policy.insert(action, effect).is_some() {
                    return Err(serde::de::Error::custom("duplicate policy action"));
                }
            }
            Ok(policy)
        }
    }
    deserializer.deserialize_map(PolicyVisitor)
}

/// Public plan request: one account, complete policy, explicit approval identity.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanAdminActivationRequest {
    pub approval_id: String,
    pub approver: String,
    pub expiry_unix: i64,
    pub user_id: i64,
    pub policy_revision: i64,
    #[serde(deserialize_with = "deserialize_policy")]
    pub policy: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PlanAdminActivationResponse {
    pub status: &'static str,
    pub confirmation_sha256: String,
    pub candidate: ApprovalDto,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDto {
    pub approval_id: String,
    pub approver: String,
    pub expiry_unix: i64,
    pub migration_id: String,
    pub user_id: i64,
    pub personal_account_id: i64,
    pub archive_sha256: String,
    pub a2_semantic_sha256: String,
    pub a2_file_sha256: String,
    pub a2_sql_sha256: String,
    pub system_id: String,
    pub database_oid: i64,
    pub instance_nonce: String,
    pub import_run: String,
    pub source_enabled: bool,
    pub source_deleted: bool,
    pub password_verifier_recognized: bool,
    pub password_login_enabled: bool,
    pub password_hash_sha256: String,
    pub login_name_sha256: String,
    pub second_factor_required: bool,
    pub role: String,
    pub auth_version: i64,
    pub policy_revision: i64,
    pub policy_sha256: String,
    #[serde(deserialize_with = "deserialize_policy")]
    pub policy: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyAdminActivationRequest {
    pub confirmation_sha256: String,
    pub approval: ApprovalDto,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ApplyAdminActivationResponse {
    pub status: &'static str,
    pub user_id: i64,
    pub approval_id: String,
    pub payload_sha256: String,
    pub auth_version: Option<i64>,
}

fn policy_map(snapshot: &PolicySnapshot) -> std::collections::BTreeMap<String, String> {
    snapshot
        .decisions
        .iter()
        .map(|row| {
            (
                row.action.catalog_name().unwrap_or("Unknown").to_owned(),
                row.effect.catalog_name().to_owned(),
            )
        })
        .collect()
}

fn approval_to_dto(approval: &Approval) -> ApprovalDto {
    ApprovalDto {
        approval_id: encode_hex_32(&approval.id),
        approver: encode_hex_32(&approval.approver),
        expiry_unix: approval.expiry_unix,
        migration_id: approval.source.migration_id.clone(),
        user_id: approval.source.user_id,
        personal_account_id: approval.source.personal_account_id,
        archive_sha256: encode_hex_32(&approval.source.archive_sha256),
        a2_semantic_sha256: encode_hex_32(&approval.source.a2_semantic_sha256),
        a2_file_sha256: encode_hex_32(&approval.source.a2_file_sha256),
        a2_sql_sha256: encode_hex_32(&approval.source.a2_sql_sha256),
        system_id: encode_hex_32(&approval.target.system_id),
        database_oid: approval.target.database_oid,
        instance_nonce: encode_hex_32(&approval.target.instance_nonce),
        import_run: encode_hex_32(&approval.target.import_run),
        source_enabled: approval.source.enabled,
        source_deleted: approval.source.deleted,
        password_verifier_recognized: approval.source.password_verifier_recognized,
        password_login_enabled: approval.source.password_login_enabled,
        password_hash_sha256: encode_hex_32(&approval.source.password_hash_sha256),
        login_name_sha256: encode_hex_32(&approval.source.login_name_sha256),
        second_factor_required: approval.source.second_factor_required,
        role: role_name(approval.source.role).to_owned(),
        auth_version: approval.source.auth_version,
        policy_revision: approval.policy.revision,
        policy_sha256: encode_hex_32(&approval.policy.sha256),
        policy: policy_map(&approval.policy_snapshot),
    }
}

fn dto_to_approval(dto: &ApprovalDto) -> Result<Approval, ApprovalError> {
    let entries: Vec<(String, String)> = dto
        .policy
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let (snapshot, binding) = snapshot_from_binding(dto.user_id, dto.policy_revision, &entries)?;
    if encode_hex_32(&binding.sha256) != dto.policy_sha256 {
        return Err(ApprovalError::PolicyMismatch);
    }
    Ok(Approval {
        id: decode_hex_32(&dto.approval_id)?,
        approver: decode_hex_32(&dto.approver)?,
        expiry_unix: dto.expiry_unix,
        source: FrozenSource {
            archive_sha256: decode_hex_32(&dto.archive_sha256)?,
            a2_semantic_sha256: decode_hex_32(&dto.a2_semantic_sha256)?,
            a2_file_sha256: decode_hex_32(&dto.a2_file_sha256)?,
            a2_sql_sha256: decode_hex_32(&dto.a2_sql_sha256)?,
            migration_id: dto.migration_id.clone(),
            user_id: dto.user_id,
            personal_account_id: dto.personal_account_id,
            enabled: dto.source_enabled,
            deleted: dto.source_deleted,
            password_verifier_recognized: dto.password_verifier_recognized,
            password_login_enabled: dto.password_login_enabled,
            password_hash_sha256: decode_hex_32(&dto.password_hash_sha256)?,
            login_name_sha256: decode_hex_32(&dto.login_name_sha256)?,
            second_factor_required: dto.second_factor_required,
            role: parse_role(&dto.role)?,
            auth_version: dto.auth_version,
        },
        target: TargetInstance {
            system_id: decode_hex_32(&dto.system_id)?,
            database_oid: dto.database_oid,
            instance_nonce: decode_hex_32(&dto.instance_nonce)?,
            import_run: decode_hex_32(&dto.import_run)?,
        },
        policy: binding,
        policy_snapshot: snapshot,
    })
}

pub fn read_bounded_json<T: serde::de::DeserializeOwned>(
    input: impl Read,
) -> Result<T, ApprovalError> {
    let mut bytes = Vec::new();
    input
        .take(MAX_STDIN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ApprovalError::InvalidJson)?;
    if bytes.len() as u64 > MAX_STDIN_BYTES {
        return Err(ApprovalError::InputTooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|_| ApprovalError::InvalidJson)
}

/// Read-only candidate emission. Does not create an approval or open a gate.
pub async fn plan_admin_activation(
    connection: &mut PgConnection,
    request: &PlanAdminActivationRequest,
) -> Result<PlanAdminActivationResponse, ApprovalError> {
    require_relation_owner(connection).await?;
    let mut tx = connection.begin().await.map_err(db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let entries: Vec<(String, String)> = request
        .policy
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let (snapshot, binding) =
        snapshot_from_binding(request.user_id, request.policy_revision, &entries)?;
    let (source, target, active, auth_version, _) =
        load_frozen_source(&mut tx, request.user_id, false, true).await?;
    let now: i64 =
        sqlx::query_scalar("SELECT floor(EXTRACT(EPOCH FROM clock_timestamp()))::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    let approval = Approval {
        id: decode_hex_32(&request.approval_id)?,
        approver: decode_hex_32(&request.approver)?,
        expiry_unix: request.expiry_unix,
        source: source.clone(),
        target,
        policy: binding,
        policy_snapshot: snapshot,
    };
    let observed = ObservedAccount {
        source,
        target: approval.target,
        policy: approval.policy,
        current_auth_version: auth_version,
        active,
        now_unix: now,
    };
    // Plan validates the candidate can be admitted now; it does not apply.
    let _ = plan_one(&approval, &observed, None, None)?;
    let confirmation = payload_sha256(&approval);
    tx.commit().await.map_err(db)?;
    Ok(PlanAdminActivationResponse {
        status: "candidate",
        confirmation_sha256: encode_hex_32(&confirmation),
        candidate: approval_to_dto(&approval),
    })
}

/// Owner-only apply. Persists complete policy rows, activates exactly one
/// account, opens the sourced password gate without changing MFA, revokes old
/// credentials, and advances auth_version by exactly one.
pub async fn apply_admin_activation(
    connection: &mut PgConnection,
    request: &ApplyAdminActivationRequest,
) -> Result<ApplyAdminActivationResponse, ApprovalError> {
    require_relation_owner(connection).await?;
    let approval = dto_to_approval(&request.approval)?;
    let expected = decode_hex_32(&request.confirmation_sha256)?;
    let payload = payload_sha256(&approval);
    if expected != payload {
        return Err(ApprovalError::ConfirmationMismatch);
    }
    let mut tx = connection.begin().await.map_err(db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    require_relation_owner(&mut tx).await?;
    // Exact same id+payload replay is read-only success even after activation or
    // expiry; a different payload for the same id is refused before source gates.
    let prior = sqlx::query(
        "SELECT payload_sha256,user_id,expected_auth_version FROM core_identity.admin_activation_receipts WHERE approval_id=$1 FOR UPDATE",
    )
    .bind(approval.id.as_slice())
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    if let Some(row) = &prior {
        let prior_payload = array32(
            row.try_get("payload_sha256")
                .map_err(|_| ApprovalError::Storage)?,
        )?;
        let prior_user: i64 = row.try_get("user_id").map_err(|_| ApprovalError::Storage)?;
        let prior_version: i64 = row
            .try_get("expected_auth_version")
            .map_err(|_| ApprovalError::Storage)?;
        if prior_user != approval.source.user_id
            || prior_payload != payload
            || prior_version != approval.source.auth_version
        {
            return Err(ApprovalError::PayloadConflict);
        }
        // A committed receipt is stable across legitimate password/MFA/version
        // changes, but never authorizes a restored clone or a different policy.
        let (source, target, _, _, _) =
            load_frozen_source(&mut tx, approval.source.user_id, false, false).await?;
        if source != approval.source {
            return Err(ApprovalError::SourceMismatch);
        }
        if target != approval.target {
            return Err(ApprovalError::TargetMismatch);
        }
        let policy = permissions::load_complete_policy(&mut tx, approval.source.user_id)
            .await
            .map_err(|_| ApprovalError::PolicyMismatch)?;
        if policy.revision != approval.policy.revision || policy.sha256 != approval.policy.sha256 {
            return Err(ApprovalError::PolicyMismatch);
        }
        tx.commit().await.map_err(db)?;
        return Ok(ApplyAdminActivationResponse {
            status: "replay",
            user_id: approval.source.user_id,
            approval_id: encode_hex_32(&approval.id),
            payload_sha256: encode_hex_32(&payload),
            auth_version: None,
        });
    }
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.admin_activation_receipts WHERE user_id=$1 AND expected_auth_version=$2 AND approval_id<>$3)")
        .bind(approval.source.user_id).bind(approval.source.auth_version).bind(approval.id.as_slice())
        .fetch_one(&mut *tx).await.map_err(db)?;
    if occupied {
        return Err(ApprovalError::CasConflict);
    }
    let (source, target, active, auth_version, _) =
        load_frozen_source(&mut tx, approval.source.user_id, true, true).await?;
    if source != approval.source {
        return Err(ApprovalError::SourceMismatch);
    }
    if target != approval.target {
        return Err(ApprovalError::TargetMismatch);
    }
    let now: i64 =
        sqlx::query_scalar("SELECT floor(EXTRACT(EPOCH FROM clock_timestamp()))::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    let existing_policy = sqlx::query(
        "SELECT revision,sha256,verified_complete FROM core_identity.permission_policies WHERE user_id=$1 FOR UPDATE",
    )
    .bind(approval.source.user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let observed_policy = if let Some(row) = existing_policy.as_ref() {
        let binding = PolicyBinding {
            revision: row
                .try_get("revision")
                .map_err(|_| ApprovalError::Storage)?,
            sha256: array32(row.try_get("sha256").map_err(|_| ApprovalError::Storage)?)?,
            verified_complete: row
                .try_get("verified_complete")
                .map_err(|_| ApprovalError::Storage)?,
        };
        if binding != approval.policy {
            return Err(ApprovalError::PolicyMismatch);
        }
        permissions::load_complete_policy(&mut tx, approval.source.user_id)
            .await
            .map_err(|_| ApprovalError::PolicyMismatch)?;
        binding
    } else {
        approval.policy
    };
    let occupied = sqlx::query(
        "SELECT approval_id,payload_sha256,expected_auth_version FROM core_identity.admin_activation_receipts
         WHERE user_id=$1 AND expected_auth_version=$2 FOR UPDATE",
    )
    .bind(approval.source.user_id)
    .bind(approval.source.auth_version)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let occupied = occupied
        .map(|row| {
            Ok::<_, ApprovalError>(StoredReceipt {
                approval_id: array32(
                    row.try_get("approval_id")
                        .map_err(|_| ApprovalError::Storage)?,
                )?,
                payload_sha256: array32(
                    row.try_get("payload_sha256")
                        .map_err(|_| ApprovalError::Storage)?,
                )?,
                user_id: approval.source.user_id,
                expected_auth_version: row
                    .try_get("expected_auth_version")
                    .map_err(|_| ApprovalError::Storage)?,
                applied: true,
            })
        })
        .transpose()?;
    let observed = ObservedAccount {
        source,
        target,
        policy: observed_policy,
        current_auth_version: auth_version,
        active,
        now_unix: now,
    };
    let plan = plan_one(&approval, &observed, None, occupied.as_ref())?;
    let Activation::Apply {
        next_auth_version,
        payload_sha256,
        expected_auth_version,
        role,
        personal_account_id,
        password_login_enabled,
        second_factor_required,
        ..
    } = plan
    else {
        return Err(ApprovalError::Storage);
    };
    if second_factor_required != approval.source.second_factor_required {
        return Err(ApprovalError::SourceMismatch);
    }
    let canonical = permissions::canonical_policy_payload(&approval.policy_snapshot)
        .map_err(|_| ApprovalError::Incomplete)?;
    if existing_policy.is_none() {
        sqlx::query(
            "INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete)
             VALUES ($1,$2,$3,$4,$5,TRUE)",
        )
        .bind(approval.source.user_id)
        .bind(approval.policy.revision)
        .bind(CONTRACT_VERSION)
        .bind(approval.policy.sha256.as_slice())
        .bind(&canonical)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        for row in &approval.policy_snapshot.decisions {
            sqlx::query(
                "INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES ($1,$2,$3)",
            )
            .bind(approval.source.user_id)
            .bind(row.action.catalog_name().ok_or(ApprovalError::Incomplete)?)
            .bind(row.effect.catalog_name())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
    }
    // Open sourced password gate while still dormant so the profile trigger
    // does not bump auth_version; activation below owns the single +1.
    if password_login_enabled {
        let opened = sqlx::query(
            "UPDATE core_identity.user_profiles SET password_login_allowed=TRUE
             WHERE user_id=$1 AND password_login_allowed=FALSE AND second_factor_required=$2",
        )
        .bind(approval.source.user_id)
        .bind(second_factor_required)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        if opened.rows_affected() != 1 {
            return Err(ApprovalError::VersionConflict);
        }
    }
    let updated = sqlx::query(
        "UPDATE core_identity.users SET active=TRUE, auth_version=$2
         WHERE id=$1 AND auth_version=$3 AND active=FALSE AND platform_role=$4",
    )
    .bind(approval.source.user_id)
    .bind(next_auth_version)
    .bind(expected_auth_version)
    .bind(role_name(role))
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    if updated.rows_affected() != 1 {
        return Err(ApprovalError::VersionConflict);
    }
    let accounts = sqlx::query(
        "UPDATE core_identity.accounts SET active=TRUE WHERE id=$1 AND kind='personal' AND active=FALSE",
    )
    .bind(personal_account_id)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    if accounts.rows_affected() != 1 {
        return Err(ApprovalError::VersionConflict);
    }
    sqlx::query(
        "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL",
    )
    .bind(approval.source.user_id)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "INSERT INTO core_identity.admin_activation_receipts(approval_id,payload_sha256,user_id,expected_auth_version)
         VALUES ($1,$2,$3,$4)",
    )
    .bind(approval.id.as_slice())
    .bind(payload_sha256.as_slice())
    .bind(approval.source.user_id)
    .bind(expected_auth_version)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "INSERT INTO core_identity.audit(actor_user_id,action,target)
         VALUES ($1,'privileged_activation',jsonb_build_object('approval_id',encode($2,'hex'),'auth_version',$3))",
    )
    .bind(approval.source.user_id)
    .bind(approval.id.as_slice())
    .bind(next_auth_version)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    // Fail closed if policy verification cannot read back the installed rows.
    permissions::load_complete_policy(&mut tx, approval.source.user_id)
        .await
        .map_err(|_| ApprovalError::PolicyMismatch)?;
    tx.commit().await.map_err(db)?;
    Ok(ApplyAdminActivationResponse {
        status: "applied",
        user_id: approval.source.user_id,
        approval_id: encode_hex_32(&approval.id),
        payload_sha256: encode_hex_32(&payload_sha256),
        auth_version: Some(next_auth_version),
    })
}

#[cfg(test)]
mod tests {
    use super::permissions::{CatalogDecision, Effect};
    use super::*;

    fn hash(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn snapshot(user_id: i64, revision: i64) -> PolicySnapshot {
        let decisions = ActionCatalog::all_allow();
        PolicySnapshot {
            user_id,
            revision,
            decisions,
        }
    }

    struct ActionCatalog;
    impl ActionCatalog {
        fn all_allow() -> [CatalogDecision; 9] {
            [
                CatalogDecision {
                    action: permissions::Action::ChannelRead,
                    effect: Effect::Allow,
                },
                CatalogDecision {
                    action: permissions::Action::ChannelOperate,
                    effect: Effect::Allow,
                },
                CatalogDecision {
                    action: permissions::Action::ChannelWrite,
                    effect: Effect::Allow,
                },
                CatalogDecision {
                    action: permissions::Action::ChannelSensitiveWrite,
                    effect: Effect::Deny,
                },
                CatalogDecision {
                    action: permissions::Action::ChannelSecretView,
                    effect: Effect::Deny,
                },
                CatalogDecision {
                    action: permissions::Action::AcquisitionRead,
                    effect: Effect::Allow,
                },
                CatalogDecision {
                    action: permissions::Action::AcquisitionWrite,
                    effect: Effect::Allow,
                },
                CatalogDecision {
                    action: permissions::Action::AcquisitionDetails,
                    effect: Effect::Deny,
                },
                CatalogDecision {
                    action: permissions::Action::AcquisitionExport,
                    effect: Effect::Deny,
                },
            ]
        }
    }

    fn approval() -> Approval {
        let policy_snapshot = snapshot(11, 3);
        let policy_sha = policy_sha256(&policy_snapshot).unwrap();
        Approval {
            id: hash(1),
            approver: hash(2),
            expiry_unix: 1_800_000_000,
            source: FrozenSource {
                archive_sha256: hash(3),
                a2_semantic_sha256: hash(4),
                a2_file_sha256: hash(5),
                a2_sql_sha256: hash(6),
                migration_id: "mig-7".into(),
                user_id: 11,
                personal_account_id: 12,
                enabled: true,
                deleted: false,
                password_verifier_recognized: true,
                password_login_enabled: true,
                password_hash_sha256: hash(13),
                login_name_sha256: hash(14),
                second_factor_required: false,
                role: PlatformRole::Root,
                auth_version: 4,
            },
            target: TargetInstance {
                system_id: hash(8),
                database_oid: 99,
                instance_nonce: hash(9),
                import_run: import_run_digest("mig-7"),
            },
            policy: PolicyBinding {
                revision: 3,
                sha256: policy_sha,
                verified_complete: true,
            },
            policy_snapshot,
        }
    }

    fn observed(approval: &Approval) -> ObservedAccount {
        ObservedAccount {
            source: approval.source.clone(),
            target: approval.target,
            policy: approval.policy,
            current_auth_version: approval.source.auth_version,
            active: false,
            now_unix: approval.expiry_unix - 1,
        }
    }

    #[test]
    fn one_inactive_root_activates_once_and_keeps_mfa_and_role() {
        let approval = approval();
        let plan = plan_one(&approval, &observed(&approval), None, None).unwrap();
        match plan {
            Activation::Apply {
                next_auth_version,
                role,
                second_factor_required,
                user_id,
                ..
            } => {
                assert_eq!(next_auth_version, 5);
                assert_eq!(role, PlatformRole::Root);
                assert!(!second_factor_required);
                assert_eq!(user_id, 11);
            }
            Activation::Replay { .. } => panic!("first admission must apply"),
        }
    }

    #[test]
    fn exact_replay_does_not_apply_again() {
        let approval = approval();
        let first = plan_one(&approval, &observed(&approval), None, None).unwrap();
        let Activation::Apply {
            payload_sha256,
            expected_auth_version,
            ..
        } = first
        else {
            panic!("expected apply");
        };
        let prior = StoredReceipt {
            approval_id: approval.id,
            payload_sha256,
            user_id: approval.source.user_id,
            expected_auth_version,
            applied: true,
        };
        assert_eq!(
            plan_one(&approval, &observed(&approval), Some(&prior), None).unwrap(),
            Activation::Replay {
                user_id: 11,
                approval_id: approval.id,
                payload_sha256
            }
        );
    }

    #[test]
    fn changed_payload_and_second_approval_conflict() {
        let approval = approval();
        let payload = payload_sha256(&approval);
        let prior = StoredReceipt {
            approval_id: approval.id,
            payload_sha256: hash(255),
            user_id: 11,
            expected_auth_version: 4,
            applied: true,
        };
        assert_eq!(
            plan_one(&approval, &observed(&approval), Some(&prior), None),
            Err(ApprovalError::PayloadConflict)
        );
        let other = StoredReceipt {
            approval_id: hash(9),
            payload_sha256: payload,
            user_id: 11,
            expected_auth_version: 4,
            applied: true,
        };
        assert_eq!(
            plan_one(&approval, &observed(&approval), None, Some(&other)),
            Err(ApprovalError::CasConflict)
        );
    }

    #[test]
    fn batch_expired_hand_activated_and_mfa_source_are_refused() {
        let approval = approval();
        let second = Approval {
            id: hash(12),
            ..approval.clone()
        };
        assert_eq!(
            plan_batch(
                &[approval.clone(), second],
                &observed(&approval),
                None,
                None
            ),
            Err(ApprovalError::Batch)
        );
        let mut late = observed(&approval);
        late.now_unix = approval.expiry_unix;
        assert_eq!(
            plan_one(&approval, &late, None, None),
            Err(ApprovalError::Expired)
        );
        let mut live = observed(&approval);
        live.active = true;
        assert_eq!(
            plan_one(&approval, &live, None, None),
            Err(ApprovalError::AlreadyActive)
        );
        let mut mfa = approval.clone();
        mfa.source.second_factor_required = true;
        let mut seen = observed(&mfa);
        seen.source.second_factor_required = true;
        match plan_one(&mfa, &seen, None, None).unwrap() {
            Activation::Apply {
                second_factor_required,
                ..
            } => assert!(second_factor_required),
            Activation::Replay { .. } => panic!("mfa fact must be preserved on apply"),
        }
        let mut blocked = approval;
        blocked.source.deleted = true;
        let mut seen = observed(&blocked);
        seen.source.deleted = true;
        assert_eq!(
            plan_one(&blocked, &seen, None, None),
            Err(ApprovalError::SourceBlocked)
        );
    }

    #[test]
    fn ordinary_user_unknown_policy_and_stale_version_do_not_activate() {
        let mut user = approval();
        user.source.role = PlatformRole::User;
        assert_eq!(
            plan_one(&user, &observed(&user), None, None),
            Err(ApprovalError::Incomplete)
        );
        let approval = approval();
        let mut policy = observed(&approval);
        policy.policy.verified_complete = false;
        assert_eq!(
            plan_one(&approval, &policy, None, None),
            Err(ApprovalError::PolicyMismatch)
        );
        let mut stale = observed(&approval);
        stale.current_auth_version = 5;
        assert_eq!(
            plan_one(&approval, &stale, None, None),
            Err(ApprovalError::VersionConflict)
        );
    }
    #[test]
    fn strict_stdin_round_trip_rejects_unknown_duplicate_and_oversize_inputs() {
        let approval = approval();
        let candidate = approval_to_dto(&approval);
        let payload = serde_json::json!({"confirmation_sha256":encode_hex_32(&payload_sha256(&approval)),"approval":candidate});
        let bytes = serde_json::to_vec(&payload).unwrap();
        let parsed: ApplyAdminActivationRequest = read_bounded_json(bytes.as_slice()).unwrap();
        assert_eq!(
            payload_sha256(&dto_to_approval(&parsed.approval).unwrap()),
            payload_sha256(&approval)
        );
        for nested in [false, true] {
            let mut unknown = payload.clone();
            let object = if nested {
                unknown["approval"].as_object_mut().unwrap()
            } else {
                unknown.as_object_mut().unwrap()
            };
            object.insert("unknown".into(), serde_json::json!(true));
            assert!(matches!(
                read_bounded_json::<ApplyAdminActivationRequest>(
                    serde_json::to_vec(&unknown).unwrap().as_slice()
                ),
                Err(ApprovalError::InvalidJson)
            ));
        }
        let encoded = serde_json::to_string(&payload).unwrap();
        let confirmation_duplicate = encoded.replacen(
            "\"confirmation_sha256\":",
            "\"confirmation_sha256\":\"duplicate\",\"confirmation_sha256\":",
            1,
        );
        let subject_duplicate = encoded.replacen("\"user_id\":", "\"user_id\":999,\"user_id\":", 1);
        let policy_duplicate = encoded.replacen(
            "\"ChannelRead\":\"allow\"",
            "\"ChannelRead\":\"allow\",\"ChannelRead\":\"deny\"",
            1,
        );
        for duplicate in [confirmation_duplicate, subject_duplicate, policy_duplicate] {
            assert_ne!(duplicate, encoded);
            assert!(matches!(
                read_bounded_json::<ApplyAdminActivationRequest>(duplicate.as_bytes()),
                Err(ApprovalError::InvalidJson)
            ));
        }
        let duplicate = br#"{"approval_id":"a","approver":"b","expiry_unix":1,"user_id":1,"policy_revision":1,"policy":{"ChannelRead":"allow","ChannelRead":"deny"}}"#;
        assert!(matches!(
            read_bounded_json::<PlanAdminActivationRequest>(duplicate.as_slice()),
            Err(ApprovalError::InvalidJson)
        ));
        let too_large = vec![b' '; MAX_STDIN_BYTES as usize + 1];
        assert!(matches!(
            read_bounded_json::<PlanAdminActivationRequest>(too_large.as_slice()),
            Err(ApprovalError::InputTooLarge)
        ));
    }
}

#[cfg(test)]
#[path = "admin_approval_pg_tests.rs"]
mod pg_tests;
