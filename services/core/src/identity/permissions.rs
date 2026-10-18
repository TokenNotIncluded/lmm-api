#![allow(dead_code)] // Called by business gates after the offline approval executor lands.
//! Pure Core authorization; not an authentication mechanism or spend permit.
//! Main semantics adapted from api.lmm.best 5999dc6563921a38df6f12f64b71656cf94c16c6,
//! apps/api-go/service/authz/{resolver,assignment,resources_channel,resources_acquisition}.go
//! (AGPL-3.0-or-later). Unknown actions fail closed even for root.
//!
//! Constructors are crate-private, deliberately not Deserialize. Callers MUST
//! load/lock current Core identity, policy and resource rows in the business
//! transaction. A decision is valid only inside that transaction. No host token,
//! UI DTO, service tier or client payer can construct authority.
//!
//! Complete catalog snapshots for imported admins are validated and hashed here;
//! an empty or hash-only `verified_complete` claim is never evidence by itself.

use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

pub const CONTRACT_VERSION: &str = "core.permissions.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformRole {
    User,
    Admin,
    Root,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Credential {
    Session,
    ApiKey,
    Host,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberRole {
    Owner,
    Admin,
    Member,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeveloperState {
    Granted,
    Denied,
    Unknown,
}
/// Paid qualification is supplied by the locked Core financial policy, not an
/// amount or balance from a browser. No financial-unit conversion lives here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaidQualification {
    Qualified,
    NotQualified,
    Unknown,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeveloperFacts {
    pub role: PlatformRole,
    pub override_level: Option<i16>,
    pub manually_activated: bool,
    pub paid: PaidQualification,
}
/// @mark identity.developer-access role=authority contract=core.permissions.v1
pub(crate) fn developer_access(facts: DeveloperFacts) -> DeveloperState {
    if facts.role != PlatformRole::User {
        return DeveloperState::Granted;
    }
    if let Some(level) = facts.override_level {
        return if (1..=4).contains(&level) {
            DeveloperState::Granted
        } else {
            DeveloperState::Denied
        };
    }
    if facts.manually_activated {
        return DeveloperState::Granted;
    }
    match facts.paid {
        PaidQualification::Qualified => DeveloperState::Granted,
        PaidQualification::NotQualified => DeveloperState::Denied,
        PaidQualification::Unknown => DeveloperState::Unknown,
    }
}

/// Route-selected operation, NEVER parsed from a caller-selected action string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    ChannelRead,
    ChannelOperate,
    ChannelWrite,
    ChannelSensitiveWrite,
    ChannelSecretView,
    AcquisitionRead,
    AcquisitionWrite,
    AcquisitionDetails,
    AcquisitionExport,
    UserRead,
    /// Existing administrator user-directory read, distinct from managing a target user.
    AdminUsersRead,
    UserManage,
    DeveloperReview,
    PermissionCatalogRead,
    UserPermissionsWrite,
    RootSettings,
    FinanceAuditRead,
    DirectoryModerate,
    OwnProfileRead,
    OwnProfileWrite,
    OwnKeyManage,
    DeveloperConsole,
    TeamRead,
    TeamInviteAdmin,
    TeamInviteMember,
    TeamRemoveAdmin,
    TeamRemoveMember,
    TeamSpend,
    Unknown,
}
impl Action {
    pub const CATALOG: [Self; 9] = [
        Self::ChannelRead,
        Self::ChannelOperate,
        Self::ChannelWrite,
        Self::ChannelSensitiveWrite,
        Self::ChannelSecretView,
        Self::AcquisitionRead,
        Self::AcquisitionWrite,
        Self::AcquisitionDetails,
        Self::AcquisitionExport,
    ];
    /// Stable catalog wire name; order follows [`Self::CATALOG`].
    pub fn catalog_name(self) -> Option<&'static str> {
        Some(match self {
            Self::ChannelRead => "ChannelRead",
            Self::ChannelOperate => "ChannelOperate",
            Self::ChannelWrite => "ChannelWrite",
            Self::ChannelSensitiveWrite => "ChannelSensitiveWrite",
            Self::ChannelSecretView => "ChannelSecretView",
            Self::AcquisitionRead => "AcquisitionRead",
            Self::AcquisitionWrite => "AcquisitionWrite",
            Self::AcquisitionDetails => "AcquisitionDetails",
            Self::AcquisitionExport => "AcquisitionExport",
            _ => return None,
        })
    }
    pub fn parse_catalog_name(name: &str) -> Option<Self> {
        Self::CATALOG
            .into_iter()
            .find(|action| action.catalog_name() == Some(name))
    }
    fn admin_default(self) -> bool {
        matches!(
            self,
            Self::ChannelRead
                | Self::ChannelOperate
                | Self::ChannelWrite
                | Self::AcquisitionRead
                | Self::AcquisitionWrite
        )
    }
    pub fn is_catalog(self) -> bool {
        Self::CATALOG.contains(&self)
    }
}

