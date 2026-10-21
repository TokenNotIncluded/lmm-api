//! Real PostgreSQL identity, selected routing, charging and ledger boundaries.
//! These tests deliberately stop before DNS or provider HTTP. HTTPS contracts
//! exercise the external-policy branch; loopback remains a separate synthetic
//! contract. Supplied usage below is test evidence, not a live vendor receipt.
use super::{
    ChargePolicy, ExternalModelContext, ModelGateway, Operation, ProviderPolicy, UpstreamCredential,
};
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::Charge,
        ledger::{Action, Ledger, Request as LedgerRequest},
    },
    funding::FundingPolicy,
    identity::IdentityStore,
    relay::{
        self, Billing, ClientProtocol, FinishReason, Limits, OutputCheckpoint, RelayError,
        RequestContext, Route, RouteProvider,
    },
    routing::{
        self, BillingMode, CredentialRef, Endpoint, MULTIPLIER_ONE, PriceConfig, PriceRule,
        PriceSnapshot, Protocol, RoutingConfig, Speed, TokenRates,
    },
};
use axum::http::{HeaderMap, HeaderValue, header};
use rand::{RngCore, rngs::OsRng};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const SMALL_INPUT: i64 = 65_536;
const GLOBAL_INPUT: i64 = 1_048_576;
const SMALL_OUTPUT: u32 = 128;
const GLOBAL_OUTPUT: u32 = 4096;
const EXTERNAL_BASE: &str = "https://selected-bound.example/v1";

struct Fixture {
    pool: PgPool,
    ledger: Ledger,
    key: String,
    key_id: i64,
    account: i64,
}

async fn fixture(pool: PgPool, amount: i64) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::check(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let principal = identity.authorize(&session.secret).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-selected-bound-pg')")
        .execute(&pool)
        .await
        .unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger
        .open_wallet(principal.owner_account_id)
        .await
        .unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "selected-bound-pg".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in disposable selected-bound PostgreSQL".into(),
            action: Action::Credit {
                account_id: principal.owner_account_id,
                amount_units: Credits::try_from(amount).unwrap(),
                payment_reference: "synthetic:selected-bound".into(),
            },
        })
        .await
        .unwrap();
    Fixture {
        pool,
        ledger,
        key: key.secret,
        key_id: key.id,
        account: principal.owner_account_id,
    }
}

fn contexts(small_input: i64) -> Vec<ExternalModelContext> {
    [
        ("small-provider", small_input, SMALL_OUTPUT),
        ("large-provider", GLOBAL_INPUT, GLOBAL_OUTPUT),
    ]
    .into_iter()
    .map(|(model, input, output)| ExternalModelContext {
        missing_cache_creation_tokens_is_zero: false,
        upstream_id: 1,
        upstream_model: model.into(),
        protocol: Protocol::OpenAiChat,
        endpoint: EXTERNAL_BASE.into(),
        max_input_tokens: u32::try_from(input).unwrap(),
        max_output_tokens: output,
        contract_revision: "synthetic-selected-bound-v1".into(),
        missing_cached_tokens_is_zero: false,
    })
    .collect()
}

