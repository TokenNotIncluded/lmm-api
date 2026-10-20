//! Axum HTTP handler implementation for HumanGrant routes.

use super::assets::{CLIENT_JS, STYLE_CSS, UI_JS, render_page};
use super::engine::HumanGrantEngine;
use super::signature::{fingerprint, verify_signature};
use super::store::HumanGrantTx;
use super::types::*;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;

pub type SharedEngine = Arc<HumanGrantEngine>;

fn json_response<T: serde::Serialize>(status: StatusCode, value: &T) -> Response {
    let mut resp = (status, Json(value)).into_response();
    apply_security_headers(&mut resp);
    resp
}

fn error_response(err: ProtocolError) -> Response {
    let status = StatusCode::from_u16(err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let retry = err.retry_after_seconds;
    let mut resp = (status, Json(err)).into_response();
    apply_security_headers(&mut resp);
    if retry > 0
        && let Ok(val) = retry.to_string().parse()
    {
        resp.headers_mut().insert(header::RETRY_AFTER, val);
    }
    resp
}

fn apply_security_headers(resp: &mut Response) {
    let headers = resp.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
    headers.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
}

fn html_response(html: String, cookie: Option<String>) -> Response {
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        )
        .body(Body::from(html))
        .unwrap();

    apply_security_headers(&mut resp);
    if let Some(c) = cookie
        && let Ok(val) = c.parse()
    {
        resp.headers_mut().append(header::SET_COOKIE, val);
    }
    resp
}

fn make_cookie_header(name: &str, value: &str, max_age: i64, secure: bool) -> String {
    if secure {
        format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict; Secure")
    } else {
        format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict")
    }
}

pub fn router(engine: SharedEngine) -> Router {
    Router::new()
        .route("/humangrant/config", get(get_config))
        .route("/humangrant/client.js", get(get_client_js))
        .route("/humangrant/ui.js", get(get_ui_js))
        .route("/humangrant/style.css", get(get_style_css))
        .route("/humangrant", get(get_agent_page))
        .route("/human/grants", get(get_grants_page))
        .route("/human/{link}", get(get_human_page))
        .route(
            "/api/user/auth/humangrant/settings",
            get(get_settings).put(put_settings),
        )
        .route("/api/user/auth/humangrant/grants", get(list_grants))
        .route(
            "/api/user/auth/humangrant/grants/{id}/revoke",
            post(revoke_grant),
        )
        .route(
            "/api/user/auth/humangrant/human/{link}/begin",
            post(human_begin),
        )
        .route(
            "/api/user/auth/humangrant/human/{link}/verify",
            post(human_verify),
        )
        .route(
            "/api/user/auth/humangrant/human/{link}/approve",
            post(human_approve),
        )
        .route(
            "/api/user/auth/humangrant/human/{link}/reject",
            post(human_reject),
        )
        .route("/v1/requests", post(create_request))
        .route("/v1/requests/{id}", get(poll_request))
        .route("/v1/requests/{id}/status.md", get(poll_request_markdown))
        .route("/v1/requests/{id}/claim", post(claim_request))
        .route("/v1/requests/{id}/cancel", post(cancel_request))
        .route("/v1/requests/{id}/email", post(email_request))
        .route("/v1/session", get(get_session))
        .route("/v1/accounts/{id}/profile", get(get_profile))
        .with_state(engine)
}

async fn get_config(State(engine): State<SharedEngine>) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };
    let settings = match engine.get_settings(&mut tx, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    let _ = tx.commit().await;

    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "enabled": settings.enabled,
            "ready": !engine.origin.is_empty(),
            "client_id": CLIENT_ID,
            "target_service": SERVICE_ID,
            "protocol_version": VERSION,
        }),
    )
}

async fn get_client_js() -> Response {
    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        CLIENT_JS,
    )
        .into_response();
    apply_security_headers(&mut resp);
    resp
}

async fn get_ui_js() -> Response {
    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        UI_JS,
    )
        .into_response();
    apply_security_headers(&mut resp);
    resp
}

async fn get_style_css() -> Response {
    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
        .into_response();
    apply_security_headers(&mut resp);
    resp
}

