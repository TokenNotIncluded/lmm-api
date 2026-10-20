//! Independent synthetic PostgreSQL acceptance: explicit OAuth budget is an
//! additional lifetime boundary, never replaced by a larger per-tool grant.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lmm_core::{
    billing::{
        Credits,
        ledger::{Action, Ledger, Request},
    },
    core_schema,
    identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore},
    market_oauth::{AuthorizationRequest, MarketIssuer, MarketProfile, Registration, TokenForm},
    tool_market::{
        self, Owner,
        credit_access::{ApproveRequest, ConsentRequest, CreditStore},
        credit_config::ConfigRequest,
        credit_execution::{DispatchRequest, ReleaseRequest, ReserveRequest},
        credits::{Price, Quote},
    },
};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, types::Json};
use std::sync::Arc;
#[path = "common/runtime.rs"]
mod runtime;
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
fn hash<T: serde::Serialize>(v: &T) -> String {
    Sha256::digest(serde_json::to_vec(v).unwrap())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
struct Fixture {
    store: CreditStore,
    issuer: MarketIssuer,
    client: String,
    access: String,
    refresh: String,
    owner: Owner,
    quote: Quote,
    grant: String,
    now: i64,
    role: runtime::RuntimeRole,
}
async fn fixture(pool: &PgPool, budget: i64) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let browser =
        BrowserAuth::new(identity.clone().with_registration_enabled(true), &[71; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "independentbuyer",
            password: "password1234",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "independentbuyer",
            "password1234",
            &BrowserSessionMetadata {
                ip: "203.0.113.71".into(),
                user_agent: "synthetic-independent".into(),
            },
        )
        .await
        .unwrap();
    let session = bundle.data["access_token"].as_str().unwrap().to_owned();
    let actor = identity.authorize(&session).await.unwrap();
    let seller = identity.bootstrap_user(201, 5).await.unwrap().secret;
    let root = identity.bootstrap_user(202, 6).await.unwrap().secret;
    let reviewer = identity.bootstrap_user(203, 5).await.unwrap().secret;
    let ledger = Ledger::from_pool(pool.clone());
    for secret in [&session, &seller, &root] {
        let p = identity.authorize(secret).await.unwrap();
        ledger.open_wallet(p.owner_account_id).await.unwrap();
    }
    ledger
        .execute(&Request {
            scope: "independent-oauth-fixture".into(),
            operation_key: "fund-real-synthetic-buyer".into(),
            actor_user_id: Some(actor.user_id),
            reason: "Isolated independent synthetic funds".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(100).unwrap(),
                payment_reference: "synthetic-no-real-payment".into(),
            },
        })
        .await
        .unwrap();
    let profile = MarketProfile::isolated_loopback_test("http://127.0.0.1:55121", 1000).unwrap();
    let issuer = MarketIssuer::open(pool.clone(), profile.clone())
        .await
        .unwrap();
    let client = issuer
        .register(Registration {
            client_name: "Independent synthetic client".into(),
            application_type: "native".into(),
            redirect_uris: vec!["http://127.0.0.1:53123/callback".into()],
            token_endpoint_auth_method: "none".into(),
            grant_types: vec!["authorization_code".into(), "refresh_token".into()],
            response_types: vec!["code".into()],
            scope: "market:discover market:invoke market:manage".into(),
        })
        .await
        .unwrap()
        .client_id;
    let page = issuer
        .begin(AuthorizationRequest {
            response_type: "code".into(),
            client_id: client.clone(),
            redirect_uri: "http://127.0.0.1:53124/callback".into(),
            scope: "market:discover market:invoke market:manage".into(),
            resource: profile.resource().into(),
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER)),
            code_challenge_method: "S256".into(),
            state: "independent-state".into(),
        })
        .await
        .unwrap();
    let page = issuer
        .continue_authorization(&page.binding, &page.csrf, &bundle.refresh_token)
        .await
        .unwrap();
    let redirect = issuer
        .consent(
            &page.binding,
            &page.csrf,
            &bundle.refresh_token,
            true,
            budget,
        )
        .await
        .unwrap();
    let code = reqwest::Url::parse(&redirect)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let token = issuer
        .token(TokenForm {
            grant_type: "authorization_code".into(),
            client_id: client.clone(),
            resource: profile.resource().into(),
            code: Some(code),
            redirect_uri: Some("http://127.0.0.1:53124/callback".into()),
            code_verifier: Some(VERIFIER.into()),
            refresh_token: None,
            scope: None,
        })
        .await
        .unwrap();
    let setup =
        CreditStore::from_pool(pool.clone()).with_market_oauth_profile(Arc::new(profile.clone()));
    setup
        .update_config(
            &root,
            &ConfigRequest {
                operation_id: "independent-policy".into(),
                enabled: true,
                fee_bps: 0,
                recipient_id: 202,
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_tool_market.publishers(user_id,active) VALUES(201,TRUE)")
        .execute(pool)
        .await
        .unwrap();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(pool)
            .await
            .unwrap();
    let price = Price::Fixed { price_quota: 2 };
    let pricing = hash(&price);
    let remote = "a".repeat(64);
    let quote = Quote {
        id: hash(&[
            "legacy-credit-quote-v2",
            "independent-version",
            "independent-tool",
            &remote,
            &pricing,
        ]),
        service_id: "independent-service".into(),
        tool_id: "independent-tool".into(),
        version_id: "independent-version".into(),
        remote_digest: remote,
        pricing_digest: pricing,
        price,
        expires: now + 3600,
    };
    setup.register_quote(&seller, &quote).await.unwrap();
    setup
        .approve_quote(
            &reviewer,
            &ApproveRequest {
                operation_id: "independent-review".into(),
                quote_id: quote.id.clone(),
                visibility: "public".into(),
                approved: true,
            },
        )
        .await
        .unwrap();
    let schema = serde_json::json!({"type":"object"});
    sqlx::query("INSERT INTO core_tool_market.credit_result_contracts(quote_id,proof_mode,witness_role,output_schema,output_schema_digest) VALUES($1,'core_host_fixed_success','independent-test-witness',$2,$3)").bind(&quote.id).bind(Json(&schema)).bind(hash(&schema)).execute(pool).await.unwrap();
    let owner = Owner {
        user: actor.user_id.to_string(),
        account: actor.user_id.to_string(),
    };
    let grant = setup
        .create_consent(
            &session,
            &ConsentRequest {
                owner: owner.clone(),
                operation_id: "independent-native-human-grant".into(),
                client_id: format!("oauth:{client}"),
                tool_id: quote.tool_id.clone(),
                version_id: quote.version_id.clone(),
                max_price_quota: 2,
                max_total_quota: 20,
                max_calls: 10,
                expires_at: now + 3500,
            },
        )
        .await
        .unwrap()
        .id;
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("core_tool_market").await;
    sqlx::raw_sql(&format!(r#"
 GRANT USAGE ON SCHEMA core_market_oauth TO {0};
 GRANT SELECT,INSERT ON core_market_oauth.clients,core_market_oauth.requests,core_market_oauth.families,core_market_oauth.codes,core_market_oauth.tokens TO {0};
 GRANT UPDATE(revoked_at) ON core_market_oauth.clients,core_market_oauth.families TO {0};
 GRANT UPDATE(consumed_at) ON core_market_oauth.requests,core_market_oauth.codes,core_market_oauth.tokens TO {0};
 GRANT EXECUTE ON FUNCTION core_market_oauth.scope_set(text[]) TO {0};
 "#,role.name)).execute(pool).await.unwrap();
    lmm_core::runtime_database::check(
        &role.pool,
        lmm_core::runtime_database::RequiredCapabilities {
            tool_market: true,
            market_oauth: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let store =
        CreditStore::from_pool(role.pool.clone()).with_market_oauth_profile(Arc::new(profile));
    Fixture {
        store,
        issuer,
        client,
        access: token.access_token,
        refresh: token.refresh_token.unwrap(),
        owner,
        quote,
        grant,
        now,
        role,
    }
}
fn reserve(f: &Fixture, key: &str) -> ReserveRequest {
    ReserveRequest {
        owner: f.owner.clone(),
        client_id: format!("oauth:{}", f.client),
        request_key: key.into(),
        quote_id: f.quote.id.clone(),
        grant_id: f.grant.clone(),
        tool_id: f.quote.tool_id.clone(),
        version_id: f.quote.version_id.clone(),
        input_digest: "b".repeat(64),
        resolve_by: f.now + 1800,
    }
}
#[sqlx::test]
async fn explicit_oauth_one_credit_denies_two_with_sufficient_wallet_and_grant(pool: PgPool) {
    let f = fixture(&pool, 1).await;
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    let out = f
        .store
        .reserve(&f.access, &reserve(&f, "over-original-oauth-cap"))
        .await;
    assert!(
        matches!(out, Err(tool_market::Error::Forbidden)),
        "OAuth budget1 admitted price2 despite sufficient wallet/grant: {out:?}"
    );
    let calls: i64 = sqlx::query_scalar("SELECT count(*) FROM core_tool_market.credit_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls, 0);
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(journals, after);
    f.role.cleanup().await;
}
#[sqlx::test]
async fn original_family_hold_survives_unknown_and_refresh(pool: PgPool) {
    let f = fixture(&pool, 3).await;
    let call = f
        .store
        .reserve(&f.access, &reserve(&f, "first-held-two"))
        .await
        .unwrap();
    assert_eq!(call.held_quota, 2);
    let dispatch = f.store.dispatch(&f.access, &call.call_id).await.unwrap();
    let unknown = f
        .store
        .unknown(&DispatchRequest {
            call_id: call.call_id,
            dispatch_id: dispatch.dispatch_id,
        })
        .await
        .unwrap();
    assert_eq!(unknown.state, "unknown");
    let rotated = f
        .issuer
        .token(TokenForm {
            grant_type: "refresh_token".into(),
            client_id: f.client.clone(),
            resource: f.issuer.profile().resource().into(),
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(f.refresh.clone()),
            scope: None,
        })
        .await
        .unwrap();
    let out = f
        .store
        .reserve(
            &rotated.access_token,
            &reserve(&f, "new-access-cannot-reset-family-cap"),
        )
        .await;
    assert!(
        matches!(out, Err(tool_market::Error::Forbidden)),
        "UNKNOWN reservation disappeared after refresh: {out:?}"
    );
    let used:i64=sqlx::query_scalar("SELECT sum(amount_credits-refunded)::bigint FROM core_charging.external_consumption WHERE domain='toolmarket'").fetch_one(&pool).await.unwrap();
    assert_eq!(used, 2);
    f.role.cleanup().await;
}
#[sqlx::test]
async fn concurrent_oauth_holds_share_original_family_budget(pool: PgPool) {
    let f = fixture(&pool, 3).await;
    let mut tasks = Vec::new();
    for i in 0..6 {
        let store = f.store.clone();
        let access = f.access.clone();
        let request = reserve(&f, &format!("concurrent-{i}"));
        tasks.push(tokio::spawn(async move {
            for _ in 0..8 {
                match store.reserve(&access, &request).await {
                    Err(tool_market::Error::Retry) => continue,
                    other => return other,
                }
            }
            Err(tool_market::Error::Retry)
        }))
    }
    let mut succeeded = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(r) => {
                assert_eq!(r.held_quota, 2);
                succeeded += 1
            }
            Err(tool_market::Error::Forbidden | tool_market::Error::Retry) => {}
            other => panic!("unexpected reserve outcome {other:?}"),
        }
    }
    assert_eq!(succeeded, 1);
    let used:i64=sqlx::query_scalar("SELECT sum(amount_credits-refunded)::bigint FROM core_charging.external_consumption WHERE domain='toolmarket'").fetch_one(&pool).await.unwrap();
    assert_eq!(used, 2);
    f.role.cleanup().await;
}

#[sqlx::test]
async fn undispatched_release_returns_only_original_family_hold(pool: PgPool) {
    let f = fixture(&pool, 3).await;
    let original = f
        .store
        .reserve(&f.access, &reserve(&f, "cancelled-before-dispatch"))
        .await
        .unwrap();
    let released = f
        .store
        .release(&ReleaseRequest {
            call_id: original.call_id,
            dispatch_id: String::new(),
            reason: "cancelled".into(),
            evidence_digest: String::new(),
            raw_result: None,
        })
        .await
        .unwrap();
    assert_eq!(released.state, "released");
    assert_eq!(released.refunded_quota, 2);
    let used:i64=sqlx::query_scalar("SELECT sum(amount_credits-refunded)::bigint FROM core_charging.external_consumption WHERE domain='toolmarket'").fetch_one(&pool).await.unwrap();
    assert_eq!(used, 0);
    let next = f
        .store
        .reserve(&f.access, &reserve(&f, "after-confirmed-release"))
        .await
        .unwrap();
    assert_eq!(next.held_quota, 2);
    let denied = f
        .store
        .reserve(&f.access, &reserve(&f, "third-still-must-fit"))
        .await;
    assert!(matches!(denied, Err(tool_market::Error::Forbidden)));
    f.role.cleanup().await;
}
