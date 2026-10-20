use super::*;

#[tokio::test]
async fn pending_mfa_storage_failure_preserves_retry_cookies() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let store = IdentityStore::from_pool(pool);
    let state = AuthState {
        browser: BrowserAuth::new(store.clone(), &[41; 32]).unwrap(),
        store,
        config: LegacyAuthConfig::new("https://app.example", &[41; 32]).unwrap(),
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!(
            "__Host-lmm_external_mfa_pending=lmmf_{}; __Host-lmm_mfa_flow={}",
            "a".repeat(64),
            "b".repeat(64)
        ))
        .unwrap(),
    );
    let response = pending_mfa(&state, &headers).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(!response.headers().contains_key(header::SET_COOKIE));
}

#[test]
fn provider_cookie_rejects_duplicates_missing_and_malformed_values() {
    let config = LegacyAuthConfig::new("https://app.example", &[41; 32]).unwrap();
    let name = cookie_name(ExternalProvider::Github, &config);
    let valid = "a".repeat(64);
    let mut headers = HeaderMap::new();
    assert!(cookie_value(&headers, &name, "", 64).is_err());
    for value in [
        format!("{name}={valid}; {name}={valid}"),
        format!("{name}={}", "a".repeat(63)),
        format!("{name}={}", "g".repeat(64)),
        format!("{name}={valid}; {name}"),
    ] {
        headers.insert(header::COOKIE, HeaderValue::from_str(&value).unwrap());
        assert!(cookie_value(&headers, &name, "", 64).is_err());
    }
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("{name}={valid}")).unwrap(),
    );
    assert_eq!(cookie_value(&headers, &name, "", 64).unwrap(), valid);
    assert!(
        cookie_value(
            &headers,
            &cookie_name(ExternalProvider::LinuxDo, &config),
            "",
            64
        )
        .is_err()
    );
}

#[test]
fn direct_mfa_redirect_keeps_proof_in_private_cookies() {
    let config = LegacyAuthConfig::new("https://app.example", &[41; 32]).unwrap();
    let token = format!("lmmf_{}", "b".repeat(64));
    let binding = "c".repeat(64);
    let challenge = crate::identity::MfaChallenge {
        require_2fa: true,
        flow_token: token.clone(),
        expires_at: 1,
    };
    let response = direct_mfa_redirect_for(&challenge, &binding, &config, "linuxdo");
    assert_eq!(
        response.headers()[header::LOCATION],
        "/oauth/linuxdo?external_mfa=1"
    );
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert_eq!(cookies.len(), 2);
    for cookie in &cookies {
        assert!(cookie.contains("Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=300"));
    }
    assert!(
        cookies
            .iter()
            .any(|v| v.starts_with(&format!("__Host-lmm_external_mfa_pending={token};")))
    );
    assert!(
        cookies
            .iter()
            .any(|v| v.starts_with(&format!("__Host-lmm_mfa_flow={binding};")))
    );
    assert!(
        !response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .contains(&token)
    );
}