fn gateway(f: &Fixture, base: &str, contexts: Vec<ExternalModelContext>) -> ModelGateway {
    let mut config: RoutingConfig = serde_json::from_value(json!({
        "version":1,
        "groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":EXTERNAL_BASE,"protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[
            {"id":1,"model":"small-model","group":"allowed","enabled":true,"speeds":["standard"],
             "targets":[{"id":1,"upstream_id":1,"upstream_model":"small-provider","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
             "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}},
            {"id":2,"model":"large-model","group":"allowed","enabled":true,"speeds":["standard"],
             "targets":[{"id":2,"upstream_id":1,"upstream_model":"large-provider","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
             "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}
        ],
        "aliases":[]
    }))
    .unwrap();
    config.upstreams[0].endpoint = if base.starts_with("https:") {
        base.to_owned().try_into().unwrap()
    } else {
        Endpoint::local_loopback(base.to_owned()).unwrap()
    };
    let prices = PriceSnapshot::compile(PriceConfig {
        version: 1,
        rules: ["small-model", "large-model"]
            .into_iter()
            .enumerate()
            .map(|(index, model)| PriceRule {
                billing_mode: BillingMode::Tokens,
                tiered: None,
                id: index as u64 + 1,
                model: model.into(),
                group: "allowed".into(),
                speed: Speed::Standard,
                // Exactly one Credit per input token, cached or uncached.
                rates: TokenRates {
                    input: 2_000_000,
                    output: 0,
                    cache_read: 2_000_000,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: MULTIPLIER_ONE,
                speed_multiplier_ppm: MULTIPLIER_ONE,
            })
            .collect(),
    })
    .unwrap();
    let policy = ProviderPolicy::new(contexts, GLOBAL_INPUT).unwrap();
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(routing::Router::new(config, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-selected-bound-provider-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        Limits {
            max_output_tokens: GLOBAL_OUTPUT,
            total_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: GLOBAL_INPUT,
            lease_seconds: 30,
        },
        policy,
    )
    .unwrap()
}

/// Reuse real authentication and authorization, then stage the same private
/// host operation that HTTP admission normally supplies to select/reserve.
async fn admit(
    g: &ModelGateway,
    f: &Fixture,
    id: &str,
    model: &str,
    output: u32,
) -> (RequestContext, relay::Request) {
    let body = serde_json::to_vec(&json!({
        "model":model,"messages":[{"role":"user","content":"hello"}],
        "max_tokens":output
    }))
    .unwrap();
    let request = relay::Request::parse(ClientProtocol::Chat, &body, &g.limits).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", f.key)).unwrap(),
    );
    let (principal, secret, groups) = g.authenticate(&headers).await.unwrap();
    assert_eq!(groups, ["allowed"]);
    g.authorize_request(&principal, &secret, model, "allowed", None)
        .await
        .unwrap();
    let context = RequestContext {
        request_id: id.into(),
        actor_user_id: principal.user_id(),
        key_id: principal.key_id(),
        oauth_source: principal.oauth_source().cloned(),
    };
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    assert!(
        g.bridge
            .active
            .lock()
            .unwrap()
            .insert(
                id.into(),
                Operation {
                    secret,
                    group: "allowed".into(),
                    worker,
                    fingerprint: Sha256::digest(body).into(),
                    selected: None,
                    output_token_ceiling: output,
                    peer: None,
                    oauth_issuer: None,
                },
            )
            .is_none()
    );
    (context, request)
}

fn selected_bound(g: &ModelGateway, id: &str) -> i64 {
    g.bridge.active.lock().unwrap()[id]
        .selected
        .as_ref()
        .unwrap()
        .input_token_ceiling
}

async fn receipt(g: &ModelGateway, id: &str) -> Charge {
    g.bridge.charging.resolve(id).await.unwrap().unwrap()
}

async fn requests(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn checkpoint_rows(pool: &PgPool, id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn assert_balance(f: &Fixture, available: i64, reserved: i64) {
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (available, reserved)
    );
    let unbalanced: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) entries")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(unbalanced, 0);
}

fn usage(input: u64) -> relay::Usage {
    relay::Usage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        cached_input_tokens: Some(0),
        ..relay::Usage::default()
    }
}

fn terminal(id: &str, route: &Route, usage: relay::Usage) -> relay::Report {
    relay::Report {
        request_id: id.into(),
        route_id: Some(route.id.clone()),
        price_version: Some(route.price_version),
        outcome: relay::Outcome::Completed,
        usage,
        finish_reason: Some(FinishReason::Stop),
        provider_complete: true,
        upstream_attempted: true,
        attempts: 1,
        upstream_status: Some(200),
        upstream_bytes: 128,
        emitted_bytes: 0,
    }
}

#[sqlx::test(migrations = false)]
async fn selected_bound_mixed_external_models_reserve_their_own_caps(pool: PgPool) {
    let f = fixture(pool, 2_000_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (small_context, small_request) =
        admit(&g, &f, "selected-small", "small-model", SMALL_OUTPUT).await;
    let small_route = g
        .bridge
        .select(&small_context, &small_request)
        .await
        .unwrap();
    assert_eq!(selected_bound(&g, "selected-small"), SMALL_INPUT);
    let small = g
        .bridge
        .reserve(&small_context, &small_route, &small_request)
        .await
        .unwrap();
    assert_eq!(receipt(&g, &small.id).await.reserved, SMALL_INPUT);
    assert_balance(&f, 2_000_000 - SMALL_INPUT, SMALL_INPUT).await;

    let (large_context, large_request) =
        admit(&g, &f, "selected-large", "large-model", GLOBAL_OUTPUT).await;
    let large_route = g
        .bridge
        .select(&large_context, &large_request)
        .await
        .unwrap();
    assert_eq!(selected_bound(&g, "selected-large"), GLOBAL_INPUT);
    let large = g
        .bridge
        .reserve(&large_context, &large_route, &large_request)
        .await
        .unwrap();
    assert_eq!(receipt(&g, &large.id).await.reserved, GLOBAL_INPUT);
    assert_balance(
        &f,
        2_000_000 - SMALL_INPUT - GLOBAL_INPUT,
        SMALL_INPUT + GLOBAL_INPUT,
    )
    .await;

    g.bridge
        .finalize(&small, &terminal(&small.id, &small_route, usage(100)))
        .await
        .unwrap();
    g.bridge
        .finalize(&large, &terminal(&large.id, &large_route, usage(1000)))
        .await
        .unwrap();
    assert_eq!(receipt(&g, &small.id).await.settled, 100);
    assert_eq!(receipt(&g, &large.id).await.settled, 1000);
    assert_balance(&f, 2_000_000 - 1100, 0).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_small_external_model_is_fundable_without_global_hold(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(&g, &f, "fundable-small", "small-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    assert_eq!(receipt(&g, &reservation.id).await.reserved, SMALL_INPUT);
    assert_balance(&f, 70_000 - SMALL_INPUT, SMALL_INPUT).await;

    let (context, request) = admit(&g, &f, "unfunded-large", "large-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    assert!(matches!(
        g.bridge.reserve(&context, &route, &request).await,
        Err(RelayError::NoFunds)
    ));
    assert!(
        g.bridge
            .charging
            .resolve("unfunded-large")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(requests(&f.pool).await, 1);
    assert_balance(&f, 70_000 - SMALL_INPUT, SMALL_INPUT).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_output_over_model_cap_is_rejected_before_any_hold(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(
        &g,
        &f,
        "excess-small-output",
        "small-model",
        SMALL_OUTPUT + 1,
    )
    .await;
    assert!(matches!(
        g.bridge.select(&context, &request).await,
        Err(RelayError::RouteUnavailable)
    ));
    assert_eq!(requests(&f.pool).await, 0);
    assert_balance(&f, 70_000, 0).await;
    // A larger request remains selectable for the other model in this gateway.
    let (context, request) =
        admit(&g, &f, "valid-large-output", "large-model", GLOBAL_OUTPUT).await;
    assert!(g.bridge.select(&context, &request).await.is_ok());
    assert_eq!(requests(&f.pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn selected_bound_external_contract_must_match_every_binding_field(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    for index in 0..5 {
        let mut contracts = contexts(SMALL_INPUT);
        match index {
            0 => contracts.clear(),
            1 => contracts[0].upstream_id = 2,
            2 => contracts[0].upstream_model = "different-provider-model".into(),
            3 => contracts[0].protocol = Protocol::OpenAiResponses,
            4 => contracts[0].endpoint = "https://different-bound.example/v1".into(),
            _ => unreachable!(),
        }
        let g = gateway(&f, EXTERNAL_BASE, contracts);
        let (context, request) = admit(
            &g,
            &f,
            &format!("invalid-binding-{index}"),
            "small-model",
            1,
        )
        .await;
        assert!(matches!(
            g.bridge.select(&context, &request).await,
            Err(RelayError::Unsupported(_) | RelayError::RouteUnavailable)
        ));
        assert_eq!(requests(&f.pool).await, 0);
        assert_balance(&f, 70_000, 0).await;
    }
}

#[sqlx::test(migrations = false)]
async fn selected_bound_bad_terminal_and_unknown_usage_keep_original_hold(pool: PgPool) {
    let f = fixture(pool, 300_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let invalid = [
        usage(SMALL_INPUT as u64 + 1),
        relay::Usage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            ..relay::Usage::default()
        },
        relay::Usage::default(),
    ];
    for (index, observed) in invalid.into_iter().enumerate() {
        let id = format!("unknown-selected-{index}");
        let (context, request) = admit(&g, &f, &id, "small-model", 1).await;
        let route = g.bridge.select(&context, &request).await.unwrap();
        let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
        let original = receipt(&g, &id).await;
        assert_eq!((original.reserved, original.settled), (SMALL_INPUT, 0));
        assert!(matches!(
            g.bridge
                .persist_output(
                    &reservation,
                    &OutputCheckpoint {
                        sequence: 1,
                        packet_sha256: [3; 32],
                        packet_bytes: 128,
                        usage: observed,
                        terminal_observed: true,
                    },
                )
                .await,
            Err(RelayError::SettlementPending)
        ));
        assert_eq!(checkpoint_rows(&f.pool, &id).await, 0);
        assert!(matches!(
            g.bridge
                .finalize(&reservation, &terminal(&id, &route, observed))
                .await,
            Err(RelayError::SettlementPending)
        ));
        let after = receipt(&g, &id).await;
        assert_eq!(
            (after.state.as_str(), after.reserved, after.settled),
            ("reconcile", SMALL_INPUT, 0)
        );
        assert_eq!(after.price, original.price);
        assert!(matches!(
            g.bridge.reserve(&context, &route, &request).await,
            Err(RelayError::Conflict)
        ));
        assert_eq!(requests(&f.pool).await, index as i64 + 1);
        assert_balance(
            &f,
            300_000 - SMALL_INPUT * (index as i64 + 1),
            SMALL_INPUT * (index as i64 + 1),
        )
        .await;
    }
    assert_eq!(
        g.reconcile_completed_streams_page(10, None).await.unwrap(),
        (0, None)
    );
    assert_balance(&f, 300_000 - 3 * SMALL_INPUT, 3 * SMALL_INPUT).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_loopback_keeps_global_synthetic_reservation(pool: PgPool) {
    let f = fixture(pool, 2_000_000).await;
    let g = gateway(&f, "http://127.0.0.1:1/v1", vec![]);
    let (context, request) = admit(&g, &f, "synthetic-global", "small-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    assert_eq!(selected_bound(&g, &context.request_id), GLOBAL_INPUT);
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    assert_eq!(receipt(&g, &reservation.id).await.reserved, GLOBAL_INPUT);
    assert_balance(&f, 2_000_000 - GLOBAL_INPUT, GLOBAL_INPUT).await;
}

#[sqlx::test(migrations = false)]
async fn selected_bound_recovery_uses_old_terminal_price_after_contract_change(pool: PgPool) {
    let f = fixture(pool, 70_000).await;
    let g = gateway(&f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let (context, request) = admit(&g, &f, "frozen-selected", "small-model", 1).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
    let original = receipt(&g, &reservation.id).await;
    g.bridge
        .persist_output(
            &reservation,
            &OutputCheckpoint {
                sequence: 1,
                packet_sha256: [4; 32],
                packet_bytes: 128,
                usage: usage(60_000),
                terminal_observed: true,
            },
        )
        .await
        .unwrap();
    let worker = g.bridge.active.lock().unwrap()[&reservation.id].worker;
    g.bridge
        .charging
        .mark_unknown(&reservation.id, &worker)
        .await
        .unwrap();
    let restarted = gateway(&f, EXTERNAL_BASE, contexts(32_768));
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(10, None)
            .await
            .unwrap(),
        (1, None)
    );
    let settled = receipt(&restarted, &reservation.id).await;
    assert_eq!(
        (settled.state.as_str(), settled.reserved, settled.settled),
        ("settled", SMALL_INPUT, 60_000)
    );
    assert_eq!(settled.price, original.price);
    assert_eq!(requests(&f.pool).await, 1);
    assert_balance(&f, 10_000, 0).await;
}
