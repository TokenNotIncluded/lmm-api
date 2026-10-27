//! R1 funds regressions against real identity, PostgreSQL and TransactionalLedger.
//! Provider evidence is synthetic. No test here contacts an official model API.
use super::*;
use crate::billing::{charging::contract::*, transactional_ledger::TransactionalLedger};
use sqlx::postgres::{PgPoolOptions, PgSslMode};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const WORKER: [u8; 32] = [73; 32];

pub(super) async fn liability_snapshot(pool: &PgPool, id: &str) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(\
         'request',(SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1),\
         'holds',(SELECT coalesce(jsonb_agg(to_jsonb(h) ORDER BY position),'[]') FROM core_charging.ledger_holds h WHERE request_id=$1),\
         'events',(SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY event_id),'[]') FROM core_charging.usage_events e WHERE request_id=$1),\
         'commands',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY operation_id),'[]') FROM core_charging.ledger_commands c WHERE request_id=$1),\
         'checks',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY request_revision,budget_id),'[]') FROM core_charging.budget_checks b WHERE request_id=$1))",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn preparation(
    id: &str,
    input: u64,
    output: u64,
    price: charging::Price,
    wire: &[u8],
) -> (
    charging::Request,
    charging::ReservationBasis,
    PreparedReservation,
) {
    let basis = charging::ReservationBasis::Tokens { input, output };
    let request = charging::Request {
        id: id.into(),
        fingerprint: Sha256::digest(id.as_bytes()).into(),
        reserve: price.reservation_ceiling(basis).unwrap(),
        price,
        lease_seconds: 30,
        maximum_seconds: 30,
    };
    let proof = PreparedReservation::context_max(
        hex_digest(&request.fingerprint),
        ReservationBinding {
            route_id: "synthetic-prepared-route".into(),
            target_id: 1,
            upstream_id: 1,
            protocol: "responses".into(),
            model_snapshot: "synthetic-model-2026-10-01".into(),
            generation_url: "https://synthetic.invalid/v1/responses".into(),
            credential_ref: CredentialRef { id: 7, revision: 1 },
            payload_policy_sha256: digest(b"adapt"),
            provider_contract_revision: "synthetic-financial-fixture-v1".into(),
            projection_revision: "context-max-v1".into(),
            quote_version: 1,
        },
        digest(wire),
        ReservationBounds {
            model_input_max: input,
            input,
            requested_output: output,
            output,
        },
        ReservationSemantics {
            input: "inclusive".into(),
            output: "includes_reasoning".into(),
            missing_cached_tokens_is_zero: false,
            missing_cache_creation_tokens_is_zero: false,
        },
    )
    .unwrap();
    (request, basis, proof)
}

fn price() -> charging::Price {
    serde_json::from_value(json!({
        "version":"synthetic-price-1","model":"small-model","group":"allowed",
        "input_per_million":1000000,"cached_per_million":1000000,
        "output_per_million":1000000
    }))
    .unwrap()
}

fn charging(f: &Fixture) -> charging::Charging {
    charging::Charging::new(f.pool.clone(), Arc::new(TransactionalLedger))
}

async fn reserve(
    charging: &charging::Charging,
    key: &str,
    spec: &charging::Request,
    basis: charging::ReservationBasis,
    proof: &PreparedReservation,
) -> charging::Result<Charge> {
    super::super::billing::retry(|| {
        charging.reserve_prepared_model_with_context(key, spec, &WORKER, None, None, basis, proof)
    })
    .await
}

fn budget(
    scope: &str,
    account_id: Option<i64>,
    user_id: Option<i64>,
    key_id: Option<i64>,
    limit: i64,
) -> charging::Budget {
    charging::Budget {
        scope: scope.into(),
        account_id,
        user_id,
        key_id,
        period: "day".into(),
        anchor: 0,
        seconds: 0,
        limit,
    }
}

