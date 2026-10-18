//! Read-only permission discovery. These responses describe catalog policy;
//! each business operation must still execute its own transactional gate.
//!
//! Catalog DTO and labels adapted from fixed main 5999dc6563921a38df6f12f64b71656cf94c16c6,
//! apps/api-go/{controller/authz.go,service/authz/{resources_channel,resources_acquisition,role}.go}
//! (AGPL-3.0-or-later). No permission mutation or grant is implemented here.
use super::*;
use crate::identity::{
    IdentityStore,
    admin_users::{AdminUsersError, AdminUsersStore},
};
use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type ReadResult<T> = Result<T, AdminUsersError>;
type PermissionsMap = BTreeMap<&'static str, BTreeMap<&'static str, bool>>;

#[derive(Serialize)]
pub struct CatalogAction {
    action: &'static str,
    label_key: &'static str,
    description_key: &'static str,
}
#[derive(Serialize)]
pub struct CatalogResource {
    resource: &'static str,
    label_key: &'static str,
    actions: Vec<CatalogAction>,
}
#[derive(Serialize)]
pub struct CatalogRole {
    key: &'static str,
    name: &'static str,
    built_in: bool,
    superuser: bool,
    grants: PermissionsMap,
}
#[derive(Serialize)]
pub struct PermissionCatalog {
    resources: Vec<CatalogResource>,
    roles: Vec<CatalogRole>,
}
#[derive(Serialize)]
pub struct EffectiveCapabilities {
    version: u8,
    contract: &'static str,
    user_id: String,
    /// Same resource/action matrix as Web self.permissions.admin_permissions.
    /// This is discovery, not a transferable execution or spending permit.
    admin_permissions: PermissionsMap,
    policy_revision: Option<String>,
    policy_sha256: Option<String>,
}

fn catalog_action(action: Action) -> CatalogAction {
    let (wire, label, description) = match action {
        Action::ChannelRead => (
            "read",
            "Read channels",
            "View channel lists and details without secrets.",
        ),
        Action::ChannelOperate => (
            "operate",
            "Operate channels",
            "Test channels, refresh balances, and enable/disable individual, batch, or tagged channels.",
        ),
        Action::ChannelWrite => (
            "write",
            "Edit channel routing",
            "Edit non-sensitive settings such as models, groups, and routing rules.",
        ),
        Action::ChannelSensitiveWrite => (
            "sensitive_write",
            "Edit sensitive channel settings",
            "Create channels or edit keys, base URLs, and overrides.",
        ),
        Action::ChannelSecretView => (
            "secret_view",
            "View channel secrets",
            "Reserved for viewing complete channel keys after secure verification.",
        ),
        Action::AcquisitionRead => (
            "read",
            "View acquisition summaries",
            "View aggregate channel and campaign results.",
        ),
        Action::AcquisitionWrite => (
            "write",
            "Manage promotion links",
            "Create, archive, and delete promotion links.",
        ),
        Action::AcquisitionDetails => (
            "details",
            "View acquisition account details",
            "View account-level source records.",
        ),
        Action::AcquisitionExport => (
            "export",
            "Export acquisition details",
            "Export account-level source records.",
        ),
        _ => unreachable!("only fixed catalog constants reach the catalog serializer"),
    };
    CatalogAction {
        action: wire,
        label_key: label,
        description_key: description,
    }
}

fn capability_matrix(role: PlatformRole, overrides: &[Override]) -> PermissionsMap {
    let mut result = PermissionsMap::new();
    for action in Action::CATALOG {
        let resource = match platform_kind(action) {
            Some(PlatformResource::Channel) => "channel",
            Some(PlatformResource::Acquisition) => "acquisition",
            _ => unreachable!("fixed catalog resource"),
        };
        result.entry(resource).or_default().insert(
            catalog_action(action).action,
            catalog_decision(role, overrides, action).allowed(),
        );
    }
    result
}

fn catalog_schema() -> PermissionCatalog {
    let resources = [
        ("acquisition", "User acquisition", &Action::CATALOG[5..]),
        ("channel", "Channel Management", &Action::CATALOG[..5]),
    ]
    .into_iter()
    .map(|(resource, label_key, actions)| CatalogResource {
        resource,
        label_key,
        actions: actions.iter().copied().map(catalog_action).collect(),
    })
    .collect();
    let roles = [
        ("root", "Root", PlatformRole::Root),
        ("admin", "Admin", PlatformRole::Admin),
    ]
    .into_iter()
    .map(|(key, name, role)| CatalogRole {
        key,
        name,
        built_in: true,
        superuser: role == PlatformRole::Root,
        grants: capability_matrix(role, &[]),
    })
    .collect();
    PermissionCatalog { resources, roles }
}