async fn get_agent_page(State(engine): State<SharedEngine>) -> Response {
    let html = render_page(
        "agent",
        &serde_json::json!({
            "origin": engine.origin,
            "client_id": CLIENT_ID,
            "target_service": SERVICE_ID,
            "action": PROFILE_READ,
        }),
    );
    html_response(html, None)
}

async fn get_grants_page() -> Response {
    let html = render_page("grants", &serde_json::json!({}));
    html_response(html, None)
}

async fn get_human_page(State(engine): State<SharedEngine>, Path(link): Path<String>) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let req_id = match tx.get::<String>("human-link", &link, now).await {
        Ok(Some(id)) => id,
        _ => return error_response(failure(404, "not_found")),
    };

    let req = match tx.get::<Request>("request", &req_id, now).await {
        Ok(Some(r)) => r,
        _ => return error_response(failure(404, "not_found")),
    };

    let (flow, key) = match engine
        .new_flow(&mut tx, &req.id, Human::default(), false, now)
        .await
    {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    let cookie = make_cookie_header(
        engine.flow_cookie_name(),
        &key,
        600,
        !engine.local || !engine.origin.starts_with("http:"),
    );

    let html = render_page(
        "human",
        &serde_json::json!({
            "link": link,
            "request_id": req.id,
            "actor": req.actor,
            "public_key": req.public_key,
            "service": req.input.target_service,
            "client": req.input.client_id,
            "purpose": req.input.purpose,
            "permissions": req.input.requested_permissions,
            "expected_human_id": req.input.expected_human_id,
            "expires_at": req.expires,
            "max_grant_expires_at": now + req.input.session_ttl_seconds,
            "phase": req.phase,
            "csrf": flow.csrf,
        }),
    );

    html_response(html, Some(cookie))
}

