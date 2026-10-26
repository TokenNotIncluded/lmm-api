use super::*;
use axum::body::{Body, to_bytes};
use ring::{digest as ring_digest, hmac as ring_hmac};
use sqlx::PgPool;
use std::collections::BTreeMap;
use tower::ServiceExt;

const ORIGIN: &str = "https://telegram.example.invalid";
const BOT_TOKEN: &str = "123456789:synthetic-only-widget-signing-key";

fn config() -> LegacyAuthConfig {
    LegacyAuthConfig::new(ORIGIN, &[43; 32])
        .unwrap()
        .with_legal_acceptance_required(false)
}

fn fields() -> Vec<(String, String)> {
    [
        ("id", "900000000047"),
        ("auth_date", "2000000000"),
        ("hash", "synthetic-deserialization-only"),
        ("first_name", "Synthetic profile"),
        ("last_name", ""),
        ("username", "synthetic_profile"),
        ("photo_url", "https://example.invalid/photo.png"),
        ("lang", "zh-TW"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn telegram_query_preserves_duplicate_rejection_and_separates_state() {
    let mut valid = fields();
    valid.push(("state".into(), "lmmt_synthetic-correlation".into()));
    let (state, _) = authorization(valid.clone(), true).unwrap();
    assert_eq!(state.as_deref(), Some("lmmt_synthetic-correlation"));
    assert!(authorization(fields(), false).is_ok());
    assert!(authorization(fields(), true).is_err());
    assert!(authorization(valid.clone(), false).is_err());

    for (name, value) in valid.clone() {
        let mut duplicate = valid.clone();
        duplicate.push((name, value));
        assert!(authorization(duplicate, true).is_err());
    }
    for required in ["id", "auth_date", "hash"] {
        let missing = valid
            .iter()
            .filter(|(name, _)| name != required)
            .cloned()
            .collect();
        assert!(authorization(missing, true).is_err());
    }
    for unknown in ["access_token", "bot_token", "role", "user", "State"] {
        let mut extra = valid.clone();
        extra.push((unknown.into(), "untrusted".into()));
        assert!(authorization(extra, true).is_err());
    }
}

#[test]
fn telegram_flow_cookies_are_host_scoped_and_purpose_specific() {
    let secure = config();
    let local = LegacyAuthConfig::new("http://api.lmm.local", &[43; 32])
        .unwrap()
        .with_insecure_local_http()
        .unwrap();
    let value = "a".repeat(64);
    for (config, prefix, secure_attribute) in [(&secure, "__Host-", "Secure; "), (&local, "", "")] {
        for (purpose, same_site) in [("login", "Strict"), ("bind", "Lax")] {
            let name = cookie_name(config, purpose);
            assert_eq!(name, format!("{prefix}lmm_telegram_{purpose}_flow"));
            for (binding, age) in [(Some(value.as_str()), 300), (None, 0)] {
                let response = flow_cookie(success(None), binding, config, purpose);
                let cookies: Vec<_> = response
                    .headers()
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .collect();
                assert_eq!(cookies.len(), 1);
                assert_eq!(
                    cookies[0].to_str().unwrap(),
                    format!(
                        "{name}={}; Path=/; {secure_attribute}HttpOnly; SameSite={same_site}; Max-Age={age}",
                        binding.unwrap_or_default()
                    )
                );
                assert!(!cookies[0].to_str().unwrap().contains("Domain="));
            }
        }
    }
}

#[test]
fn telegram_binding_cookie_rejects_duplicates_malformed_and_other_purposes() {
    let config = config();
    let login = cookie_name(&config, "login");
    let bind = cookie_name(&config, "bind");
    let value = "a".repeat(64);
    let other = "b".repeat(64);
    let mut headers = HeaderMap::new();
    assert!(binding(&headers, &config, "login").is_err());
    for invalid in [
        login.to_string(),
        format!("{login}="),
        format!("{login}={}", "a".repeat(63)),
        format!("{login}={}", "a".repeat(65)),
        format!("{login}={}", "g".repeat(64)),
        format!("{login}={value}; {login}={value}"),
        format!("{login}={value}; {login}"),
        format!("{bind}={value}"),
        format!("lmm_telegram_login_flow={value}"),
        format!("new_api_refresh={value}"),
    ] {
        headers.insert(header::COOKIE, HeaderValue::from_str(&invalid).unwrap());
        assert!(binding(&headers, &config, "login").is_err());
    }
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("{login}={value}; {bind}={other}")).unwrap(),
    );
    assert_eq!(binding(&headers, &config, "login").unwrap(), value);
    assert_eq!(binding(&headers, &config, "bind").unwrap(), other);
    headers.append(
        header::COOKIE,
        HeaderValue::from_str(&format!("{login}={value}")).unwrap(),
    );
    assert!(binding(&headers, &config, "login").is_err());
    assert_eq!(binding(&headers, &config, "bind").unwrap(), other);
}

fn redirect_url(response: &Response) -> reqwest::Url {
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    reqwest::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap()
}

#[test]
fn telegram_bind_redirect_contains_only_consumed_flow_correlation() {
    let config = config();
    let flow = format!("lmmt_{}", "a".repeat(64));
    let response = bind_redirect(&config, &flow, "success", None);
    let url = redirect_url(&response);
    assert_eq!(url.origin().ascii_serialization(), ORIGIN);
    assert_eq!(url.path(), "/oauth/telegram");
    assert!(url.fragment().is_none());
    assert_eq!(
        url.query_pairs().into_owned().collect::<Vec<_>>(),
        [
            ("telegram_bind".into(), "success".into()),
            ("flow_token".into(), flow),
        ]
    );
    assert_eq!(
        response.headers()[header::SET_COOKIE],
        "__Host-lmm_telegram_bind_flow=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0"
    );
    for malformed in [
        "access_token=do-not-reflect".to_owned(),
        format!("lmmt_{}", "a".repeat(63)),
        format!("lmmt_{}", "g".repeat(64)),
        format!("lmmt_{}😺", "a".repeat(60)),
        format!("lmmt_{}?hash=do-not-reflect", "a".repeat(64)),
    ] {
        let response = bind_redirect(
            &config,
            &malformed,
            "error",
            Some("TELEGRAM_BIND_FLOW_INVALID"),
        );
        let url = redirect_url(&response);
        assert_eq!(
            url.query_pairs().into_owned().collect::<Vec<_>>(),
            [
                ("telegram_bind".into(), "error".into()),
                ("error_code".into(), "TELEGRAM_BIND_FLOW_INVALID".into()),
            ]
        );
    }
}

async fn setup(pool: &PgPool) -> (IdentityStore, Router) {
    crate::core_schema::init(pool).await.unwrap();
    // Fresh installation must contain Telegram; never repair a missing
    // component in the fixture and silently bypass installer integration.
    let installed: bool =
        sqlx::query_scalar("SELECT to_regclass('core_telegram_auth.flows') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        installed,
        "compiled fresh installer must include Telegram component"
    );
    let store = IdentityStore::from_pool(pool.clone())
        .with_mfa_keys(
            "telegram-test".into(),
            vec![("telegram-test".into(), vec![47; 32])],
        )
        .unwrap()
        .with_telegram_login(
            crate::identity::TelegramLogin::new("lmm_test_bot", BOT_TOKEN).unwrap(),
        )
        .unwrap()
        .with_registration_enabled(false);
    let app = super::super::router(store.clone(), config());
    (store, app)
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    peer: &str,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 65_536).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    (status, headers, body)
}

