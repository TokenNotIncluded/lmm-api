//! Real local provider HTTP and fresh PostgreSQL. Only external replies are synthetic.
use super::*;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use reqwest::Url;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use totp_rs::{Algorithm, Builder, Secret};

const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_BINDING: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const MFA_BINDING: &str = "3333333333333333333333333333333333333333333333333333333333333333";

#[derive(Clone)]
struct ProviderState {
    provider: ExternalProvider,
    token_calls: Arc<AtomicUsize>,
    user_calls: Arc<AtomicUsize>,
}

struct ProviderServer {
    state: ProviderState,
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ProviderServer {
    async fn start(provider: ExternalProvider) -> Self {
        let state = ProviderState {
            provider,
            token_calls: Arc::default(),
            user_calls: Arc::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/token", post(token))
            .route("/user", get(user_info))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            state,
            origin,
            task,
        }
    }

    fn config(&self) -> ExternalOAuth {
        let path = match self.state.provider {
            ExternalProvider::Github => "/oauth/github",
            ExternalProvider::LinuxDo => "/oauth/linuxdo",
            ExternalProvider::Discord => "/oauth/discord",
        };
        ExternalOAuth::new(
            self.state.provider,
            "synthetic-client",
            "synthetic-secret",
            &format!("https://external-login.example.invalid{path}"),
        )
        .unwrap()
        .with_test_endpoints(
            Url::parse(&format!("{}/token", self.origin)).unwrap(),
            Url::parse(&format!("{}/user", self.origin)).unwrap(),
        )
        .unwrap()
    }

    fn calls(&self) -> (usize, usize) {
        (
            self.state.token_calls.load(Ordering::SeqCst),
            self.state.user_calls.load(Ordering::SeqCst),
        )
    }
}

async fn token(State(state): State<ProviderState>, body: Bytes) -> Response {
    state.token_calls.fetch_add(1, Ordering::SeqCst);
    let form = Url::parse(&format!(
        "http://fixture.invalid/?{}",
        std::str::from_utf8(&body).unwrap()
    ))
    .unwrap();
    let pairs = form
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    let code = pairs.get("code").unwrap().as_ref();
    if code == "rejected" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    }
    assert!(matches!(code, "existing" | "different" | "unknown"));
    Json(json!({"access_token":format!("token_{code}"),"token_type":"Bearer"})).into_response()
}

async fn user_info(State(state): State<ProviderState>, headers: HeaderMap) -> Response {
    state.user_calls.fetch_add(1, Ordering::SeqCst);
    let subject: i64 = match headers.get("authorization").unwrap().to_str().unwrap() {
        "Bearer token_existing" => 42,
        "Bearer token_different" => 43,
        "Bearer token_unknown" => 44,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let id = if state.provider == ExternalProvider::Discord {
        json!(subject.to_string())
    } else {
        json!(subject)
    };
    Json(json!({"id":id,"email":"same@example.invalid","email_verified":true,"verified":true,"username":"root","roles":["superadmin"],"trust_level":4})).into_response()
}

async fn setup(pool: &PgPool) -> IdentityStore {
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys(
            "external-test".into(),
            vec![("external-test".into(), vec![47; 32])],
        )
        .unwrap();
    store.init_database().await.unwrap();
    sqlx::raw_sql(crate::abuse::SCHEMA)
        .execute(pool)
        .await
        .unwrap();
    store
}

fn browser(store: &IdentityStore) -> BrowserAuth {
    BrowserAuth::new(store.clone(), &[43; 32]).unwrap()
}

fn metadata() -> BrowserSessionMetadata {
    BrowserSessionMetadata {
        ip: "192.0.2.42".into(),
        user_agent: "synthetic external login acceptance".into(),
    }
}

async fn begin(browser: &BrowserAuth, provider: ExternalProvider) -> String {
    let url = browser
        .begin_external_login(provider, BINDING)
        .await
        .unwrap();
    let url = Url::parse(&url).unwrap();
    url.query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned()
}

async fn finish(
    browser: &BrowserAuth,
    provider: ExternalProvider,
    state: &str,
    code: &str,
) -> MfaResult<BrowserLoginOutcome> {
    browser
        .finish_external_login(provider, state, BINDING, code, &metadata(), MFA_BINDING)
        .await
}

async fn login(browser: &BrowserAuth, provider: ExternalProvider, code: &str) -> BrowserBundle {
    let state = begin(browser, provider).await;
    match finish(browser, provider, &state, code).await.unwrap() {
        BrowserLoginOutcome::Session(bundle) => bundle,
        BrowserLoginOutcome::Challenge(_) => panic!("unexpected factor challenge for new account"),
    }
}

fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}

