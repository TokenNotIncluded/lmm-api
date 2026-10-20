//! Real personal session, PostgreSQL ledger, and counting synthetic TCP provider.
use lmm_core::{
    assistant_model::{AssistantModel, Binding, Error, PersonalTextPolicy, State},
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::*,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/runtime.rs"]
mod runtime_authority;
use model_fixture::Provider;
async fn fixture(
    pool: &PgPool,
    amount: i64,
) -> (
    lmm_core::identity::IdentityStore,
    lmm_core::billing::ledger::Ledger,
    String,
    String,
    i64,
    i64,
) {
    let result = model_fixture::fixture(pool, amount).await;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-assistant-test')").execute(pool).await.unwrap();
    result
}
fn gateway(pool: &PgPool, p: &Provider) -> ModelGateway {
    gateway_endpoint(pool, &p.endpoint)
}
fn gateway_endpoint(pool: &PgPool, endpoint: &str) -> ModelGateway {
    let prices = PriceSnapshot::compile(PriceConfig {
        version: 1,
        rules: vec![PriceRule {
            billing_mode: BillingMode::Tokens,
            id: 1,
            model: "local-model".into(),
            group: "allowed".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: 2_000_000,
                output: 4_000_000,
                cache_read: 500_000,
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm: MULTIPLIER_ONE,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        }],
    })
    .unwrap();
    let router = Arc::new(
        lmm_core::routing::Router::new(
            RoutingConfig {
                version: 1,
                groups: vec![ModelGroup {
                    name: "allowed".into(),
                    enabled: true,
                }],
                upstreams: vec![UpstreamConfig {
                    id: 1,
                    endpoint: Endpoint::local_loopback(endpoint.to_owned()).unwrap(),
                    protocol: Protocol::OpenAiChat,
                    credential: CredentialRef { id: 7, revision: 1 },
                    enabled: true,
                }],
                routes: vec![ModelRoute {
                    id: 1,
                    model: "local-model".into(),
                    group: "allowed".into(),
                    enabled: true,
                    speeds: vec![Speed::Standard],
                    targets: vec![RouteTarget {
                        id: 1,
                        upstream_id: 1,
                        upstream_model: "provider-model".into(),
                        enabled: true,
                        priority: 0,
                        weight: 1,
                        speeds: vec![Speed::Standard],
                        payload: PayloadPolicy::Adapt,
                        timeout_ms: 5000,
                    }],
                    failover: FailoverPolicy {
                        max_attempts: 1,
                        retry_on: vec![],
                        allow_replay_after_send: false,
                    },
                }],
                aliases: vec![],
            },
            Arc::new(prices),
        )
        .unwrap(),
    );
    ModelGateway::new(
        pool.clone(),
        router,
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-upstream-key".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_secs(1),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 4096,
            lease_seconds: 30,
        },
    )
    .unwrap()
}
fn service(pool: &PgPool, p: &Provider) -> AssistantModel {
    AssistantModel::new(
        pool.clone(),
        gateway(pool, p),
        PersonalTextPolicy {
            model: "local-model".into(),
            group: "allowed".into(),
            budget_credits: 10000,
            max_output_tokens: 32,
            ttl_seconds: 300,
            gift_tools: false,
            sponsor: None,
        },
    )
    .unwrap()
}
async fn binding(s: &AssistantModel, session: &str, key: &str, text: &str) -> Binding {
    let c = s.conversation(session, key, true).await.unwrap();
    let g = s
        .turn(session, &c.core_conversation_id, "turn-1", true)
        .await
        .unwrap();
    Binding {
        delegation_id: g.delegation_id,
        core_conversation_id: g.core_conversation_id,
        turn_id: g.turn_id,
        round: 0,
        operation_key: "turn-1:0".into(),
        text_sha256: Sha256::digest(text.as_bytes()).to_vec(),
    }
}
#[sqlx::test]
async fn answer_settles_once_and_replays_without_dispatch(pool: PgPool) {
    let (_, ledger, session, key, _, account) = fixture(&pool, 100000).await;
    let p = Provider::start(true).await;
    let s = service(&pool, &p);
    let b = binding(&s, &session, "conversation-1", "hello").await;
    let result = s.reserve_text(&session, &b, "hello").await.unwrap();
    assert_eq!(result.state, State::Settled);
    assert_eq!(
        result.answer_text.as_deref(),
        Some("local synthetic answer")
    );
    assert_eq!(result.settled_credits, Some(49));
    assert_eq!(
        s.reserve_text(&session, &b, "hello")
            .await
            .unwrap()
            .request_id,
        result.request_id
    );
    assert_eq!(
        s.operation(&session, &b).await.unwrap().answer_text,
        result.answer_text
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
    let row:(String,Option<i64>,Option<serde_json::Value>,String)=sqlx::query_as("SELECT state,key_id,oauth_source,assistant_grant_id FROM core_charging.requests WHERE id=$1").bind(&result.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(row, ("settled".into(), None, None, b.delegation_id.clone()));
    assert_eq!(ledger.balance(account).await.unwrap().reserved_units, 0);
    assert_eq!(
        s.conversation(&key, "forbidden", true).await.unwrap_err(),
        Error::Forbidden
    );
    let mut altered = b.clone();
    altered.text_sha256 = Sha256::digest(b"different").to_vec();
    assert_eq!(
        s.reserve_text(&session, &altered, "different")
            .await
            .unwrap_err(),
        Error::Conflict
    );
    s.revoke(&session, &b.delegation_id).await.unwrap();
    assert_eq!(
        s.operation(&session, &b).await.unwrap().state,
        State::Settled
    );
}
#[sqlx::test]
async fn missing_usage_is_unknown_and_never_replayed_after_reconstruction(pool: PgPool) {
    let (_, _, session, _, _, _) = fixture(&pool, 100000).await;
    let p = Provider::start(false).await;
    let s = service(&pool, &p);
    let b = binding(&s, &session, "conversation-2", "hello").await;
    let op = s.reserve_text(&session, &b, "hello").await.unwrap();
    assert_eq!(op.state, State::Unknown);
    assert!(op.answer_text.is_none());
    assert!(op.settled_credits.is_none());
    // Even an accidental Core writer cannot expose an answer without its
    // authoritative settlement in the same commit.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE core_assistant.operations SET answer_text='forged',completed_at=extract(epoch FROM clock_timestamp())::bigint WHERE request_id=$1").bind(&op.request_id).execute(&mut *tx).await.unwrap();
    assert!(tx.commit().await.is_err());
    drop(s);
    let s = service(&pool, &p);
    assert_eq!(
        s.operation(&session, &b).await.unwrap().state,
        State::Unknown
    );
    assert_eq!(
        s.reserve_text(&session, &b, "hello").await.unwrap().state,
        State::Unknown
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        s.turn(&session, &b.core_conversation_id, "turn-2", true)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let held:i64=sqlx::query_scalar("SELECT reserved FROM core_charging.requests WHERE id=$1 AND state='reconcile' AND settled=0").bind(op.request_id).fetch_one(&pool).await.unwrap();
    assert!(held > 0);
}
#[sqlx::test]
async fn cross_owner_revocation_and_digest_fail_before_dispatch(pool: PgPool) {
    let (identity, _, session, _, _, _) = fixture(&pool, 100000).await;
    let other = identity.bootstrap_user(8, 1).await.unwrap();
    let p = Provider::start(true).await;
    let s = service(&pool, &p);
    let b = binding(&s, &session, "conversation-3", "hello").await;
    assert_eq!(
        s.reserve_text(&other.secret, &b, "hello")
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        s.reserve_text(&session, &b, "tampered").await.unwrap_err(),
        Error::Invalid
    );
    s.revoke(&session, &b.delegation_id).await.unwrap();
    assert_eq!(
        s.reserve_text(&session, &b, "hello").await.unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test]
async fn budget_and_expiry_reject_without_leaving_a_charge(pool: PgPool) {
    let (_, _, session, _, _, _) = fixture(&pool, 100000).await;
    let p = Provider::start(true).await;
    let s = AssistantModel::new(
        pool.clone(),
        gateway(&pool, &p),
        PersonalTextPolicy {
            model: "local-model".into(),
            group: "allowed".into(),
            budget_credits: 1,
            max_output_tokens: 32,
            ttl_seconds: 300,
            gift_tools: false,
            sponsor: None,
        },
    )
    .unwrap();
    let b = binding(&s, &session, "too-small-budget", "hello").await;
    assert_eq!(
        s.reserve_text(&session, &b, "hello").await.unwrap_err(),
        Error::NoFunds
    );
    let charges: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(charges, 0);
    let s = AssistantModel::new(
        pool.clone(),
        gateway(&pool, &p),
        PersonalTextPolicy {
            model: "local-model".into(),
            group: "allowed".into(),
            budget_credits: 10000,
            max_output_tokens: 32,
            ttl_seconds: 1,
            gift_tools: false,
            sponsor: None,
        },
    )
    .unwrap();
    let b = binding(&s, &session, "expired-grant", "hello").await;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        s.reserve_text(&session, &b, "hello").await.unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test]
async fn concurrent_reservation_never_dispatches_twice(pool: PgPool) {
    let (_, _, session, _, _, _) = fixture(&pool, 100000).await;
    let p = Provider::start(true).await;
    let s = service(&pool, &p);
    let b = binding(&s, &session, "concurrent", "hello").await;
    let (a, both) = tokio::join!(
        s.reserve_text(&session, &b, "hello"),
        s.reserve_text(&session, &b, "hello")
    );
    assert!(a.is_ok() || both.is_ok());
    for result in [a, both] {
        if let Err(error) = result {
            assert_eq!(error, Error::Conflict)
        }
    }
    assert_eq!(
        s.operation(&session, &b).await.unwrap().state,
        State::Settled
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test]
async fn cancelled_rpc_does_not_cancel_core_worker_or_replay_dispatch(pool: PgPool) {
    let (_, _, session, _, _, _) = fixture(&pool, 100000).await;
    let p = Provider::start(true).await;
    let s = service(&pool, &p);
    let b = binding(&s, &session, "cancelled-rpc", "pause").await;
    let caller = {
        let s = s.clone();
        let session = session.clone();
        let b = b.clone();
        tokio::spawn(async move { s.reserve_text(&session, &b, "pause").await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while p.hits.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    let _ = caller.await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let op = s.operation(&session, &b).await.unwrap();
            if op.state == State::Unknown {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        s.reserve_text(&session, &b, "pause").await.unwrap().state,
        State::Unknown
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[sqlx::test]
async fn rpc_requires_single_host_protocol_and_original_personal_session(pool: PgPool) {
    use lmm_core::{
        assistant_model::rpc::AssistantRpc,
        internal_rpc::pb::{self, core_assistant_model_server::CoreAssistantModel},
    };
    let (_, _, session, key, _, _) = fixture(&pool, 100000).await;
    let p = Provider::start(true).await;
    let rpc = AssistantRpc::new(
        service(&pool, &p),
        b"synthetic-assistant-service-token-123456",
        true,
    )
    .unwrap();
    assert_eq!(
        rpc.features(),
        vec![
            "assistant.personal_text.reserve.v1",
            "assistant.personal_text.execute.v1"
        ]
    );
    let make = |user: &str| {
        let mut r = tonic::Request::new(pb::AssistantConversationKey {
            creation_key: "rpc-conversation".into(),
        });
        r.metadata_mut().insert(
            "authorization",
            "Bearer synthetic-assistant-service-token-123456"
                .parse()
                .unwrap(),
        );
        r.metadata_mut()
            .insert("x-lmm-protocol", "1".parse().unwrap());
        r.metadata_mut()
            .insert("x-lmm-user-credential", user.parse().unwrap());
        r
    };
    let mut r = make(&session);
    r.metadata_mut().remove("authorization");
    assert_eq!(
        rpc.begin_conversation(r).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut r = make(&session);
    r.metadata_mut().append(
        "authorization",
        "Bearer synthetic-assistant-service-token-123456"
            .parse()
            .unwrap(),
    );
    assert_eq!(
        rpc.begin_conversation(r).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut r = make(&session);
    r.metadata_mut()
        .insert("x-lmm-protocol", "2".parse().unwrap());
    assert_eq!(
        rpc.begin_conversation(r).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let mut r = make(&session);
    r.metadata_mut()
        .append("x-lmm-user-credential", session.parse().unwrap());
    assert_eq!(
        rpc.begin_conversation(r).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        rpc.begin_conversation(make(&key)).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    assert!(
        rpc.begin_conversation(make(&session))
            .await
            .unwrap()
            .into_inner()
            .core_conversation_id
            .starts_with("lmm_cv_")
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test]
async fn token_limit_answer_settles_and_core_builds_ordered_context(pool: PgPool) {
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};
    use std::sync::Mutex;
    let (_, ledger, session, _, _, account) = fixture(&pool, 100000).await;
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let requests = captured.clone();
    let app=Router::new().route("/v1/chat/completions",post(move |Json(input):Json<Value>|{requests.lock().unwrap().push(input);async{Json(json!({"choices":[{"message":{"role":"assistant","content":"bounded response","tool_calls":null,"function_call":null},"finish_reason":"length"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}}))}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let s = AssistantModel::new(
        pool.clone(),
        gateway_endpoint(&pool, &endpoint),
        PersonalTextPolicy {
            model: "local-model".into(),
            group: "allowed".into(),
            budget_credits: 10000,
            max_output_tokens: 32,
            ttl_seconds: 300,
            gift_tools: false,
            sponsor: None,
        },
    )
    .unwrap();
    let b = binding(&s, &session, "ordered-context", "first").await;
    assert_eq!(
        s.reserve_text(&session, &b, "first").await.unwrap().state,
        State::Settled
    );
    let g = s
        .turn(&session, &b.core_conversation_id, "turn-2", true)
        .await
        .unwrap();
    let second = Binding {
        delegation_id: g.delegation_id,
        core_conversation_id: g.core_conversation_id,
        turn_id: g.turn_id,
        round: 0,
        operation_key: "turn-2:0".into(),
        text_sha256: Sha256::digest(b"second").to_vec(),
    };
    assert_eq!(
        s.reserve_text(&session, &second, "second")
            .await
            .unwrap()
            .state,
        State::Settled
    );
    let requests = captured.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]["messages"],
        json!([{"role":"user","content":"first"},{"role":"assistant","content":"bounded response"},{"role":"user","content":"second"}])
    );
    drop(requests);
    assert_eq!(ledger.balance(account).await.unwrap().reserved_units, 0);
    server.abort();
    let _ = server.await;
}

async fn synthetic_gift_qualification(pool: &PgPool) {
    use hmac::{Hmac, Mac};
    // Explicit installer-owned synthetic attestation. This is not evidence of
    // a real Google registration, provider identity, or production eligibility.
    let key: Vec<u8> = sqlx::query_scalar("SELECT secret FROM core_gift.risk_key WHERE singleton")
        .fetch_one(pool)
        .await
        .unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
    mac.update(b"identity\0synthetic-onboarding@example.invalid");
    let email = mac.finalize().into_bytes().to_vec();
    sqlx::query("UPDATE core_identity.user_profiles SET email='synthetic-onboarding@example.invalid' WHERE user_id=7").execute(pool).await.unwrap();
    sqlx::query("UPDATE core_gift.policy SET enabled=TRUE,exclusive_cap=1000")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_gift.qualifications(user_id,email_hash,network_hash,evidence_digest,evidence_kind,verified_at,registration_hold,reward_used) VALUES(7,$1,$2,$3,'owner_reviewed_registration_v1',clock_timestamp(),FALSE,FALSE)").bind(email).bind(vec![21u8;32]).bind(vec![22u8;32]).execute(pool).await.unwrap();
}
async fn synthetic_treasury(
    pool: &PgPool,
    identity: &lmm_core::identity::IdentityStore,
    ledger: &lmm_core::billing::ledger::Ledger,
) -> i64 {
    use lmm_core::billing::{
        Credits,
        ledger::{Action, Request},
    };
    let treasury = identity.bootstrap_user(8, 6).await.unwrap();
    let account = identity
        .authorize(&treasury.secret)
        .await
        .unwrap()
        .owner_account_id;
    ledger.open_wallet(account).await.unwrap();
    ledger
        .execute(&Request {
            scope: "assistant-sponsor-test".into(),
            operation_key: "synthetic-treasury-funds".into(),
            actor_user_id: Some(8),
            reason: "Synthetic treasury for isolated assistant test".into(),
            action: Action::Credit {
                account_id: account,
                amount_units: Credits::try_from(100000).unwrap(),
                payment_reference: "synthetic:assistant-treasury".into(),
            },
        })
        .await
        .unwrap();
    let _ = pool;
    account
}
#[sqlx::test]
async fn zero_balance_user_uses_bounded_treasury_and_real_gift_ledger(pool: PgPool) {
    use axum::{Json, Router, routing::post};
    use lmm_core::assistant_model::SponsorPolicy;
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicUsize;
    let (identity, ledger, session, _, _, account) = fixture(&pool, 0).await;
    let treasury = synthetic_treasury(&pool, &identity, &ledger).await;
    synthetic_gift_qualification(&pool).await;
    // A qualified new user need not possess ordinary developer routing access.
    sqlx::query("DELETE FROM core_identity.user_routing_groups WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM core_identity.developer_access WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    // Removing installer-owned routing grants correctly revokes the old session.
    // Seed only a synthetic password credential, then use the real login path to
    // obtain current Session authority; never rewrite a credential's auth version.
    assert!(identity.authorize(&session).await.is_err());
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    let login = lmm_core::identity::LoginRequest {
        login_name: "native_7".into(),
        password: "synthetic onboarding login password 7".into(),
    };
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let hash = Argon2::default()
        .hash_password(login.password.as_bytes(), &salt)
        .unwrap()
        .to_string();
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES($1,7,$2)")
        .bind(&login.login_name).bind(hash).execute(&pool).await.unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET password_login_allowed=TRUE WHERE user_id=7",
    )
    .execute(&pool)
    .await
    .unwrap();
    let session = identity.login(&login).await.unwrap().secret;
    assert_eq!(identity.authorize(&session).await.unwrap().user_id, 7);
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app=Router::new().route("/v1/chat/completions",post(move |Json(request):Json<Value>|{let attempt=counter.fetch_add(1,Ordering::SeqCst);assert_eq!(request["tools"].as_array().unwrap().len(),2);assert_eq!(request["parallel_tool_calls"],false);async move{let choice=if attempt==0{json!({"message":{"role":"assistant","content":"请继续介绍你的使用需求。"},"finish_reason":"stop"})}else{json!({"message":{"role":"assistant","content":null,"tool_calls":[{"id":"trusted_gift_call","type":"function","function":{"name":"prepare_new_user_gift","arguments":"{\"amount_credits\":25}"}}]},"finish_reason":"tool_calls"})};Json(json!({"choices":[choice],"usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}}))}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    let s = AssistantModel::new(
        pool.clone(),
        gateway_endpoint(&pool, &endpoint),
        PersonalTextPolicy {
            model: "local-model".into(),
            group: "allowed".into(),
            budget_credits: 5000,
            max_output_tokens: 32,
            ttl_seconds: 300,
            gift_tools: true,
            sponsor: Some(SponsorPolicy {
                policy_id: "synthetic-new-user-v1".into(),
                treasury_account_id: treasury,
                max_credits_per_user: 10000,
                global_budget_credits: 20000,
                max_calls_per_user: 2,
                expires_at_unix: now + 600,
            }),
        },
    )
    .unwrap();
    let first = "I want to connect my desktop application to this service and understand how to configure a safe endpoint before issuing my first request.";
    let b = binding(&s, &session, "sponsored-onboarding", first).await;
    let first_op = s.reserve_text(&session, &b, first).await.unwrap();
    assert_eq!(first_op.state, State::Settled);
    assert!(first_op.gift_receipt.is_none());
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 0);
    let text = "My second question concerns the command line client configuration and the steps needed to inspect usage without exposing credentials in logs.";
    let g = s
        .turn(&session, &b.core_conversation_id, "turn-2", true)
        .await
        .unwrap();
    let next = Binding {
        delegation_id: g.delegation_id,
        core_conversation_id: g.core_conversation_id,
        turn_id: g.turn_id,
        round: 0,
        operation_key: "turn-2:0".into(),
        text_sha256: Sha256::digest(text.as_bytes()).to_vec(),
    };
    let result = s.reserve_text(&session, &next, text).await.unwrap();
    assert_eq!(result.state, State::Settled);
    let receipt = result.gift_receipt.as_ref().unwrap();
    assert_eq!(receipt.owner_user_id, 7);
    assert_eq!(receipt.credit_amount, 25);
    assert!(receipt.claimed_at_unix.is_some());
    assert!(receipt.ledger_transaction_id.is_some());
    assert!(result.answer_text.as_ref().unwrap().contains("25"));
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 25);
    let platform = ledger.balance(treasury).await.unwrap();
    assert_eq!(
        (platform.available_units, platform.reserved_units),
        (99902, 0)
    );
    let original = s.reserve_text(&session, &next, text).await.unwrap();
    assert_eq!(original.gift_receipt, result.gift_receipt);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let payers:Vec<(i64,i64,Option<i64>)>=sqlx::query_as("SELECT owner_account_id,payer_account_id,assistant_sponsor_account_id FROM core_charging.requests ORDER BY id").fetch_all(&pool).await.unwrap();
    assert_eq!(
        payers,
        vec![
            (account, treasury, Some(treasury)),
            (account, treasury, Some(treasury))
        ]
    );
    let gift = lmm_core::gift::GiftStore::new(pool.clone())
        .get(&session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gift.gift_id, receipt.gift_id);
    let (gift, already) = lmm_core::gift::GiftStore::new(pool.clone())
        .claim_stored(&session)
        .await
        .unwrap();
    assert!(already);
    assert_eq!(gift.gift_id, receipt.gift_id);
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 25);
    assert!(matches!(
        s.turn(&session, &b.core_conversation_id, "turn-3", true)
            .await,
        Err(Error::Forbidden | Error::NoFunds)
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    server.abort();
    let _ = server.await;
}

#[sqlx::test]
async fn real_runtime_role_executes_without_updating_immutable_authority_tables(pool: PgPool) {
    let (_, ledger, session, _, _, account) = fixture(&pool, 100000).await;
    let role = runtime_authority::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_assistant").await;
    let cannot_update: bool = sqlx::query_scalar("SELECT NOT has_any_column_privilege(current_user,'core_assistant.conversations','UPDATE') AND NOT has_any_column_privilege(current_user,'core_identity.user_routing_groups','UPDATE')")
        .fetch_one(&role.pool).await.unwrap();
    assert!(cannot_update);
    let provider = Provider::start(true).await;
    let service = service(&role.pool, &provider);
    let binding = binding(
        &service,
        &session,
        "restricted-role",
        "actual runtime user text",
    )
    .await;
    let result = service
        .reserve_text(&session, &binding, "actual runtime user text")
        .await
        .unwrap();
    assert_eq!(result.state, State::Settled);
    assert_eq!(result.settled_credits, Some(49));
    assert!(result.answer_text.is_some());
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    let status = service.operation(&session, &binding).await.unwrap();
    assert_eq!(status.request_id, result.request_id);
    assert_eq!(status.state, State::Settled);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (99951, 0)
    );
    drop(service);
    drop(provider);
    role.cleanup().await;
}