fn cookie_from(headers: &HeaderMap, purpose: &str) -> String {
    let name = cookie_name(&config(), purpose);
    let matching: Vec<_> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .filter(|value| value.starts_with(&format!("{name}=")))
        .collect();
    assert_eq!(matching.len(), 1);
    matching[0].split(';').next().unwrap().to_owned()
}

fn signed_query(subject: &str, marker: &str) -> Vec<(String, String)> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut fields = BTreeMap::from([
        ("auth_date".to_owned(), now.to_string()),
        ("first_name".to_owned(), marker.to_owned()),
        ("id".to_owned(), subject.to_owned()),
    ]);
    // Sign independently from the production RustCrypto verifier. No provider
    // network, real bot credential, assertion fixture, or verifier is reused.
    let data = fields
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let key = ring_digest::digest(&ring_digest::SHA256, BOT_TOKEN.as_bytes());
    let key = ring_hmac::Key::new(ring_hmac::HMAC_SHA256, key.as_ref());
    let signature = ring_hmac::sign(&key, data.as_bytes());
    let hash: String = signature
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    fields.insert("hash".into(), hash);
    fields.into_iter().collect()
}

fn callback(path: &str, fields: Vec<(String, String)>) -> String {
    let mut url = reqwest::Url::parse(ORIGIN).unwrap();
    url.set_path(path);
    url.query_pairs_mut().extend_pairs(fields);
    format!("{}?{}", url.path(), url.query().unwrap())
}