fn sid(bundle: &BrowserBundle) -> &str {
    bundle.data["session"]["sid"].as_str().unwrap()
}

fn user(bundle: &BrowserBundle) -> i64 {
    bundle.data["user"]["id"].as_i64().unwrap()
}

async fn assert_current_login_method(browser: &BrowserAuth, bundle: &BrowserBundle, method: &str) {
    assert_eq!(bundle.data["session"]["login_method"], method);
    let listed = browser.login_sessions(access(bundle)).await.unwrap();
    assert_eq!(listed["success"], true);
    let current: Vec<_> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["current"] == true)
        .collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["sid"], sid(bundle));
    assert_eq!(current[0]["login_method"], method);
}

async fn counts(pool: &PgPool) -> (i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.users),(SELECT count(*) FROM core_identity.oauth_identities),(SELECT count(*) FROM core_identity.credentials),(SELECT count(*) FROM core_identity.web_sessions)")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn external_identity_reuses_only_exact_issuer_subject_and_ignores_email_and_roles(
    pool: PgPool,
) {
    let mut store = setup(&pool).await;
    let mut providers = Vec::new();
    for provider in [
        ExternalProvider::Github,
        ExternalProvider::LinuxDo,
        ExternalProvider::Discord,
    ] {
        let server = ProviderServer::start(provider).await;
        store = store.with_external_oauth(server.config()).unwrap();
        providers.push(server);
    }
    let browser = browser(&store);
    let mut user_ids = std::collections::BTreeSet::new();
    for server in &providers {
        let provider = server.state.provider;
        let method = match provider {
            ExternalProvider::Github => "oauth:github",
            ExternalProvider::LinuxDo => "oauth:linuxdo",
            ExternalProvider::Discord => "oauth:discord",
        };
        let first = login(&browser, provider, "existing").await;
        let again = login(&browser, provider, "existing").await;
        let different = login(&browser, provider, "different").await;
        assert_eq!(user(&first), user(&again));
        assert_ne!(sid(&first), sid(&again));
        assert_ne!(user(&first), user(&different));
        assert!(user_ids.insert(user(&first)));
        assert!(user_ids.insert(user(&different)));
        for bundle in [&first, &again, &different] {
            assert_current_login_method(&browser, bundle, method).await;
            let actor = store.authorize(access(bundle)).await.unwrap();
            assert_eq!(actor.user_id, user(bundle));
            assert_eq!(actor.credential_kind, CredentialKind::Session);
            assert_eq!(actor.platform_level, 0);
            let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.web_sessions WHERE sid=$1 AND current_credential_id=$2 AND revoked_at IS NULL)")
                .bind(sid(bundle)).bind(actor.credential_id).fetch_one(&pool).await.unwrap();
            assert!(live);
        }
        let refreshed = browser.refresh(&first.refresh_token, None).await.unwrap();
        assert_eq!(sid(&refreshed), sid(&first));
        assert_eq!(user(&refreshed), user(&first));
        assert_current_login_method(&browser, &refreshed, method).await;
        let actor = store.authorize(access(&refreshed)).await.unwrap();
        assert_eq!(actor.credential_kind, CredentialKind::Session);
        assert_eq!(actor.user_id, user(&first));
        assert_eq!(actor.platform_level, 0);
        assert!(matches!(
            store.authorize(access(&first)).await,
            Err(IdentityError::Unauthorized)
        ));
        assert_eq!(server.calls(), (3, 3));
    }
    assert_eq!(counts(&pool).await, (6, 6, 12, 9));
    let authority: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.users WHERE platform_role<>'user'),(SELECT count(*) FROM core_identity.user_profiles WHERE email<>'' OR password_login_allowed OR email_login_allowed),(SELECT count(*) FROM core_abuse.registration_observations)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(authority, (0, 0, 6));
    let stored: Vec<(String, String)> = sqlx::query_as(
        "SELECT issuer,subject FROM core_identity.oauth_identities ORDER BY issuer,subject",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(stored.len(), 6);
    assert!(
        stored
            .iter()
            .all(|(_, subject)| matches!(subject.as_str(), "42" | "43"))
    );
}

