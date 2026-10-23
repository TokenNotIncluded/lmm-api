//! Native, reviewed routing publication. Database authority is restricted to
//! explicit functions; the runtime receives no direct routing-table writes.
mod owner_baseline;
mod plan;
#[cfg(test)]
mod tests;

use crate::accounts::{Account, AccountKind};
use crate::identity::{self, CredentialKind, IdentityError, permissions};
use crate::routing::{self, store::ReleaseDocument};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use plan::{Expected, FxSnapshot, Plan};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

pub const SCHEMA: &str = include_str!("../../schema/routing_admin.sql");
pub use owner_baseline::{BaselineReceipt, install_owner_baseline};
type Result<T> = std::result::Result<T, AdminError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminError {
    Unauthorized,
    Forbidden,
    Invalid,
    InvalidSource,
    UnsupportedPrice,
    Unrepresentable,
    ExchangeRateUnavailable,
    Conflict,
    Expired,
    TooLarge,
    Busy,
    BaselineRequired,
    Unavailable,
}
impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "routing administration: {self:?}")
    }
}
impl std::error::Error for AdminError {}
impl From<sqlx::Error> for AdminError {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "23505" | "40P01") => Self::Conflict,
            Some("42501") => Self::Forbidden,
            Some("P2701") => Self::Expired,
            Some("P2702") => Self::Busy,
            Some("P2703") => Self::BaselineRequired,
            Some("23514" | "22023") => Self::Invalid,
            _ => Self::Unavailable,
        }
    }
}
impl From<IdentityError> for AdminError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            IdentityError::Conflict => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}
impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "routing_admin_unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "routing_admin_forbidden"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "routing_admin_invalid"),
            Self::InvalidSource => (StatusCode::BAD_REQUEST, "routing_admin_invalid_source"),
            Self::UnsupportedPrice => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "routing_admin_unsupported_price",
            ),
            Self::Unrepresentable => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "routing_admin_inexact_exchange",
            ),
            Self::ExchangeRateUnavailable => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "routing_admin_exchange_unavailable",
            ),
            Self::Conflict => (StatusCode::CONFLICT, "routing_admin_conflict"),
            Self::Expired => (StatusCode::GONE, "routing_admin_preview_expired"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "routing_admin_too_large"),
            Self::Busy => (StatusCode::TOO_MANY_REQUESTS, "routing_admin_busy"),
            Self::BaselineRequired => (
                StatusCode::SERVICE_UNAVAILABLE,
                "routing_admin_baseline_required",
            ),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "routing_admin_unavailable"),
        };
        (
            status,
            [(header::CACHE_CONTROL, "no-store")],
            Json(json!({"success":false,"code":code,"message":code})),
        )
            .into_response()
    }
}
#[derive(Clone)]
pub struct RoutingAdmin {
    inner: Arc<Inner>,
}
struct Inner {
    pool: PgPool,
    routing: Arc<routing::Router>,
    host: ReleaseDocument,
    apply: Mutex<()>,
    durable_seen: AtomicBool,
}
impl RoutingAdmin {
    pub fn new(pool: PgPool, routing: Arc<routing::Router>) -> Result<Self> {
        let host = routing
            .catalog_document()
            .map_err(|_| AdminError::Unavailable)?;
        Ok(Self {
            inner: Arc::new(Inner {
                pool,
                routing,
                host,
                apply: Mutex::new(()),
                durable_seen: AtomicBool::new(false),
            }),
        })
    }
    /// Startup and next-admission barrier. No request may fall back to a file
    /// after durable state has been observed or a publication receipt exists.
    pub async fn refresh(&self) -> Result<()> {
        let mut connection = self.inner.pool.acquire().await?;
        self.refresh_in(&mut connection).await
    }
    pub(crate) async fn refresh_in(&self, connection: &mut PgConnection) -> Result<()> {
        let _guard = self.inner.apply.lock().await;
        let active: Option<Value> = sqlx::query_scalar("SELECT core_routing_admin.read_active()")
            .fetch_one(connection)
            .await?;
        let Some(active) = active else {
            if self.inner.durable_seen.load(Ordering::Acquire) {
                return Err(AdminError::Unavailable);
            }
            return Ok(());
        };
        self.inner.durable_seen.store(true, Ordering::Release);
        let document: ReleaseDocument =
            serde_json::from_value(active).map_err(|_| AdminError::Unavailable)?;
        self.install(document)
    }
    fn install(&self, document: ReleaseDocument) -> Result<()> {
        if !plan::same_host_contract(&self.inner.host, &document)? {
            return Err(AdminError::Unavailable);
        }
        // Canonicalize route ordering using the same compiler as serving.
        let canonical = document
            .clone()
            .into_router()
            .map_err(|_| AdminError::Unavailable)?
            .catalog_document()
            .map_err(|_| AdminError::Unavailable)?;
        let current = self
            .inner
            .routing
            .catalog_document()
            .map_err(|_| AdminError::Unavailable)?;
        if canonical.routing.version == current.routing.version {
            if canonical != current {
                return Err(AdminError::Unavailable);
            }
            return Ok(());
        }
        if canonical.routing.version < current.routing.version
            || canonical.pricing.version < current.pricing.version
        {
            return Err(AdminError::Unavailable);
        }
        canonical
            .apply(&self.inner.routing)
            .map_err(|_| AdminError::Unavailable)?;
        Ok(())
    }
    pub fn http_router(&self) -> Router {
        Router::new()
            .route("/api/core/routing-admin/catalog", get(catalog))
            .route("/api/core/routing-admin/prices/export", get(export_prices))
            .route(
                "/api/core/routing-admin/prices/preview",
                post(preview_prices),
            )
            .route(
                "/api/core/routing-admin/priorities/preview",
                post(preview_priorities),
            )
            .route(
                "/api/core/routing-admin/priorities/undo-preview",
                post(preview_undo),
            )
            .route("/api/core/routing-admin/commit", post(commit))
            .layer(DefaultBodyLimit::max(34 * 1024 * 1024))
            .with_state(self.clone())
    }
    async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.inner.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5000ms'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout='1000ms'")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    fn current(&self) -> Result<ReleaseDocument> {
        self.inner
            .routing
            .catalog_document()
            .map_err(|_| AdminError::Unavailable)
    }
    async fn save(
        &self,
        tx: &mut PgConnection,
        secret: &str,
        actor: &Authority,
        plan: Plan,
    ) -> Result<Value> {
        if !actor.allowed(&plan.kind) {
            return Err(AdminError::Forbidden);
        }
        let id = random_id()?;
        let stored = StoredPlan {
            contract: "core.routing-admin.preview.v1".into(),
            preview_id: id.clone(),
            authority: actor.binding.clone(),
            plan,
        };
        let payload = serde_json::to_string(&stored).map_err(|_| AdminError::Unavailable)?;
        if payload.len() > 32 * 1024 * 1024 {
            return Err(AdminError::TooLarge);
        }
        let digest = plan::hex(Sha256::digest(payload.as_bytes()).as_slice());
        let expiry: i64 =
            sqlx::query_scalar("SELECT core_routing_admin.store_preview($1,$2,$3,$4)")
                .bind(secret_digest(secret))
                .bind(&id)
                .bind(&payload)
                .bind(&digest)
                .fetch_one(tx)
                .await?;
        Ok(
            json!({"preview_id":id,"confirmation_sha256":digest,"expires_at":expiry.to_string(),"kind":stored.plan.kind,"expected":Expected::from_release(&stored.plan.before)?,"next":Expected::from_release(&stored.plan.after)?,"source_sha256":stored.plan.source_sha256,"exchange_rate":stored.plan.fx,"changes":stored.plan.changes}),
        )
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Binding {
    user_id: String,
    credential_id: String,
    auth_version: String,
    policy_revision: Option<String>,
    policy_sha256: Option<String>,
}
struct Authority {
    binding: Binding,
    can_prices: bool,
    can_priorities: bool,
}
impl Authority {
    fn allowed(&self, kind: &str) -> bool {
        match kind {
            "prices" => self.can_prices,
            "priorities" => self.can_priorities,
            _ => false,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPlan {
    contract: String,
    preview_id: String,
    authority: Binding,
    plan: Plan,
}

async fn authorize(c: &mut PgConnection, secret: &str) -> Result<Authority> {
    let principal = identity::authorize_in_transaction(c, secret).await?;
    if principal.credential_kind != CredentialKind::Session
        || principal.owner
            != (Account {
                kind: AccountKind::Personal,
                id: principal.user_id,
            })
    {
        return Err(AdminError::Forbidden);
    }
    let row = sqlx::query(
        "SELECT platform_role,auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
    )
    .bind(principal.user_id)
    .fetch_one(&mut *c)
    .await?;
    let role: String = row.try_get("platform_role")?;
    let role = match role.as_str() {
        "superadmin" => permissions::PlatformRole::Root,
        "admin" => permissions::PlatformRole::Admin,
        _ => return Err(AdminError::Forbidden),
    };
    let auth_version: i64 = row.try_get("auth_version")?;
    let has_policy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.permission_policies WHERE user_id=$1) OR EXISTS(SELECT FROM core_identity.privileged_import_facts WHERE user_id=$1)")
        .bind(principal.user_id).fetch_one(&mut *c).await?;
    let mut binding = Binding {
        user_id: principal.user_id.to_string(),
        credential_id: principal.credential_id.to_string(),
        auth_version: auth_version.to_string(),
        policy_revision: None,
        policy_sha256: None,
    };
    let (can_prices, can_priorities) = if has_policy {
        let loaded = permissions::load_complete_policy(c, principal.user_id)
            .await
            .map_err(|_| AdminError::Forbidden)?;
        binding.policy_revision = Some(loaded.revision.to_string());
        binding.policy_sha256 = Some(plan::hex(&loaded.sha256));
        let actor = permissions::Actor {
            user_id: principal.user_id,
            personal_account_id: principal.owner_account_id,
            role,
            credential: permissions::Credential::Session,
            active: true,
            account_active: true,
            auth_version,
            credential_auth_version: auth_version,
            developer: permissions::DeveloperState::Granted,
        };
        (
            permissions::decide(
                &actor,
                &loaded.as_policy(),
                permissions::Action::RootSettings,
                permissions::Scope::Platform(permissions::PlatformResource::Settings),
                None,
            )
            .allowed(),
            permissions::decide(
                &actor,
                &loaded.as_policy(),
                permissions::Action::ChannelWrite,
                permissions::Scope::Platform(permissions::PlatformResource::Channel),
                None,
            )
            .allowed(),
        )
    } else {
        (role == permissions::PlatformRole::Root, true)
    };
    if !can_prices && !can_priorities {
        return Err(AdminError::Forbidden);
    }
    Ok(Authority {
        binding,
        can_prices,
        can_priorities,
    })
}
async fn fx(c: &mut PgConnection) -> Result<Option<FxSnapshot>> {
    let row = sqlx::query(
        "SELECT key,value,revision FROM core_console.options WHERE key='USDExchangeRate'",
    )
    .fetch_optional(c)
    .await?;
    row.map(|row| {
        Ok(FxSnapshot {
            key: row.try_get("key")?,
            raw: row.try_get("value")?,
            revision: row.try_get::<i64, _>("revision")?.to_string(),
        })
    })
    .transpose()
}
fn secret_digest(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}
fn bearer(headers: &HeaderMap) -> Result<&str> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(AdminError::Unauthorized);
    }
    let (scheme, secret) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .ok_or(AdminError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(AdminError::Unauthorized);
    }
    Ok(secret)
}
fn random_id() -> Result<String> {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| AdminError::Unavailable)?;
    Ok(plan::hex(&bytes))
}
fn success(value: Value) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"success":true,"data":value})),
    )
        .into_response()
}
async fn catalog(State(state): State<RoutingAdmin>, headers: HeaderMap) -> Result<Response> {
    let secret = bearer(&headers)?;
    let mut tx = state.begin().await?;
    let actor = authorize(&mut tx, secret).await?;
    state.refresh_in(&mut tx).await?;
    let document = state.current()?;
    let fx = fx(&mut tx).await?;
    let result = plan::catalog(&document, fx, actor.can_prices, actor.can_priorities)?;
    tx.commit().await?;
    Ok(success(result))
}
async fn export_prices(State(state): State<RoutingAdmin>, headers: HeaderMap) -> Result<Response> {
    let secret = bearer(&headers)?;
    let mut tx = state.begin().await?;
    let actor = authorize(&mut tx, secret).await?;
    if !actor.can_prices {
        return Err(AdminError::Forbidden);
    }
    state.refresh_in(&mut tx).await?;
    let document =
        serde_json::to_string_pretty(&plan::PriceSource::from_release(&state.current()?))
            .map_err(|_| AdminError::Unavailable)?;
    tx.commit().await?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=core-native-prices-v1.json",
            ),
        ],
        document,
    )
        .into_response())
}
async fn preview_prices(
    State(state): State<RoutingAdmin>,
    headers: HeaderMap,
    Json(input): Json<plan::PricePreviewRequest>,
) -> Result<Response> {
    let secret = bearer(&headers)?;
    let mut tx = state.begin().await?;
    let actor = authorize(&mut tx, secret).await?;
    if !actor.can_prices {
        return Err(AdminError::Forbidden);
    }
    state.refresh_in(&mut tx).await?;
    let plan = plan::prices(&state.current()?, input, fx(&mut tx).await?)?;
    let result = state.save(&mut tx, secret, &actor, plan).await?;
    tx.commit().await?;
    Ok(success(result))
}
async fn preview_priorities(
    State(state): State<RoutingAdmin>,
    headers: HeaderMap,
    Json(input): Json<plan::PriorityPreviewRequest>,
) -> Result<Response> {
    let secret = bearer(&headers)?;
    let mut tx = state.begin().await?;
    let actor = authorize(&mut tx, secret).await?;
    if !actor.can_priorities {
        return Err(AdminError::Forbidden);
    }
    state.refresh_in(&mut tx).await?;
    let plan = plan::priorities(&state.current()?, input)?;
    let result = state.save(&mut tx, secret, &actor, plan).await?;
    tx.commit().await?;
    Ok(success(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoRequest {
    expected: Expected,
    operation_id: String,
}
async fn preview_undo(
    State(state): State<RoutingAdmin>,
    headers: HeaderMap,
    Json(input): Json<UndoRequest>,
) -> Result<Response> {
    let secret = bearer(&headers)?;
    let mut tx = state.begin().await?;
    let actor = authorize(&mut tx, secret).await?;
    if !actor.can_priorities {
        return Err(AdminError::Forbidden);
    }
    state.refresh_in(&mut tx).await?;
    let current = state.current()?;
    input.expected.verify(&current)?;
    let payload: String = sqlx::query_scalar("SELECT core_routing_admin.read_operation($1,$2)")
        .bind(secret_digest(secret))
        .bind(&input.operation_id)
        .fetch_one(&mut *tx)
        .await?;
    let stored: StoredPlan = serde_json::from_str(&payload).map_err(|_| AdminError::Unavailable)?;
    if stored.plan.kind != "priorities"
        || Expected::from_release(&stored.plan.after)? != input.expected
    {
        return Err(AdminError::Conflict);
    }
    let changes = stored
        .plan
        .changes
        .as_array()
        .ok_or(AdminError::Unavailable)?;
    let route_id = changes
        .first()
        .and_then(|v| v["route_id"].as_str())
        .ok_or(AdminError::Unavailable)?
        .to_owned();
    let changes = changes
        .iter()
        .map(|change| {
            Ok(plan::PriorityChange {
                target_id: change["target_id"]
                    .as_str()
                    .ok_or(AdminError::Unavailable)?
                    .into(),
                priority: u16::try_from(change["before"].as_u64().ok_or(AdminError::Unavailable)?)
                    .map_err(|_| AdminError::Unavailable)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let plan = plan::priorities(
        &current,
        plan::PriorityPreviewRequest {
            expected: input.expected,
            route_id,
            changes,
        },
    )?;
    let result = state.save(&mut tx, secret, &actor, plan).await?;
    tx.commit().await?;
    Ok(success(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitRequest {
    preview_id: String,
    confirmation_sha256: String,
    operation_id: String,
}
async fn commit(
    State(state): State<RoutingAdmin>,
    headers: HeaderMap,
    Json(input): Json<CommitRequest>,
) -> Result<Response> {
    if [
        &input.preview_id,
        &input.confirmation_sha256,
        &input.operation_id,
    ]
    .iter()
    .any(|s| {
        s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(AdminError::Invalid);
    }
    let secret = bearer(&headers)?;
    let mut tx = state.begin().await?;
    let actor = authorize(&mut tx, secret).await?;
    let payload: String = sqlx::query_scalar("SELECT core_routing_admin.read_preview($1,$2)")
        .bind(secret_digest(secret))
        .bind(&input.preview_id)
        .fetch_one(&mut *tx)
        .await?;
    if plan::hex(Sha256::digest(payload.as_bytes()).as_slice()) != input.confirmation_sha256 {
        return Err(AdminError::Conflict);
    }
    let stored: StoredPlan = serde_json::from_str(&payload).map_err(|_| AdminError::Unavailable)?;
    if stored.authority != actor.binding || !actor.allowed(&stored.plan.kind) {
        return Err(AdminError::Forbidden);
    }
    if !plan::same_host_contract(&state.inner.host, &stored.plan.after)? {
        return Err(AdminError::Unavailable);
    }
    stored
        .plan
        .after
        .clone()
        .into_router()
        .map_err(|_| AdminError::Unavailable)?;
    let created: bool =
        sqlx::query_scalar("SELECT core_routing_admin.publish_preview($1,$2,$3,$4)")
            .bind(secret_digest(secret))
            .bind(&input.preview_id)
            .bind(&input.confirmation_sha256)
            .bind(&input.operation_id)
            .fetch_one(&mut *tx)
            .await?;
    if created {
        crate::notifications::publish_routing_change(
            &mut tx,
            &stored.plan.before.routing,
            &stored.plan.after.routing,
        )
        .await?;
    }
    tx.commit().await?;
    // A commit acknowledgement describes durable success even if this node's
    // refresh fails. All next admissions still fail closed at refresh().
    let ready = state.refresh().await.is_ok();
    Ok(success(
        json!({"operation_id":input.operation_id,"published":Expected::from_release(&stored.plan.after)?,"applied_on_this_node":ready,"replayed":!created}),
    ))
}
