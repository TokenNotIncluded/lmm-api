//! Native ordinary Telegram Widget endpoints, independent of the Go service.
use super::*;
use crate::identity::TelegramAuthorization;
use axum::extract::Path;
use serde::de::value::MapDeserializer;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/oauth/telegram/login/start", post(login_start))
        .route("/api/oauth/telegram/login", get(login))
        .route("/api/oauth/telegram/bind/start", post(bind_start))
        .route("/api/oauth/telegram/bind/{flow}", get(bind))
}

fn cookie_name(config: &LegacyAuthConfig, purpose: &str) -> String {
    format!(
        "{}lmm_telegram_{}_flow",
        if config.secure { "__Host-" } else { "" },
        purpose
    )
}

fn flow_cookie(
    response: Response,
    binding: Option<&str>,
    config: &LegacyAuthConfig,
    purpose: &str,
) -> Response {
    external_login::set_flow_cookie(
        response,
        &cookie_name(config, purpose),
        binding,
        config,
        300,
        if purpose == "bind" { "Lax" } else { "Strict" },
    )
}

fn binding<'a>(
    headers: &'a HeaderMap,
    config: &LegacyAuthConfig,
    purpose: &str,
) -> Result<&'a str, IdentityError> {
    external_login::cookie_value(headers, &cookie_name(config, purpose), "", 64)
}

fn available(state: &AuthState) -> Result<(), IdentityError> {
    state
        .store
        .telegram_bot_name()
        .map(|_| ())
        .ok_or(IdentityError::Forbidden)
}

async fn login_start(State(state): State<AuthState>) -> Response {
    let result = async {
        available(&state)?;
        let binding = external_login::random_binding()?;
        let flow = state.browser.begin_telegram_login(&binding).await?;
        Ok::<_, BrowserAuthError>((binding, flow))
    }
    .await;
    match result {
        Ok((binding, flow)) => flow_cookie(
            success(Some(json!(flow))),
            Some(&binding),
            &state.config,
            "login",
        ),
        Err(error) => auth_failure(error),
    }
}

// Deserialize the ordered decoded query pairs directly. In particular, never
// collect into a map first: duplicate signed fields or state must be rejected.
fn authorization(
    pairs: Vec<(String, String)>,
    with_state: bool,
) -> Result<(Option<String>, TelegramAuthorization), IdentityError> {
    let mut state = None;
    let mut fields = Vec::new();
    for (key, value) in pairs {
        if key == "state" {
            if !with_state || state.replace(value).is_some() {
                return Err(IdentityError::Invalid);
            }
        } else {
            fields.push((key, value));
        }
    }
    if with_state && state.is_none() {
        return Err(IdentityError::Invalid);
    }
    let values = fields
        .into_iter()
        .map(|(key, value)| (key, Value::String(value)));
    let authorization =
        TelegramAuthorization::deserialize(MapDeserializer::<_, serde_json::Error>::new(values))
            .map_err(|_| IdentityError::Invalid)?;
    Ok((state, authorization))
}

async fn login(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let result = async {
        available(&state)?;
        if !valid_origin(&headers, &state.config.origin) {
            return Err(IdentityError::Forbidden.into());
        }
        let Query(pairs) = query.map_err(|_| IdentityError::Invalid)?;
        let (flow, input) = authorization(pairs, true)?;
        let cookie = binding(&headers, &state.config, "login")?;
        let mfa_binding = external_login::random_binding()?;
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
            .finish_telegram_login(
                flow.as_deref().ok_or(IdentityError::Invalid)?,
                cookie,
                &input,
                &metadata,
                &mfa_binding,
            )
            .await?;
        Ok::<_, MfaError>((outcome, mfa_binding))
    }
    .await;
    let response = match result {
        Ok((BrowserLoginOutcome::Session(bundle), _)) => bundle_response(bundle, &state.config),
        Ok((BrowserLoginOutcome::Challenge(challenge), binding)) => mfa_flow_cookie(
            success(Some(json!(challenge))),
            Some(&binding),
            &state.config,
        ),
        Err(error) => mfa_failure(error),
    };
    flow_cookie(response, None, &state.config, "login")
}

async fn bind_start(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let result = async {
        available(&state)?;
        let access = bearer(&headers)?.ok_or(IdentityError::Unauthorized)?;
        let binding = external_login::random_binding()?;
        let flow = state
            .browser
            .begin_telegram_bind(access, &binding, expected_sid(&headers)?)
            .await?;
        let mut callback =
            reqwest::Url::parse(&state.config.origin).map_err(|_| IdentityError::Invalid)?;
        callback.set_path(&format!("/api/oauth/telegram/bind/{}", flow.flow_token));
        Ok::<_, BrowserAuthError>((binding, flow, callback))
    }
    .await;
    match result {
        Ok((binding, flow, callback)) => flow_cookie(
            success(Some(
                json!({"flow_token":flow.flow_token,"expires_at":flow.expires_at,"callback_url":callback.as_str()}),
            )),
            Some(&binding),
            &state.config,
            "bind",
        ),
        Err(error) => auth_failure(error),
    }
}

fn bind_redirect(
    config: &LegacyAuthConfig,
    flow: &str,
    outcome: &str,
    code: Option<&str>,
) -> Response {
    let mut url = reqwest::Url::parse(&config.origin).expect("validated canonical browser origin");
    url.set_path("/oauth/telegram");
    url.query_pairs_mut().append_pair("telegram_bind", outcome);
    // This token is only a consumed-flow correlation for the same-origin
    // opener. No assertion, bearer, refresh secret or proof enters the URL.
    if flow.starts_with("lmmt_")
        && flow.len() == 69
        && flow[5..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        url.query_pairs_mut().append_pair("flow_token", flow);
    }
    if let Some(code) = code {
        url.query_pairs_mut().append_pair("error_code", code);
    }
    flow_cookie(
        Redirect::to(url.as_str()).into_response(),
        None,
        config,
        "bind",
    )
}

async fn bind(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(flow): Path<String>,
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let result = async {
        available(&state)?;
        let Query(pairs) = query.map_err(|_| IdentityError::Invalid)?;
        let (_, input) = authorization(pairs, false)?;
        state
            .browser
            .finish_telegram_bind(&flow, binding(&headers, &state.config, "bind")?, &input)
            .await
    }
    .await;
    match result {
        Ok(()) => bind_redirect(&state.config, &flow, "success", None),
        Err(error) => bind_redirect(
            &state.config,
            &flow,
            "error",
            Some(match error {
                BrowserAuthError::Identity(IdentityError::Conflict) => {
                    "TELEGRAM_BIND_ALREADY_BOUND"
                }
                BrowserAuthError::Identity(IdentityError::Storage) => {
                    "TELEGRAM_BIND_INTERNAL_ERROR"
                }
                _ => "TELEGRAM_BIND_FLOW_INVALID",
            }),
        ),
    }
}

#[cfg(test)]
#[path = "telegram_tests.rs"]
mod tests;
