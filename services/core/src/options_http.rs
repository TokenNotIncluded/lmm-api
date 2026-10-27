//! Read-only compatibility for the console settings API.
//!
//! Runtime flags must come from the same parsed configuration used at startup.
//! Persisting an option does not change host-owned TOML or model configuration.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;

use crate::identity::{CredentialKind, IdentityError, IdentityStore};

/// Public, effective configuration; never pass raw TOML or secret references.
#[derive(Clone)]
pub struct RuntimeOptions {
    pub server_address: String,
    pub registration_enabled: bool,
}

#[derive(Clone)]
pub struct OptionsHttp {
    pool: PgPool,
    identity: IdentityStore,
    runtime: RuntimeOptions,
}

impl OptionsHttp {
    pub fn new(pool: PgPool, identity: IdentityStore, runtime: RuntimeOptions) -> Self {
        Self {
            pool,
            identity,
            runtime,
        }
    }
}

pub fn router(state: OptionsHttp) -> Router {
    Router::new()
        .route("/api/option/", get(read).put(write_unavailable))
        .route("/api/option", get(read).put(write_unavailable))
        .route("/api/option/bulk", post(write_unavailable))
        .route("/api/option/validate", post(write_unavailable))
        .with_state(state)
}

#[derive(Serialize)]
struct SystemOption {
    key: String,
    value: String,
}

async fn authorize(state: &OptionsHttp, headers: &HeaderMap) -> Result<(), IdentityError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or(IdentityError::Unauthorized)?;
    if values.next().is_some() {
        return Err(IdentityError::Unauthorized);
    }
    let mut fields = value
        .to_str()
        .map_err(|_| IdentityError::Unauthorized)?
        .split_whitespace();
    let secret = match (fields.next(), fields.next(), fields.next()) {
        (Some(scheme), Some(secret), None) if scheme.eq_ignore_ascii_case("bearer") => secret,
        _ => return Err(IdentityError::Unauthorized),
    };
    let actor = state.identity.authorize(secret).await?;
    if actor.credential_kind != CredentialKind::Session || actor.platform_level < 6 {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

fn runtime_options(runtime: &RuntimeOptions) -> Vec<SystemOption> {
    // @context-fold begin effective-public-runtime-options
    [
        ("ServerAddress", runtime.server_address.clone()),
        ("RegisterEnabled", runtime.registration_enabled.to_string()),
    ]
    .into_iter()
    .map(|(key, value)| SystemOption {
        key: key.into(),
        value,
    })
    .collect()
    // @context-fold end effective-public-runtime-options
}

async fn read(State(state): State<OptionsHttp>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state, &headers).await {
        return auth_error(error);
    }
    // Only these persisted options are consumed by the current Core console.
    // @context-fold begin read-only-settings-projection
    let stored = sqlx::query_as::<_, (String, String)>(
        "SELECT key,value FROM core_console.options
         WHERE key IN ('Notice','Announcements','console_setting.announcements')
         ORDER BY key",
    )
    .fetch_all(&state.pool)
    .await;
    let stored = match stored {
        Ok(rows) => rows,
        Err(_) => {
            return response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"success":false,"code":"SYSTEM_OPTIONS_STORAGE_UNAVAILABLE",
                    "message":"Core console settings could not be read"}),
            );
        }
    };
    let mut data = runtime_options(&state.runtime);
    data.extend(
        stored
            .into_iter()
            .map(|(key, value)| SystemOption { key, value }),
    );
    response(
        StatusCode::OK,
        json!({
            "success":true,
            "message":"Partial read-only settings. Changes require the host configuration upgrade procedure.",
            "data":data,
            "capabilities":{
                "read_only":true,
                "partial":true,
                "model_price_locks":false,
                "trust_level_benefits":false
            }
        }),
    )
    // @context-fold end read-only-settings-projection
}

async fn write_unavailable(State(state): State<OptionsHttp>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state, &headers).await {
        return auth_error(error);
    }
    response(
        StatusCode::CONFLICT,
        json!({"success":false,"code":"SYSTEM_OPTIONS_READ_ONLY",
            "message":"These settings are read-only. Update the host-owned Core configuration through the configuration upgrade procedure; no changes were saved."}),
    )
}

fn auth_error(error: IdentityError) -> Response {
    let (status, code, message) = match error {
        IdentityError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "AUTH_UNAUTHORIZED",
            "Authentication is required",
        ),
        IdentityError::Forbidden => (
            StatusCode::FORBIDDEN,
            "SYSTEM_OPTIONS_FORBIDDEN",
            "A superadministrator console session is required",
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "SYSTEM_OPTIONS_AUTH_UNAVAILABLE",
            "Console authorization is unavailable",
        ),
    };
    response(
        status,
        json!({"success":false,"code":code,"message":message}),
    )
}

fn response(status: StatusCode, body: serde_json::Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    fn runtime() -> RuntimeOptions {
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            registration_enabled: false,
        }
    }

    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        secret: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(secret) = secret {
            request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn missing_authentication_does_not_touch_storage() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let app = router(OptionsHttp::new(
            pool.clone(),
            IdentityStore::from_pool(pool),
            runtime(),
        ));
        for (method, path) in [
            ("GET", "/api/option/"),
            ("PUT", "/api/option/"),
            ("POST", "/api/option/bulk"),
        ] {
            let (status, _) = call(&app, method, path, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[sqlx::test(migrations = false)]
    async fn authentic_settings_are_allowlisted_and_writes_fail(pool: PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        let identity = IdentityStore::from_pool(pool.clone());
        let root = identity.bootstrap_user(1, 6).await.unwrap();
        let admin = identity.bootstrap_user(2, 5).await.unwrap();
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES ('Notice','Existing notice'),('OAuthSecret','never expose'),('RegisterEnabled','true')")
            .execute(&pool).await.unwrap();
        let app = router(OptionsHttp::new(pool.clone(), identity, runtime()));
        let (status, _) = call(&app, "GET", "/api/option/", Some(&admin.secret)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) = call(&app, "GET", "/api/option/", Some(&root.secret)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["capabilities"]["partial"], true);
        let options: std::collections::BTreeMap<String, String> =
            serde_json::from_value::<Vec<serde_json::Value>>(body["data"].clone())
                .unwrap()
                .into_iter()
                .map(|item| {
                    (
                        item["key"].as_str().unwrap().into(),
                        item["value"].as_str().unwrap().into(),
                    )
                })
                .collect();
        assert_eq!(options["Notice"], "Existing notice");
        assert_eq!(options["RegisterEnabled"], "false");
        assert!(!options.contains_key("OAuthSecret"));
        assert_eq!(options["ServerAddress"], runtime().server_address);
        for (method, path) in [
            ("PUT", "/api/option/"),
            ("POST", "/api/option/bulk"),
            ("POST", "/api/option/validate"),
        ] {
            let (status, body) = call(&app, method, path, Some(&root.secret)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["code"], "SYSTEM_OPTIONS_READ_ONLY");
        }
        let notice: String =
            sqlx::query_scalar("SELECT value FROM core_console.options WHERE key='Notice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(notice, "Existing notice");
    }
}
