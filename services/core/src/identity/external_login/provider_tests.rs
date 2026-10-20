use super::*;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn configured(provider: ExternalProvider) -> ExternalOAuth {
    let path = match provider {
        ExternalProvider::Github => "/oauth/github",
        ExternalProvider::LinuxDo => "/api/oauth/linuxdo",
        ExternalProvider::Discord => "/oauth/discord",
    };
    ExternalOAuth::new(
        provider,
        "test-client",
        "test-secret",
        &format!("https://example.test{path}"),
    )
    .expect("fixed HTTPS callback")
}

#[test]
fn fixed_provider_names_and_issuers() {
    for (provider, slug, issuer) in [
        (ExternalProvider::Github, "github", "https://github.com"),
        (
            ExternalProvider::LinuxDo,
            "linuxdo",
            "https://connect.linux.do",
        ),
        (ExternalProvider::Discord, "discord", "https://discord.com"),
    ] {
        assert_eq!(ExternalProvider::parse(slug), Ok(provider));
        assert_eq!(provider.slug(), slug);
        assert_eq!(provider.issuer(), issuer);
        assert_eq!(configured(provider).provider(), provider);
    }
    for bad in [
        "GitHub",
        "linux.do",
        "https://github.com",
        "",
        "github\n",
        "google",
    ] {
        assert_eq!(ExternalProvider::parse(bad), Err(IdentityError::Invalid));
    }
}

#[test]
fn callback_transport_authority_and_path_are_canonical() {
    for provider in [
        ExternalProvider::Github,
        ExternalProvider::LinuxDo,
        ExternalProvider::Discord,
    ] {
        for bad in [
            "http://example.test/oauth/github",
            "https://example.test/oauth/google",
            "https://example.test/core/v1/auth/oauth/github/callback",
            "https://person@example.test/oauth/github",
            "https://example.test/oauth/github?next=/console",
            "https://example.test/oauth/github#state",
            "https://example.test/oauth/%67ithub",
            "https://example.test/oauth/github/../github",
            "https://EXAMPLE.test/oauth/github",
            "https://127.0.0.1/oauth/github",
            "lmm://oauth/github",
        ] {
            assert!(matches!(
                ExternalOAuth::new(provider, "client", "secret", bad),
                Err(IdentityError::Invalid)
            ));
        }
    }
    assert!(
        ExternalOAuth::new(
            ExternalProvider::Github,
            "client",
            "secret",
            "https://example.test/api/oauth/github"
        )
        .is_ok()
    );
    assert!(
        ExternalOAuth::new(
            ExternalProvider::LinuxDo,
            "client",
            "secret",
            "https://example.test/oauth/linuxdo"
        )
        .is_ok()
    );
    assert!(
        ExternalOAuth::new(
            ExternalProvider::Discord,
            "client",
            "secret",
            "https://example.test/api/oauth/discord"
        )
        .is_err()
    );
    for (client, secret) in [
        ("", "secret"),
        ("bad:id", "secret"),
        ("client\n", "secret"),
        ("client", ""),
        ("client", "bad\nsecret"),
    ] {
        assert!(
            ExternalOAuth::new(
                ExternalProvider::Github,
                client,
                secret,
                "https://example.test/oauth/github"
            )
            .is_err()
        );
    }
}

#[test]
fn authorization_has_minimum_scope_state_and_documented_pkce() {
    let challenge = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let state = "s".repeat(64);
    for provider in [
        ExternalProvider::Github,
        ExternalProvider::LinuxDo,
        ExternalProvider::Discord,
    ] {
        let config = configured(provider);
        let raw = config.authorization_url(&state, &challenge).unwrap();
        assert!(!raw.contains("test-secret"));
        let url = Url::parse(&raw).unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(url.origin().ascii_serialization(), provider.issuer());
        assert_eq!(
            query.get("scope").map(|v| v.as_ref()),
            Some(provider.scope())
        );
        assert_eq!(query.get("state").map(|v| v.as_ref()), Some(state.as_str()));
        assert_eq!(
            query.get("redirect_uri").map(|v| v.as_ref()),
            Some(config.redirect_uri())
        );
        assert_eq!(query.contains_key("code_challenge"), provider.uses_pkce());
        if provider.uses_pkce() {
            assert_eq!(
                query.get("code_challenge_method").map(|v| v.as_ref()),
                Some("S256")
            );
        }
        for bad in ["", "short", "bad state with spaces", &"s".repeat(257)] {
            assert!(config.authorization_url(bad, &challenge).is_err());
        }
        for bad in [
            "",
            "plain",
            &"A".repeat(42),
            &"A".repeat(44),
            &"+".repeat(43),
            &"_".repeat(43),
        ] {
            assert!(config.authorization_url(&state, bad).is_err());
        }
    }
}

