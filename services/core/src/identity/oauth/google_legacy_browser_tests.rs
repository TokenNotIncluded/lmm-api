use super::*;
use crate::identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Request},
};
use totp_rs::{Algorithm as TotpAlgorithm, Builder, Secret};
use tower::ServiceExt;

const SPA_CALLBACK: &str = "https://example.com/oauth/google";

async fn fixture(pool: &PgPool) -> (IdentityStore, MockProvider, tokio::task::JoinHandle<()>) {
    let mock = MockProvider {
        redirect_uri: Some(Arc::from(SPA_CALLBACK)),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let provider = Router::new()
        .route("/token", post(exchange))
        .route("/keys", get(public_keys))
        .with_state(mock.clone());
    let task = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let mut google = GoogleOAuth::new(
        "client-1".into(),
        "test-only-client-secret".into(),
        SPA_CALLBACK.into(),
    )
    .unwrap();
    google.client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    google.token_endpoint = Url::parse(&format!("http://{address}/token")).unwrap();
    google.keys_endpoint = Url::parse(&format!("http://{address}/keys")).unwrap();
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_google_oauth(google)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    crate::core_schema::init(pool).await.unwrap();
    (store, mock, task)
}

fn cookie(headers: &HeaderMap, name: &str) -> String {
    let prefix = format!("{name}=");
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .unwrap_or_else(|| panic!("expected cookie {name}"))
        .to_owned()
}

fn pair(raw: &str) -> &str {
    raw.split(';').next().unwrap()
}

fn has_refresh(headers: &HeaderMap) -> bool {
    headers.get_all("set-cookie").iter().any(|value| {
        value
            .to_str()
            .is_ok_and(|value| value.starts_with("new_api_refresh="))
    })
}

async fn spa_callback(app: &Router, path: &str, binding: &str) -> (StatusCode, HeaderMap, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("cookie", binding)
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["referrer-policy"], "no-referrer");
    let body =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    (status, headers, body)
}

async fn public_status(store: &IdentityStore) -> Value {
    let config =
        crate::legacy_auth::LegacyAuthConfig::new("https://example.com", &[43; 32]).unwrap();
    let app = crate::http::configured_router_with_rpc(
        store.pool.clone(),
        crate::http::Capabilities::default(),
        None,
        crate::http::PublicStatus {
            google_client_id: config.google_client_id(store).map(str::to_owned),
            google_browser_callback: config.google_browser_callback(store).map(str::to_owned),
            two_factor_enabled: store.mfa_enabled(),
            ..Default::default()
        },
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap()
}

#[tokio::test]
async fn google_spa_callback_configuration_requires_exact_canonical_origin_and_known_path() {
    // No database connection is made by this configuration/status projection.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    for (path, browser) in [
        ("/core/v1/auth/oauth/google/callback", false),
        ("/api/oauth/google/callback", true),
        ("/api/oauth/google", true),
        ("/oauth/google", true),
    ] {
        let google = GoogleOAuth::new(
            "client-1".into(),
            "test-only-client-secret".into(),
            format!("https://example.com{path}"),
        )
        .unwrap();
        let store = IdentityStore::from_pool(pool.clone()).with_google_oauth(google);
        assert_eq!(
            store.google_browser_callback("https://example.com"),
            browser.then_some(path)
        );
        let status = public_status(&store).await;
        assert_eq!(status["data"]["google_oauth"], browser);
        assert_eq!(
            status["data"]["google_browser_callback"],
            if browser { json!(path) } else { Value::Null }
        );
        assert!(status["data"].get("google_client_secret").is_none());
    }
    for origin in [
        "https://foreign.invalid",
        "https://example.com.attacker.invalid",
        "https://example.com:8443",
    ] {
        // The constructor has no site-origin argument. A canonical foreign
        // URI is valid configuration, but cannot enable this site's bridge.
        let google = GoogleOAuth::new(
            "client-1".into(),
            "test-only-client-secret".into(),
            format!("{origin}/oauth/google"),
        )
        .unwrap();
        let store = IdentityStore::from_pool(pool.clone()).with_google_oauth(google);
        assert!(
            store
                .google_browser_callback("https://example.com")
                .is_none()
        );
        let status = public_status(&store).await;
        assert_eq!(status["data"]["google_oauth"], false);
        assert!(status["data"]["google_browser_callback"].is_null());
    }
    for uri in [
        "http://example.com/oauth/google",
        "https://user@example.com/oauth/google",
        "https://user:password@example.com/oauth/google",
        "https://example.com/oauth/google?next=https://foreign.invalid",
        "https://example.com/oauth/google#fragment",
        "https://example.com/oauth/google/",
        "https://example.com/oauth/google.evil",
        "https://example.com//oauth/google",
        "https://example.com/other/../oauth/google",
        "https://example.com/%2e%2e/oauth/google",
        "https://example.com/%6fauth/google",
        "https://example.com/oauth%2fgoogle",
        "https://EXAMPLE.COM/oauth/google",
        "https://example.com:443/oauth/google",
        " https://example.com/oauth/google",
        "https://example.com/oauth/google ",
        "https://example.com/oauth/google\n",
        "https://example.com\\oauth\\google",
    ] {
        assert!(
            GoogleOAuth::new(
                "client-1".into(),
                "test-only-client-secret".into(),
                uri.into()
            )
            .is_err()
        );
    }
}

