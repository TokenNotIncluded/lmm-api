//! Fresh, synthetic PostgreSQL and real HTTP. No existing account or provider.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lmm_core::{
    core_schema,
    identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore},
    market_oauth::{
        self, AuthorizationRequest, Error, MarketIssuer, MarketProfile, Registration, TokenForm,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
#[allow(dead_code)]
mod runtime;
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
fn registration(name: &str) -> Registration {
    Registration {
        client_name: name.into(),
        application_type: "native".into(),
        redirect_uris: vec!["http://127.0.0.1:53123/callback".into()],
        token_endpoint_auth_method: "none".into(),
        grant_types: vec!["authorization_code".into(), "refresh_token".into()],
        response_types: vec!["code".into()],
        scope: "market:discover market:invoke market:manage".into(),
    }
}
fn request(issuer: &MarketIssuer, id: &str) -> AuthorizationRequest {
    AuthorizationRequest {
        response_type: "code".into(),
        client_id: id.into(),
        redirect_uri: "http://127.0.0.1:53124/callback".into(),
        scope: "market:discover market:invoke market:manage".into(),
        resource: issuer.profile().resource().into(),
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
        code_challenge_method: "S256".into(),
        state: "synthetic-state".into(),
    }
}
async fn init(pool: &PgPool) {
    core_schema::init(pool).await.unwrap();
    let present: bool =
        sqlx::query_scalar("SELECT to_regnamespace('core_market_oauth') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap();
    if !present {
        sqlx::raw_sql(market_oauth::SCHEMA)
            .execute(pool)
            .await
            .unwrap();
    }
}
async fn browser(pool: &PgPool, name: &str) -> String {
    let auth = BrowserAuth::new(
        IdentityStore::from_pool(pool.clone()).with_registration_enabled(true),
        &[44; 32],
    )
    .unwrap();
    auth.register(&BrowserRegistration {
        username: name,
        password: "password1234",
        email: "",
    })
    .await
    .unwrap();
    auth.login(
        name,
        "password1234",
        &BrowserSessionMetadata {
            ip: "203.0.113.32".into(),
            user_agent: "synthetic-market-oauth".into(),
        },
    )
    .await
    .unwrap()
    .refresh_token
}
async fn issuer(pool: &PgPool) -> MarketIssuer {
    MarketIssuer::open(
        pool.clone(),
        MarketProfile::isolated_loopback_test("http://127.0.0.1:55121", 1000).unwrap(),
    )
    .await
    .unwrap()
}
async fn code(i: &MarketIssuer, client: &str, cookie: &str) -> String {
    let p = i.begin(request(i, client)).await.unwrap();
    assert!(p.account.is_none());
    let p = i
        .continue_authorization(&p.binding, &p.csrf, cookie)
        .await
        .unwrap();
    assert!(p.account.is_some());
    let target = i
        .consent(&p.binding, &p.csrf, cookie, true, 500)
        .await
        .unwrap();
    reqwest::Url::parse(&target)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned()
}
fn exchange(i: &MarketIssuer, client: &str, code: &str) -> TokenForm {
    TokenForm {
        grant_type: "authorization_code".into(),
        client_id: client.into(),
        resource: i.profile().resource().into(),
        code: Some(code.into()),
        redirect_uri: Some("http://127.0.0.1:53124/callback".into()),
        code_verifier: Some(VERIFIER.into()),
        refresh_token: None,
        scope: None,
    }
}
fn refresh(i: &MarketIssuer, client: &str, token: &str, scope: Option<&str>) -> TokenForm {
    TokenForm {
        grant_type: "refresh_token".into(),
        client_id: client.into(),
        resource: i.profile().resource().into(),
        code: None,
        redirect_uri: None,
        code_verifier: None,
        refresh_token: Some(token.into()),
        scope: scope.map(str::to_owned),
    }
}
#[test]
fn production_and_dcr_transport_do_not_relax() {
    assert!(MarketProfile::isolated_loopback_test("http://127.0.0.1:12001", 1000).is_ok());
    assert!(
        MarketProfile::isolated_loopback_test("http://127.0.0.1:12001", 1000)
            .unwrap()
            .with_resource("http://127.0.0.2:12002/mcp/market")
            .is_err()
    );
    assert!(
        MarketProfile::isolated_loopback_test("http://[::1]:12001", 1000)
            .unwrap()
            .with_resource("http://[::1]:12002/mcp/market")
            .is_ok()
    );
    assert!(MarketProfile::isolated_loopback_test("http://[::1%25lo]:12001", 1000).is_err());
    let p = MarketProfile::isolated_loopback_test("http://127.0.0.1:55121", 1000)
        .unwrap()
        .with_resource("http://127.0.0.1:55122/mcp/market")
        .unwrap();
    assert_eq!(p.issuer(), "http://127.0.0.1:55121/api/oauth2/market");
    assert_eq!(p.resource(), "http://127.0.0.1:55122/mcp/market");
    assert!(
        MarketProfile::new("https://issuer.example.test", 1000)
            .unwrap()
            .with_resource("http://127.0.0.1:55122/mcp/market")
            .is_err()
    );
    for origin in [
        "http://example.com",
        "http://127.0.0.1:55121",
        "https://example.com/path",
        "https://user@example.com",
    ] {
        assert!(MarketProfile::new(origin, 1000).is_err())
    }
    assert!(MarketProfile::new("https://example.com", 1000).is_ok());
    assert!(MarketProfile::isolated_loopback_test("http://0.0.0.0:55121", 1000).is_err());
    assert!(serde_json::from_value::<Registration>(json!({"client_name":"evil","redirect_uris":[],"token_endpoint_auth_method":"none","grant_types":[],"response_types":[],"scope":"market:discover","jwks_uri":"https://private.invalid"})).is_err());
}
#[sqlx::test]
async fn real_browser_pkce_scope_resource_and_refresh_replay(pool: PgPool) {
    init(&pool).await;
    let i = issuer(&pool).await;
    let cookie = browser(&pool, "marketalice").await;
    let client = i
        .register(registration("Local MCP"))
        .await
        .unwrap()
        .client_id;
    let other = i
        .register(registration("Other MCP"))
        .await
        .unwrap()
        .client_id;
    let c = code(&i, &client, &cookie).await;
    assert!(matches!(
        i.token(exchange(&i, &other, &c)).await,
        Err(Error::Unauthorized)
    ));
    let mut wrong = exchange(&i, &client, &c);
    wrong.resource = "http://127.0.0.1:55121/api/oauth2".into();
    assert!(matches!(i.token(wrong).await, Err(Error::Unauthorized)));
    let mut wrong = exchange(&i, &client, &c);
    wrong.code_verifier = Some("x".repeat(43));
    assert!(matches!(i.token(wrong).await, Err(Error::Unauthorized)));
    let token = i.token(exchange(&i, &client, &c)).await.unwrap();
    assert!(token.refresh_token.is_some());
    assert!(matches!(
        i.token(exchange(&i, &client, &c)).await,
        Err(Error::Unauthorized)
    ));
    let a = i
        .validate(&token.access_token, &["market:discover", "market:invoke"])
        .await
        .unwrap();
    assert!(a.can_invoke());
    assert!(a.can_manage());
    assert_eq!(a.budget_credits(), 500);
    assert!(
        i.validate(&token.access_token, &["models:invoke"])
            .await
            .is_err()
    );
    let original = token.refresh_token.unwrap();
    assert!(matches!(
        i.token(refresh(&i, &other, &original, None)).await,
        Err(Error::Unauthorized)
    ));
    let narrowed = i
        .token(refresh(&i, &client, &original, Some("market:discover")))
        .await
        .unwrap();
    assert!(matches!(
        i.validate(
            &narrowed.access_token,
            &["market:discover", "market:invoke"]
        )
        .await,
        Err(Error::Forbidden)
    ));
    let latest = narrowed.refresh_token.unwrap();
    assert!(matches!(
        i.token(refresh(
            &i,
            &client,
            &latest,
            Some("market:discover market:manage")
        ))
        .await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        i.token(refresh(&i, &client, &original, None)).await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        i.validate(&narrowed.access_token, &["market:discover"])
            .await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        i.token(refresh(&i, &client, &latest, None)).await,
        Err(Error::Unauthorized)
    ));
}
#[sqlx::test]
async fn csrf_cookie_identity_denial_revocation_and_user_version(pool: PgPool) {
    init(&pool).await;
    let i = issuer(&pool).await;
    let alice = browser(&pool, "oauthalice").await;
    let bob = browser(&pool, "oauthbob").await;
    let client = i
        .register(registration("Scoped MCP"))
        .await
        .unwrap()
        .client_id;
    let p = i.begin(request(&i, &client)).await.unwrap();
    assert!(matches!(
        i.continue_authorization(&p.binding, &"0".repeat(64), &alice)
            .await,
        Err(Error::Unauthorized)
    ));
    let p = i
        .continue_authorization(&p.binding, &p.csrf, &alice)
        .await
        .unwrap();
    assert!(matches!(
        i.consent(&p.binding, &p.csrf, &bob, true, 500).await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        i.consent(&p.binding, &p.csrf, &alice, true, 500).await,
        Err(Error::Unauthorized)
    ));
    let p = i.begin(request(&i, &client)).await.unwrap();
    let p = i
        .continue_authorization(&p.binding, &p.csrf, &alice)
        .await
        .unwrap();
    let denied = i
        .consent(&p.binding, &p.csrf, &alice, false, 0)
        .await
        .unwrap();
    assert!(denied.contains("error=access_denied"));
    assert!(!denied.contains("code="));
    let c = code(&i, &client, &alice).await;
    let t = i.token(exchange(&i, &client, &c)).await.unwrap();
    let actor = i
        .validate(&t.access_token, &["market:discover"])
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1")
        .bind(actor.user_id())
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        i.validate(&t.access_token, &["market:discover"]).await,
        Err(Error::Unauthorized)
    ));
}
#[sqlx::test]
async fn public_http_metadata_dcr_and_no_credential_fallback(pool: PgPool) {
    init(&pool).await;
    let mut listener = None;
    for port in 55130..55200 {
        if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            listener = Some(l);
            break;
        }
    }
    let listener = listener.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let i = MarketIssuer::open(
        pool.clone(),
        MarketProfile::isolated_loopback_test(&origin, 1000).unwrap(),
    )
    .await
    .unwrap();
    let validator = i.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            market_oauth::http::router(i)
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let m = http
        .get(format!(
            "{origin}/.well-known/oauth-protected-resource/mcp/market"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(m.headers()["cache-control"], "no-store");
    let m: Value = m.json().await.unwrap();
    assert_eq!(m["resource"], format!("{origin}/mcp/market"));
    assert_eq!(
        m["authorization_servers"][0],
        format!("{origin}/api/oauth2/market")
    );
    let url = format!("{origin}/api/oauth2/market/register");
    let r = http
        .post(&url)
        .json(&registration("Real HTTP"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let r: Value = r.json().await.unwrap();
    assert!(r["client_id"].as_str().unwrap().starts_with("lmm-mcp-"));
    let cookie = browser(&pool, "httpmarket").await;
    let client = r["client_id"].as_str().unwrap();
    let authorization = request(&validator, client);
    let query = [
        ("response_type", authorization.response_type.as_str()),
        ("client_id", client),
        ("redirect_uri", authorization.redirect_uri.as_str()),
        ("scope", authorization.scope.as_str()),
        ("resource", authorization.resource.as_str()),
        ("code_challenge", authorization.code_challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", "synthetic-state"),
    ];
    let form = |html: &str| {
        let csrf = regex::Regex::new("name=csrf value=([a-f0-9]{64})")
            .unwrap()
            .captures(html)
            .unwrap()[1]
            .to_string();
        let binding =
            regex::Regex::new("action=/api/oauth2/market/(?:continue|consent)/([a-f0-9]{64})")
                .unwrap()
                .captures(html)
                .unwrap()[1]
                .to_string();
        (binding, csrf)
    };
    let html = http
        .get(format!("{origin}/api/oauth2/market/authorize"))
        .query(&query)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let (binding, csrf) = form(&html);
    let continued = http
        .post(format!("{origin}/api/oauth2/market/continue/{binding}"))
        .header("Origin", &origin)
        .header("Cookie", format!("new_api_refresh={cookie}"))
        .form(&[("csrf", csrf)])
        .send()
        .await
        .unwrap();
    assert_eq!(continued.status(), 200);
    let (binding, csrf) = form(&continued.text().await.unwrap());
    let consent = http
        .post(format!("{origin}/api/oauth2/market/consent/{binding}"))
        .header("Origin", &origin)
        .header("Cookie", format!("new_api_refresh={cookie}"))
        .form(&[
            ("csrf", csrf),
            ("decision", "allow".into()),
            ("budget_credits", "500".into()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(consent.status(), 303);
    let location = reqwest::Url::parse(consent.headers()["location"].to_str().unwrap()).unwrap();
    let code = location
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let token = http
        .post(format!("{origin}/api/oauth2/market/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("resource", validator.profile().resource()),
            ("code", &code),
            ("redirect_uri", authorization.redirect_uri.as_str()),
            ("code_verifier", VERIFIER),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token.status(), 200);
    let token: Value = token.json().await.unwrap();
    let raw = token["access_token"].as_str().unwrap();
    let principal = validator
        .validate(raw, &["market:discover", "market:invoke"])
        .await
        .unwrap();
    assert_eq!(principal.budget_credits(), 500);
    let private_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let private_origin = format!("http://{}", private_listener.local_addr().unwrap());
    let private_router =
        market_oauth::http::private_router(validator.clone(), &[b'k'; 32]).unwrap();
    let private =
        tokio::spawn(async move { axum::serve(private_listener, private_router).await.unwrap() });
    let endpoint = format!("{private_origin}/internal/tool-market/v1/oauth-authorize");
    let body = json!({"token":raw,"required_scopes":["market:discover","market:invoke"]});
    let denied = http
        .post(&endpoint)
        .header("X-Lmm-Protocol", "1")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let authorized = http
        .post(&endpoint)
        .header("X-Lmm-Protocol", "1")
        .bearer_auth("k".repeat(32))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), 200);
    let a: Value = authorized.json().await.unwrap();
    assert_eq!(a["credential_kind"], "oauth");
    assert_eq!(a["user_id"], a["account_id"]);
    assert_eq!(a["client_id"], format!("oauth:{client}"));
    assert_eq!(a["resource"], validator.profile().resource());
    let mixed = http
        .post(&endpoint)
        .header("X-Lmm-Protocol", "1")
        .header("X-Lmm-User-Credential", "cannot-substitute-session")
        .bearer_auth("k".repeat(32))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(mixed.status(), 400);
    private.abort();
    let _ = private.await;
    let r = http
        .post(&url)
        .bearer_auth("a-session-cannot-register")
        .json(&registration("No fallback"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    server.abort();
    let _ = server.await;
}
#[sqlx::test]
async fn exact_runtime_role_can_issue_without_model_or_group_grant(pool: PgPool) {
    init(&pool).await;
    let cookie = browser(&pool, "leastprivmarket").await;
    let role = runtime::RuntimeRole::create(&pool).await;
    let required = lmm_core::runtime_database::RequiredCapabilities {
        market_oauth: true,
        ..Default::default()
    };
    assert_eq!(
        lmm_core::runtime_database::check(&role.pool, required).await,
        Err(lmm_core::runtime_database::RuntimeDatabaseError::MissingPrivileges)
    );
    role.optional("core_market_oauth").await;
    lmm_core::runtime_database::check(&role.pool, required)
        .await
        .unwrap();
    for grant in [
        "SELECT ON core_market_oauth.clients",
        "INSERT ON core_market_oauth.tokens",
        "UPDATE(consumed_at) ON core_market_oauth.codes",
        "EXECUTE ON FUNCTION core_market_oauth.scope_set(text[])",
    ] {
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            lmm_core::runtime_database::check(&role.pool, required).await,
            Err(lmm_core::runtime_database::RuntimeDatabaseError::MissingPrivileges),
            "{grant}"
        );
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    for grant in [
        "UPDATE ON core_market_oauth.families",
        "UPDATE(scopes) ON core_market_oauth.tokens",
        "UPDATE(redirect_uri) ON core_market_oauth.codes",
        "DELETE ON core_market_oauth.requests",
    ] {
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            lmm_core::runtime_database::check(&role.pool, Default::default()).await,
            Err(lmm_core::runtime_database::RuntimeDatabaseError::UnsafePrivileges),
            "{grant}"
        );
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        // PostgreSQL's table-wide REVOKE also removes the prior column grant.
        // Restore the exact baseline before the next independent mutation.
        role.optional("core_market_oauth").await;
        lmm_core::runtime_database::check(&role.pool, required)
            .await
            .unwrap();
    }
    lmm_core::runtime_database::check(&role.pool, required)
        .await
        .unwrap();
    let i = issuer(&role.pool).await;
    let client = i
        .register(registration("Least privilege"))
        .await
        .unwrap()
        .client_id;
    let c = code(&i, &client, &cookie).await;
    let token = i.token(exchange(&i, &client, &c)).await.unwrap();
    let a = i
        .validate(&token.access_token, &["market:discover", "market:invoke"])
        .await
        .unwrap();
    let no_groups: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.user_routing_groups WHERE user_id=$1",
    )
    .bind(a.user_id())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(no_groups, 0);
    let mut tx = role.pool.begin().await.unwrap();
    let ids = market_oauth::revoke_client_in_transaction(
        &mut tx,
        i.profile(),
        a.user_id(),
        a.client_id(),
    )
    .await
    .unwrap();
    assert_eq!(ids, [a.family_id().to_owned()]);
    tx.commit().await.unwrap();
    assert!(matches!(
        i.validate(&token.access_token, &["market:discover"]).await,
        Err(Error::Unauthorized)
    ));
    role.cleanup().await;
}
#[sqlx::test]
async fn formal_revoke_is_client_and_resource_scoped(pool: PgPool) {
    init(&pool).await;
    let i = issuer(&pool).await;
    let cookie = browser(&pool, "revokemarket").await;
    let client = i
        .register(registration("Revocable MCP"))
        .await
        .unwrap()
        .client_id;
    let other = i
        .register(registration("Unrelated MCP"))
        .await
        .unwrap()
        .client_id;
    let c = code(&i, &client, &cookie).await;
    let token = i.token(exchange(&i, &client, &c)).await.unwrap();
    i.revoke(&other, &token.access_token).await.unwrap();
    assert!(
        i.validate(&token.access_token, &["market:discover"])
            .await
            .is_ok()
    );
    let other_resource = MarketIssuer::open(
        pool.clone(),
        i.profile()
            .clone()
            .with_resource("http://127.0.0.1:55122/mcp/market")
            .unwrap(),
    )
    .await
    .unwrap();
    other_resource
        .revoke(&client, &token.access_token)
        .await
        .unwrap();
    assert!(matches!(
        other_resource
            .validate(&token.access_token, &["market:discover"])
            .await,
        Err(Error::Unauthorized)
    ));
    assert!(
        i.validate(&token.access_token, &["market:discover"])
            .await
            .is_ok()
    );
    i.revoke(&client, token.refresh_token.as_deref().unwrap())
        .await
        .unwrap();
    assert!(matches!(
        i.validate(&token.access_token, &["market:discover"]).await,
        Err(Error::Unauthorized)
    ));
    i.revoke(&client, &token.access_token).await.unwrap();
}
#[sqlx::test]
async fn dcr_rejects_redirect_and_scope_confusion(pool: PgPool) {
    init(&pool).await;
    let i = issuer(&pool).await;
    for uri in [
        "http://example.com/callback",
        "http://127.0.0.1.evil.test:55121/callback",
        "http://127.0.0.1:55121/callback?next=steal",
        "https://example.com/callback#fragment",
        "https://user@example.com/callback",
        "https://example.com/a%2fb",
    ] {
        let mut r = registration("Invalid redirect");
        r.redirect_uris = vec![uri.into()];
        assert!(matches!(i.register(r).await, Err(Error::Invalid)), "{uri}");
    }
    let mut r = registration("Models forbidden");
    r.scope = "market:discover models:invoke".into();
    assert!(matches!(i.register(r).await, Err(Error::Invalid)));
    let mut r = registration("Client secret forbidden");
    r.token_endpoint_auth_method = "client_secret_post".into();
    assert!(matches!(i.register(r).await, Err(Error::Invalid)));
    let mut r = registration("Mixed application");
    r.redirect_uris.push("https://example.com/callback".into());
    assert!(matches!(i.register(r).await, Err(Error::Invalid)));
}