async fn get_settings(State(engine): State<SharedEngine>, headers: HeaderMap) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let refresh_cookie = engine.parse_cookie_value(&headers, "new_api_refresh");
    let (human, account) = match tx.authenticate_human(refresh_cookie, now).await {
        Ok(Some(res)) => res,
        _ => return error_response(failure(403, "forbidden")),
    };

    if !account.root {
        return error_response(failure(403, "forbidden"));
    }

    let settings = match engine.get_settings(&mut tx, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };

    let (flow, key) = match engine.new_flow(&mut tx, "settings", human, true, now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    let cookie = make_cookie_header(
        engine.flow_cookie_name(),
        &key,
        600,
        !engine.local || !engine.origin.starts_with("http:"),
    );
    let mut resp = json_response(
        StatusCode::OK,
        &serde_json::json!({
            "enabled": settings.enabled,
            "csrf": flow.csrf,
        }),
    );
    resp.headers_mut()
        .insert(header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

#[derive(serde::Deserialize)]
struct SettingsInput {
    enabled: Option<bool>,
}

async fn put_settings(
    State(engine): State<SharedEngine>,
    headers: HeaderMap,
    Json(input): Json<SettingsInput>,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let refresh_cookie = engine.parse_cookie_value(&headers, "new_api_refresh");
    let (human, account) = match tx.authenticate_human(refresh_cookie, now).await {
        Ok(Some(res)) => res,
        _ => return error_response(failure(403, "forbidden")),
    };

    if !account.root {
        return error_response(failure(403, "forbidden"));
    }

    let flow = match engine.check_flow(&mut tx, &headers, "settings", now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if flow.human.id != human.id {
        return error_response(failure(403, "csrf_failed"));
    }

    let Some(new_enabled) = input.enabled else {
        return error_response(failure(400, "invalid_request"));
    };

    let mut settings = match tx.get_for_update::<Settings>("settings", "site", now).await {
        Ok(Some(s)) => s,
        Ok(None) => Settings::default(),
        Err(e) => return error_response(e),
    };

    if settings.enabled != new_enabled {
        settings.enabled = new_enabled;
        settings.epoch += 1;
        if let Err(e) = tx.put("settings", "site", "", 0, &settings).await {
            return error_response(e);
        }
        if let Err(e) = engine
            .audit(&mut tx, "settings", &human.id, &human.id, "site", now)
            .await
        {
            return error_response(e);
        }
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "enabled": settings.enabled,
        }),
    )
}

async fn list_grants(State(engine): State<SharedEngine>, headers: HeaderMap) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let refresh_cookie = engine.parse_cookie_value(&headers, "new_api_refresh");
    let (human, _) = match tx.authenticate_human(refresh_cookie, now).await {
        Ok(Some(res)) => res,
        _ => return error_response(failure(401, "human_login_required")),
    };

    let settings = match engine.get_settings(&mut tx, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };

    let rows = match tx.list::<Grant>("grant", &human.id, 200, now).await {
        Ok(r) => r,
        Err(e) => return error_response(e),
    };

    let mut list = Vec::new();
    for g in &rows {
        let status = engine
            .grant_status(&mut tx, g, &settings, now)
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        list.push(serde_json::json!({
            "grant_id": g.grant_id,
            "request_id": g.request_id,
            "grantor_id": g.human.id,
            "actor_id": g.actor_id,
            "public_key_id": g.actor_id,
            "client_id": g.client_id,
            "target_service": g.target_service,
            "permissions": g.permissions,
            "created_at": g.created_at,
            "expires_at": g.expires_at,
            "revoked_at": g.revoked_at,
            "status": status,
            "audit_id": g.audit_id,
        }));
    }

    let (flow, key) = match engine.new_flow(&mut tx, "grants", human, true, now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    let cookie = make_cookie_header(
        engine.flow_cookie_name(),
        &key,
        600,
        !engine.local || !engine.origin.starts_with("http:"),
    );
    let mut resp = json_response(
        StatusCode::OK,
        &serde_json::json!({
            "grants": list,
            "csrf": flow.csrf,
            "truncated": rows.len() == 200,
        }),
    );
    resp.headers_mut()
        .insert(header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

async fn revoke_grant(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let refresh_cookie = engine.parse_cookie_value(&headers, "new_api_refresh");
    let (human, _) = match tx.authenticate_human(refresh_cookie, now).await {
        Ok(Some(res)) => res,
        _ => return error_response(failure(401, "human_login_required")),
    };

    let flow = match engine.check_flow(&mut tx, &headers, "grants", now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if flow.human.id != human.id {
        return error_response(failure(403, "csrf_failed"));
    }

    let mut grant = match tx.get_for_update::<Grant>("grant", &id, now).await {
        Ok(Some(g)) => g,
        Ok(None) => return error_response(failure(404, "not_found")),
        Err(e) => return error_response(e),
    };

    if grant.human.id != human.id {
        return error_response(failure(403, "forbidden"));
    }

    if grant.revoked_at == 0 {
        grant.revoked_at = now;
        if let Err(e) = tx
            .put(
                "grant",
                &id,
                &human.id,
                grant.expires_at + RETENTION_SECONDS,
                &grant,
            )
            .await
        {
            return error_response(e);
        }
        if let Err(e) = engine
            .audit(&mut tx, "revoke", &grant.actor_id, &human.id, &id, now)
            .await
        {
            return error_response(e);
        }
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(StatusCode::OK, &serde_json::json!({ "status": "revoked" }))
}

async fn human_begin(
    State(engine): State<SharedEngine>,
    Path(link): Path<String>,
    headers: HeaderMap,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let req_id = match tx.get::<String>("human-link", &link, now).await {
        Ok(Some(id)) => id,
        _ => return error_response(failure(404, "not_found")),
    };

    let mut req = match tx.get_for_update::<Request>("request", &req_id, now).await {
        Ok(Some(r)) => r,
        _ => return error_response(failure(404, "not_found")),
    };

    let _flow = match engine.check_flow(&mut tx, &headers, &req.id, now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if req.phase == "waiting_human" {
        req.phase = "authenticating_human".to_string();
        req.updated = now;
        if let Err(e) = tx
            .put(
                "request",
                &req.id,
                &req.actor,
                req.created + RETENTION_SECONDS,
                &req,
            )
            .await
        {
            return error_response(e);
        }
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({ "status": "authenticating_human" }),
    )
}

async fn human_verify(
    State(engine): State<SharedEngine>,
    Path(link): Path<String>,
    headers: HeaderMap,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let req_id = match tx.get::<String>("human-link", &link, now).await {
        Ok(Some(id)) => id,
        _ => return error_response(failure(404, "not_found")),
    };

    let mut req = match tx.get_for_update::<Request>("request", &req_id, now).await {
        Ok(Some(r)) => r,
        _ => return error_response(failure(404, "not_found")),
    };

    let _flow = match engine.check_flow(&mut tx, &headers, &req.id, now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    let refresh_cookie = engine.parse_cookie_value(&headers, "new_api_refresh");
    let (human, account) = match tx.authenticate_human(refresh_cookie, now).await {
        Ok(Some(res)) => res,
        _ => return error_response(failure(401, "human_login_required")),
    };

    if let Some(expected) = &req.input.expected_human_id
        && expected != &human.id
    {
        return error_response(failure(403, "expected_human_mismatch"));
    }

    if req.phase == "waiting_human" {
        return error_response(failure(409, "invalid_state"));
    }

    let cookie_key = engine
        .parse_cookie_value(&headers, engine.flow_cookie_name())
        .unwrap_or("");
    let old_flow_id = fingerprint(cookie_key.as_bytes());
    let _ = tx.delete("flow", &old_flow_id).await;

    let (rotated, key) = match engine
        .new_flow(&mut tx, &req.id, human.clone(), true, now)
        .await
    {
        Ok(res) => res,
        Err(e) => return error_response(e),
    };

    if req.phase != "awaiting_consent" {
        req.phase = "awaiting_consent".to_string();
        req.updated = now;
        if let Err(e) = tx
            .put(
                "request",
                &req.id,
                &req.actor,
                req.created + RETENTION_SECONDS,
                &req,
            )
            .await
        {
            return error_response(e);
        }
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    let cookie = make_cookie_header(
        engine.flow_cookie_name(),
        &key,
        600,
        !engine.local || !engine.origin.starts_with("http:"),
    );
    let mut resp = json_response(
        StatusCode::OK,
        &serde_json::json!({
            "status": "awaiting_consent",
            "human_id": human.id,
            "account": account,
            "csrf": rotated.csrf,
        }),
    );
    resp.headers_mut()
        .insert(header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

async fn human_approve(
    State(engine): State<SharedEngine>,
    Path(link): Path<String>,
    headers: HeaderMap,
    Json(consent): Json<Approval>,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let req_id = match tx.get::<String>("human-link", &link, now).await {
        Ok(Some(id)) => id,
        _ => return error_response(failure(404, "not_found")),
    };

    let mut req = match tx.get_for_update::<Request>("request", &req_id, now).await {
        Ok(Some(r)) => r,
        _ => return error_response(failure(404, "not_found")),
    };

    let flow = match engine.check_flow(&mut tx, &headers, &req.id, now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if !flow.verified || req.phase != "awaiting_consent" {
        return error_response(failure(403, "forbidden"));
    }

    let refresh_cookie = engine.parse_cookie_value(&headers, "new_api_refresh");
    let (human, _) = match tx.authenticate_human(refresh_cookie, now).await {
        Ok(Some(res)) => res,
        _ => return error_response(failure(403, "forbidden")),
    };

    if human != flow.human {
        return error_response(failure(403, "forbidden"));
    }

    if let Some(expected) = &req.input.expected_human_id
        && expected != &human.id
    {
        return error_response(failure(403, "expected_human_mismatch"));
    }

    if consent.expires_at <= now || consent.expires_at > now + req.input.session_ttl_seconds {
        return error_response(failure(400, "invalid_request"));
    }

    for p in &consent.permissions {
        if p.resource != human.id || !req.input.requested_permissions.contains(p) {
            return error_response(failure(403, "forbidden"));
        }
    }

    let grant_id = HumanGrantEngine::random_id();
    let audit_id = match engine
        .audit(&mut tx, "approve", &req.actor, &human.id, &grant_id, now)
        .await
    {
        Ok(id) => id,
        Err(e) => return error_response(e),
    };

    let grant = Grant {
        grant_id: grant_id.clone(),
        request_id: req.id.clone(),
        human,
        actor_id: req.actor.clone(),
        public_key: req.public_key.clone(),
        client_id: req.input.client_id.clone(),
        target_service: req.input.target_service.clone(),
        requested: req.input.requested_permissions.clone(),
        permissions: consent.permissions,
        created_at: now,
        expires_at: consent.expires_at,
        revoked_at: 0,
        token_hash: String::new(),
        epoch: req.epoch,
        audit_id,
    };

    if let Err(e) = tx
        .put(
            "grant",
            &grant_id,
            &grant.human.id,
            grant.expires_at + RETENTION_SECONDS,
            &grant,
        )
        .await
    {
        return error_response(e);
    }

    req.phase = "approved".to_string();
    req.updated = now;
    req.grant_id = Some(grant_id.clone());
    if let Err(e) = tx
        .put(
            "request",
            &req.id,
            &req.actor,
            req.created + RETENTION_SECONDS,
            &req,
        )
        .await
    {
        return error_response(e);
    }

    let cookie_key = engine
        .parse_cookie_value(&headers, engine.flow_cookie_name())
        .unwrap_or("");
    let _ = tx.delete("flow", &fingerprint(cookie_key.as_bytes())).await;

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "status": "approved",
            "grant_id": grant_id,
            "expires_at": grant.expires_at,
        }),
    )
}

async fn human_reject(
    State(engine): State<SharedEngine>,
    Path(link): Path<String>,
    headers: HeaderMap,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    let req_id = match tx.get::<String>("human-link", &link, now).await {
        Ok(Some(id)) => id,
        _ => return error_response(failure(404, "not_found")),
    };

    let mut req = match tx.get_for_update::<Request>("request", &req_id, now).await {
        Ok(Some(r)) => r,
        _ => return error_response(failure(404, "not_found")),
    };

    let flow = match engine.check_flow(&mut tx, &headers, &req.id, now).await {
        Ok(f) => f,
        Err(e) => return error_response(e),
    };

    if !flow.verified || req.phase != "awaiting_consent" {
        return error_response(failure(403, "forbidden"));
    }

    req.phase = "rejected".to_string();
    req.updated = now;
    if let Err(e) = tx
        .put(
            "request",
            &req.id,
            &req.actor,
            req.created + RETENTION_SECONDS,
            &req,
        )
        .await
    {
        return error_response(e);
    }
    if let Err(e) = engine
        .audit(&mut tx, "reject", &req.actor, &flow.human.id, &req.id, now)
        .await
    {
        return error_response(e);
    }

    let cookie_key = engine
        .parse_cookie_value(&headers, engine.flow_cookie_name())
        .unwrap_or("");
    let _ = tx.delete("flow", &fingerprint(cookie_key.as_bytes())).await;

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(StatusCode::OK, &serde_json::json!({ "status": "rejected" }))
}

// ----------------------------------------------------------------------------
// Agent Endpoints (Ed25519 Signed)
// ----------------------------------------------------------------------------

fn extract_header_map(headers: &HeaderMap) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (k, v) in headers {
        if let Ok(val) = v.to_str() {
            map.insert(k.as_str().to_ascii_lowercase(), val.to_string());
        }
    }
    map
}

async fn create_request(
    State(engine): State<SharedEngine>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("POST", "/v1/requests", &hmap, &body, &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let settings = match engine.get_settings(&mut tx, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    if !settings.enabled {
        return error_response(failure(403, "feature_disabled"));
    }

    let limits = [(proof.actor.as_str(), 60, 30), ("requests:all", 60, 300)];
    match engine.check_limits(&mut tx, &limits, now).await {
        Ok(true) => {}
        Ok(false) => return error_response(failure(429, "rate_limited")),
        Err(e) => return error_response(e),
    }

    let input: CreateRequest = match serde_json::from_slice(&body) {
        Ok(i) => i,
        Err(_) => return error_response(failure(400, "invalid_request")),
    };

    if input.client_id != CLIENT_ID || input.target_service != SERVICE_ID {
        return error_response(failure(400, "invalid_request"));
    }
    if input.purpose.is_empty()
        || input.purpose.chars().count() > 240
        || input.purpose.chars().any(char::is_control)
    {
        return error_response(failure(400, "invalid_request"));
    }
    if input.session_ttl_seconds < 60 || input.session_ttl_seconds > 86400 {
        return error_response(failure(400, "invalid_request"));
    }
    if input.requested_permissions.is_empty() {
        return error_response(failure(400, "invalid_request"));
    }

    let req_id = HumanGrantEngine::random_id();
    let link = HumanGrantEngine::random_id();

    let req = Request {
        id: req_id.clone(),
        link: link.clone(),
        actor: proof.actor.clone(),
        public_key: proof.public_key.clone(),
        input,
        epoch: settings.epoch,
        created: now,
        updated: now,
        expires: now + 600,
        phase: "waiting_human".to_string(),
        poll_after: 5,
        last_poll: 0,
        email_status: "not_requested".to_string(),
        last_email: 0,
        grant_id: None,
    };

    if let Err(e) = tx
        .put(
            "request",
            &req_id,
            &proof.actor,
            req.created + RETENTION_SECONDS,
            &req,
        )
        .await
    {
        return error_response(e);
    }
    if let Err(e) = tx
        .put(
            "human-link",
            &link,
            "",
            req.created + RETENTION_SECONDS,
            &req_id,
        )
        .await
    {
        return error_response(e);
    }
    if let Err(e) = engine
        .audit(&mut tx, "request", &proof.actor, "", &req_id, now)
        .await
    {
        return error_response(e);
    }

    let state = match engine.build_state(&mut tx, &req, &settings, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(StatusCode::CREATED, &state)
}

async fn poll_request(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    handle_poll(engine, id, headers, false).await
}

async fn poll_request_markdown(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    handle_poll(engine, id, headers, true).await
}

async fn handle_poll(
    engine: SharedEngine,
    id: String,
    headers: HeaderMap,
    markdown: bool,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let path = if markdown {
        format!("/v1/requests/{id}/status.md")
    } else {
        format!("/v1/requests/{id}")
    };

    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("GET", &path, &hmap, &[], &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let settings = match engine.get_settings(&mut tx, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };

    let mut req = match tx.get::<Request>("request", &id, now).await {
        Ok(Some(r)) => r,
        _ => return error_response(failure(404, "not_found")),
    };

    if req.actor != proof.actor || req.public_key != proof.public_key {
        return error_response(failure(403, "forbidden"));
    }

    if req.expires <= now
        && req.phase != "succeeded"
        && req.phase != "rejected"
        && req.phase != "cancelled"
    {
        req.phase = "expired".to_string();
        req.updated = now;
        let _ = tx
            .put(
                "request",
                &req.id,
                &req.actor,
                req.created + RETENTION_SECONDS,
                &req,
            )
            .await;
    }

    let state = match engine.build_state(&mut tx, &req, &settings, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    if markdown {
        let md = HumanGrantEngine::state_to_markdown(&state);
        let mut resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/markdown; charset=utf-8")
            .body(Body::from(md))
            .unwrap();
        apply_security_headers(&mut resp);
        resp
    } else {
        json_response(StatusCode::OK, &state)
    }
}

async fn claim_request(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let path = format!("/v1/requests/{id}/claim");
    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("POST", &path, &hmap, &body, &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let settings = match engine.get_settings(&mut tx, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };

    let mut req = match tx.get_for_update::<Request>("request", &id, now).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(failure(404, "not_found")),
        Err(e) => return error_response(e),
    };

    if req.actor != proof.actor || req.public_key != proof.public_key {
        return error_response(failure(403, "forbidden"));
    }

    if req.phase == "succeeded" {
        return error_response(failure(409, "credential_already_issued"));
    }
    if req.phase == "expired" {
        return error_response(failure(410, "expired"));
    }
    if req.phase != "approved" {
        return error_response(failure(409, "invalid_state"));
    }

    let Some(gid) = &req.grant_id else {
        return error_response(failure(403, "forbidden"));
    };

    let mut grant = match tx.get_for_update::<Grant>("grant", gid, now).await {
        Ok(Some(g)) => g,
        Ok(None) => return error_response(failure(403, "forbidden")),
        Err(e) => return error_response(e),
    };

    let status = match engine.grant_status(&mut tx, &grant, &settings, now).await {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    if status != "pending_claim" {
        return error_response(failure(403, "forbidden"));
    }

    let token = HumanGrantEngine::random_id();
    grant.token_hash = fingerprint(token.as_bytes());
    if let Err(e) = tx
        .put(
            "grant",
            &grant.grant_id,
            &grant.human.id,
            grant.expires_at + RETENTION_SECONDS,
            &grant,
        )
        .await
    {
        return error_response(e);
    }
    if let Err(e) = tx
        .put(
            "session",
            &grant.token_hash,
            &grant.human.id,
            grant.expires_at,
            &grant.grant_id,
        )
        .await
    {
        return error_response(e);
    }

    req.phase = "succeeded".to_string();
    req.updated = now;
    if let Err(e) = tx
        .put(
            "request",
            &req.id,
            &req.actor,
            req.created + RETENTION_SECONDS,
            &req,
        )
        .await
    {
        return error_response(e);
    }

    if let Err(e) = engine
        .audit(
            &mut tx,
            "claim",
            &grant.actor_id,
            &grant.human.id,
            &grant.grant_id,
            now,
        )
        .await
    {
        return error_response(e);
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "access_token": token,
            "token_type": "HumanGrant",
            "grant_id": grant.grant_id,
            "expires_at": grant.expires_at,
        }),
    )
}

async fn cancel_request(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let path = format!("/v1/requests/{id}/cancel");
    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("POST", &path, &hmap, &body, &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let mut req = match tx.get_for_update::<Request>("request", &id, now).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(failure(404, "not_found")),
        Err(e) => return error_response(e),
    };

    if req.actor != proof.actor || req.public_key != proof.public_key {
        return error_response(failure(403, "forbidden"));
    }

    if req.phase == "succeeded"
        || req.phase == "rejected"
        || req.phase == "cancelled"
        || req.phase == "expired"
    {
        return error_response(failure(409, "invalid_state"));
    }

    req.phase = "cancelled".to_string();
    req.updated = now;
    if let Err(e) = tx
        .put(
            "request",
            &req.id,
            &req.actor,
            req.created + RETENTION_SECONDS,
            &req,
        )
        .await
    {
        return error_response(e);
    }

    if let Some(gid) = &req.grant_id
        && let Ok(Some(mut g)) = tx.get_for_update::<Grant>("grant", gid, now).await
    {
        g.revoked_at = now;
        let _ = tx
            .put(
                "grant",
                gid,
                &g.human.id,
                g.expires_at + RETENTION_SECONDS,
                &g,
            )
            .await;
    }

    if let Err(e) = engine
        .audit(&mut tx, "cancel", &req.actor, "", &req.id, now)
        .await
    {
        return error_response(e);
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({ "status": "cancelled" }),
    )
}

async fn email_request(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let path = format!("/v1/requests/{id}/email");
    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("POST", &path, &hmap, &body, &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let mut req = match tx.get_for_update::<Request>("request", &id, now).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(failure(404, "not_found")),
        Err(e) => return error_response(e),
    };

    if req.actor != proof.actor || req.public_key != proof.public_key {
        return error_response(failure(403, "forbidden"));
    }

    let input: EmailRequest = match serde_json::from_slice(&body) {
        Ok(i) => i,
        Err(_) => return error_response(failure(400, "invalid_request")),
    };

    req.email_status = "sent".to_string();
    req.last_email = now;
    if let Err(e) = tx
        .put(
            "request",
            &req.id,
            &req.actor,
            req.created + RETENTION_SECONDS,
            &req,
        )
        .await
    {
        return error_response(e);
    }

    // Local outbox record
    let op_payload = serde_json::json!({
        "id": HumanGrantEngine::random_id(),
        "request_id": req.id,
        "destination": input.email,
        "state": "local_spooled",
        "created": now,
        "expires": req.expires,
    });
    if let Err(e) = tx
        .put(
            "local-mail",
            &HumanGrantEngine::random_id(),
            "",
            req.expires,
            &op_payload,
        )
        .await
    {
        return error_response(e);
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::ACCEPTED,
        &serde_json::json!({
            "status": "accepted",
            "message": "Invitation spooled to local outbox.",
        }),
    )
}

async fn resolve_token(
    engine: &HumanGrantEngine,
    tx: &mut HumanGrantTx,
    proof: &Proof,
    now: i64,
) -> Result<Grant, ProtocolError> {
    if proof.credential == "anonymous" {
        return Err(failure(403, "forbidden"));
    }

    let token_hash = fingerprint(proof.credential.as_bytes());
    let grant_id = tx
        .get::<String>("session", &token_hash, now)
        .await?
        .ok_or_else(|| failure(401, "invalid_session"))?;

    let grant = tx
        .get::<Grant>("grant", &grant_id, now)
        .await?
        .ok_or_else(|| failure(401, "invalid_session"))?;

    if grant.actor_id != proof.actor || grant.public_key != proof.public_key {
        return Err(failure(401, "invalid_session"));
    }

    if !bool::from(grant.token_hash.as_bytes().ct_eq(token_hash.as_bytes())) {
        return Err(failure(401, "invalid_session"));
    }

    let settings = engine.get_settings(tx, now).await?;
    let status = engine.grant_status(tx, &grant, &settings, now).await?;
    if status != "active" {
        return Err(failure(401, "invalid_session"));
    }

    Ok(grant)
}

async fn get_session(State(engine): State<SharedEngine>, headers: HeaderMap) -> Response {
    let now = chrono::Utc::now().timestamp();
    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("GET", "/v1/session", &hmap, &[], &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let grant = match resolve_token(&engine, &mut tx, &proof, now).await {
        Ok(g) => g,
        Err(e) => return error_response(e),
    };

    let effective = engine.effective_permissions(&grant);

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "actor": {
                "kind": "ai",
                "id": grant.actor_id,
            },
            "grantor_id": grant.human.id,
            "grant_id": grant.grant_id,
            "client_id": grant.client_id,
            "target_service": grant.target_service,
            "expires_at": grant.expires_at,
            "audit_id": grant.audit_id,
            "effective_permissions": effective,
        }),
    )
}

async fn get_profile(
    State(engine): State<SharedEngine>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let path = format!("/v1/accounts/{id}/profile");
    let hmap = extract_header_map(&headers);
    let proof = match verify_signature("GET", &path, &hmap, &[], &engine.origin, now) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };

    let mut tx = match engine.store.begin().await {
        Ok(t) => t,
        Err(e) => return error_response(e),
    };

    if let Err(e) = tx
        .atomic_reserve_nonce(&proof.actor, &proof.nonce, proof.expires)
        .await
    {
        return error_response(e);
    }

    let grant = match resolve_token(&engine, &mut tx, &proof, now).await {
        Ok(g) => g,
        Err(e) => return error_response(e),
    };

    let effective = engine.effective_permissions(&grant);
    let target_permission = Permission {
        action: PROFILE_READ.to_string(),
        resource: id.clone(),
    };

    if !effective.contains(&target_permission) {
        return error_response(failure(403, "forbidden"));
    }

    let account = match tx.current_human(&grant.human, now).await {
        Ok(Some(a)) => a,
        _ => return error_response(failure(403, "forbidden")),
    };

    if let Err(e) = engine
        .audit(
            &mut tx,
            PROFILE_READ,
            &grant.actor_id,
            &grant.human.id,
            &grant.grant_id,
            now,
        )
        .await
    {
        return error_response(e);
    }

    if let Err(e) = tx.commit().await {
        return error_response(e);
    }

    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "actor": {
                "kind": "ai",
                "id": grant.actor_id,
            },
            "grantor_id": grant.human.id,
            "account": {
                "id": account.id,
                "username": account.username,
                "display_name": account.display_name,
            }
        }),
    )
}
