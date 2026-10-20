//! Read-only public and personal-Session price metadata. No provider requests,
//! reservations, mutations, OAuth adaptation, or supplier health claims.
//! Visibility matches go-v0.2.118 middleware/header_nav.go and pricing.go.
use crate::{
    identity::{self, CredentialKind, IdentityError},
    model_http::pricing::{PricingProjection, PricingSurface},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct PricingHttp {
    pool: PgPool,
    surface: Option<Arc<PricingSurface>>,
}
#[derive(Debug)]
pub enum Error {
    Unauthorized,
    Forbidden,
    Invalid,
    Unavailable,
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<IdentityError> for Error {
    fn from(e: IdentityError) -> Self {
        match e {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            IdentityError::Invalid => Self::Invalid,
            _ => Self::Unavailable,
        }
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "pricing_unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "pricing_forbidden"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "pricing_invalid"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "pricing_unavailable"),
        };
        (status, Json(json!({"error":{"code":code}}))).into_response()
    }
}
#[derive(Serialize)]
pub struct PricingResponse {
    pub schema_version: u32,
    pub visibility: &'static str,
    pub observed_at: u64,
    pub credits_per_usd: i64,
    pub rate_basis: &'static str,
    pub invocation_authorized: bool,
    #[serde(flatten)]
    pub catalog: PricingProjection,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeInput {
    models: Vec<String>,
}
#[derive(Serialize)]
struct RuntimeResponse {
    schema_version: u32,
    visibility: &'static str,
    observed_at: u64,
    data: BTreeMap<String, RuntimeState>,
}
#[derive(Serialize)]
struct RuntimeState {
    status: &'static str,
    source: &'static str,
    observed_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Visibility {
    enabled: bool,
    require_auth: bool,
}
impl Default for Visibility {
    fn default() -> Self {
        Self {
            enabled: true,
            require_auth: false,
        }
    }
}
fn flag(value: &Value, fallback: bool) -> bool {
    match value {
        Value::Bool(v) => *v,
        Value::String(v) => match v.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => fallback,
        },
        Value::Number(v) => {
            if v.as_f64() == Some(1.0) {
                true
            } else if v.as_f64() == Some(0.0) {
                false
            } else {
                fallback
            }
        }
        _ => fallback,
    }
}
fn visibility(raw: Option<&str>) -> Visibility {
    let fallback = Visibility::default();
    let Some(raw) = raw.filter(|v| !v.trim().is_empty()) else {
        return fallback;
    };
    let Ok(doc) = serde_json::from_str::<Value>(raw) else {
        return fallback;
    };
    let Some(v) = doc.get("pricing") else {
        return fallback;
    };
    if let Some(object) = v.as_object() {
        Visibility {
            enabled: object.get("enabled").map_or(true, |v| flag(v, true)),
            require_auth: object.get("requireAuth").is_some_and(|v| flag(v, false)),
        }
    } else {
        Visibility {
            enabled: flag(v, true),
            require_auth: false,
        }
    }
}
impl PricingHttp {
    /// Host supplies the actual startup-validated gateway surface and runtime pool.
    pub fn new(pool: PgPool, surface: Arc<PricingSurface>) -> Self {
        Self {
            pool,
            surface: Some(surface),
        }
    }
    /// A metadata projection failure must not stop an otherwise valid gateway.
    /// Requests fail explicitly with 503, rather than exposing an empty catalog.
    pub fn from_gateway(pool: PgPool, gateway: &crate::model_http::ModelGateway) -> Self {
        Self {
            pool,
            surface: PricingSurface::from_gateway(gateway).ok().map(Arc::new),
        }
    }
    pub fn unavailable(pool: PgPool) -> Self {
        Self {
            pool,
            surface: None,
        }
    }
    pub fn router(self) -> Router {
        Router::new()
            .route("/core/v1/pricing/public", get(public))
            .route("/core/v1/pricing/session", get(session))
            .route("/core/v1/pricing/runtime", post(runtime))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .layer(middleware::map_response(no_store))
            .with_state(self)
    }
    async fn read(&self, secret: Option<&str>) -> Result<PricingResponse, Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let raw:Option<Option<String>>=sqlx::query_scalar("SELECT CASE WHEN octet_length(value)<=65536 THEN value ELSE NULL END FROM core_console.options WHERE key='HeaderNavModules'").fetch_optional(&mut *tx).await?;
        let raw = match raw {
            None => None,
            Some(Some(v)) => Some(v),
            Some(None) => return Err(Error::Unavailable),
        };
        let policy = visibility(raw.as_deref());
        if !policy.enabled || (secret.is_none() && policy.require_auth) {
            return Err(Error::Forbidden);
        }
        let (allowed, model_filter) = if let Some(secret) = secret {
            let actor = identity::authenticate_readonly_in_transaction(&mut tx, secret).await?;
            if actor.credential_kind != CredentialKind::Session
                || actor.owner.kind != crate::accounts::AccountKind::Personal
            {
                return Err(Error::Forbidden);
            }
            let groups:Vec<String>=sqlx::query_scalar("SELECT group_name FROM core_identity.user_routing_groups WHERE user_id=$1 AND enabled ORDER BY group_name").bind(actor.user_id).fetch_all(&mut *tx).await?;
            let models = crate::abuse::public_models_in_transaction(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Unavailable)?;
            (Some(groups), models)
        } else {
            (None, None)
        };
        let timestamp: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let observed_at = u64::try_from(timestamp).map_err(|_| Error::Unavailable)?;
        let catalog = self
            .surface
            .as_ref()
            .ok_or(Error::Unavailable)?
            .project(allowed.as_deref(), model_filter.as_deref())
            .map_err(|_| Error::Unavailable)?;
        tx.commit().await?;
        Ok(PricingResponse {
            schema_version: 1,
            visibility: if secret.is_some() {
                "session"
            } else {
                "public"
            },
            observed_at,
            credits_per_usd: crate::billing::CREDITS_PER_USD,
            rate_basis: "effective_group_rate",
            invocation_authorized: false,
            catalog,
        })
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str, Error> {
    if ["x-api-key", "x-goog-api-key", "x-auth-session"]
        .iter()
        .any(|k| headers.contains_key(*k))
    {
        return Err(Error::Invalid);
    }
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(Error::Unauthorized);
    }
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    let mut pieces = value.split_ascii_whitespace();
    match (pieces.next(), pieces.next(), pieces.next()) {
        (Some(scheme), Some(secret), None)
            if scheme.eq_ignore_ascii_case("Bearer") && secret.starts_with("lmms_") =>
        {
            Ok(secret)
        }
        (Some(_), Some(_), None) => Err(Error::Forbidden),
        _ => Err(Error::Unauthorized),
    }
}
fn no_query(query: Option<String>) -> Result<(), Error> {
    if query.is_some() {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
async fn public(
    State(state): State<PricingHttp>,
    RawQuery(query): RawQuery,
) -> Result<Json<PricingResponse>, Error> {
    no_query(query)?;
    state.read(None).await.map(Json)
}
async fn session(
    State(state): State<PricingHttp>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Json<PricingResponse>, Error> {
    no_query(query)?;
    state.read(Some(bearer(&headers)?)).await.map(Json)
}
async fn runtime(
    State(state): State<PricingHttp>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    Json(input): Json<RuntimeInput>,
) -> Result<Json<RuntimeResponse>, Error> {
    no_query(query)?;
    validate_models(&input.models)?;
    let secret = if headers.contains_key(header::AUTHORIZATION) {
        Some(bearer(&headers)?)
    } else {
        None
    };
    let view = state.read(secret).await?;
    let data = input
        .models
        .into_iter()
        .map(|model| {
            let entries: Vec<_> = view
                .catalog
                .models
                .iter()
                .filter(|m| m.model == model)
                .collect();
            // Unknown means configuration exists; it never implies a live upstream
            // or permission to spend. Avoid revealing inaccessible group/model names.
            let status = if entries.is_empty() {
                if secret.is_some() {
                    "no_access"
                } else {
                    "not_listed"
                }
            } else if entries.iter().all(|m| m.billing_status == "unavailable") {
                "unavailable"
            } else {
                "unknown"
            };
            let source = if status == "no_access" {
                "account_permissions"
            } else {
                "routing_configuration"
            };
            (
                model,
                RuntimeState {
                    status,
                    source,
                    observed_at: view.observed_at,
                },
            )
        })
        .collect();
    Ok(Json(RuntimeResponse {
        schema_version: 1,
        visibility: view.visibility,
        observed_at: view.observed_at,
        data,
    }))
}
fn validate_models(models: &[String]) -> Result<(), Error> {
    if models.is_empty()
        || models.len() > 200
        || models
            .iter()
            .any(|m| m.is_empty() || m.len() > 256 || m.chars().any(char::is_control))
        || models
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != models.len()
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
async fn no_store(mut response: Response) -> Response {
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::VARY, "Authorization, Cookie"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, axum::http::HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pricing_visibility_matches_frozen_main() {
        assert_eq!(
            visibility(None),
            Visibility {
                enabled: true,
                require_auth: false
            }
        );
        for raw in [
            r#"{"pricing":false}"#,
            r#"{"pricing":"false"}"#,
            r#"{"pricing":0}"#,
            r#"{"pricing":0.0}"#,
            r#"{"pricing":{"enabled":false}}"#,
        ] {
            assert!(!visibility(Some(raw)).enabled)
        }
        for raw in [
            r#"{"pricing":{"requireAuth":true}}"#,
            r#"{"pricing":{"requireAuth":"1"}}"#,
            r#"{"pricing":{"requireAuth":1.0}}"#,
        ] {
            assert!(visibility(Some(raw)).require_auth)
        }
        assert_eq!(visibility(Some("malformed")), Visibility::default())
    }
    #[test]
    fn pricing_credentials_do_not_adapt_other_authorities() {
        let mut h = HeaderMap::new();
        assert!(matches!(bearer(&h), Err(Error::Unauthorized)));
        for token in ["lmm_at_delegated", "lmmk_apikey", "opaque"] {
            h.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            assert!(matches!(bearer(&h), Err(Error::Forbidden)))
        }
        h.insert(header::AUTHORIZATION, "Bearer lmms_native".parse().unwrap());
        assert_eq!(bearer(&h).unwrap(), "lmms_native");
        h.append(header::AUTHORIZATION, "Bearer lmms_other".parse().unwrap());
        assert!(matches!(bearer(&h), Err(Error::Unauthorized)))
    }
    #[test]
    fn pricing_runtime_bounds_unknown_inputs() {
        assert!(validate_models(&["model".into()]).is_ok());
        assert!(validate_models(&[]).is_err());
        assert!(validate_models(&["model".into(), "model".into()]).is_err());
        assert!(validate_models(&vec!["model".into(); 201]).is_err());
        assert!(validate_models(&["bad\nname".into()]).is_err())
    }
}
#[cfg(test)]
#[path = "pricing_http_tests.rs"]
mod pg_tests;