#[sqlx::test(migrations = false)]
async fn google_spa_callback_exchanges_original_redirect_and_returns_bound_json_session_once(
    pool: PgPool,
) {
    let (store, mock, provider) = fixture(&pool).await;
    let app = web_app(store.clone());
    let status = public_status(&store).await;
    assert_eq!(status["data"]["google_browser_callback"], "/oauth/google");
    assert_eq!(status["data"]["google_oauth"], true);
    let started = web_call(
        &app,
        "POST",
        "/api/oauth/google",
        None,
        Some(json!({"accepted_legal":true})),
    )
    .await;
    assert_eq!(started.0, StatusCode::OK);
    let binding = cookie(&started.1, "__Host-lmm_web_google_flow");
    assert!(binding.contains("Secure; HttpOnly; SameSite=Lax"));
    let url = started.2["data"]["authorization_url"].as_str().unwrap();
    let parsed = Url::parse(url).unwrap();
    assert_eq!(parsed.origin().ascii_serialization(), ISSUER);
    let parameters: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(parameters["redirect_uri"], SPA_CALLBACK);
    assert_eq!(parameters["code_challenge_method"], "S256");
    let (state, code) = prepare_code(&store, &mock, url, "spa-google-subject").await;
    let path = format!("/api/oauth/google?state={state}&code={code}");
    let before = identity_counts(&pool).await;
    let wrong = format!("__Host-lmm_web_google_flow={}", "e".repeat(64));
    let duplicate = format!("{}; {}", pair(&binding), pair(&binding));
    for cookies in [None, Some(wrong.as_str()), Some(duplicate.as_str())] {
        let rejected = web_call(&app, "GET", &path, cookies, None).await;
        assert_eq!(rejected.0, StatusCode::UNAUTHORIZED);
        assert!(!has_refresh(&rejected.1));
        assert_eq!(identity_counts(&pool).await, before);
    }
    assert_eq!(mock.exchanges.load(Ordering::SeqCst), 0);
    // The shared provider also requires redirect_uri=SPA_CALLBACK when it
    // validates the code exchange, not merely in the authorization URL.
    // Accept is caller-controlled and cannot change the configured SPA mode.
    let response = spa_callback(&app, &path, pair(&binding)).await;
    assert_eq!(response.0, StatusCode::OK);
    assert!(response.1.get("location").is_none());
    assert_eq!(response.2["success"], true);
    let actor = store
        .authorize(response.2["data"]["access_token"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(actor.platform_level, 0);
    assert_eq!(response.2["data"]["session"]["login_method"], "google");
    let refresh = cookie(&response.1, "new_api_refresh");
    assert!(refresh.contains("HttpOnly; Secure; SameSite=Strict"));
    assert!(refresh.contains("Path=/api/user/auth"));
    assert!(cookie(&response.1, "__Host-lmm_web_google_flow").contains("Max-Age=0"));
    assert_eq!(mock.exchanges.load(Ordering::SeqCst), 1);
    let before_replay = identity_counts(&pool).await;
    let replay = web_call(&app, "GET", &path, Some(pair(&binding)), None).await;
    assert_eq!(replay.0, StatusCode::UNAUTHORIZED);
    assert!(!has_refresh(&replay.1));
    assert_eq!(identity_counts(&pool).await, before_replay);
    assert_eq!(mock.exchanges.load(Ordering::SeqCst), 1);
    let refreshed = web_call(
        &app,
        "POST",
        "/api/user/auth/refresh",
        Some(pair(&refresh)),
        None,
    )
    .await;
    assert_eq!(refreshed.0, StatusCode::OK);
    assert_eq!(refreshed.2["data"]["user"]["id"], actor.user_id);
    assert!(refreshed.2["data"]["session"]["sid"] == response.2["data"]["session"]["sid"]);
    for (claim, bad) in [
        ("iss", json!("https://foreign.invalid")),
        ("nonce", json!("b".repeat(64))),
        ("__test_bad_signature", json!(true)),
    ] {
        let (binding, state, code) = web_flow(&app, &store, &mock, "untrusted-spa-subject").await;
        mock.codes.lock().await.get_mut(&code).unwrap().0[claim] = bad;
        let before = identity_counts(&pool).await;
        let path = format!("/api/oauth/google?state={state}&code={code}");
        let response = web_call(&app, "GET", &path, Some(&binding), None).await;
        assert_eq!(response.0, StatusCode::UNAUTHORIZED);
        assert!(!has_refresh(&response.1));
        assert_eq!(identity_counts(&pool).await, before);
    }
    provider.abort();
}

#[sqlx::test(migrations = false)]
async fn google_spa_callback_returns_json_mfa_without_session_then_completes_once(pool: PgPool) {
    let (store, mock, provider) = fixture(&pool).await;
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "spa-mfa",
            password: "test passphrase",
            email: "",
        })
        .await
        .unwrap();
    let original = browser
        .login(
            "spa-mfa",
            "test passphrase",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let original_access = original.data["access_token"].as_str().unwrap();
    let user = original.data["user"]["id"].as_i64().unwrap();
    sqlx::query(
        "INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) VALUES ($1,$2,$3)",
    )
    .bind(ISSUER)
    .bind("spa-mfa-subject")
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
    assert!(store.authorize(original_access).await.is_err());
    let closed = store.clone().with_registration_enabled(false);
    let app = web_app(closed.clone());
    let status = public_status(&closed).await;
    assert_eq!(status["data"]["two_factor_enabled"], true);
    let (binding, state, code) = web_flow(&app, &closed, &mock, "spa-mfa-subject").await;
    let before = identity_counts(&pool).await;
    let path = format!("/api/oauth/google?state={state}&code={code}");
    let response = spa_callback(&app, &path, &binding).await;
    assert_eq!(response.0, StatusCode::OK);
    assert!(response.1.get("location").is_none());
    assert_eq!(response.2["success"], true);
    assert_eq!(response.2["data"]["require_2fa"], true);
    assert!(response.2["data"].get("access_token").is_none());
    assert!(!has_refresh(&response.1));
    assert_eq!(identity_counts(&pool).await, before);
    let mfa_binding = cookie(&response.1, "__Host-lmm_mfa_flow");
    assert!(mfa_binding.contains("Secure; HttpOnly; SameSite=Strict"));
    let flow = response.2["data"]["flow_token"].as_str().unwrap();
    // Recovery completes the actual factor without reusing enrollment's TOTP.
    let input = json!({"flow_token":flow,"code":setup.backup_codes[0]});
    let wrong_binding = format!("__Host-lmm_mfa_flow={}", "d".repeat(64));
    let rejected = web_call(
        &app,
        "POST",
        "/api/user/login/2fa",
        Some(&wrong_binding),
        Some(input.clone()),
    )
    .await;
    assert_eq!(rejected.0, StatusCode::BAD_REQUEST);
    assert_eq!(rejected.2["code"], "MFA_FLOW_EXPIRED");
    assert!(!has_refresh(&rejected.1));
    assert_eq!(identity_counts(&pool).await, before);
    let response = web_call(
        &app,
        "POST",
        "/api/user/login/2fa",
        Some(pair(&mfa_binding)),
        Some(input.clone()),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.2["data"]["user"]["id"], user);
    assert!(response.2["data"]["session"]["sid"] != enabled.data["session"]["sid"]);
    let access = response.2["data"]["access_token"].as_str().unwrap();
    let actor = store.authorize(access).await.unwrap();
    let proof: (i16, String, bool) = sqlx::query_as("SELECT factor_level,method,mfa_verified_at IS NOT NULL FROM core_identity.session_authentication WHERE credential_id=$1")
        .bind(actor.credential_id).fetch_one(&pool).await.unwrap();
    assert_eq!(proof, (2, "recovery".into(), true));
    let refresh = cookie(&response.1, "new_api_refresh");
    assert!(refresh.contains("HttpOnly; Secure; SameSite=Strict"));
    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/user/2fa/status")
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    assert_eq!(status.headers()["cache-control"], "no-store");
    let status: Value =
        serde_json::from_slice(&to_bytes(status.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(status["data"]["enabled"], true);
    assert_eq!(status["data"]["backup_codes_remaining"], 3);
    let after = identity_counts(&pool).await;
    let replay = web_call(
        &app,
        "POST",
        "/api/user/login/2fa",
        Some(pair(&mfa_binding)),
        Some(input),
    )
    .await;
    assert_eq!(replay.0, StatusCode::BAD_REQUEST);
    assert_eq!(replay.2["code"], "MFA_FLOW_EXPIRED");
    assert!(!has_refresh(&replay.1));
    assert_eq!(identity_counts(&pool).await, after);
    let exchanged = mock.exchanges.load(Ordering::SeqCst);
    assert_eq!(
        web_call(&app, "GET", &path, Some(&binding), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(mock.exchanges.load(Ordering::SeqCst), exchanged);
    provider.abort();
}