#[sqlx::test(migrations = false)]
async fn wrong_provider_state_cookie_and_expired_flow_never_exchange_a_code(pool: PgPool) {
    let github = ProviderServer::start(ExternalProvider::Github).await;
    let linux = ProviderServer::start(ExternalProvider::LinuxDo).await;
    let store = setup(&pool)
        .await
        .with_external_oauth(github.config())
        .unwrap()
        .with_external_oauth(linux.config())
        .unwrap();
    let browser = browser(&store);
    let state = begin(&browser, ExternalProvider::Github).await;
    for (provider, supplied_state, binding) in [
        (ExternalProvider::LinuxDo, state.as_str(), BINDING),
        (
            ExternalProvider::Github,
            "lmmo_0000000000000000000000000000000000000000000000000000000000000000",
            BINDING,
        ),
        (ExternalProvider::Github, state.as_str(), OTHER_BINDING),
    ] {
        assert!(matches!(
            browser
                .finish_external_login(
                    provider,
                    supplied_state,
                    binding,
                    "existing",
                    &metadata(),
                    MFA_BINDING
                )
                .await,
            Err(MfaError::Identity(IdentityError::Unauthorized))
        ));
    }
    assert_eq!(github.calls(), (0, 0));
    assert_eq!(linux.calls(), (0, 0));
    assert_eq!(counts(&pool).await, (0, 0, 0, 0));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_identity.oauth_flows")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // A rejected mix-up does not consume the valid provider's original flow.
    assert!(matches!(
        finish(&browser, ExternalProvider::Github, &state, "existing").await,
        Ok(BrowserLoginOutcome::Session(_))
    ));
    assert_eq!(github.calls(), (1, 1));
    let expired = begin(&browser, ExternalProvider::Github).await;
    sqlx::query("UPDATE core_identity.oauth_flows SET expires_at=clock_timestamp()-interval '1 second' WHERE state_digest=$1")
        .bind(digest(&expired).unwrap()).execute(&pool).await.unwrap();
    let before = counts(&pool).await;
    assert!(matches!(
        finish(&browser, ExternalProvider::Github, &expired, "existing").await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(github.calls(), (1, 1));
    assert_eq!(counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn callback_is_consumed_once_after_success_provider_rejection_and_network_failure(
    pool: PgPool,
) {
    let github = ProviderServer::start(ExternalProvider::Github).await;
    let store = setup(&pool)
        .await
        .with_external_oauth(github.config())
        .unwrap();
    let browser = browser(&store);
    let success = begin(&browser, ExternalProvider::Github).await;
    assert!(
        finish(&browser, ExternalProvider::Github, &success, "existing")
            .await
            .is_ok()
    );
    assert!(matches!(
        finish(&browser, ExternalProvider::Github, &success, "existing").await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(github.calls(), (1, 1));
    let before = counts(&pool).await;
    let rejected = begin(&browser, ExternalProvider::Github).await;
    assert!(matches!(
        finish(&browser, ExternalProvider::Github, &rejected, "rejected").await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert!(matches!(
        finish(&browser, ExternalProvider::Github, &rejected, "existing").await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(github.calls(), (2, 1));
    assert_eq!(counts(&pool).await, before);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let dead = ExternalOAuth::new(
        ExternalProvider::Github,
        "synthetic-client",
        "synthetic-secret",
        "https://external-login.example.invalid/oauth/github",
    )
    .unwrap()
    .with_test_endpoints(
        Url::parse(&format!("{closed}/token")).unwrap(),
        Url::parse(&format!("{closed}/user")).unwrap(),
    )
    .unwrap();
    let dead_store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_external_oauth(dead)
        .unwrap();
    let dead_browser = BrowserAuth::new(dead_store, &[43; 32]).unwrap();
    let failed = begin(&dead_browser, ExternalProvider::Github).await;
    assert!(matches!(
        finish(&dead_browser, ExternalProvider::Github, &failed, "existing").await,
        Err(MfaError::Identity(IdentityError::Storage))
    ));
    // Replacing a failed provider connection cannot revive the consumed state.
    assert!(matches!(
        finish(&browser, ExternalProvider::Github, &failed, "existing").await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(github.calls(), (2, 1));
    assert_eq!(counts(&pool).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_identity.oauth_flows")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test(migrations = false)]
async fn disabled_registration_accepts_known_subject_and_denies_unknown_without_partial_user(
    pool: PgPool,
) {
    let base = setup(&pool).await;
    for provider in [
        ExternalProvider::Github,
        ExternalProvider::LinuxDo,
        ExternalProvider::Discord,
    ] {
        let server = ProviderServer::start(provider).await;
        let store = base.clone().with_external_oauth(server.config()).unwrap();
        let original = login(&browser(&store), provider, "existing").await;
        for closed in [
            store.clone().with_registration_enabled(false),
            store.clone().with_oauth_register_enabled(false),
            store
                .clone()
                .with_oauth_registration_disabled_methods(&[provider.slug().to_owned()])
                .unwrap(),
        ] {
            let closed_browser = browser(&closed);
            let known = login(&closed_browser, provider, "existing").await;
            assert_eq!(user(&known), user(&original));
            let before = counts(&pool).await;
            let state = begin(&closed_browser, provider).await;
            assert!(matches!(
                finish(&closed_browser, provider, &state, "unknown").await,
                Err(MfaError::Identity(IdentityError::Forbidden))
            ));
            assert_eq!(counts(&pool).await, before);
            assert!(matches!(
                finish(&closed_browser, provider, &state, "existing").await,
                Err(MfaError::Identity(IdentityError::Unauthorized))
            ));
        }
        assert_eq!(server.calls(), (7, 7));
    }
}

#[sqlx::test(migrations = false)]
async fn registration_email_domain_external_new_subject_denied_existing_login_allowed(
    pool: PgPool,
) {
    let base = setup(&pool).await;
    for provider in [
        ExternalProvider::Github,
        ExternalProvider::LinuxDo,
        ExternalProvider::Discord,
    ] {
        let server = ProviderServer::start(provider).await;
        let store = base.clone().with_external_oauth(server.config()).unwrap();
        let original = login(&browser(&store), provider, "existing").await;
        for restricted in [
            store
                .clone()
                .with_registration_email_policy(true, &["gmail.com".to_owned()])
                .unwrap(),
            store.with_registration_email_verification(true),
        ] {
            let web = browser(&restricted);
            let known = login(&web, provider, "existing").await;
            assert_eq!(user(&known), user(&original));
            let before = counts(&pool).await;
            let state = begin(&web, provider).await;
            assert!(matches!(
                finish(&web, provider, &state, "unknown").await,
                Err(MfaError::Identity(IdentityError::Forbidden))
            ));
            assert_eq!(before, counts(&pool).await);
        }
    }
}

#[sqlx::test(migrations = false)]
async fn inactive_user_and_account_cannot_issue_external_browser_families(pool: PgPool) {
    let github = ProviderServer::start(ExternalProvider::Github).await;
    let store = setup(&pool)
        .await
        .with_external_oauth(github.config())
        .unwrap();
    let browser = browser(&store);
    let disabled_user = login(&browser, ExternalProvider::Github, "existing").await;
    let disabled_account = login(&browser, ExternalProvider::Github, "different").await;
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=$1")
        .bind(user(&disabled_user))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)").bind(user(&disabled_account)).execute(&pool).await.unwrap();
    let before = counts(&pool).await;
    for code in ["existing", "different"] {
        let state = begin(&browser, ExternalProvider::Github).await;
        assert!(matches!(
            finish(&browser, ExternalProvider::Github, &state, code).await,
            Err(MfaError::Identity(IdentityError::Unauthorized))
        ));
        assert_eq!(counts(&pool).await, before);
    }
    assert!(matches!(
        store.authorize(access(&disabled_user)).await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        store.authorize(access(&disabled_account)).await,
        Err(IdentityError::Unauthorized)
    ));
    assert_eq!(github.calls(), (4, 4));
}

async fn epoch(pool: &PgPool) -> u64 {
    let epoch: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(pool)
            .await
            .unwrap();
    u64::try_from(epoch).unwrap()
}

fn totp(secret: &str, epoch: u64) -> String {
    let secret = Secret::try_from_base32(secret).unwrap();
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(epoch)
        .to_string()
}

#[sqlx::test(migrations = false)]
async fn external_login_honors_real_totp_without_issuing_a_family_before_factor_completion(
    pool: PgPool,
) {
    let discord = ProviderServer::start(ExternalProvider::Discord).await;
    let store = setup(&pool)
        .await
        .with_external_oauth(discord.config())
        .unwrap();
    let browser = browser(&store);
    let original = login(&browser, ExternalProvider::Discord, "existing").await;
    let factor = browser.mfa_setup(access(&original)).await.unwrap();
    let enrollment_epoch = epoch(&pool).await;
    let enabled = browser
        .mfa_enable(access(&original), &totp(&factor.secret, enrollment_epoch))
        .await
        .unwrap();
    assert!(matches!(
        store.authorize(access(&original)).await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(browser.mfa_status(access(&enabled)).await.unwrap().enabled);
    let before = counts(&pool).await;
    let state = begin(&browser, ExternalProvider::Discord).await;
    let challenge = match finish(&browser, ExternalProvider::Discord, &state, "existing")
        .await
        .unwrap()
    {
        BrowserLoginOutcome::Challenge(challenge) => challenge,
        BrowserLoginOutcome::Session(_) => panic!("provider identity bypassed local second factor"),
    };
    assert!(challenge.require_2fa);
    assert_eq!(counts(&pool).await, before);
    assert!(store.authorize(&challenge.flow_token).await.is_err());
    assert!(matches!(
        browser
            .pending_external_mfa(&challenge.flow_token, OTHER_BINDING)
            .await,
        Err(MfaError::FlowExpired)
    ));
    assert_eq!(
        browser
            .pending_external_mfa(&challenge.flow_token, MFA_BINDING)
            .await
            .unwrap()
            .flow_token,
        challenge.flow_token
    );
    let completion = MfaCompletion {
        flow_token: challenge.flow_token.clone(),
        code: totp(&factor.secret, enrollment_epoch + 30),
    };
    assert!(matches!(
        browser.complete_mfa_login(&completion, OTHER_BINDING).await,
        Err(MfaError::FlowExpired)
    ));
    assert_eq!(counts(&pool).await, before);
    let logged_in = browser
        .complete_mfa_login(&completion, MFA_BINDING)
        .await
        .unwrap();
    let actor = store.authorize(access(&logged_in)).await.unwrap();
    assert_eq!(actor.user_id, user(&original));
    assert_eq!(actor.credential_kind, CredentialKind::Session);
    assert_ne!(sid(&logged_in), sid(&enabled));
    assert_current_login_method(&browser, &logged_in, "2fa").await;
    let assurance: (i16, String, bool) = sqlx::query_as("SELECT factor_level,method,mfa_verified_at IS NOT NULL FROM core_identity.session_authentication WHERE credential_id=$1")
        .bind(actor.credential_id).fetch_one(&pool).await.unwrap();
    assert_eq!(assurance, (2, "totp".into(), true));
    let after = counts(&pool).await;
    assert_eq!(after, (before.0, before.1, before.2 + 1, before.3 + 1));
    let refreshed = browser
        .refresh(&logged_in.refresh_token, None)
        .await
        .unwrap();
    assert_eq!(sid(&refreshed), sid(&logged_in));
    assert_eq!(user(&refreshed), user(&logged_in));
    assert_current_login_method(&browser, &refreshed, "2fa").await;
    let refreshed_actor = store.authorize(access(&refreshed)).await.unwrap();
    assert_eq!(refreshed_actor.user_id, user(&original));
    assert_eq!(refreshed_actor.credential_kind, CredentialKind::Session);
    let refreshed_assurance: (i16, String, bool) = sqlx::query_as("SELECT factor_level,method,mfa_verified_at IS NOT NULL FROM core_identity.session_authentication WHERE credential_id=$1")
        .bind(refreshed_actor.credential_id).fetch_one(&pool).await.unwrap();
    assert_eq!(refreshed_assurance, (2, "totp".into(), true));
    assert!(matches!(
        store.authorize(access(&logged_in)).await,
        Err(IdentityError::Unauthorized)
    ));
    let after_refresh = counts(&pool).await;
    assert_eq!(after_refresh, (after.0, after.1, after.2 + 1, after.3));
    assert!(matches!(
        browser.complete_mfa_login(&completion, MFA_BINDING).await,
        Err(MfaError::FlowExpired)
    ));
    assert!(matches!(
        finish(&browser, ExternalProvider::Discord, &state, "existing").await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(counts(&pool).await, after_refresh);
    assert_eq!(discord.calls(), (2, 2));
}