async fn login_account(store: &IdentityStore) -> BrowserBundle {
    let browser =
        BrowserAuth::new(store.clone().with_registration_enabled(true), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "telegram-http-test",
            password: "synthetic-pass-47",
            email: "",
        })
        .await
        .unwrap();
    browser
        .login(
            "telegram-http-test",
            "synthetic-pass-47",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn telegram_http_login_get_requires_origin_before_consuming_flow(pool: PgPool) {
    let (_, app) = setup(&pool).await;
    let (status, headers, body) = call(
        &app,
        "POST",
        "/api/oauth/telegram/login/start",
        &[("origin", ORIGIN)],
        "192.0.2.47:12001",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_from(&headers, "login");
    let mut fields = signed_query("900000000047", "Unbound synthetic login");
    fields.push((
        "state".into(),
        body["data"]["flow_token"].as_str().unwrap().into(),
    ));
    let path = callback("/api/oauth/telegram/login", fields);
    for extra in [None, Some("https://attacker.example.invalid")] {
        let mut headers = vec![("cookie", cookie.as_str())];
        if let Some(origin) = extra {
            headers.push(("origin", origin));
        }
        let (status, _, body) = call(&app, "GET", &path, &headers, "192.0.2.47:12001").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "AUTH_FORBIDDEN");
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_telegram_auth.flows WHERE consumed_at IS NOT NULL),(SELECT count(*) FROM core_telegram_auth.assertions)")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(counts, (0, 0));
    }
    let (status, _, body) = call(
        &app,
        "GET",
        &path,
        &[("cookie", &cookie), ("origin", ORIGIN)],
        "192.0.2.47:12001",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "AUTH_UNAUTHORIZED");
    let counts: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_telegram_auth.flows WHERE consumed_at IS NOT NULL),(SELECT count(*) FROM core_telegram_auth.assertions),(SELECT count(*) FROM core_identity.users),(SELECT count(*) FROM core_identity.web_sessions)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 0, 0));
}

#[sqlx::test(migrations = false)]
async fn telegram_http_get_login_and_bind_callbacks_share_browser_rate_limits(pool: PgPool) {
    let (_, app) = setup(&pool).await;
    for (path, peer, expected) in [
        (
            "/api/oauth/telegram/login",
            "192.0.2.48:12001",
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/oauth/telegram/bind/invalid",
            "192.0.2.49:12001",
            StatusCode::SEE_OTHER,
        ),
    ] {
        for _ in 0..60 {
            let response = call(&app, "GET", path, &[], peer).await;
            assert_eq!(response.0, expected);
        }
        let (status, headers, body) = call(&app, "GET", path, &[], peer).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(headers[header::RETRY_AFTER], "60");
        assert_eq!(body["code"], "AUTH_RATE_LIMITED");
        assert!(!headers.contains_key(header::SET_COOKIE));
        let other_path = if path == "/api/oauth/telegram/login" {
            "/api/oauth/telegram/bind/invalid"
        } else {
            "/api/oauth/telegram/login"
        };
        let response = call(&app, "GET", other_path, &[], peer).await;
        assert_eq!(response.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.2["code"], "AUTH_RATE_LIMITED");
    }
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.users),(SELECT count(*) FROM core_telegram_auth.flows),(SELECT count(*) FROM core_telegram_auth.assertions)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (0, 0, 0));
}

