//! Separate native profile metadata contract. The frozen admin-users read-v1
//! DTO stays unchanged; no legacy general user update is implemented here.
//!
//! Safe display-name subset adapted from fixed main 5999dc6563921a38df6f12f64b71656cf94c16c6,
//! controller/user.go UpdateUser/canManageTargetRole and model/user.go
//! EditWithTx/DisplayName validation (AGPL-3.0-or-later).
use super::{AdminUsersError, AdminUsersStore};
use crate::identity::{self, CredentialKind, IdentityError};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

pub const PROFILE_CONTRACT: &str = "core.admin-user-profile.v1";
type ProfileResult<T> = Result<T, ProfileError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    Unauthorized,
    Forbidden,
    Invalid,
    NotFound,
    Conflict,
    Unavailable,
}
impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProfileError {}
impl From<IdentityError> for ProfileError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            IdentityError::Invalid => Self::Invalid,
            IdentityError::Conflict => Self::Conflict,
            IdentityError::Storage => Self::Unavailable,
        }
    }
}
impl From<AdminUsersError> for ProfileError {
    fn from(error: AdminUsersError) -> Self {
        match error {
            AdminUsersError::Unauthorized => Self::Unauthorized,
            AdminUsersError::Forbidden => Self::Forbidden,
            AdminUsersError::Invalid => Self::Invalid,
            _ => Self::Unavailable,
        }
    }
}
impl From<sqlx::Error> for ProfileError {
    fn from(error: sqlx::Error) -> Self {
        if error
            .as_database_error()
            .is_some_and(|db| matches!(db.code().as_deref(), Some("40001" | "40P01" | "55P03")))
        {
            Self::Conflict
        } else {
            // An I/O error during COMMIT can be ambiguous. Never expose SQL,
            // credentials or claim that an unavailable mutation was rejected.
            Self::Unavailable
        }
    }
}
impl IntoResponse for ProfileError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "ADMIN_PROFILE_UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "ADMIN_PROFILE_FORBIDDEN"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "ADMIN_PROFILE_INVALID"),
            Self::NotFound => (StatusCode::NOT_FOUND, "ADMIN_PROFILE_NOT_FOUND"),
            Self::Conflict => (StatusCode::CONFLICT, "ADMIN_PROFILE_CONFLICT"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "ADMIN_PROFILE_UNAVAILABLE"),
        };
        (status, Json(serde_json::json!({"error":{"code":code}}))).into_response()
    }
}