#[derive(Clone)]
pub struct PermissionReadStore {
    admin: AdminUsersStore,
}
impl PermissionReadStore {
    pub fn new(identity: IdentityStore) -> Self {
        Self {
            admin: AdminUsersStore::new(identity),
        }
    }
    pub async fn catalog(&self, secret: &str) -> ReadResult<PermissionCatalog> {
        let mut tx = self.admin.begin().await?;
        // Exactly the already verified admin-user read gate: current personal
        // native Session, platform role, generation, MFA and imported policy.
        authorize_admin_read_snapshot(&mut tx, secret).await?;
        let result = catalog_schema();
        tx.commit().await?;
        Ok(result)
    }
    pub async fn capabilities(&self, secret: &str) -> ReadResult<EffectiveCapabilities> {
        let mut tx = self.admin.begin().await?;
        let principal = authorize_admin_read_snapshot(&mut tx, secret).await?;
        let role: String =
            sqlx::query_scalar("SELECT platform_role FROM core_identity.users WHERE id=$1")
                .bind(principal.user_id)
                .fetch_one(&mut *tx)
                .await?;
        let role = match role.as_str() {
            "superadmin" => PlatformRole::Root,
            "admin" => PlatformRole::Admin,
            _ => return Err(AdminUsersError::Forbidden),
        };
        let imported = has_privileged_import_facts(&mut tx, principal.user_id)
            .await
            .map_err(|_| AdminUsersError::Unavailable)?;
        let loaded = match load_complete_policy(&mut tx, principal.user_id).await {
            Ok(policy) => Some(policy),
            // Only native owner-created development admins may use the same
            // role baseline as the existing admin directory gate. An imported
            // administrator never obtains this fallback.
            Err(PolicyLoadError::Missing) if !imported => None,
            Err(PolicyLoadError::Storage) => return Err(AdminUsersError::Unavailable),
            Err(
                PolicyLoadError::Missing
                | PolicyLoadError::Incomplete
                | PolicyLoadError::HashMismatch,
            ) => return Err(AdminUsersError::Forbidden),
        };
        let overrides = loaded.as_ref().map_or(&[][..], |p| p.overrides.as_slice());
        let result = EffectiveCapabilities {
            version: 1,
            contract: CONTRACT_VERSION,
            user_id: principal.user_id.to_string(),
            admin_permissions: capability_matrix(role, overrides),
            policy_revision: loaded.as_ref().map(|p| p.revision.to_string()),
            policy_sha256: loaded
                .as_ref()
                .map(|p| p.sha256.iter().map(|b| format!("{b:02x}")).collect()),
        };
        tx.commit().await?;
        Ok(result)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParameters {}
#[derive(Serialize)]
struct CatalogEnvelope {
    success: bool,
    message: &'static str,
    data: PermissionCatalog,
}
fn bearer(headers: &HeaderMap) -> ReadResult<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .ok_or(AdminUsersError::Unauthorized)?
        .to_str()
        .map_err(|_| AdminUsersError::Unauthorized)?;
    if values.next().is_some() {
        return Err(AdminUsersError::Unauthorized);
    }
    let (scheme, secret) = value.split_once(' ').ok_or(AdminUsersError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(AdminUsersError::Unauthorized);
    }
    Ok(secret)
}
pub fn router(store: PermissionReadStore) -> Router {
    Router::new()
        .route("/core/v1/authz/catalog", get(native_catalog))
        .route("/core/v1/authz/capabilities", get(capabilities))
        .route("/api/authz/catalog", get(legacy_catalog))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "private, no-store".parse().expect("static header"),
                );
                response.headers_mut().insert(
                    header::VARY,
                    "Authorization".parse().expect("static header"),
                );
                response
            },
        ))
        .with_state(store)
}
async fn native_catalog(
    State(store): State<PermissionReadStore>,
    headers: HeaderMap,
    query: Result<Query<NoParameters>, QueryRejection>,
) -> ReadResult<Json<PermissionCatalog>> {
    let secret = bearer(&headers)?;
    query.map_err(|_| AdminUsersError::Invalid)?;
    Ok(Json(store.catalog(secret).await?))
}
async fn capabilities(
    State(store): State<PermissionReadStore>,
    headers: HeaderMap,
    query: Result<Query<NoParameters>, QueryRejection>,
) -> ReadResult<Json<EffectiveCapabilities>> {
    let secret = bearer(&headers)?;
    query.map_err(|_| AdminUsersError::Invalid)?;
    Ok(Json(store.capabilities(secret).await?))
}
async fn legacy_catalog(
    State(store): State<PermissionReadStore>,
    headers: HeaderMap,
    query: Result<Query<NoParameters>, QueryRejection>,
) -> Response {
    let result: ReadResult<PermissionCatalog> = async {
        let secret = bearer(&headers)?;
        query.map_err(|_| AdminUsersError::Invalid)?;
        store.catalog(secret).await
    }
    .await;
    match result {
        Ok(data) => Json(CatalogEnvelope {
            success: true,
            message: "",
            data,
        })
        .into_response(),
        Err(error) => {
            let (status, code) = match error {
                AdminUsersError::Unauthorized => (StatusCode::UNAUTHORIZED, "AUTHZ_UNAUTHORIZED"),
                AdminUsersError::Forbidden => (StatusCode::FORBIDDEN, "AUTHZ_FORBIDDEN"),
                AdminUsersError::Invalid => (StatusCode::BAD_REQUEST, "AUTHZ_INVALID"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "AUTHZ_UNAVAILABLE"),
            };
            (
                status,
                Json(serde_json::json!({"success":false,"message":code})),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_matches_fixed_main_shape_and_nine_actions() {
        let value = serde_json::to_value(CatalogEnvelope {
            success: true,
            message: "",
            data: catalog_schema(),
        })
        .unwrap();
        assert_eq!(value["success"], true);
        assert_eq!(value["message"], "");
        assert_eq!(value["data"].as_object().unwrap().len(), 2);
        let resources = value["data"]["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(
            resources
                .iter()
                .map(|r| r["actions"].as_array().unwrap().len())
                .sum::<usize>(),
            9
        );
        for resource in resources {
            assert_eq!(resource.as_object().unwrap().len(), 3);
            for action in resource["actions"].as_array().unwrap() {
                assert_eq!(action.as_object().unwrap().len(), 3);
                assert!(action["action"].is_string());
                assert!(action["label_key"].is_string());
                assert!(action["description_key"].is_string());
            }
        }
        for (index, expected) in [(0, 9), (1, 5)] {
            let role = &value["data"]["roles"][index];
            assert_eq!(role.as_object().unwrap().len(), 5);
            assert_eq!(
                role["grants"]
                    .as_object()
                    .unwrap()
                    .values()
                    .flat_map(|r| r.as_object().unwrap().values())
                    .filter(|v| v.as_bool() == Some(true))
                    .count(),
                expected
            );
        }
    }
    #[test]
    fn effective_catalog_keeps_explicit_deny_and_root_semantics() {
        let rows = [
            Override {
                subject_user_id: 7,
                action: Action::ChannelRead,
                effect: Effect::Deny,
            },
            Override {
                subject_user_id: 7,
                action: Action::AcquisitionExport,
                effect: Effect::Allow,
            },
        ];
        let admin = capability_matrix(PlatformRole::Admin, &rows);
        assert!(!admin["channel"]["read"]);
        assert!(admin["acquisition"]["export"]);
        let root = capability_matrix(PlatformRole::Root, &rows);
        assert!(root.values().flat_map(|r| r.values()).all(|v| *v));
        let user = capability_matrix(PlatformRole::User, &rows);
        assert!(user.values().flat_map(|r| r.values()).all(|v| !v));
    }
    #[test]
    fn duplicate_or_non_bearer_authority_is_rejected() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), Err(AdminUsersError::Unauthorized));
        headers.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), Err(AdminUsersError::Unauthorized));
        headers.insert(header::AUTHORIZATION, "Bearer session".parse().unwrap());
        assert_eq!(bearer(&headers), Ok("session"));
        headers.append(header::AUTHORIZATION, "Bearer second".parse().unwrap());
        assert_eq!(bearer(&headers), Err(AdminUsersError::Unauthorized));
    }
}