#[sqlx::test(migrations = false)]
async fn telegram_http_bind_requires_bearer_start_then_cookie_only_callback(pool: PgPool) {
    let (store, app) = setup(&pool).await;
    let bundle = login_account(&store).await;
    let access = bundle.data["access_token"].as_str().unwrap();
    let sid = bundle.data["session"]["sid"].as_str().unwrap();
    let bearer = format!("Bearer {access}");
    let (status, _, body) = call(
        &app,
        "POST",
        "/api/oauth/telegram/bind/start",
        &[("origin", ORIGIN)],
        "192.0.2.50:12001",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "AUTH_UNAUTHORIZED");
    let (status, headers, body) = call(
        &app,
        "POST",
        "/api/oauth/telegram/bind/start",
        &[
            ("origin", ORIGIN),
            ("authorization", &bearer),
            ("x-auth-session", sid),
        ],
        "192.0.2.50:12001",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_from(&headers, "bind");
    let flow = body["data"]["flow_token"].as_str().unwrap();
    let url = reqwest::Url::parse(body["data"]["callback_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.origin().ascii_serialization(), ORIGIN);
    assert!(url.path() == format!("/api/oauth/telegram/bind/{flow}"));
    assert!(url.query().is_none());
    assert!(url.fragment().is_none());
    let path = callback(
        url.path(),
        signed_query("900000000050", "Synthetic binding"),
    );
    let (status, headers, _) = call(&app, "GET", &path, &[], "192.0.2.50:12001").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let failure = reqwest::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    assert!(
        failure
            .query_pairs()
            .any(|(name, value)| name == "telegram_bind" && value == "error")
    );
    let before: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_telegram_auth.bindings),(SELECT count(*) FROM core_telegram_auth.assertions)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(before, (0, 0));

    // A top-level provider navigation supplies neither bearer nor Origin nor
    // the refresh cookie, whose Path remains /api/user/auth.
    let (status, headers, body) = call(
        &app,
        "GET",
        &path,
        &[("cookie", &cookie)],
        "192.0.2.50:12001",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(body.is_null());
    let success = reqwest::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    assert!(
        success.query_pairs().into_owned().collect::<Vec<_>>()
            == [
                ("telegram_bind".into(), "success".into()),
                ("flow_token".into(), flow.into())
            ]
    );
    let names: Vec<_> = headers.get_all(header::SET_COOKIE).iter().collect();
    assert_eq!(names.len(), 1);
    assert_eq!(
        cookie_from(&headers, "bind"),
        "__Host-lmm_telegram_bind_flow="
    );
    let owner: i64 = sqlx::query_scalar("SELECT user_id FROM core_telegram_auth.bindings WHERE subject='900000000050' AND revoked_at IS NULL")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(owner, bundle.data["user"]["id"].as_i64().unwrap());
    let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_telegram_auth.bindings),(SELECT count(*) FROM core_telegram_auth.assertions),(SELECT count(*) FROM core_identity.web_sessions)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(after, (1, 1, 1));

    let (_, headers, _) = call(
        &app,
        "GET",
        &path,
        &[("cookie", &cookie)],
        "192.0.2.50:12001",
    )
    .await;
    let replay = reqwest::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    assert!(
        replay
            .query_pairs()
            .any(|(name, value)| name == "telegram_bind" && value == "error")
    );
    let unchanged: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_telegram_auth.bindings),(SELECT count(*) FROM core_telegram_auth.assertions),(SELECT count(*) FROM core_identity.web_sessions)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(after, unchanged);
}
