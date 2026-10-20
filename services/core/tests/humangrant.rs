//! PostgreSQL integration and acceptance suite for native HumanGrant 0.1 slice.

#[path = "../src/humangrant/mod.rs"]
pub mod humangrant;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request as HttpRequest, StatusCode, header},
};
use base64::prelude::*;
use humangrant::{
    engine::HumanGrantEngine,
    signature::{fingerprint, origin, verify_signature},
    store::{DDL as HG_DDL, HumanGrantStore},
    types::*,
};
use lmm_core::{
    core_schema,
    identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore},
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const ORIGIN: &str = "https://auth.example.test";
const PASSWORD: &str = "humangrant-test123";

struct AiKey {
    pair: Ed25519KeyPair,
    public: String,
    actor: String,
}

fn generate_ai_key() -> AiKey {
    let mut seed = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut seed);
    let key_pair = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    let pub_bytes = key_pair.public_key().as_ref();
    let pub_b64 = BASE64_URL_SAFE_NO_PAD.encode(pub_bytes);
    let actor = format!("ed25519:{}", fingerprint(pub_bytes));
    AiKey {
        pair: key_pair,
        public: pub_b64,
        actor,
    }
}

fn sign_request(
    key: &AiKey,
    method: &str,
    path: &str,
    body: &[u8],
    token: &str,
    created: i64,
    nonce: &str,
) -> HashMap<String, String> {
    let AiKey {
        pair: key_pair,
        public: pub_b64,
        actor,
    } = key;
    let digest = Sha256::digest(body);
    let content_digest = format!("sha-256=:{}:", BASE64_STANDARD.encode(digest));
    let content_type = "application/json";
    let authorization = format!("HumanGrant {token}");
    let expires = created + 60;
    let params = format!(
        r#"("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization");created={created};expires={expires};nonce="{nonce}";keyid="{actor}";alg="ed25519";tag="humangrant-0.1""#
    );
    let base = format!(
        "\"@method\": {method}\n\"@target-uri\": {ORIGIN}{path}\n\"content-digest\": {content_digest}\n\"content-type\": {content_type}\n\"x-hg-key\": {pub_b64}\n\"authorization\": {authorization}\n\"@signature-params\": {params}"
    );

    let sig = key_pair.sign(base.as_bytes());
    let sig_b64 = BASE64_STANDARD.encode(sig.as_ref());

    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), content_type.to_string());
    headers.insert("content-digest".to_string(), content_digest);
    headers.insert("x-hg-key".to_string(), pub_b64.to_string());
    headers.insert("authorization".to_string(), authorization);
    headers.insert("signature-input".to_string(), format!("hg={params}"));
    headers.insert("signature".to_string(), format!("hg=:{sig_b64}:"));
    headers
}

async fn setup_app(pool: PgPool) -> (BrowserAuth, Router, HumanGrantStore) {
    // Browser login reads assembled Core policy state, including core_abuse.
    // Build the same fresh schema contract used by the real Core before
    // installing HumanGrant's own tables.
    core_schema::init(&pool).await.unwrap();
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();

    // HumanGrant schema
    sqlx::raw_sql(HG_DDL).execute(&pool).await.unwrap();

    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    let hg_store = HumanGrantStore::new(pool.clone());
    let canonical_origin = origin(ORIGIN, true).unwrap();
    let engine = Arc::new(HumanGrantEngine::new(
        hg_store.clone(),
        canonical_origin,
        false,
    ));
    let app = humangrant::router(engine);

    (browser, app, hg_store)
}

