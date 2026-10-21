//! Fixed provider browser endpoints. Callers never supply provider endpoints.
use super::*;
use crate::identity::ExternalProvider;
use axum::extract::Path;
#[cfg(test)]
#[path = "external_login_tests.rs"]
mod tests;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/oauth/{provider}/start", post(start))
        .route("/api/oauth/github", get(github_callback))
        .route("/api/oauth/linuxdo", get(linuxdo_callback))
        .route("/api/oauth/discord", get(discord_callback))
}
pub(super) fn random_binding() -> Result<String, IdentityError> {
    let mut random = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut random)
        .map_err(|_| IdentityError::Storage)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn cookie_name(provider: ExternalProvider, config: &LegacyAuthConfig) -> String {
    format!(
        "{}lmm_external_{}_flow",
        if config.secure { "__Host-" } else { "" },
        provider.slug()
    )
}
pub(super) fn cookie_value<'a>(
    headers: &'a HeaderMap,
    name: &str,
    prefix: &str,
    length: usize,
) -> Result<&'a str, IdentityError> {
    let mut found = None;
    for raw in headers.get_all(header::COOKIE) {
        for part in raw
            .to_str()
            .map_err(|_| IdentityError::Unauthorized)?
            .split(';')
        {
            if part.trim() == name {
                return Err(IdentityError::Unauthorized);
            }
            if let Some((key, value)) = part.trim().split_once('=') {
                if key == name {
                    if found.is_some()
                        || value.len() != length
                        || !value.starts_with(prefix)
                        || !value[prefix.len()..].bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(IdentityError::Unauthorized);
                    }
                    found = Some(value);
                }
            }
        }
    }
    found.ok_or(IdentityError::Unauthorized)
}
pub(super) fn set_flow_cookie(
    mut response: Response,
    name: &str,
    value: Option<&str>,
    config: &LegacyAuthConfig,
    age: u16,
    same_site: &str,
) -> Response {
    let secure = if config.secure { "Secure; " } else { "" };
    let cookie = format!(
        "{name}={}; Path=/; {secure}HttpOnly; SameSite={same_site}; Max-Age={}",
        value.unwrap_or_default(),
        if value.is_some() { age } else { 0 }
    );
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    accepted_legal: bool,
}
async fn start(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
    input: Result<Json<Start>, JsonRejection>,
) -> Response {
    let result = async {
        let provider = ExternalProvider::parse(&provider)?;
        if state
            .config
            .external_client_id(&state.store, provider)
            .is_none()
        {
            return Err(IdentityError::Forbidden.into());
        }
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        if state.config.require_legal_acceptance && !input.accepted_legal {
            return Err(IdentityError::Invalid.into());
        }
        let binding = random_binding()?;
        let url = state
            .browser
            .begin_external_login(provider, &binding)
            .await?;
        Ok::<_, BrowserAuthError>((provider, binding, url))
    }
    .await;
    match result {
        Ok((provider, binding, url)) => set_flow_cookie(
            success(Some(json!({"authorization_url":url}))),
            &cookie_name(provider, &state.config),
            Some(&binding),
            &state.config,
            600,
            "Lax",
        ),
        Err(error) => auth_failure(error),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    iss: Option<String>,
    external_mfa: Option<String>,
}
async fn github_callback(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    callback(state, headers, peer, query, ExternalProvider::Github).await
}
async fn linuxdo_callback(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    callback(state, headers, peer, query, ExternalProvider::LinuxDo).await
}
async fn discord_callback(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    callback(state, headers, peer, query, ExternalProvider::Discord).await
}
async fn callback(
    state: AuthState,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    query: Result<Query<Callback>, QueryRejection>,
    provider: ExternalProvider,
) -> Response {
    if let Ok(Query(query)) = &query {
        if query.external_mfa.as_deref() == Some("1") {
            if query.code.is_some()
                || query.state.is_some()
                || query.error.is_some()
                || query.iss.is_some()
            {
                return auth_failure(IdentityError::Invalid.into());
            }
            return pending_mfa(&state, &headers).await;
        }
    }
    let direct_browser = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|part| part.trim().split(';').next() == Some("text/html"))
        });
    let result = async {
        if state
            .config
            .external_client_id(&state.store, provider)
            .is_none()
        {
            return Err(IdentityError::Forbidden.into());
        }
        let Query(query) = query.map_err(|_| IdentityError::Invalid)?;
        if query.error.is_some()
            || query
                .error_description
                .as_ref()
                .is_some_and(|v| v.len() > 4096)
            || query
                .iss
                .as_deref()
                .is_some_and(|iss| iss != provider.issuer())
            || (provider == ExternalProvider::Discord
                && query.iss.as_deref() != Some(provider.issuer()))
        {
            return Err(IdentityError::Unauthorized.into());
        }
        let binding = cookie_value(&headers, &cookie_name(provider, &state.config), "", 64)?;
        let mfa_binding = random_binding()?;
        let metadata = BrowserSessionMetadata {
            ip: peer.map(|p| p.0.ip().to_string()).unwrap_or_default(),
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect(),
        };
        let outcome = state
            .browser
            .finish_external_login(
                provider,
                query.state.as_deref().ok_or(IdentityError::Unauthorized)?,
                binding,
                query.code.as_deref().ok_or(IdentityError::Unauthorized)?,
                &metadata,
                &mfa_binding,
            )
            .await?;
        Ok::<_, MfaError>((outcome, mfa_binding))
    }
    .await;
    let response = match result {
        Ok((BrowserLoginOutcome::Session(bundle), _)) if direct_browser => set_cookie(
            Redirect::to("/console").into_response(),
            Some((&bundle.refresh_token, bundle.expires_at)),
            &state.config,
        ),
        Ok((BrowserLoginOutcome::Session(bundle), _)) => bundle_response(bundle, &state.config),
        Ok((BrowserLoginOutcome::Challenge(challenge), binding)) if direct_browser => {
            direct_mfa_redirect_for(&challenge, &binding, &state.config, provider.slug())
        }
        Ok((BrowserLoginOutcome::Challenge(challenge), binding)) => mfa_flow_cookie(
            success(Some(json!(challenge))),
            Some(&binding),
            &state.config,
        ),
        Err(error) => mfa_failure(error),
    };
    set_flow_cookie(
        response,
        &cookie_name(provider, &state.config),
        None,
        &state.config,
        600,
        "Lax",
    )
}
fn pending_name(config: &LegacyAuthConfig) -> &'static str {
    if config.secure {
        "__Host-lmm_external_mfa_pending"
    } else {
        "lmm_external_mfa_pending"
    }
}
pub(super) fn direct_mfa_redirect(
    challenge: &crate::identity::MfaChallenge,
    binding: &str,
    config: &LegacyAuthConfig,
) -> Response {
    direct_mfa_redirect_for(challenge, binding, config, "google")
}
fn direct_mfa_redirect_for(
    challenge: &crate::identity::MfaChallenge,
    binding: &str,
    config: &LegacyAuthConfig,
    provider: &str,
) -> Response {
    let response = mfa_flow_cookie(
        Redirect::to(&format!("/oauth/{provider}?external_mfa=1")).into_response(),
        Some(binding),
        config,
    );
    set_flow_cookie(
        response,
        pending_name(config),
        Some(&challenge.flow_token),
        config,
        300,
        "Strict",
    )
}
pub(super) async fn pending_mfa(state: &AuthState, headers: &HeaderMap) -> Response {
    let result = async {
        let token = cookie_value(headers, pending_name(&state.config), "lmmf_", 69)?;
        let binding = mfa_binding_cookie(headers, &state.config)?;
        state.browser.pending_external_mfa(token, binding).await
    }
    .await;
    // This is a read-only continuation. Keep the short-lived cookie when a
    // response is lost or storage is temporarily unavailable; a retry must
    // recover the same challenge. MFA completion consumes the database proof,
    // and both private cookies expire after five minutes.
    match result {
        Ok(challenge) => success(Some(json!(challenge))),
        Err(error) => mfa_failure(error),
    }
}