#[derive(Serialize)]
pub struct ProfileOperations {
    read: bool,
    update_display_name: bool,
}
#[derive(Serialize)]
pub struct DisplayNameLimits {
    max_code_points: u8,
    max_utf8_bytes: u8,
    controls_allowed: bool,
}
#[derive(Serialize)]
pub struct ProfileCapabilities {
    version: u8,
    contract: &'static str,
    operations: ProfileOperations,
    target_platform_roles: [&'static str; 1],
    display_name_limits: DisplayNameLimits,
    preconditions: [&'static str; 2],
}
fn profile_capabilities() -> ProfileCapabilities {
    ProfileCapabilities {
        version: 1,
        contract: PROFILE_CONTRACT,
        operations: ProfileOperations {
            read: true,
            update_display_name: true,
        },
        target_platform_roles: ["user"],
        display_name_limits: DisplayNameLimits {
            max_code_points: 20,
            max_utf8_bytes: 80,
            controls_allowed: false,
        },
        preconditions: ["auth_version", "display_name"],
    }
}

#[derive(Serialize)]
pub struct AdminProfile {
    pub version: u8,
    pub contract: &'static str,
    pub user_id: String,
    pub auth_version: String,
    pub display_name: String,
    pub platform_role: &'static str,
    pub active: bool,
    pub account_active: bool,
}
#[derive(Serialize)]
pub struct ProfileUpdate {
    pub version: u8,
    pub contract: &'static str,
    pub changed: bool,
    pub profile: AdminProfile,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileUpdateRequest {
    pub expected_auth_version: String,
    pub expected_display_name: String,
    pub display_name: String,
}
struct ValidatedUpdate<'a> {
    expected_version: i64,
    expected_name: &'a str,
    new_name: &'a str,
}
impl ProfileUpdateRequest {
    fn validate(&self) -> ProfileResult<ValidatedUpdate<'_>> {
        let version = &self.expected_auth_version;
        if version.is_empty()
            || version.len() > 19
            || version.starts_with('0')
            || !version.bytes().all(|b| b.is_ascii_digit())
            || self.expected_display_name.len() > 1024
            || self.expected_display_name.chars().count() > 256
            || self.display_name.len() > 80
            || self.display_name.chars().count() > 20
            || self.display_name.chars().any(char::is_control)
        {
            return Err(ProfileError::Invalid);
        }
        let expected_version: i64 = version.parse().map_err(|_| ProfileError::Invalid)?;
        if expected_version <= 0 {
            return Err(ProfileError::Invalid);
        }
        Ok(ValidatedUpdate {
            expected_version,
            expected_name: &self.expected_display_name,
            new_name: &self.display_name,
        })
    }
}
fn should_change(
    version: i64,
    current_name: &str,
    input: &ValidatedUpdate<'_>,
) -> ProfileResult<bool> {
    if version != input.expected_version || current_name != input.expected_name {
        return Err(ProfileError::Conflict);
    }
    Ok(current_name != input.new_name)
}
fn display_digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn audit_target(user: i64, version: i64, old_name: &str, new_name: &str) -> serde_json::Value {
    serde_json::json!({"user_id":user,"field":"display_name",
        "before_sha256":display_digest(old_name),"after_sha256":display_digest(new_name),
        "target_auth_version":version.to_string()})
}

#[derive(Clone, Copy)]
enum TargetAccess {
    Snapshot,
    Locked,
}
async fn target_profile(
    c: &mut PgConnection,
    id: i64,
    access: TargetAccess,
) -> ProfileResult<(AdminProfile, i64)> {
    if id <= 0 {
        return Err(ProfileError::Invalid);
    }
    let suffix = match access {
        TargetAccess::Snapshot => "",
        TargetAccess::Locked => " FOR SHARE",
    };
    let user = sqlx::query(&format!("SELECT personal_account_id,platform_role,auth_version,active FROM core_identity.users WHERE id=$1{suffix}"))
        .bind(id).fetch_optional(&mut *c).await?.ok_or(ProfileError::NotFound)?;
    // Establish and hold the ordinary target's role before taking its profile
    // UPDATE lock. Cross-root requests never upgrade actor factor/profile locks.
    if user.try_get::<String, _>("platform_role")? != "user" {
        return Err(ProfileError::Forbidden);
    }
    let account_id: i64 = user.try_get("personal_account_id")?;
    let version: i64 = user.try_get("auth_version")?;
    if account_id <= 0 || version <= 0 {
        return Err(ProfileError::Unavailable);
    }
    let account = sqlx::query(&format!(
        "SELECT kind,active FROM core_identity.accounts WHERE id=$1{suffix}"
    ))
    .bind(account_id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(ProfileError::Unavailable)?;
    if account.try_get::<String, _>("kind")? != "personal" {
        return Err(ProfileError::Unavailable);
    }
    let suffix = match access {
        TargetAccess::Snapshot => "",
        TargetAccess::Locked => " FOR UPDATE",
    };
    let name: String = sqlx::query_scalar(&format!(
        "SELECT display_name FROM core_identity.user_profiles WHERE user_id=$1{suffix}"
    ))
    .bind(id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(ProfileError::NotFound)?;
    // Inactive ordinary accounts remain inactive; metadata repair is not login,
    // recovery, factor removal or privileged migration activation.
    Ok((
        AdminProfile {
            version: 1,
            contract: PROFILE_CONTRACT,
            user_id: id.to_string(),
            auth_version: version.to_string(),
            display_name: name,
            platform_role: "user",
            active: user.try_get("active")?,
            account_active: account.try_get("active")?,
        },
        version,
    ))
}

impl AdminUsersStore {
    pub async fn profile_capabilities(&self, secret: &str) -> ProfileResult<ProfileCapabilities> {
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let result = profile_capabilities();
        tx.commit().await?;
        Ok(result)
    }
    pub async fn user_profile(&self, secret: &str, id: i64) -> ProfileResult<AdminProfile> {
        if id <= 0 {
            return Err(ProfileError::Invalid);
        }
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let (profile, _) = target_profile(&mut tx, id, TargetAccess::Snapshot).await?;
        tx.commit().await?;
        Ok(profile)
    }
    pub async fn update_user_profile(
        &self,
        secret: &str,
        id: i64,
        input: &ProfileUpdateRequest,
    ) -> ProfileResult<ProfileUpdate> {
        if id <= 0 {
            return Err(ProfileError::Invalid);
        }
        let input = input.validate()?;
        let mut tx = self.identity.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        // This is a writer: obtain the current native authority and factor
        // SHARE locks first. The read-only accessor is never used as a substitute.
        let locked = identity::authorize_in_transaction(&mut tx, secret).await?;
        if locked.credential_kind != CredentialKind::Session {
            return Err(ProfileError::Forbidden);
        }
        // Reuse the existing personal-session/current-role/imported-policy gate
        // while those authority locks remain held in this same transaction.
        let actor = Self::authorize(&mut tx, secret).await?;
        if actor.user_id != locked.user_id
            || actor.credential_id != locked.credential_id
            || actor.user_version != locked.user_version
        {
            return Err(ProfileError::Unauthorized);
        }
        let (mut profile, version) = target_profile(&mut tx, id, TargetAccess::Locked).await?;
        let changed = should_change(version, &profile.display_name, &input)?;
        if changed {
            let result = sqlx::query("UPDATE core_identity.user_profiles SET display_name=$2 WHERE user_id=$1 AND display_name=$3")
                .bind(id).bind(input.new_name).bind(input.expected_name).execute(&mut *tx).await?;
            if result.rows_affected() != 1 {
                return Err(ProfileError::Conflict);
            }
            identity::audit(
                &mut tx,
                &actor,
                "admin.user.display_name.update",
                audit_target(id, version, input.expected_name, input.new_name),
            )
            .await?;
            profile.display_name = input.new_name.to_owned();
        }
        tx.commit().await?;
        Ok(ProfileUpdate {
            version: 1,
            contract: PROFILE_CONTRACT,
            changed,
            profile,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParameters {}
fn bearer(headers: &HeaderMap) -> ProfileResult<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let raw = values
        .next()
        .ok_or(ProfileError::Unauthorized)?
        .to_str()
        .map_err(|_| ProfileError::Unauthorized)?;
    if values.next().is_some() {
        return Err(ProfileError::Unauthorized);
    }
    let (scheme, secret) = raw.split_once(' ').ok_or(ProfileError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(ProfileError::Unauthorized);
    }
    Ok(secret)
}
pub(super) fn routes() -> Router<AdminUsersStore> {
    Router::new()
        .route(
            "/core/v1/admin/user-profiles/capabilities",
            get(capabilities),
        )
        .route("/core/v1/admin/user-profiles/{id}", get(read).patch(update))
        .layer(DefaultBodyLimit::max(4 * 1024))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                for (name, value) in [
                    (header::CACHE_CONTROL, "private, no-store"),
                    (header::VARY, "Authorization"),
                    (header::REFERRER_POLICY, "no-referrer"),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ] {
                    response
                        .headers_mut()
                        .insert(name, value.parse().expect("static header"));
                }
                response
            },
        ))
}
async fn capabilities(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    query: Result<Query<NoParameters>, QueryRejection>,
) -> ProfileResult<Json<ProfileCapabilities>> {
    let secret = bearer(&headers)?;
    query.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(store.profile_capabilities(secret).await?))
}
async fn read(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    path: Result<Path<i64>, PathRejection>,
    query: Result<Query<NoParameters>, QueryRejection>,
) -> ProfileResult<Json<AdminProfile>> {
    let secret = bearer(&headers)?;
    let Path(id) = path.map_err(|_| ProfileError::Invalid)?;
    query.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(store.user_profile(secret, id).await?))
}
async fn update(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    path: Result<Path<i64>, PathRejection>,
    query: Result<Query<NoParameters>, QueryRejection>,
    input: Result<Json<ProfileUpdateRequest>, JsonRejection>,
) -> ProfileResult<Json<ProfileUpdate>> {
    let secret = bearer(&headers)?;
    let Path(id) = path.map_err(|_| ProfileError::Invalid)?;
    query.map_err(|_| ProfileError::Invalid)?;
    let Json(input) = input.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(store.update_user_profile(secret, id, &input).await?))
}

#[cfg(test)]
#[path = "profile_mutation_tests.rs"]
mod tests;