async fn register_and_login(browser: &BrowserAuth, username: &str) -> (i64, String) {
    browser
        .register(&BrowserRegistration {
            username,
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    login(browser, username).await
}

async fn login(browser: &BrowserAuth, username: &str) -> (i64, String) {
    let bundle = browser
        .login(
            username,
            PASSWORD,
            &BrowserSessionMetadata {
                ip: "127.0.0.1".into(),
                user_agent: "test-client".into(),
            },
        )
        .await
        .unwrap();

    let user_id = bundle.data["user"]["id"].as_i64().unwrap();
    let refresh_token = bundle.refresh_token;
    (user_id, refresh_token)
}

async fn call_http(
    app: &Router,
    method: &str,
    path: &str,
    headers: HashMap<String, String>,
    body: Vec<u8>,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = HttpRequest::builder().method(method).uri(path);
    for (k, v) in headers {
        builder = builder.header(k, v);
    }

    let resp = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let body_bytes = to_bytes(resp.into_body(), 65536).await.unwrap();

    let json_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, resp_headers, json_val)
}

#[test]
fn test_public_signature_vector() {
    let method = "POST";
    let origin_str = "https://auth.example.test";
    let path = "/v1/requests";
    let created = 1700000000;
    let body = r#"{"client_id":"example-cli","target_service":"notes","purpose":"Read a note","requested_permissions":[{"action":"notes.read","resource":"alice-note"}],"expected_human_id":"alice"}"#.as_bytes();

    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    headers.insert(
        "content-digest".to_string(),
        "sha-256=:jN6odj5Tli9ZfoLdtLxtE/sUNtMKAKM2V7xq8/qri5Y=:".to_string(),
    );
    headers.insert(
        "x-hg-key".to_string(),
        "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo".to_string(),
    );
    headers.insert(
        "authorization".to_string(),
        "HumanGrant anonymous".to_string(),
    );
    headers.insert(
        "signature-input".to_string(),
        r#"hg=("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization");created=1700000000;expires=1700000060;nonce="AAECAwQFBgcICQoLDA0ODxAREhMUFRYX";keyid="ed25519:If4x36FUomFia_hUBG_SJxt77UtqvkWqWId-9H-XIbk";alg="ed25519";tag="humangrant-0.1""#.to_string(),
    );
    headers.insert(
        "signature".to_string(),
        "hg=:jmjuzgV58zQY0A8N+T9sss24Jwa8Kah5bvs2VSdWA43TScypSfoPkzlERC6XLBOvx8+D+ZqQ0WOVKwS2/Gd3AA==:".to_string(),
    );

    let proof = verify_signature(method, path, &headers, body, origin_str, created + 10)
        .expect("signature must verify against public vector");

    assert_eq!(
        proof.actor,
        "ed25519:If4x36FUomFia_hUBG_SJxt77UtqvkWqWId-9H-XIbk"
    );
    assert_eq!(proof.credential, "anonymous");
    assert_eq!(proof.nonce, "AAECAwQFBgcICQoLDA0ODxAREhMUFRYX");
    assert_eq!(proof.created, 1700000000);
    assert_eq!(proof.expires, 1700000060);
}

#[sqlx::test(migrations = false)]
async fn test_humangrant_full_lifecycle_postgres(pool: PgPool) {
    let (browser, app, hg_store) = setup_app(pool.clone()).await;

    // 1. Register users
    let (superadmin_id, superadmin_refresh) = register_and_login(&browser, "superadmin_user").await;
    let (admin_id, admin_refresh) = register_and_login(&browser, "admin_user").await;
    let (alice_id, alice_refresh) = register_and_login(&browser, "alice").await;
    let (_bob_id, bob_refresh) = register_and_login(&browser, "bob").await;

    // Elevate superadmin to platform_role 'superadmin' (RoleRootUser = 100)
    sqlx::query("UPDATE core_identity.users SET platform_role = 'superadmin' WHERE id = $1")
        .bind(superadmin_id)
        .execute(&pool)
        .await
        .unwrap();

    // Elevate admin to platform_role 'admin' (RoleAdminUser = 10)
    sqlx::query("UPDATE core_identity.users SET platform_role = 'admin' WHERE id = $1")
        .bind(admin_id)
        .execute(&pool)
        .await
        .unwrap();

    // Role changes advance auth_version; pre-elevation credentials stay invalid.
    for refresh in [&superadmin_refresh, &admin_refresh] {
        let mut headers = HashMap::new();
        headers.insert("cookie".to_string(), format!("new_api_refresh={refresh}"));
        let (status, _, _) = call_http(
            &app,
            "GET",
            "/api/user/auth/humangrant/grants",
            headers,
            vec![],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (logged_in_superadmin, superadmin_refresh) = login(&browser, "superadmin_user").await;
    let (logged_in_admin, admin_refresh) = login(&browser, "admin_user").await;
    assert_eq!(logged_in_superadmin, superadmin_id);
    assert_eq!(logged_in_admin, admin_id);

    // Verify RootAuth check:
    // User alice (platform_role = 'user') gets 403
    let mut alice_get_headers = HashMap::new();
    alice_get_headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={alice_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/settings",
        alice_get_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin user (platform_role = 'admin') ALSO gets 403 (RootAuth requires superadmin!)
    let mut admin_get_headers = HashMap::new();
    admin_get_headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={admin_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/settings",
        admin_get_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Superadmin user gets 200 OK
    let mut superadmin_get_headers = HashMap::new();
    superadmin_get_headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={superadmin_refresh}"),
    );
    let (status, _, settings_init) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/settings",
        superadmin_get_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings_init["enabled"], true);

    // 2. Public config check
    let (status, _, config) =
        call_http(&app, "GET", "/humangrant/config", HashMap::new(), vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(config["enabled"], true);
    assert_eq!(config["ready"], true);
    assert_eq!(config["client_id"], CLIENT_ID);

    // 3. AI creates Request with Ed25519 signature
    let ai_key = generate_ai_key();
    let now = chrono::Utc::now().timestamp();
    let nonce_bytes = [1u8; 24];
    let nonce = BASE64_URL_SAFE_NO_PAD.encode(nonce_bytes);

    let req_body = json!({
        "client_id": CLIENT_ID,
        "target_service": SERVICE_ID,
        "purpose": "Read user public profile for testing",
        "requested_permissions": [
            {
                "action": PROFILE_READ,
                "resource": alice_id.to_string(),
            }
        ],
        "expected_human_id": alice_id.to_string(),
        "session_ttl_seconds": 3600
    });
    let req_bytes = serde_json::to_vec(&req_body).unwrap();

    let sig_headers = sign_request(
        &ai_key,
        "POST",
        "/v1/requests",
        &req_bytes,
        "anonymous",
        now,
        &nonce,
    );

    let (status, _, create_resp) =
        call_http(&app, "POST", "/v1/requests", sig_headers, req_bytes.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(create_resp["phase"], "waiting_human");
    let request_id = create_resp["request_id"].as_str().unwrap().to_string();
    let auth_url = create_resp["human_authorization_url"]
        .as_str()
        .unwrap()
        .to_string();
    let link = auth_url.split("/human/").nth(1).unwrap().to_string();

    // Replay protection test: repeating the same request with identical nonce must fail with 401 signature_replay!
    let sig_headers_replay = sign_request(
        &ai_key,
        "POST",
        "/v1/requests",
        &req_bytes,
        "anonymous",
        now,
        &nonce,
    );
    let (replay_status, _, replay_resp) = call_http(
        &app,
        "POST",
        "/v1/requests",
        sig_headers_replay,
        req_bytes.clone(),
    )
    .await;
    assert_eq!(replay_status, StatusCode::UNAUTHORIZED);
    assert_eq!(replay_resp["error"], "signature_replay");

    // Strict parameter check: duplicate param in signature-input must fail
    let mut bad_param_headers = sign_request(
        &ai_key,
        "POST",
        "/v1/requests",
        &req_bytes,
        "anonymous",
        now,
        &BASE64_URL_SAFE_NO_PAD.encode([99u8; 24]),
    );
    let orig_sig_input = bad_param_headers.get("signature-input").unwrap().clone();
    bad_param_headers.insert(
        "signature-input".to_string(),
        format!("{orig_sig_input};created={now}"),
    );
    let (bad_p_status, _, _) = call_http(
        &app,
        "POST",
        "/v1/requests",
        bad_param_headers,
        req_bytes.clone(),
    )
    .await;
    assert_eq!(bad_p_status, StatusCode::UNAUTHORIZED);

    // 4. Human opens review page GET /human/:link
    let (status, headers, _) = call_http(
        &app,
        "GET",
        &format!("/human/{link}"),
        HashMap::new(),
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie_header = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    let flow_cookie = cookie_header.split(';').next().unwrap().to_string();

    // 5. Human begin with invalid CSRF -> 403
    let mut begin_headers = HashMap::new();
    begin_headers.insert("cookie".to_string(), flow_cookie.clone());
    begin_headers.insert("x-hg-csrf".to_string(), "dummy_csrf".to_string());
    let (status, _, _) = call_http(
        &app,
        "POST",
        &format!("/api/user/auth/humangrant/human/{link}/begin"),
        begin_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Fetch the flow from DB to read the initial CSRF
    let mut tx = hg_store.begin().await.unwrap();
    let flow_key = flow_cookie.split('=').nth(1).unwrap();
    let flow: Flow = tx
        .get("flow", &fingerprint(flow_key.as_bytes()), now)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();

    let mut begin_headers = HashMap::new();
    begin_headers.insert("cookie".to_string(), flow_cookie.clone());
    begin_headers.insert("x-hg-csrf".to_string(), flow.csrf.clone());
    let (status, _, begin_resp) = call_http(
        &app,
        "POST",
        &format!("/api/user/auth/humangrant/human/{link}/begin"),
        begin_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(begin_resp["status"], "authenticating_human");

    // 6. Expected human mismatch test: Bob tries to verify Alice's expected request
    let mut verify_headers_bob = HashMap::new();
    verify_headers_bob.insert(
        "cookie".to_string(),
        format!("{flow_cookie}; new_api_refresh={bob_refresh}"),
    );
    verify_headers_bob.insert("x-hg-csrf".to_string(), flow.csrf.clone());
    let (status, _, verify_bob_resp) = call_http(
        &app,
        "POST",
        &format!("/api/user/auth/humangrant/human/{link}/verify"),
        verify_headers_bob,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(verify_bob_resp["error"], "expected_human_mismatch");

    // 7. Human verify with Alice's refresh cookie
    let mut verify_headers = HashMap::new();
    verify_headers.insert(
        "cookie".to_string(),
        format!("{flow_cookie}; new_api_refresh={alice_refresh}"),
    );
    verify_headers.insert("x-hg-csrf".to_string(), flow.csrf.clone());
    let (status, v_headers, verify_resp) = call_http(
        &app,
        "POST",
        &format!("/api/user/auth/humangrant/human/{link}/verify"),
        verify_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verify_resp["status"], "awaiting_consent");
    assert_eq!(verify_resp["human_id"], alice_id.to_string());
    let new_csrf = verify_resp["csrf"].as_str().unwrap().to_string();
    let new_cookie = v_headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // 8. Human approve
    let consent_body = json!({
        "permissions": [
            {
                "action": PROFILE_READ,
                "resource": alice_id.to_string(),
            }
        ],
        "expires_at": now + 3600
    });
    let mut approve_headers = HashMap::new();
    approve_headers.insert(
        "cookie".to_string(),
        format!("{new_cookie}; new_api_refresh={alice_refresh}"),
    );
    approve_headers.insert("x-hg-csrf".to_string(), new_csrf.clone());
    approve_headers.insert("content-type".to_string(), "application/json".to_string());
    let (status, _, approve_resp) = call_http(
        &app,
        "POST",
        &format!("/api/user/auth/humangrant/human/{link}/approve"),
        approve_headers,
        serde_json::to_vec(&consent_body).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approve_resp["status"], "approved");
    let grant_id = approve_resp["grant_id"].as_str().unwrap().to_string();

    // 9. AI polls request status
    let poll_nonce = BASE64_URL_SAFE_NO_PAD.encode([2u8; 24]);
    let poll_headers = sign_request(
        &ai_key,
        "GET",
        &format!("/v1/requests/{request_id}"),
        &[],
        "anonymous",
        now + 1,
        &poll_nonce,
    );
    let (status, _, poll_resp) = call_http(
        &app,
        "GET",
        &format!("/v1/requests/{request_id}"),
        poll_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(poll_resp["phase"], "approved");

    // 10. Concurrency claim race test:
    // Two concurrent asynchronous tasks attempt to claim the exact same request with different fresh nonces.
    // Thanks to PostgreSQL FOR UPDATE row locking and atomic transactional updates:
    // EXACTLY ONE task succeeds (200 OK) and the other task gets 409 Conflict!
    let claim_nonce_a = BASE64_URL_SAFE_NO_PAD.encode([10u8; 24]);
    let claim_headers_a = sign_request(
        &ai_key,
        "POST",
        &format!("/v1/requests/{request_id}/claim"),
        &[],
        "anonymous",
        now + 2,
        &claim_nonce_a,
    );

    let claim_nonce_b = BASE64_URL_SAFE_NO_PAD.encode([11u8; 24]);
    let claim_headers_b = sign_request(
        &ai_key,
        "POST",
        &format!("/v1/requests/{request_id}/claim"),
        &[],
        "anonymous",
        now + 2,
        &claim_nonce_b,
    );

    let app_a = app.clone();
    let app_b = app.clone();
    let path_claim = format!("/v1/requests/{request_id}/claim");
    let path_claim_b = path_claim.clone();

    let task_a = tokio::spawn(async move {
        call_http(&app_a, "POST", &path_claim, claim_headers_a, vec![]).await
    });
    let task_b = tokio::spawn(async move {
        call_http(&app_b, "POST", &path_claim_b, claim_headers_b, vec![]).await
    });

    let (res_a, res_b) = tokio::join!(task_a, task_b);
    let (status_a, _, resp_a) = res_a.unwrap();
    let (status_b, _, resp_b) = res_b.unwrap();

    let success_resp = if status_a == StatusCode::OK {
        assert_eq!(status_b, StatusCode::CONFLICT);
        assert_eq!(resp_b["error"], "credential_already_issued");
        resp_a
    } else {
        assert_eq!(status_a, StatusCode::CONFLICT);
        assert_eq!(resp_a["error"], "credential_already_issued");
        assert_eq!(status_b, StatusCode::OK);
        resp_b
    };

    let access_token = success_resp["access_token"].as_str().unwrap().to_string();
    assert_eq!(success_resp["token_type"], "HumanGrant");

    // 11. Third claim must also immediately fail with 409
    let claim_nonce3 = BASE64_URL_SAFE_NO_PAD.encode([12u8; 24]);
    let claim_headers3 = sign_request(
        &ai_key,
        "POST",
        &format!("/v1/requests/{request_id}/claim"),
        &[],
        "anonymous",
        now + 3,
        &claim_nonce3,
    );
    let (claim3_status, _, claim3_resp) = call_http(
        &app,
        "POST",
        &format!("/v1/requests/{request_id}/claim"),
        claim_headers3,
        vec![],
    )
    .await;
    assert_eq!(claim3_status, StatusCode::CONFLICT);
    assert_eq!(claim3_resp["error"], "credential_already_issued");

    // 12. Session introspection GET /v1/session
    let session_nonce = BASE64_URL_SAFE_NO_PAD.encode([5u8; 24]);
    let session_headers = sign_request(
        &ai_key,
        "GET",
        "/v1/session",
        &[],
        &access_token,
        now + 4,
        &session_nonce,
    );
    let (status, _, session_resp) =
        call_http(&app, "GET", "/v1/session", session_headers, vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session_resp["grantor_id"], alice_id.to_string());
    assert_eq!(session_resp["grant_id"], grant_id);
    let perms = session_resp["effective_permissions"].as_array().unwrap();
    assert_eq!(perms.len(), 1);
    assert_eq!(perms[0]["action"], PROFILE_READ);

    // 13. Profile read GET /v1/accounts/:id/profile
    let profile_nonce = BASE64_URL_SAFE_NO_PAD.encode([6u8; 24]);
    let profile_headers = sign_request(
        &ai_key,
        "GET",
        &format!("/v1/accounts/{alice_id}/profile"),
        &[],
        &access_token,
        now + 5,
        &profile_nonce,
    );
    let (status, _, profile_resp) = call_http(
        &app,
        "GET",
        &format!("/v1/accounts/{alice_id}/profile"),
        profile_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile_resp["account"]["username"], "alice");
    assert_eq!(profile_resp["grantor_id"], alice_id.to_string());

    // 14. Unauthorized profile read (trying to read Bob's account with Alice's grant) -> 403 forbidden
    let bob_profile_nonce = BASE64_URL_SAFE_NO_PAD.encode([7u8; 24]);
    let bob_profile_headers = sign_request(
        &ai_key,
        "GET",
        "/v1/accounts/99999/profile",
        &[],
        &access_token,
        now + 6,
        &bob_profile_nonce,
    );
    let (status, _, bob_resp) = call_http(
        &app,
        "GET",
        "/v1/accounts/99999/profile",
        bob_profile_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(bob_resp["error"], "forbidden");

    // 15. Alice lists grants and revokes
    let mut list_headers = HashMap::new();
    list_headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={alice_refresh}"),
    );
    let (status, l_headers, list_resp) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        list_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let grants_arr = list_resp["grants"].as_array().unwrap();
    assert_eq!(grants_arr.len(), 1);
    assert_eq!(grants_arr[0]["grant_id"], grant_id);
    let revoke_csrf = list_resp["csrf"].as_str().unwrap().to_string();
    let grants_cookie = l_headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let mut revoke_headers = HashMap::new();
    revoke_headers.insert(
        "cookie".to_string(),
        format!("{grants_cookie}; new_api_refresh={alice_refresh}"),
    );
    revoke_headers.insert("x-hg-csrf".to_string(), revoke_csrf);
    let (status, _, revoke_resp) = call_http(
        &app,
        "POST",
        &format!("/api/user/auth/humangrant/grants/{grant_id}/revoke"),
        revoke_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoke_resp["status"], "revoked");

    // 16. Immediate block verification: Token must be instantly blocked!
    let blocked_nonce = BASE64_URL_SAFE_NO_PAD.encode([8u8; 24]);
    let blocked_headers = sign_request(
        &ai_key,
        "GET",
        "/v1/session",
        &[],
        &access_token,
        now + 7,
        &blocked_nonce,
    );
    let (blocked_status, _, blocked_resp) =
        call_http(&app, "GET", "/v1/session", blocked_headers, vec![]).await;
    assert_eq!(blocked_status, StatusCode::UNAUTHORIZED);
    assert_eq!(blocked_resp["error"], "invalid_session");

    // 17. Superadmin root settings off/on epoch invalidation test
    let mut super_get_headers = HashMap::new();
    super_get_headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={superadmin_refresh}"),
    );
    let (status, set_headers, settings_resp) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/settings",
        super_get_headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings_resp["enabled"], true);
    let set_csrf = settings_resp["csrf"].as_str().unwrap().to_string();
    let set_cookie = set_headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let mut super_put_headers = HashMap::new();
    super_put_headers.insert(
        "cookie".to_string(),
        format!("{set_cookie}; new_api_refresh={superadmin_refresh}"),
    );
    super_put_headers.insert("x-hg-csrf".to_string(), set_csrf);
    super_put_headers.insert("content-type".to_string(), "application/json".to_string());
    let (status, _, put_resp) = call_http(
        &app,
        "PUT",
        "/api/user/auth/humangrant/settings",
        super_put_headers,
        serde_json::to_vec(&json!({ "enabled": false })).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(put_resp["enabled"], false);

    // With settings disabled, creating a new request fails with feature_disabled
    let new_nonce = BASE64_URL_SAFE_NO_PAD.encode([9u8; 24]);
    let dis_headers = sign_request(
        &ai_key,
        "POST",
        "/v1/requests",
        &req_bytes,
        "anonymous",
        now + 8,
        &new_nonce,
    );
    let (dis_status, _, dis_resp) =
        call_http(&app, "POST", "/v1/requests", dis_headers, req_bytes).await;
    assert_eq!(dis_status, StatusCode::FORBIDDEN);
    assert_eq!(dis_resp["error"], "feature_disabled");
}

#[sqlx::test(migrations = false)]
async fn test_browser_auth_session_validity_postgres(pool: PgPool) {
    let (browser, app, _hg_store) = setup_app(pool.clone()).await;

    let (alice_id, alice_refresh) = register_and_login(&browser, "alice_validity").await;

    // 1. Account deactivation (a.active = false)
    sqlx::query("UPDATE core_identity.accounts SET active = false WHERE id = (SELECT personal_account_id FROM core_identity.users WHERE id = $1)")
        .bind(alice_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut headers = HashMap::new();
    headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={alice_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Restore account
    sqlx::query("UPDATE core_identity.accounts SET active = true WHERE id = (SELECT personal_account_id FROM core_identity.users WHERE id = $1)")
        .bind(alice_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut headers = HashMap::new();
    headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={alice_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 2. User deactivation (u.active = false)
    sqlx::query("UPDATE core_identity.users SET active = false WHERE id = $1")
        .bind(alice_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut headers = HashMap::new();
    headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={alice_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Restore user
    sqlx::query("UPDATE core_identity.users SET active = true WHERE id = $1")
        .bind(alice_id)
        .execute(&pool)
        .await
        .unwrap();

    // 3. Credential revoked (c.revoked_at IS NOT NULL)
    sqlx::query("UPDATE core_identity.credentials SET revoked_at = clock_timestamp() WHERE id = (SELECT current_credential_id FROM core_identity.web_sessions WHERE user_id = $1)")
        .bind(alice_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut headers = HashMap::new();
    headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={alice_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 4. Weekly logout (weekly_logout = true AND created_at <= now - 7 days)
    // Register and login a fresh user
    let (bob_id, bob_refresh) = register_and_login(&browser, "bob_weekly").await;
    // Keep the session lifetime within 30 days and its absolute expiry in the future.
    // With weekly logout disabled, the same eight-day-old session remains usable.
    sqlx::query("UPDATE core_identity.web_sessions SET weekly_logout = false, created_at = statement_timestamp() - interval '8 days', expires_at = statement_timestamp() + interval '22 days' WHERE user_id = $1")
        .bind(bob_id)
        .execute(&pool)
        .await
        .unwrap();

    let valid_lifetime: bool = sqlx::query_scalar("SELECT created_at <= clock_timestamp() - interval '7 days' AND expires_at > clock_timestamp() AND expires_at <= created_at + interval '30 days' FROM core_identity.web_sessions WHERE user_id = $1")
        .bind(bob_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(valid_lifetime);
    let mut headers = HashMap::new();
    headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={bob_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    sqlx::query("UPDATE core_identity.web_sessions SET weekly_logout = true WHERE user_id = $1")
        .bind(bob_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut headers = HashMap::new();
    headers.insert(
        "cookie".to_string(),
        format!("new_api_refresh={bob_refresh}"),
    );
    let (status, _, _) = call_http(
        &app,
        "GET",
        "/api/user/auth/humangrant/grants",
        headers,
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