fn exact_gateway(f: &Fixture) -> ModelGateway {
    let original = gateway_with_output_rate(f, EXTERNAL_BASE, contexts(GLOBAL_INPUT), 2_000_000);
    let mut document = original.bridge.routing.catalog_document().unwrap();
    document.routing.upstreams[0].protocol = Protocol::OpenAiResponses;
    document.routing.upstreams[0].endpoint =
        "https://api.openai.com/v1".to_owned().try_into().unwrap();
    document.routing.routes.truncate(1);
    document.pricing.rules.truncate(1);
    document.routing.routes[0].targets[0].upstream_model = "synthetic-model-2026-10-01".into();
    let context = ExternalModelContext {
        upstream_id: 1,
        upstream_model: "synthetic-model-2026-10-01".into(),
        protocol: Protocol::OpenAiResponses,
        endpoint: "https://api.openai.com/v1".into(),
        max_input_tokens: GLOBAL_INPUT as u32,
        max_output_tokens: GLOBAL_OUTPUT,
        contract_revision: "synthetic-financial-profile-not-provider-acceptance".into(),
        missing_cached_tokens_is_zero: false,
        missing_cache_creation_tokens_is_zero: false,
        reservation: super::super::provider_policy::ReservationProfile::OpenaiResponsesExactV1 {
            count_timeout_ms: std::num::NonZeroU64::new(2000).unwrap(),
        },
    };
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(
            routing::Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-not-a-provider-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        original.limits.clone(),
        original.bridge.policy,
        ProviderPolicy::new(vec![context], GLOBAL_INPUT).unwrap(),
    )
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_exact_count_bridge_rejects_missing_or_changed_evidence(pool: PgPool) {
    let f = fixture(pool, 96).await;
    let g = exact_gateway(&f);
    let (context, request) = admit(&g, &f, "prepared-exact-count", "small-model", 8).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let body = request.upstream_body(&route).unwrap();
    // This internal observation is synthetic; it exercises the A+B boundary,
    // while input_count_tests separately exercise count HTTP/projection parsing.
    let mut observation = relay::input_count::CountObservation {
        api: "responses.input_tokens",
        input_tokens: 24,
        requested_output: 8,
        wire_sha256: Sha256::digest(&body).into(),
        request_sha256: [11; 32],
        response_sha256: [12; 32],
        projection_revision: relay::input_count::PROJECTION_REVISION,
    };
    assert!(
        g.bridge
            .reserve_prepared(&context, &route, &request, &body, None)
            .await
            .is_err()
    );
    let mut wrong_route = route.clone();
    wrong_route.model = "another-model-2026-10-01".into();
    assert!(
        g.bridge
            .reserve_prepared(&context, &wrong_route, &request, &body, Some(&observation))
            .await
            .is_err()
    );
    let mut changed = body.clone();
    changed.push(b' ');
    assert!(
        g.bridge
            .reserve_prepared(&context, &route, &request, &changed, Some(&observation))
            .await
            .is_err()
    );
    assert_eq!(requests(&f.pool).await, 0);
    assert_balance(&f, 96, 0).await;
    let reservation = g
        .bridge
        .reserve_prepared(&context, &route, &request, &body, Some(&observation))
        .await
        .unwrap();
    let charge = receipt(&g, &reservation.id).await;
    assert_eq!(
        charge.reserved, 32,
        "reviewed context ceiling must not replace the request count"
    );
    let detail:serde_json::Value=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'").bind(&reservation.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(detail["mode"], "openai_responses_exact_v1");
    assert_eq!(detail["bounds"]["input"], 24);
    assert_eq!(detail["bounds"]["model_input_max"], GLOBAL_INPUT);
    let before = liability_snapshot(&f.pool, &reservation.id).await;
    observation.response_sha256 = [13; 32];
    assert!(
        g.bridge
            .reserve_prepared(&context, &route, &request, &body, Some(&observation))
            .await
            .is_err()
    );
    assert_eq!(liability_snapshot(&f.pool, &reservation.id).await, before);
    // One extra input token is outside the accepted count even though the
    // model's full context and the remaining wallet would permit it.
    assert!(
        g.bridge
            .finalize(&reservation, &terminal(&reservation.id, &route, usage(25)))
            .await
            .is_err()
    );
    let after = receipt(&g, &reservation.id).await;
    assert_eq!((after.state.as_str(), after.settled), ("reconcile", 0));
    let raw:serde_json::Value=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'").bind(&reservation.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(raw["input_token_ceiling"], 24);
    assert_balance(&f, 64, 32).await;
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_same_id_replay_binds_proof_with_and_without_trust(pool: PgPool) {
    let f = fixture(pool, 1000).await;
    for trusted in [false, true] {
        let charging = charging(&f);
        if trusted {
            charging.enable_trust_pricing();
        }
        let id = format!("prepared-replay-{trusted}");
        let (spec, basis, proof) = preparation(&id, 24, 8, price(), b"original-wire");
        let first = reserve(&charging, &f.key, &spec, basis, &proof)
            .await
            .unwrap();
        assert_eq!(first.reserved, 32);
        assert_eq!(first.price.user_trust.is_some(), trusted);
        let before = liability_snapshot(&f.pool, &id).await;
        let duplicate = reserve(&charging, &f.key, &spec, basis, &proof)
            .await
            .unwrap();
        assert_eq!(duplicate, first);
        assert_eq!(liability_snapshot(&f.pool, &id).await, before);
        let (_, _, changed_proof) = preparation(&id, 24, 8, price(), b"changed-wire");
        assert_eq!(
            reserve(&charging, &f.key, &spec, basis, &changed_proof).await,
            Err(charging::Error::Conflict)
        );
        assert_eq!(liability_snapshot(&f.pool, &id).await, before);
    }
    assert_eq!(requests(&f.pool).await, 2);
    assert_balance(&f, 936, 64).await;
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_team_member_and_key_caps_share_real_pending_spend(pool: PgPool) {
    use crate::{accounts::TeamRole, identity::InviteRequest};
    let f = fixture(pool, 1000).await;
    let identity = IdentityStore::from_pool(f.pool.clone());
    let member = identity.bootstrap_user(8, 1).await.unwrap();
    let team = identity.create_team(&f.session).await.unwrap();
    let invite = identity
        .invite(
            &f.session,
            team,
            &InviteRequest {
                recipient_user_id: 8,
                role: TeamRole::Member,
                can_spend: true,
                ttl_seconds: 3600,
            },
        )
        .await
        .unwrap();
    identity
        .accept_invite(&member.secret, &invite.secret)
        .await
        .unwrap();
    let team_account = identity.list_teams(&f.session, 0).await.unwrap()[0].account_id;
    f.ledger.open_wallet(team_account).await.unwrap();
    f.ledger
        .execute(&LedgerRequest {
            scope: "prepared-team-test".into(),
            operation_key: "team-credit".into(),
            actor_user_id: Some(7),
            reason: "Synthetic team funds".into(),
            action: Action::Credit {
                account_id: team_account,
                amount_units: Credits::try_from(1000).unwrap(),
                payment_reference: "synthetic:team".into(),
            },
        })
        .await
        .unwrap();
    let mut keys = Vec::new();
    for session in [&member.secret, &member.secret, &f.session] {
        keys.push(
            identity
                .issue_key(
                    session,
                    Account {
                        kind: AccountKind::Team,
                        id: team,
                    },
                    FundingPolicy::inherit_account_settings(),
                    3600,
                )
                .await
                .unwrap(),
        );
    }
    let charging = charging(&f);
    for b in [
        budget("account", Some(team_account), None, None, 96),
        budget("member", Some(team_account), Some(8), None, 64),
        budget("key", None, None, Some(keys[0].id), 32),
    ] {
        charging.set_budget(&f.session, &b).await.unwrap();
    }
    // The same member has two keys; a key cap must not become another budget.
    for (id, key_index, expected) in [
        ("prepared-team-key-1", 0, true),
        ("prepared-team-key-over", 0, false),
        ("prepared-team-key-2", 1, true),
        ("prepared-team-member-over", 1, false),
    ] {
        let (spec, basis, proof) = preparation(id, 24, 8, price(), b"team-wire");
        let result = reserve(&charging, &keys[key_index].secret, &spec, basis, &proof).await;
        if expected {
            let charge = result.unwrap();
            assert_eq!(charge.payer_account_id, team_account);
            assert_eq!(charge.reserved, 32);
        } else {
            assert_eq!(result, Err(charging::Error::BudgetExceeded));
            assert!(charging.resolve(id).await.unwrap().is_none());
        }
    }
    // Both contenders see enough wallet funds; only one fits the shared team
    // budget after the member's two holds. This exercises the SSI predicate.
    let mut contenders = tokio::task::JoinSet::new();
    for index in 0..2 {
        let charging = charging.clone();
        let key = keys[2].secret.clone();
        contenders.spawn(async move {
            let (spec, basis, proof) = preparation(
                &format!("prepared-team-owner-{index}"),
                24,
                8,
                price(),
                b"owner-wire",
            );
            reserve(&charging, &key, &spec, basis, &proof).await
        });
    }
    let mut admitted = 0;
    while let Some(result) = contenders.join_next().await {
        match result.unwrap() {
            Ok(charge) => {
                assert_eq!(charge.payer_account_id, team_account);
                admitted += 1;
            }
            Err(error) => assert_eq!(error, charging::Error::BudgetExceeded),
        }
    }
    assert_eq!(admitted, 1);
    let team_balance = f.ledger.balance(team_account).await.unwrap();
    assert_eq!(
        (team_balance.available_units, team_balance.reserved_units),
        (904, 96)
    );
    assert_balance(&f, 1000, 0).await;
    assert_eq!(requests(&f.pool).await, 3);
    // Preparation/authentication can complete before a network count wait. The
    // financial transaction must read the authority and caps again afterward.
    let (spec, basis, proof) = preparation("prepared-team-revoked", 24, 8, price(), b"wait-wire");
    identity.authorize(&keys[1].secret).await.unwrap();
    identity
        .remove_member(&f.session, team, 8, 1)
        .await
        .unwrap();
    let before = liability_snapshot(&f.pool, "prepared-team-key-2").await;
    assert!(matches!(
        reserve(&charging, &keys[1].secret, &spec, basis, &proof).await,
        Err(charging::Error::Unauthorized | charging::Error::Forbidden)
    ));
    assert!(charging.resolve(&spec.id).await.unwrap().is_none());
    assert_eq!(
        liability_snapshot(&f.pool, "prepared-team-key-2").await,
        before
    );
    assert_eq!(f.ledger.balance(team_account).await.unwrap(), team_balance);
    assert_balance(&f, 1000, 0).await;
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_key_revocation_after_route_preparation_never_reserves(pool: PgPool) {
    let f = fixture(pool, 1000).await;
    let g = gateway_with_output_rate(&f, EXTERNAL_BASE, contexts(24), 2_000_000);
    let (context, request) = admit(&g, &f, "prepared-key-revoked", "small-model", 8).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let body = request.upstream_body(&route).unwrap();
    IdentityStore::from_pool(f.pool.clone())
        .revoke(&f.session, f.key_id)
        .await
        .unwrap();
    assert!(matches!(
        g.bridge
            .reserve_prepared(&context, &route, &request, &body, None)
            .await,
        Err(RelayError::Unauthorized | RelayError::Forbidden)
    ));
    assert_eq!(requests(&f.pool).await, 0);
    assert_balance(&f, 1000, 0).await;
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_tier_cache_maximum_and_reasoning_use_frozen_terms(pool: PgPool) {
    let f = fixture(pool, 2000).await;
    let charging = charging(&f);
    let tier_price: charging::Price = serde_json::from_value(json!({
        "version":"frozen-nonmonotone-tariff","model":"small-model","group":"allowed",
        "input_per_million":0,"cached_per_million":0,"output_per_million":0,
        "tiered":{
            "input_semantics":"inclusive","group_multiplier_ppm":1000000,
            "speed_multiplier_ppm":1000000,"credits_per_usd":500000,
            "tariff":{"revision":1,"source_digest":"45".repeat(32),"uses_cache_read":true,"uses_cache_write":false,
                "tiers":[
                    {"up_to_input_tokens":100,"rates":{
                        "input":{"numerator":10,"denominator":1},
                        "output":{"numerator":3,"denominator":1},
                        "cache_read":{"numerator":20,"denominator":1},
                        "cache_write":{"numerator":0,"denominator":1}}},
                    {"up_to_input_tokens":null,"rates":{
                        "input":{"numerator":1,"denominator":1},
                        "output":{"numerator":1,"denominator":1},
                        "cache_read":{"numerator":1,"denominator":10},
                        "cache_write":{"numerator":0,"denominator":1}}}
                ]}
        }
    }))
    .unwrap();
    let (spec, basis, proof) =
        preparation("prepared-cache-tier", 150, 10, tier_price, b"tariff-wire");
    // Earlier tiers and the more expensive cache class both matter: 100*10+10*1.5.
    assert_eq!(spec.reserve, 1015);
    let original = reserve(&charging, &f.key, &spec, basis, &proof)
        .await
        .unwrap();
    assert!(charging.start(&spec.id, &WORKER).await.unwrap());
    let (frozen, contract) = charging.reservation_contract(&spec.id).await.unwrap();
    let observed = relay::Usage {
        input_tokens: Some(100),
        cached_input_tokens: Some(100),
        output_tokens: Some(10),
        reasoning_output_tokens: Some(7),
        ..Default::default()
    };
    let final_usage =
        super::super::billing::frozen_priced_usage(&contract, &frozen, observed).unwrap();
    assert_eq!(
        final_usage.output, 10,
        "reasoning is a subset, not seven extra tokens"
    );
    assert_eq!(frozen.price.cost(final_usage).unwrap(), 1015);
    let (changed, _, changed_proof) = preparation(&spec.id, 150, 10, price(), b"tariff-wire");
    assert_eq!(
        reserve(&charging, &f.key, &changed, basis, &changed_proof).await,
        Err(charging::Error::Conflict)
    );
    assert_eq!(
        charging.resolve(&spec.id).await.unwrap().unwrap().price,
        original.price
    );
    charging
        .settle(&spec.id, &WORKER, final_usage, charging::Outcome::Completed)
        .await
        .unwrap();
    assert_balance(&f, 985, 0).await;
}

// Same real PostgreSQL-frame fault as tests/assembled_billing.rs: suppress only
// an observed server COMMIT+ReadyForQuery, never manufacture CommitUnknown.
async fn lost_ack_pool(pool: &PgPool) -> (PgPool, tokio::task::JoinHandle<()>) {
    let options = pool.connect_options();
    let upstream = (options.get_host().to_owned(), options.get_port());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let server = TcpStream::connect(upstream).await.unwrap();
        let (mut client_read, mut client_write) = client.into_split();
        let (mut server_read, mut server_write) = server.into_split();
        let upload =
            tokio::spawn(async move { tokio::io::copy(&mut client_read, &mut server_write).await });
        let mut suppress_commit = false;
        loop {
            let mut header = [0_u8; 5];
            server_read.read_exact(&mut header).await.unwrap();
            let length = u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize;
            assert!((4..=1_048_576).contains(&length));
            let mut payload = vec![0; length - 4];
            server_read.read_exact(&mut payload).await.unwrap();
            if header[0] == b'C' && payload == b"COMMIT\0" {
                suppress_commit = true;
            }
            if suppress_commit {
                if header[0] == b'Z' {
                    upload.abort();
                    let _ = upload.await;
                    break;
                }
            } else {
                client_write.write_all(&header).await.unwrap();
                client_write.write_all(&payload).await.unwrap();
            }
        }
    });
    let fault = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(
            options
                .as_ref()
                .clone()
                .host("127.0.0.1")
                .port(port)
                .ssl_mode(PgSslMode::Disable),
        )
        .await
        .unwrap();
    (fault, proxy)
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_real_lost_ack_reserve_start_capture_preserve_one_operation(
    pool: PgPool,
) {
    let f = fixture(pool, 1000).await;
    let charging = charging(&f);
    let (spec, basis, proof) = preparation("prepared-three-acks", 24, 8, price(), b"one-wire");
    let actual = charging::Usage {
        input: 10,
        cached: 0,
        output: 2,
        ..Default::default()
    };
    for step in 0..3 {
        let (fault_pool, proxy) = lost_ack_pool(&f.pool).await;
        let fault = charging::Charging::new(fault_pool.clone(), Arc::new(TransactionalLedger));
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            match step {
                0 => fault
                    .reserve_prepared_model_with_context(
                        &f.key, &spec, &WORKER, None, None, basis, &proof,
                    )
                    .await
                    .map(|_| ()),
                1 => fault.start(&spec.id, &WORKER).await.map(|_| ()),
                _ => fault
                    .settle(&spec.id, &WORKER, actual, charging::Outcome::Completed)
                    .await
                    .map(|_| ()),
            }
        })
        .await
        .unwrap();
        assert_eq!(result, Err(charging::Error::CommitUnknown));
        tokio::time::timeout(Duration::from_secs(10), proxy)
            .await
            .unwrap()
            .unwrap();
        fault_pool.close().await;
        let durable = charging.resolve(&spec.id).await.unwrap().unwrap();
        let before = liability_snapshot(&f.pool, &spec.id).await;
        match step {
            0 => {
                assert_eq!(durable.state, "reserved");
                let resolved = charging
                    .resolve_prepared_model(&spec, &WORKER, basis, &proof)
                    .await
                    .unwrap();
                assert_eq!(resolved, durable);
                assert_eq!(
                    reserve(&charging, &f.key, &spec, basis, &proof)
                        .await
                        .unwrap(),
                    durable
                );
                assert_balance(&f, 968, 32).await;
            }
            1 => {
                assert_eq!(durable.state, "streaming");
                assert!(
                    !charging.start(&spec.id, &WORKER).await.unwrap(),
                    "lost start ACK cannot authorize another dispatch"
                );
                assert_balance(&f, 968, 32).await;
            }
            _ => {
                assert_eq!((durable.state.as_str(), durable.settled), ("settled", 12));
                assert_eq!(
                    charging
                        .settle(&spec.id, &WORKER, actual, charging::Outcome::Completed)
                        .await
                        .unwrap(),
                    durable
                );
                assert_balance(&f, 988, 0).await;
            }
        }
        assert_eq!(liability_snapshot(&f.pool, &spec.id).await, before);
    }
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_charging.requests),(SELECT count(*) FROM core_charging.ledger_holds),(SELECT count(*) FROM core_charging.usage_events WHERE event_id='start'),(SELECT count(*) FROM core_charging.ledger_commands)").fetch_one(&f.pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1, 2));
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_restart_keeps_missing_corrupt_and_audit_only_holds(pool: PgPool) {
    let f = fixture(pool, 1000).await;
    let g = gateway_with_output_rate(&f, EXTERNAL_BASE, contexts(24), 2_000_000);
    let mut cases = Vec::new();
    for kind in ["valid", "missing", "corrupt", "audit-only"] {
        let id = format!("prepared-recovery-{kind}");
        let (context, request) = admit(&g, &f, &id, "small-model", 8).await;
        let route = g.bridge.select(&context, &request).await.unwrap();
        let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
        let worker = g.bridge.active.lock().unwrap()[&id].worker;
        if kind == "audit-only" {
            g.bridge
                .persist_provider_observation(&id, &worker, &route.id, usage(10), 24, 8)
                .await
                .unwrap();
        } else {
            g.bridge
                .persist_output(
                    &reservation,
                    &OutputCheckpoint {
                        sequence: 1,
                        packet_sha256: [44; 32],
                        packet_bytes: 40,
                        usage: usage(10),
                        terminal_observed: true,
                    },
                )
                .await
                .unwrap();
        }
        g.bridge.charging.mark_unknown(&id, &worker).await.unwrap();
        cases.push((kind, id, worker));
    }
    // Fault injection is confined to this disposable installer-owned database.
    // The serializable guard remains enabled and the immutable guard is restored
    // in the same transaction before any product recovery/checkpoint code runs.
    let mut damage = f.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *damage)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events DISABLE TRIGGER usage_immutable")
        .execute(&mut *damage)
        .await
        .unwrap();
    sqlx::query("DELETE FROM core_charging.usage_events WHERE request_id='prepared-recovery-missing' AND event_id='reservation-contract:v1'").execute(&mut *damage).await.unwrap();
    sqlx::query("UPDATE core_charging.usage_events SET detail=jsonb_set(detail,'{wire_sha256}',to_jsonb(repeat('0',64))) WHERE request_id='prepared-recovery-corrupt' AND event_id='reservation-contract:v1'").execute(&mut *damage).await.unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events ENABLE TRIGGER usage_immutable")
        .execute(&mut *damage)
        .await
        .unwrap();
    damage.commit().await.unwrap();
    let mut frozen = Vec::new();
    for (_, id, _) in &cases {
        frozen.push(liability_snapshot(&f.pool, id).await);
    }
    drop(g);
    let restarted = gateway_with_output_rate(&f, EXTERNAL_BASE, contexts(1), 200_000_000);
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(10, None)
            .await
            .unwrap(),
        (1, None)
    );
    assert_eq!(
        receipt(&restarted, "prepared-recovery-valid").await.settled,
        11
    );
    for ((kind, id, worker), before) in cases.iter().zip(&frozen) {
        if *kind == "valid" {
            continue;
        }
        assert_eq!(liability_snapshot(&f.pool, id).await, *before);
        assert_eq!(receipt(&restarted, id).await.state, "reconcile");
        if matches!(*kind, "missing" | "corrupt") {
            assert!(
                restarted
                    .bridge
                    .charging
                    .reservation_contract(id)
                    .await
                    .is_err()
            );
            let original = receipt(&restarted, id).await;
            assert!(
                restarted
                    .bridge
                    .charging
                    .reconcile(
                        id,
                        original.revision,
                        charging::Usage {
                            input: 10,
                            cached: 0,
                            output: 1,
                            ..Default::default()
                        },
                        charging::Outcome::Completed,
                        "synthetic-malformed-proof"
                    )
                    .await
                    .is_err()
            );
            assert!(
                restarted
                    .bridge
                    .charging
                    .checkpoint(id, worker, "bad-proof", charging::Usage::default())
                    .await
                    .is_err()
            );
        }
    }
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(10, None)
            .await
            .unwrap(),
        (0, None)
    );
    assert_balance(&f, 893, 96).await;
    let no_money_in_audit:bool=sqlx::query_scalar("SELECT bool_and(NOT(detail ? 'usage')) FROM core_charging.usage_events WHERE event_id IN ('reservation-contract:v1','reservation-admission:v1','provider-observation:terminal')").fetch_one(&f.pool).await.unwrap();
    assert!(no_money_in_audit);
}