impl Effect {
    pub fn catalog_name(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
    pub fn parse_catalog_name(name: &str) -> Option<Self> {
        match name {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// One explicit catalog decision in a complete nine-action snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogDecision {
    pub action: Action,
    pub effect: Effect,
}

/// Human-reviewed complete catalog policy for one subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySnapshot {
    pub user_id: i64,
    pub revision: i64,
    pub decisions: [CatalogDecision; 9],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicySnapshotError {
    UnknownAction,
    UnknownEffect,
    UnknownField,
    MissingAction,
    DuplicateAction,
    Incomplete,
    InvalidRevision,
    InvalidUser,
}

/// Fixed ordered canonical bytes hashed for persistence and approval binding.
pub fn canonical_policy_payload(snapshot: &PolicySnapshot) -> Result<Vec<u8>, PolicySnapshotError> {
    if snapshot.user_id <= 0 || snapshot.revision <= 0 {
        return Err(if snapshot.user_id <= 0 {
            PolicySnapshotError::InvalidUser
        } else {
            PolicySnapshotError::InvalidRevision
        });
    }
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(CONTRACT_VERSION.as_bytes());
    out.push(0);
    out.extend_from_slice(&snapshot.revision.to_be_bytes());
    out.push(0);
    out.extend_from_slice(&snapshot.user_id.to_be_bytes());
    out.push(0);
    for (expected, row) in Action::CATALOG.iter().zip(snapshot.decisions.iter()) {
        if row.action != *expected || !row.action.is_catalog() {
            return Err(PolicySnapshotError::Incomplete);
        }
        let name = row
            .action
            .catalog_name()
            .ok_or(PolicySnapshotError::UnknownAction)?;
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(row.effect.catalog_name().as_bytes());
        out.push(0);
    }
    Ok(out)
}

pub fn policy_sha256(snapshot: &PolicySnapshot) -> Result<[u8; 32], PolicySnapshotError> {
    Ok(Sha256::digest(canonical_policy_payload(snapshot)?).into())
}

/// Reject unknown/missing/duplicate catalog actions and unknown fields.
pub fn complete_policy_snapshot(
    user_id: i64,
    revision: i64,
    entries: &[(String, String)],
) -> Result<PolicySnapshot, PolicySnapshotError> {
    if user_id <= 0 {
        return Err(PolicySnapshotError::InvalidUser);
    }
    if revision <= 0 {
        return Err(PolicySnapshotError::InvalidRevision);
    }
    if entries.len() != Action::CATALOG.len() {
        return Err(PolicySnapshotError::Incomplete);
    }
    let mut decisions = [CatalogDecision {
        action: Action::ChannelRead,
        effect: Effect::Allow,
    }; 9];
    let mut seen = [false; 9];
    for (action_name, effect_name) in entries {
        let action =
            Action::parse_catalog_name(action_name).ok_or(PolicySnapshotError::UnknownAction)?;
        let effect =
            Effect::parse_catalog_name(effect_name).ok_or(PolicySnapshotError::UnknownEffect)?;
        let index = Action::CATALOG
            .iter()
            .position(|item| *item == action)
            .ok_or(PolicySnapshotError::UnknownAction)?;
        if seen[index] {
            return Err(PolicySnapshotError::DuplicateAction);
        }
        seen[index] = true;
        decisions[index] = CatalogDecision { action, effect };
    }
    if seen.iter().any(|flag| !*flag) {
        return Err(PolicySnapshotError::MissingAction);
    }
    let snapshot = PolicySnapshot {
        user_id,
        revision,
        decisions,
    };
    let _ = canonical_policy_payload(&snapshot)?;
    Ok(snapshot)
}

/// Runtime-loaded complete policy. `verified_complete` is true only after row
/// and hash verification against the canonical payload.
#[derive(Debug)]
pub struct LoadedPolicy {
    pub subject_user_id: i64,
    pub revision: i64,
    pub sha256: [u8; 32],
    pub(crate) overrides: Vec<Override>,
}

impl LoadedPolicy {
    pub(crate) fn as_policy(&self) -> Policy<'_> {
        Policy {
            subject_user_id: self.subject_user_id,
            contract: CONTRACT_VERSION,
            revision: self.revision,
            sha256: self.sha256,
            verified_complete: true,
            overrides: &self.overrides,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyLoadError {
    Missing,
    Incomplete,
    HashMismatch,
    Storage,
}

fn policy_storage(error: sqlx::Error) -> PolicyLoadError {
    let _ = error;
    PolicyLoadError::Storage
}

/// Load and verify a persisted complete catalog policy. Hash-only rows fail closed.
pub async fn load_complete_policy(
    connection: &mut PgConnection,
    user_id: i64,
) -> Result<LoadedPolicy, PolicyLoadError> {
    let header = sqlx::query(
        "SELECT revision,contract,sha256,canonical_payload,verified_complete
         FROM core_identity.permission_policies WHERE user_id=$1",
    )
    .bind(user_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(policy_storage)?
    .ok_or(PolicyLoadError::Missing)?;
    let revision: i64 = header
        .try_get("revision")
        .map_err(|_| PolicyLoadError::Storage)?;
    let contract: String = header
        .try_get("contract")
        .map_err(|_| PolicyLoadError::Storage)?;
    let sha: Vec<u8> = header
        .try_get("sha256")
        .map_err(|_| PolicyLoadError::Storage)?;
    let payload: Vec<u8> = header
        .try_get("canonical_payload")
        .map_err(|_| PolicyLoadError::Storage)?;
    let verified: bool = header
        .try_get("verified_complete")
        .map_err(|_| PolicyLoadError::Storage)?;
    if !verified || contract != CONTRACT_VERSION || revision <= 0 {
        return Err(PolicyLoadError::Incomplete);
    }
    let sha256: [u8; 32] = sha.try_into().map_err(|_| PolicyLoadError::Incomplete)?;
    if sha256 != Sha256::digest(&payload).as_slice() {
        return Err(PolicyLoadError::HashMismatch);
    }
    let rows = sqlx::query(
        "SELECT action,effect FROM core_identity.permission_policy_rows
         WHERE user_id=$1 ORDER BY action",
    )
    .bind(user_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(policy_storage)?;
    if rows.len() != Action::CATALOG.len() {
        return Err(PolicyLoadError::Incomplete);
    }
    let mut entries = Vec::with_capacity(rows.len());
    for row in &rows {
        let action: String = row
            .try_get("action")
            .map_err(|_| PolicyLoadError::Storage)?;
        let effect: String = row
            .try_get("effect")
            .map_err(|_| PolicyLoadError::Storage)?;
        entries.push((action, effect));
    }
    let snapshot = complete_policy_snapshot(user_id, revision, &entries)
        .map_err(|_| PolicyLoadError::Incomplete)?;
    let expected_payload =
        canonical_policy_payload(&snapshot).map_err(|_| PolicyLoadError::Incomplete)?;
    if expected_payload != payload {
        return Err(PolicyLoadError::HashMismatch);
    }
    let expected_sha = policy_sha256(&snapshot).map_err(|_| PolicyLoadError::Incomplete)?;
    if expected_sha != sha256 {
        return Err(PolicyLoadError::HashMismatch);
    }
    let overrides = snapshot
        .decisions
        .iter()
        .map(|row| Override {
            subject_user_id: user_id,
            action: row.action,
            effect: row.effect,
        })
        .collect();
    Ok(LoadedPolicy {
        subject_user_id: user_id,
        revision,
        sha256,
        overrides,
    })
}

/// True when the account was imported as privileged and must use persisted policy.
pub async fn has_privileged_import_facts(
    connection: &mut PgConnection,
    user_id: i64,
) -> Result<bool, PolicyLoadError> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM core_identity.privileged_import_facts WHERE user_id=$1)",
    )
    .bind(user_id)
    .fetch_one(&mut *connection)
    .await
    .map_err(policy_storage)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformResource {
    Channel,
    Acquisition,
    PermissionCatalog,
    Settings,
    FinanceAudit,
    Directory,
}
/// Resource IDs are resolved storage identities, not user-controlled payer claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Platform(PlatformResource),
    User {
        user_id: i64,
        account_id: i64,
        role: PlatformRole,
    },
    Team {
        team_id: i64,
        account_id: i64,
    },
    Empty,
}
impl Scope {
    fn valid(self) -> bool {
        match self {
            Self::Platform(_) => true,
            Self::User {
                user_id,
                account_id,
                ..
            } => user_id > 0 && account_id > 0,
            Self::Team {
                team_id,
                account_id,
            } => team_id > 0 && account_id > 0,
            Self::Empty => false,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    Allow,
    Deny,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Override {
    pub subject_user_id: i64,
    pub action: Action,
    pub effect: Effect,
}
/// Must be an entire verified snapshot, not a filtered list that lost denies.
#[derive(Debug)]
pub(crate) struct Policy<'a> {
    pub subject_user_id: i64,
    pub contract: &'a str,
    pub revision: i64,
    pub sha256: [u8; 32],
    pub verified_complete: bool,
    pub overrides: &'a [Override],
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Actor {
    pub user_id: i64,
    pub personal_account_id: i64,
    pub role: PlatformRole,
    pub credential: Credential,
    pub active: bool,
    pub account_active: bool,
    pub auth_version: i64,
    pub credential_auth_version: i64,
    pub developer: DeveloperState,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Membership {
    pub user_id: i64,
    pub team_id: i64,
    pub account_id: i64,
    pub role: MemberRole,
    pub can_spend: bool,
    pub active: bool,
    pub revision: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Denial {
    UnknownAction,
    InvalidScope,
    Unauthenticated,
    CredentialKind,
    StaleAuthority,
    PolicyUnavailable,
    ExplicitDeny,
    RoleDenied,
    TenantDenied,
    DeveloperUnknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Deny(Denial),
}
impl PermissionDecision {
    pub fn allowed(self) -> bool {
        self == Self::Allow
    }
}
fn deny(reason: Denial) -> PermissionDecision {
    PermissionDecision::Deny(reason)
}
fn platform_kind(action: Action) -> Option<PlatformResource> {
    use Action::*;
    Some(match action {
        ChannelRead | ChannelOperate | ChannelWrite | ChannelSensitiveWrite | ChannelSecretView => {
            PlatformResource::Channel
        }
        AcquisitionRead | AcquisitionWrite | AcquisitionDetails | AcquisitionExport => {
            PlatformResource::Acquisition
        }
        PermissionCatalogRead => PlatformResource::PermissionCatalog,
        RootSettings => PlatformResource::Settings,
        FinanceAuditRead => PlatformResource::FinanceAudit,
        DirectoryModerate | AdminUsersRead => PlatformResource::Directory,
        _ => return None,
    })
}
/// @mark identity.permissions role=authority contract=core.permissions.v1
pub(crate) fn decide(
    actor: &Actor,
    policy: &Policy<'_>,
    action: Action,
    scope: Scope,
    membership: Option<&Membership>,
) -> PermissionDecision {
    use Action::*;
    if action == Unknown {
        return deny(Denial::UnknownAction);
    }
    if !scope.valid() {
        return deny(Denial::InvalidScope);
    }
    if actor.user_id <= 0
        || actor.personal_account_id <= 0
        || !actor.active
        || !actor.account_active
    {
        return deny(Denial::Unauthenticated);
    }
    if actor.credential != Credential::Session {
        return deny(Denial::CredentialKind);
    }
    if actor.auth_version <= 0 || actor.auth_version != actor.credential_auth_version {
        return deny(Denial::StaleAuthority);
    }
    if !policy.verified_complete
        || policy.subject_user_id != actor.user_id
        || policy.contract != CONTRACT_VERSION
        || policy.revision <= 0
        || policy.sha256 == [0; 32]
        || policy
            .overrides
            .iter()
            .any(|p| p.subject_user_id != actor.user_id || !p.action.is_catalog())
    {
        return deny(Denial::PolicyUnavailable);
    }
    // Validate resource family before root/overrides: an allow never changes scope.
    if let Some(kind) = platform_kind(action) {
        if scope != Scope::Platform(kind) {
            return deny(Denial::InvalidScope);
        }
    } else {
        let correct = match action {
            UserRead | UserManage | DeveloperReview | UserPermissionsWrite | OwnProfileRead
            | OwnProfileWrite | OwnKeyManage | DeveloperConsole => {
                matches!(scope, Scope::User { .. })
            }
            TeamRead | TeamInviteAdmin | TeamInviteMember | TeamRemoveAdmin | TeamRemoveMember
            | TeamSpend => matches!(scope, Scope::Team { .. }),
            _ => false,
        };
        if !correct {
            return deny(Denial::InvalidScope);
        }
    }
    if action.is_catalog() {
        // Main root precedes overrides; deny wins among conflicting user rows.
        if actor.role == PlatformRole::Root {
            return PermissionDecision::Allow;
        }
        if actor.role != PlatformRole::Admin {
            return deny(Denial::RoleDenied);
        }
        let mut allow = false;
        for row in policy.overrides.iter().filter(|r| r.action == action) {
            if row.effect == Effect::Deny {
                return deny(Denial::ExplicitDeny);
            }
            allow = true;
        }
        return if allow || action.admin_default() {
            PermissionDecision::Allow
        } else {
            deny(Denial::RoleDenied)
        };
    }
    let allowed = match action {
        PermissionCatalogRead | FinanceAuditRead | AdminUsersRead => {
            actor.role != PlatformRole::User
        }
        RootSettings | DirectoryModerate => actor.role == PlatformRole::Root,
        UserRead | UserManage | DeveloperReview | UserPermissionsWrite => {
            let Scope::User { role, .. } = scope else {
                return deny(Denial::InvalidScope);
            };
            if action == UserPermissionsWrite && actor.role != PlatformRole::Root {
                false
            } else {
                can_manage_role(actor.role, role)
            }
        }
        OwnProfileRead | OwnProfileWrite | OwnKeyManage | DeveloperConsole => {
            let Scope::User {
                user_id,
                account_id,
                role,
            } = scope
            else {
                return deny(Denial::InvalidScope);
            };
            if user_id != actor.user_id
                || account_id != actor.personal_account_id
                || role != actor.role
            {
                return deny(Denial::TenantDenied);
            }
            if matches!(action, OwnKeyManage | DeveloperConsole) && actor.role == PlatformRole::User
            {
                match actor.developer {
                    DeveloperState::Unknown => return deny(Denial::DeveloperUnknown),
                    DeveloperState::Denied => return deny(Denial::RoleDenied),
                    DeveloperState::Granted => {}
                }
            }
            true
        }
        TeamRead | TeamInviteAdmin | TeamInviteMember | TeamRemoveAdmin | TeamRemoveMember
        | TeamSpend => {
            let Scope::Team {
                team_id,
                account_id,
            } = scope
            else {
                return deny(Denial::InvalidScope);
            };
            let Some(m) = membership else {
                return deny(Denial::TenantDenied);
            };
            if !m.active
                || m.revision <= 0
                || m.user_id != actor.user_id
                || m.team_id != team_id
                || m.account_id != account_id
            {
                return deny(Denial::TenantDenied);
            }
            match action {
                TeamRead => true,
                TeamInviteAdmin | TeamRemoveAdmin => m.role == MemberRole::Owner,
                TeamInviteMember | TeamRemoveMember => m.role != MemberRole::Member,
                TeamSpend => m.role == MemberRole::Owner || m.can_spend,
                _ => false,
            }
        }
        _ => false,
    };
    if allowed {
        PermissionDecision::Allow
    } else {
        deny(Denial::RoleDenied)
    }
}
/// Main controller/user.go canManageTargetRole: root may manage root; admin
/// may manage only strictly lower platform roles. Service tiers never enter here.
pub fn can_manage_role(actor: PlatformRole, target: PlatformRole) -> bool {
    actor == PlatformRole::Root || (actor == PlatformRole::Admin && target == PlatformRole::User)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(role: PlatformRole) -> Actor {
        Actor {
            user_id: 7,
            personal_account_id: 8,
            role,
            credential: Credential::Session,
            active: true,
            account_active: true,
            auth_version: 3,
            credential_auth_version: 3,
            developer: DeveloperState::Granted,
        }
    }
    fn policy(overrides: &[Override]) -> Policy<'_> {
        Policy {
            subject_user_id: 7,
            contract: CONTRACT_VERSION,
            revision: 1,
            sha256: [9; 32],
            verified_complete: true,
            overrides,
        }
    }
    fn member(role: MemberRole, can_spend: bool) -> Membership {
        Membership {
            user_id: 7,
            team_id: 4,
            account_id: 5,
            role,
            can_spend,
            active: true,
            revision: 1,
        }
    }

    #[test]
    fn root_catalog_allows_known_actions_and_unknown_stays_closed() {
        let decision = decide(
            &actor(PlatformRole::Root),
            &policy(&[]),
            Action::ChannelSecretView,
            Scope::Platform(PlatformResource::Channel),
            None,
        );
        assert_eq!(decision, PermissionDecision::Allow);
        let unknown = decide(
            &actor(PlatformRole::Root),
            &policy(&[]),
            Action::Unknown,
            Scope::Platform(PlatformResource::Channel),
            None,
        );
        assert_eq!(unknown, PermissionDecision::Deny(Denial::UnknownAction));
    }

    #[test]
    fn explicit_deny_beats_admin_default_and_api_key_cannot_administer() {
        let deny_row = [Override {
            subject_user_id: 7,
            action: Action::ChannelRead,
            effect: Effect::Deny,
        }];
        let denied = decide(
            &actor(PlatformRole::Admin),
            &policy(&deny_row),
            Action::ChannelRead,
            Scope::Platform(PlatformResource::Channel),
            None,
        );
        assert_eq!(denied, PermissionDecision::Deny(Denial::ExplicitDeny));
        let mut key = actor(PlatformRole::Root);
        key.credential = Credential::ApiKey;
        let keyed = decide(
            &key,
            &policy(&[]),
            Action::ChannelRead,
            Scope::Platform(PlatformResource::Channel),
            None,
        );
        assert_eq!(keyed, PermissionDecision::Deny(Denial::CredentialKind));
    }

    #[test]
    fn complete_catalog_snapshot_hashes_in_fixed_order() {
        let entries: Vec<(String, String)> = Action::CATALOG
            .iter()
            .enumerate()
            .map(|(index, action)| {
                (
                    action.catalog_name().unwrap().to_owned(),
                    if index < 5 { "allow" } else { "deny" }.to_owned(),
                )
            })
            .collect();
        let snapshot = complete_policy_snapshot(7, 1, &entries).unwrap();
        assert_eq!(
            policy_sha256(&snapshot).unwrap(),
            policy_sha256(&complete_policy_snapshot(7, 1, &entries).unwrap()).unwrap()
        );
        assert!(matches!(
            complete_policy_snapshot(7, 1, &entries[..8]),
            Err(PolicySnapshotError::Incomplete)
        ));
        let mut unknown = entries.clone();
        unknown[0].0 = "NotAnAction".into();
        assert_eq!(
            complete_policy_snapshot(7, 1, &unknown),
            Err(PolicySnapshotError::UnknownAction)
        );
    }

    #[test]
    fn team_spend_and_cross_account_profile_are_scoped() {
        let scope = Scope::Team {
            team_id: 4,
            account_id: 5,
        };
        let plain = member(MemberRole::Member, false);
        let spend = decide(
            &actor(PlatformRole::User),
            &policy(&[]),
            Action::TeamSpend,
            scope,
            Some(&plain),
        );
        assert_eq!(spend, PermissionDecision::Deny(Denial::RoleDenied));
        let owner = member(MemberRole::Owner, false);
        let allowed = decide(
            &actor(PlatformRole::User),
            &policy(&[]),
            Action::TeamSpend,
            scope,
            Some(&owner),
        );
        assert_eq!(allowed, PermissionDecision::Allow);
        let other = Scope::User {
            user_id: 99,
            account_id: 100,
            role: PlatformRole::User,
        };
        let crossed = decide(
            &actor(PlatformRole::User),
            &policy(&[]),
            Action::OwnProfileRead,
            other,
            None,
        );
        assert_eq!(crossed, PermissionDecision::Deny(Denial::TenantDenied));
    }
}