#[test]
fn provider_subjects_are_canonical_and_not_profile_claims() {
    for id in [1, 42, i64::MAX] {
        assert_eq!(positive_numeric_subject(id).unwrap(), id.to_string());
    }
    for id in [0, -1, i64::MIN] {
        assert_eq!(
            positive_numeric_subject(id),
            Err(IdentityError::Unauthorized)
        );
    }
    for body in [
        r#"{"id":"42"}"#,
        r#"{"id":1.0}"#,
        r#"{"id":9223372036854775808}"#,
        r#"{"sub":"42","email":"verified@example.test"}"#,
    ] {
        assert!(serde_json::from_str::<NumericUserReply>(body).is_err());
    }
    for id in ["1", "80351110224678912", "18446744073709551615"] {
        assert_eq!(discord_subject(id.into()).unwrap(), id);
    }
    for id in [
        "",
        "0",
        "01",
        "+1",
        "1.0",
        " 1",
        "1\n",
        "18446744073709551616",
        "999999999999999999999",
        "bot@example.test",
    ] {
        assert_eq!(discord_subject(id.into()), Err(IdentityError::Unauthorized));
    }
    assert!(serde_json::from_str::<DiscordUserReply>(r#"{"id":42}"#).is_err());
}

#[derive(Clone)]
struct Fixture {
    provider: ExternalProvider,
    mode: &'static str,
    requests: Arc<Mutex<Vec<(String, HeaderMap, Bytes)>>>,
}

async fn token(State(state): State<Fixture>, headers: HeaderMap, body: Bytes) -> Response {
    state
        .requests
        .lock()
        .unwrap()
        .push(("token".into(), headers, body));
    match state.mode {
        "redirect" => (StatusCode::FOUND, [("location", "/user")]).into_response(),
        "oversize" => Json(json!({"access_token":"x".repeat(MAX_PROVIDER_BODY),"token_type":"Bearer"})).into_response(),
        "wrong_token_type" => Json(json!({"access_token":"test_access_token","token_type":"Basic"})).into_response(),
        _ => Json(json!({"access_token":"test_access_token","token_type":"Bearer","refresh_token":"not-retained","email":"not-authority"})).into_response(),
    }
}

async fn user(State(state): State<Fixture>, headers: HeaderMap, body: Bytes) -> Response {
    state
        .requests
        .lock()
        .unwrap()
        .push(("user".into(), headers, body));
    if state.mode == "wrong_subject" {
        return Json(json!({"id":"email@example.test","email_verified":true,"roles":["admin"]}))
            .into_response();
    }
    if state.provider == ExternalProvider::Discord {
        Json(json!({"id":"80351110224678912","email":"ignored@example.test","verified":true,"roles":["admin"]})).into_response()
    } else {
        Json(json!({"id":42,"email":"ignored@example.test","trust_level":4,"roles":["admin"]}))
            .into_response()
    }
}

async fn fixture(
    provider: ExternalProvider,
    mode: &'static str,
) -> (ExternalOAuth, Fixture, tokio::task::JoinHandle<()>) {
    let state = Fixture {
        provider,
        mode,
        requests: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/token", post(token))
        .route("/user", get(user))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let config = configured(provider)
        .with_test_endpoints(
            Url::parse(&format!("http://{address}/token")).unwrap(),
            Url::parse(&format!("http://{address}/user")).unwrap(),
        )
        .unwrap();
    (config, state, server)
}

#[tokio::test]
async fn exchange_uses_fixed_user_proof_and_confidential_client_credentials() {
    for provider in [
        ExternalProvider::Github,
        ExternalProvider::LinuxDo,
        ExternalProvider::Discord,
    ] {
        let (config, state, server) = fixture(provider, "ok").await;
        let proof = config
            .exchange("authorization_code", &"v".repeat(64))
            .await
            .unwrap();
        assert_eq!(proof.provider(), provider);
        assert_eq!(proof.issuer(), provider.issuer());
        assert_eq!(
            proof.subject(),
            if provider == ExternalProvider::Discord {
                "80351110224678912"
            } else {
                "42"
            }
        );
        let requests = state.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "token");
        let form = reqwest::Url::parse(&format!(
            "http://example.test/?{}",
            std::str::from_utf8(&requests[0].2).unwrap()
        ))
        .unwrap();
        let pairs = form
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            pairs.get("grant_type").map(|v| v.as_ref()),
            Some("authorization_code")
        );
        assert_eq!(
            pairs.get("redirect_uri").map(|v| v.as_ref()),
            Some(config.redirect_uri())
        );
        assert_eq!(pairs.contains_key("code_verifier"), provider.uses_pkce());
        if provider == ExternalProvider::Github {
            assert!(!requests[0].1.contains_key("authorization"));
            assert_eq!(
                pairs.get("client_id").map(|v| v.as_ref()),
                Some("test-client")
            );
            assert_eq!(
                pairs.get("client_secret").map(|v| v.as_ref()),
                Some("test-secret")
            );
        } else {
            let expected = format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("test-client:test-secret")
            );
            assert_eq!(
                requests[0].1.get("authorization").unwrap(),
                expected.as_str()
            );
            assert!(!pairs.contains_key("client_secret"));
        }
        assert_eq!(requests[1].0, "user");
        assert_eq!(
            requests[1].1.get("authorization").unwrap(),
            "Bearer test_access_token"
        );
        assert!(requests[1].2.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn exchange_rejects_redirects_oversized_body_bad_tokens_and_bad_subjects() {
    for mode in ["redirect", "oversize", "wrong_token_type", "wrong_subject"] {
        let (config, state, server) = fixture(ExternalProvider::Github, mode).await;
        assert!(matches!(
            config.exchange("authorization_code", &"v".repeat(64)).await,
            Err(IdentityError::Unauthorized)
        ));
        assert_eq!(
            state.requests.lock().unwrap().len(),
            if mode == "wrong_subject" { 2 } else { 1 }
        );
        server.abort();
    }
}

#[tokio::test]
async fn invalid_codes_and_verifiers_do_not_make_provider_requests() {
    let (config, state, server) = fixture(ExternalProvider::Github, "ok").await;
    for code in ["", "code\r\n", "code space", &"x".repeat(4097)] {
        assert!(matches!(
            config.exchange(code, &"v".repeat(64)).await,
            Err(IdentityError::Invalid)
        ));
    }
    for verifier in ["", &"v".repeat(42), &"v".repeat(129), &"+".repeat(43)] {
        assert!(matches!(
            config.exchange("code", verifier).await,
            Err(IdentityError::Invalid)
        ));
    }
    assert!(state.requests.lock().unwrap().is_empty());
    server.abort();
}

#[test]
fn endpoint_override_is_test_only_and_numeric_loopback_only() {
    for bad in [
        "https://127.0.0.1:12345/token",
        "http://localhost:12345/token",
        "http://example.test:12345/token",
        "http://127.0.0.1/token",
        "http://127.0.0.1:12345/token?provider=github",
        "http://user@127.0.0.1:12345/token",
    ] {
        assert!(
            configured(ExternalProvider::Github)
                .with_test_endpoints(
                    Url::parse(bad).unwrap(),
                    Url::parse("http://127.0.0.1:12345/user").unwrap()
                )
                .is_err()
        );
    }
}
