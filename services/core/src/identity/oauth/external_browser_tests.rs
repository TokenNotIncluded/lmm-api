use super::*;
use crate::identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata};
use axum::http::HeaderMap;
use totp_rs::{Algorithm as TotpAlgorithm, Builder, Secret};

async fn legacy_google_fixture(
    pool: &PgPool,
) -> (IdentityStore, MockProvider, tokio::task::JoinHandle<()>) {
    let mock = MockProvider {
        redirect_uri: Some(Arc::from("https://example.com/api/oauth/google")),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let provider = Router::new()
        .route("/token", post(exchange))
        .route("/keys", get(public_keys))
        .with_state(mock.clone());
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let mut config = GoogleOAuth::new(
        "client-1".into(),
        "test-only-client-secret".into(),
        mock.redirect_uri.as_deref().unwrap().into(),
    )
    .unwrap();
    config.client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    config.token_endpoint = Url::parse(&format!("http://{address}/token")).unwrap();
    config.keys_endpoint = Url::parse(&format!("http://{address}/keys")).unwrap();
    let store = IdentityStore::from_pool(pool.clone())
        .with_google_oauth(config)
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    crate::core_schema::init(pool).await.unwrap();
    (store, mock, server)
}

fn response_cookie(headers: &HeaderMap, name: &str) -> String {
    let prefix = format!("{name}=");
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .unwrap_or_else(|| panic!("expected cookie {name}"))
        .to_owned()
}

fn cookie_pair(raw: &str) -> &str {
    raw.split(';').next().unwrap()
}

fn has_refresh_cookie(headers: &HeaderMap) -> bool {
    headers.get_all("set-cookie").iter().any(|value| {
        value
            .to_str()
            .is_ok_and(|value| value.starts_with("new_api_refresh="))
    })
}

#[sqlx::test(migrations = false)]
async fn legacy_google_callback_preserves_pkce_claim_checks_refresh_and_subject_ownership(
    pool: PgPool,
) {
    let (store, mock, server) = legacy_google_fixture(&pool).await;
    let native = store
        .register(&LoginRequest {
            login_name: "email-owner".into(),
            password: "test-only long passphrase".into(),
        })
        .await
        .unwrap();
    let email_owner = store.authorize(&native.secret).await.unwrap().user_id;
    sqlx::query("UPDATE core_identity.user_profiles SET email='alice@example.invalid',email_login_allowed=TRUE WHERE user_id=$1")
        .bind(email_owner).execute(&pool).await.unwrap();
    let app = web_app(store.clone());
    let (status, headers, started) = web_call(
        &app,
        "POST",
        "/api/oauth/google",
        None,
        Some(json!({"accepted_legal":true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let start_cookie = response_cookie(&headers, "__Host-lmm_web_google_flow");
    assert!(start_cookie.contains("Secure; HttpOnly; SameSite=Lax"));
    let authorization_url = started["data"]["authorization_url"].as_str().unwrap();
    let url = Url::parse(authorization_url).unwrap();
    assert_eq!(url.origin().ascii_serialization(), ISSUER);
    assert_eq!(url.path(), "/o/oauth2/v2/auth");
    let parameters: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
        parameters["redirect_uri"],
        "https://example.com/api/oauth/google"
    );
    assert_eq!(parameters["response_type"], "code");
    assert_eq!(parameters["code_challenge_method"], "S256");
    assert_eq!(parameters["nonce"].len(), 64);
    assert!(!authorization_url.contains(cookie_pair(&start_cookie).split_once('=').unwrap().1));
    let (state, code) =
        prepare_code(&store, &mock, authorization_url, "legacy-browser-subject").await;
    mock.codes.lock().await.get_mut(&code).unwrap().0["email_verified"] = json!(true);
    let callback = format!("/api/oauth/google?state={state}&code={code}");
    let result = web_call(
        &app,
        "GET",
        &callback,
        Some(cookie_pair(&start_cookie)),
        None,
    )
    .await;
    assert_eq!(result.0, StatusCode::SEE_OTHER);
    assert_eq!(result.1["location"], "/console");
    assert!(result.2.is_null());
    let refresh = response_cookie(&result.1, "new_api_refresh");
    assert!(refresh.contains("HttpOnly; Secure; SameSite=Strict"));
    assert!(refresh.contains("Path=/api/user/auth"));
    assert!(response_cookie(&result.1, "__Host-lmm_web_google_flow").contains("Max-Age=0"));
    let refreshed = web_call(
        &app,
        "POST",
        "/api/user/auth/refresh",
        Some(cookie_pair(&refresh)),
        None,
    )
    .await;
    assert_eq!(refreshed.0, StatusCode::OK);
    let actor = store
        .authorize(refreshed.2["data"]["access_token"].as_str().unwrap())
        .await
        .unwrap();
    assert_ne!(actor.user_id, email_owner);
    assert_eq!(actor.platform_level, 0);
    assert_eq!(refreshed.2["data"]["session"]["login_method"], "google");
    assert!(has_refresh_cookie(&refreshed.1));
    let subject_owner: i64 = sqlx::query_scalar(
        "SELECT user_id FROM core_identity.oauth_identities WHERE issuer=$1 AND subject=$2",
    )
    .bind(ISSUER)
    .bind("legacy-browser-subject")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(subject_owner, actor.user_id);
    assert_eq!(
        store.authorize(&native.secret).await.unwrap().user_id,
        email_owner
    );

    for (claim, invalid) in [
        ("iss", json!("https://attacker.invalid")),
        ("nonce", json!("a".repeat(64))),
    ] {
        let (cookie, state, code) = web_flow(&app, &store, &mock, "untrusted-subject").await;
        mock.codes.lock().await.get_mut(&code).unwrap().0[claim] = invalid;
        let before = identity_counts(&pool).await;
        let path = format!("/api/oauth/google?state={state}&code={code}");
        let rejected = web_call(&app, "GET", &path, Some(&cookie), None).await;
        assert_eq!(rejected.0, StatusCode::UNAUTHORIZED);
        assert!(!has_refresh_cookie(&rejected.1));
        assert_eq!(identity_counts(&pool).await, before);
    }

    let closed = store.clone().with_registration_enabled(false);
    let closed_app = web_app(closed.clone());
    for (subject, expected) in [
        ("legacy-browser-subject", StatusCode::SEE_OTHER),
        ("unlinked-but-same-email", StatusCode::FORBIDDEN),
    ] {
        let (cookie, state, code) = web_flow(&closed_app, &closed, &mock, subject).await;
        mock.codes.lock().await.get_mut(&code).unwrap().0["email_verified"] = json!(true);
        let before = identity_counts(&pool).await;
        let path = format!("/api/oauth/google?state={state}&code={code}");
        let response = web_call(&closed_app, "GET", &path, Some(&cookie), None).await;
        assert_eq!(response.0, expected);
        if expected == StatusCode::SEE_OTHER {
            let refresh = response_cookie(&response.1, "new_api_refresh");
            let bundle = web_call(
                &closed_app,
                "POST",
                "/api/user/auth/refresh",
                Some(cookie_pair(&refresh)),
                None,
            )
            .await;
            assert_eq!(bundle.0, StatusCode::OK);
            assert_eq!(bundle.2["data"]["user"]["id"], actor.user_id);
        } else {
            assert!(!has_refresh_cookie(&response.1));
            assert_eq!(identity_counts(&pool).await, before);
        }
    }
    let exchanges = mock.exchanges.load(Ordering::SeqCst);
    let replay = web_call(
        &app,
        "GET",
        &callback,
        Some(cookie_pair(&start_cookie)),
        None,
    )
    .await;
    assert_eq!(replay.0, StatusCode::UNAUTHORIZED);
    assert_eq!(mock.exchanges.load(Ordering::SeqCst), exchanges);
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 2);
    server.abort();
}

#[sqlx::test(migrations = false)]
async fn legacy_google_mfa_uses_cookie_bound_single_use_challenge_and_fresh_family(pool: PgPool) {
    let (store, mock, server) = legacy_google_fixture(&pool).await;
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "google-mfa",
            password: "test passphrase",
            email: "",
        })
        .await
        .unwrap();
    let original = browser
        .login(
            "google-mfa",
            "test passphrase",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let other = browser
        .login(
            "google-mfa",
            "test passphrase",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let original_access = original.data["access_token"].as_str().unwrap();
    let user = original.data["user"]["id"].as_i64().unwrap();
    let old_version: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=$1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) VALUES ($1,$2,$3)",
    )
    .bind(ISSUER)
    .bind("legacy-mfa-subject")
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let setup = browser.mfa_setup(original_access).await.unwrap();
    let epoch: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    let secret = Secret::try_from_base32(&setup.secret).unwrap();
    let code = Builder::new()
        .with_algorithm(TotpAlgorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(u64::try_from(epoch).unwrap())
        .to_string();
    let enabled = browser.mfa_enable(original_access, &code).await.unwrap();
    let version: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=$1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, old_version + 1);
    assert!(store.authorize(original_access).await.is_err());
    assert!(
        store
            .authorize(other.data["access_token"].as_str().unwrap())
            .await
            .is_err()
    );
    assert!(
        browser
            .refresh(&original.refresh_token, None)
            .await
            .is_err()
    );
    let other_generation_revoked: bool = sqlx::query_scalar(
        "SELECT s.user_version<>u.auth_version FROM core_identity.web_sessions s JOIN core_identity.users u ON u.id=s.user_id WHERE s.sid=$1",
    )
    .bind(other.data["session"]["sid"].as_str().unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(other_generation_revoked);
    assert!(browser.refresh(&other.refresh_token, None).await.is_err());
    let app = web_app(store.clone().with_registration_enabled(false));
    let (cookie, state, code) = web_flow(&app, &store, &mock, "legacy-mfa-subject").await;
    let before = identity_counts(&pool).await;
    let callback = format!("/api/oauth/google?state={state}&code={code}");
    let response = web_call(&app, "GET", &callback, Some(&cookie), None).await;
    assert_eq!(response.0, StatusCode::SEE_OTHER);
    assert_eq!(response.1["location"], "/oauth/google?external_mfa=1");
    assert!(response.2.is_null());
    assert!(!has_refresh_cookie(&response.1));
    assert_eq!(identity_counts(&pool).await, before);
    let pending = response_cookie(&response.1, "__Host-lmm_external_mfa_pending");
    let binding = response_cookie(&response.1, "__Host-lmm_mfa_flow");
    for raw in [&pending, &binding] {
        assert!(raw.contains("Path=/; Secure; HttpOnly; SameSite=Strict"));
        assert!(raw.contains("Max-Age=300"));
        let value = cookie_pair(raw).split_once('=').unwrap().1;
        assert!(!response.1["location"].to_str().unwrap().contains(value));
    }
    assert_eq!(response.1.get_all("set-cookie").iter().count(), 3);
    assert!(response_cookie(&response.1, "__Host-lmm_web_google_flow").contains("Max-Age=0"));
    let pending_cookies = format!("{}; {}", cookie_pair(&pending), cookie_pair(&binding));
    let pending_response = web_call(
        &app,
        "GET",
        "/api/oauth/google?external_mfa=1",
        Some(&pending_cookies),
        None,
    )
    .await;
    assert_eq!(pending_response.0, StatusCode::OK);
    assert_eq!(pending_response.2["data"]["require_2fa"], true);
    assert!(!has_refresh_cookie(&pending_response.1));
    assert!(
        pending_response
            .1
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none()
    );
    assert_eq!(identity_counts(&pool).await, before);
    // A lost first response must be recoverable with the unchanged cookies.
    // Continue using the retry's body, not the first response's flow token.
    let recovered = web_call(
        &app,
        "GET",
        "/api/oauth/google?external_mfa=1",
        Some(&pending_cookies),
        None,
    )
    .await;
    assert_eq!(recovered.0, StatusCode::OK);
    assert!(recovered.2 == pending_response.2);
    assert!(!has_refresh_cookie(&recovered.1));
    assert!(recovered.1.get_all("set-cookie").iter().next().is_none());
    assert_eq!(identity_counts(&pool).await, before);
    let flow = recovered.2["data"]["flow_token"].as_str().unwrap();
    assert!(cookie_pair(&pending).split_once('=').unwrap().1 == flow);
    // Enrollment already consumed the current TOTP step. Complete this real
    // challenge with a generated recovery code, without bypassing replay rules.
    let completion = json!({"flow_token":flow,"code":setup.backup_codes[0]});
    let wrong_binding = format!("__Host-lmm_mfa_flow={}", "f".repeat(64));
    let rejected = web_call(
        &app,
        "POST",
        "/api/user/login/2fa",
        Some(&wrong_binding),
        Some(completion.clone()),
    )
    .await;
    assert_eq!(rejected.0, StatusCode::BAD_REQUEST);
    assert_eq!(rejected.2["code"], "MFA_FLOW_EXPIRED");
    assert!(!has_refresh_cookie(&rejected.1));
    assert_eq!(identity_counts(&pool).await, before);
    let completed = web_call(
        &app,
        "POST",
        "/api/user/login/2fa",
        Some(cookie_pair(&binding)),
        Some(completion.clone()),
    )
    .await;
    assert_eq!(completed.0, StatusCode::OK);
    assert_eq!(completed.2["data"]["user"]["id"], user);
    assert_eq!(completed.2["data"]["session"]["login_method"], "2fa");
    assert!(completed.2["data"]["session"]["sid"] != enabled.data["session"]["sid"]);
    let access = completed.2["data"]["access_token"].as_str().unwrap();
    let actor = store.authorize(access).await.unwrap();
    assert_eq!(actor.user_id, user);
    let proof: (i16, String, bool) = sqlx::query_as("SELECT factor_level,method,mfa_verified_at IS NOT NULL FROM core_identity.session_authentication WHERE credential_id=$1")
        .bind(actor.credential_id).fetch_one(&pool).await.unwrap();
    assert_eq!(proof, (2, "recovery".into(), true));
    let consumed: bool = sqlx::query_scalar(
        "SELECT consumed_at IS NOT NULL FROM core_identity.mfa_challenges WHERE digest=$1",
    )
    .bind(digest(flow).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(consumed);
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.mfa_backup_codes WHERE user_id=$1 AND consumed_at IS NULL")
        .bind(user).fetch_one(&pool).await.unwrap();
    assert_eq!(remaining, 3);
    let refresh = response_cookie(&completed.1, "new_api_refresh");
    assert!(refresh.contains("HttpOnly; Secure; SameSite=Strict"));
    assert!(response_cookie(&completed.1, "__Host-lmm_mfa_flow").contains("Max-Age=0"));
    let after = identity_counts(&pool).await;
    let replay = web_call(
        &app,
        "POST",
        "/api/user/login/2fa",
        Some(cookie_pair(&binding)),
        Some(completion),
    )
    .await;
    assert_eq!(replay.0, StatusCode::BAD_REQUEST);
    assert_eq!(replay.2["code"], "MFA_FLOW_EXPIRED");
    assert!(!has_refresh_cookie(&replay.1));
    assert_eq!(identity_counts(&pool).await, after);
    let consumed_pending = web_call(
        &app,
        "GET",
        "/api/oauth/google?external_mfa=1",
        Some(&pending_cookies),
        None,
    )
    .await;
    assert_eq!(consumed_pending.0, StatusCode::BAD_REQUEST);
    assert_eq!(consumed_pending.2["code"], "MFA_FLOW_EXPIRED");
    assert!(consumed_pending.2["data"].get("flow_token").is_none());
    assert!(!has_refresh_cookie(&consumed_pending.1));
    assert!(
        consumed_pending
            .1
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none()
    );
    assert_eq!(identity_counts(&pool).await, after);
    let refreshed = web_call(
        &app,
        "POST",
        "/api/user/auth/refresh",
        Some(cookie_pair(&refresh)),
        None,
    )
    .await;
    assert_eq!(refreshed.0, StatusCode::OK);
    assert_eq!(refreshed.2["data"]["session"]["login_method"], "2fa");
    let sessions = browser
        .login_sessions(refreshed.2["data"]["access_token"].as_str().unwrap())
        .await
        .unwrap();
    let current = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["current"] == true)
        .unwrap();
    assert_eq!(current["login_method"], "2fa");
    let actor = store
        .authorize(refreshed.2["data"]["access_token"].as_str().unwrap())
        .await
        .unwrap();
    let level: i16 = sqlx::query_scalar(
        "SELECT factor_level FROM core_identity.session_authentication WHERE credential_id=$1",
    )
    .bind(actor.credential_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(level, 2);
    let exchanges = mock.exchanges.load(Ordering::SeqCst);
    assert_eq!(
        web_call(&app, "GET", &callback, Some(&cookie), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(mock.exchanges.load(Ordering::SeqCst), exchanges);
    server.abort();
}