#[sqlx::test(migrations = false)]
async fn prepared_contract_damaged_live_proof_keeps_raw_terminal_audit_without_capture(
    pool: PgPool,
) {
    let f = fixture(pool, 1000).await;
    let g = gateway_with_output_rate(&f, EXTERNAL_BASE, contexts(24), 2_000_000);
    for missing in [false, true] {
        let id = format!("prepared-live-damage-{missing}");
        let (context, request) = admit(&g, &f, &id, "small-model", 8).await;
        let route = g.bridge.select(&context, &request).await.unwrap();
        let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
        let original = receipt(&g, &id).await;
        let worker = g.bridge.active.lock().unwrap()[&id].worker;
        let mut damage = f.pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *damage)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE core_charging.usage_events DISABLE TRIGGER usage_immutable")
            .execute(&mut *damage)
            .await
            .unwrap();
        if missing {
            sqlx::query("DELETE FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'").bind(&id).execute(&mut *damage).await.unwrap();
        } else {
            sqlx::query("UPDATE core_charging.usage_events SET detail=jsonb_set(detail,'{bounds,input}','1'::jsonb) WHERE request_id=$1 AND event_id='reservation-contract:v1'").bind(&id).execute(&mut *damage).await.unwrap();
        }
        sqlx::query("ALTER TABLE core_charging.usage_events ENABLE TRIGGER usage_immutable")
            .execute(&mut *damage)
            .await
            .unwrap();
        damage.commit().await.unwrap();
        let before = liability_snapshot(&f.pool, &id).await;
        let attempted = charging::Usage {
            input: 10,
            cached: 0,
            output: 1,
            ..Default::default()
        };
        assert!(g.bridge.charging.start(&id, &worker).await.is_err());
        assert!(
            g.bridge
                .charging
                .checkpoint(&id, &worker, "without-proof", attempted)
                .await
                .is_err()
        );
        assert!(
            g.bridge
                .charging
                .settle(&id, &worker, attempted, charging::Outcome::Completed)
                .await
                .is_err()
        );
        assert_eq!(liability_snapshot(&f.pool, &id).await, before);
        assert!(
            g.bridge
                .finalize(&reservation, &terminal(&id, &route, usage(10)))
                .await
                .is_err()
        );
        let after = receipt(&g, &id).await;
        assert_eq!(
            (after.state.as_str(), after.reserved, after.settled),
            ("reconcile", 32, 0)
        );
        assert_eq!(after.price, original.price);
        assert_eq!(after.usage, original.usage);
        let snapshot = liability_snapshot(&f.pool, &id).await;
        assert_eq!(snapshot["holds"], before["holds"]);
        assert_eq!(snapshot["commands"], before["commands"]);
        let raw:serde_json::Value=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'").bind(&id).fetch_one(&f.pool).await.unwrap();
        assert!(raw.get("usage").is_none());
        assert_eq!(raw["input_token_ceiling"], 24);
        let accepted:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND (event_id='final' OR event_id LIKE 'usage:%')").bind(&id).fetch_one(&f.pool).await.unwrap();
        assert_eq!(accepted, 0);
        assert_eq!(checkpoint_rows(&f.pool, &id).await, 0);
    }
    assert_balance(&f, 936, 64).await;
}
