//! A07 recovery tests: verifying upstream-cost cache_creation recovery,
//! token pricing rejection of cache_creation even with reported cost,
//! bounded fair cursor progression preventing queue starvation,
//! and gateway cursor isolation (clone vs rebuild vs concurrent).

use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{Charging, Price, Request as ChargeRequest},
        ledger::{Action, Ledger, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
    model_http::{ChargePolicy, ModelGateway},
    relay::Limits,
    routing::{PriceConfig, PriceSnapshot, Router, RoutingConfig},
};
use serde_json::json;
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

struct Fixture {
    gateway: ModelGateway,
    charging: Charging,
    ledger: Ledger,
    key: String,
    account: i64,
}

fn build_gateway(pool: &PgPool) -> ModelGateway {
    let router = Router::new(
        RoutingConfig {
            version: 1,
            groups: vec![],
            upstreams: vec![],
            routes: vec![],
            aliases: vec![],
        },
        Arc::new(
            PriceSnapshot::compile(PriceConfig {
                version: 1,
                rules: vec![],
            })
            .unwrap(),
        ),
    )
    .unwrap();
    ModelGateway::new(
        pool.clone(),
        Arc::new(router),
        vec![],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 100_000,
            lease_seconds: 300,
        },
    )
    .unwrap()
}

async fn fixture(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    core_schema::check(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let principal = identity.authorize(&session.secret).await.unwrap();
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
            scope: "a07-recovery-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in a disposable SQLx test database".into(),
            action: Action::Credit {
                account_id: principal.owner_account_id,
                amount_units: Credits::try_from(50_000_000_i64).unwrap(),
                payment_reference: "synthetic:a07-initial".into(),
            },
        })
        .await
        .unwrap();
    let gateway = build_gateway(pool);
    Fixture {
        gateway,
        charging: Charging::new(pool.clone(), Arc::new(TransactionalLedger)),
        ledger,
        key: key.secret,
        account: principal.owner_account_id,
    }
}

async fn expire(pool: &PgPool, f: &Fixture) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_charging.requests SET lease_until=0, revision=revision+1 WHERE state='streaming'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    f.gateway.recover_expired(100).await.unwrap();
}

async fn seed_terminal_with_price(
    pool: &PgPool,
    f: &Fixture,
    id: &str,
    price: Price,
    cost: Option<u128>,
    cache_creation: Option<u64>,
) {
    let worker = [42_u8; 32];
    let request = ChargeRequest {
        id: id.to_owned(),
        fingerprint: [17; 32],
        price,
        reserve: 625_000,
        lease_seconds: 300,
        maximum_seconds: 300,
    };
    f.charging.reserve(&f.key, &request, &worker).await.unwrap();
    assert!(f.charging.start(id, &worker).await.unwrap());
    let mut usage_json = json!({
        "input_tokens": 1,
        "cached_input_tokens": 0,
        "output_tokens": 0,
    });
    if let Some(c) = cost {
        usage_json["upstream_cost_atto_usd"] = json!(c);
    }
    if let Some(cc) = cache_creation {
        usage_json["cache_creation_input_tokens"] = json!(cc);
    }
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT core_model.checkpoint_output($1,$2,1,$3,25,$4,true)")
        .bind(id)
        .bind(worker.as_slice())
        .bind([9_u8; 32].as_slice())
        .bind(usage_json)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn seed_terminal(
    pool: &PgPool,
    f: &Fixture,
    id: &str,
    cost: Option<u128>,
    cache_creation: Option<u64>,
) {
    let price = if cost.is_some() {
        Price {
            embedding: None,
            search: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: Some(1_250_000),
            tiered: None,
            version: "synthetic-v1".into(),
            model: "openrouter/fusion".into(),
            group: "allowed".into(),
            input_per_million: 0,
            cached_per_million: 0,
            output_per_million: 0,
        }
    } else {
        Price {
            embedding: None,
            search: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "synthetic-v1".into(),
            model: "token/model".into(),
            group: "allowed".into(),
            input_per_million: 10_000_000,
            cached_per_million: 0,
            output_per_million: 20_000_000,
        }
    };
    seed_terminal_with_price(pool, f, id, price, cost, cache_creation).await;
}

#[sqlx::test(migrations = false)]
async fn test_a07_upstream_cost_with_cache_creation_recovery_settles(pool: PgPool) {
    let f = fixture(&pool).await;
    seed_terminal(
        &pool,
        &f,
        "test-upstream-cache-recov",
        Some(123_456_789_000_000_000),
        Some(1),
    )
    .await;
    expire(&pool, &f).await;

    // Checkpoint recovery should now succeed with unified complete_usage
    let recovered = f.gateway.reconcile_completed_streams(10).await.unwrap();
    assert_eq!(
        recovered, 1,
        "upstream-cost checkpoint with cache_creation=1 must settle"
    );

    let charge = f
        .charging
        .resolve("test-upstream-cache-recov")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.settled, 77_161);
    assert_eq!(charge.reserved, 625_000);

    // Wallet hold must be completely released (reserved=0)
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.reserved_units, 0);

    // Subsequent recovery returns 0 (idempotent, terminal observed)
    assert_eq!(f.gateway.reconcile_completed_streams(10).await.unwrap(), 0);
}

#[sqlx::test(migrations = false)]
async fn test_a07_token_based_with_cache_creation_refuses_recovery(pool: PgPool) {
    let f = fixture(&pool).await;
    seed_terminal(&pool, &f, "test-token-cache-refuse", None, Some(1)).await;
    expire(&pool, &f).await;

    // Recovery must reject unsupported cache creation for token model
    let recovered = f.gateway.reconcile_completed_streams(10).await.unwrap();
    assert_eq!(
        recovered, 0,
        "token-based model with non-zero cache_creation must retain hold"
    );

    let charge = f
        .charging
        .resolve("test-token-cache-refuse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.state, "reconcile");
    assert_eq!(charge.settled, 0);
    assert_eq!(charge.reserved, 625_000);

    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.reserved_units, 625_000);
}

#[sqlx::test(migrations = false)]
async fn test_a07_token_based_with_cache_creation_and_reported_cost_refuses_recovery(pool: PgPool) {
    let f = fixture(&pool).await;
    let token_price = Price {
        embedding: None,
        search: None,
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
        version: "synthetic-v1".into(),
        model: "token/model".into(),
        group: "allowed".into(),
        input_per_million: 10_000_000,
        cached_per_million: 0,
        output_per_million: 20_000_000,
    };
    seed_terminal_with_price(
        &pool,
        &f,
        "test-token-cost-cache-refuse",
        token_price,
        Some(123_456_789_000_000_000),
        Some(1),
    )
    .await;
    expire(&pool, &f).await;

    // Even if external provider reports upstream cost, token-priced model cannot settle cache_creation tokens!
    let recovered = f.gateway.reconcile_completed_streams(10).await.unwrap();
    assert_eq!(
        recovered, 0,
        "token pricing must refuse cache_creation even when provider reported upstream cost"
    );

    let charge = f
        .charging
        .resolve("test-token-cost-cache-refuse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.state, "reconcile");
    assert_eq!(charge.settled, 0);
    assert_eq!(charge.reserved, 625_000);

    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.reserved_units, 625_000);
}

#[sqlx::test(migrations = false)]
async fn test_a07_paged_recovery_advances_past_holds_without_starvation(pool: PgPool) {
    let f = fixture(&pool).await;
    for n in 0..64 {
        let worker = [42_u8; 32];
        let request = ChargeRequest {
            id: format!("need-res-{n:03}"),
            fingerprint: [17; 32],
            price: Price {
                embedding: None,
                search: None,
                user_trust: None,
                image: None,
                upstream_cost_group_ppm: Some(1_250_000),
                tiered: None,
                version: "synthetic-v1".into(),
                model: "openrouter/fusion".into(),
                group: "allowed".into(),
                input_per_million: 0,
                cached_per_million: 0,
                output_per_million: 0,
            },
            reserve: 625_000,
            lease_seconds: 300,
            maximum_seconds: 300,
        };
        f.charging.reserve(&f.key, &request, &worker).await.unwrap();
        assert!(
            f.charging
                .start(&format!("need-res-{n:03}"), &worker)
                .await
                .unwrap()
        );
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT core_model.checkpoint_output($1,$2,1,$3,25,$4,true)")
            .bind(format!("need-res-{n:03}"))
            .bind(worker.as_slice())
            .bind([9_u8; 32].as_slice())
            .bind(json!({"input_tokens":1,"cached_input_tokens":0,"output_tokens":0,"upstream_cost_atto_usd": 2_000_000_000_000_000_000_u128}))
            .execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }

    seed_terminal(
        &pool,
        &f,
        "valid-65th-request",
        Some(123_456_789_000_000_000),
        None,
    )
    .await;
    expire(&pool, &f).await;

    // Automatic reconciliation must advance past the 64 NeedReservation rows
    // across successive calls and reach the 65th row!
    let mut automatic = Vec::new();
    for _ in 0..3 {
        automatic.push(f.gateway.reconcile_completed_streams(64).await.unwrap());
    }
    assert!(
        automatic.iter().sum::<usize>() >= 1,
        "paged recovery must reach the 65th valid receipt without starvation: got {automatic:?}"
    );

    let charge65 = f
        .charging
        .resolve("valid-65th-request")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge65.state, "settled");
    assert_eq!(charge65.settled, 77_161);
}

#[sqlx::test(migrations = false)]
async fn test_a07_cloned_gateway_shares_cursor_and_rebuilt_gateway_resets_cursor(pool: PgPool) {
    let f = fixture(&pool).await;
    for id in ["cursor-iso-1", "cursor-iso-2", "cursor-iso-3"] {
        seed_terminal(&pool, &f, id, Some(123_456_789_000_000_000), None).await;
    }
    expire(&pool, &f).await;

    let g1 = f.gateway.clone();
    let g2 = f.gateway.clone();

    // Call 1 on g1 fetches 1 row and recovers it
    assert_eq!(g1.reconcile_completed_streams(1).await.unwrap(), 1);

    // Call 2 on g2 (clone) shares Bridge.recovery_cursor so it fetches the next row
    assert_eq!(g2.reconcile_completed_streams(1).await.unwrap(), 1);

    // Now rebuild a fresh gateway on the same pool: its recovery_cursor starts at None
    let g_rebuilt = build_gateway(&pool);
    // Scanning from None skips the 2 already-settled rows and recovers the 3rd row
    assert_eq!(g_rebuilt.reconcile_completed_streams(1).await.unwrap(), 1);

    // Subsequent call hits EOF and returns 0
    assert_eq!(g_rebuilt.reconcile_completed_streams(1).await.unwrap(), 0);
}

#[sqlx::test(migrations = false)]
async fn test_a07_concurrent_reconcile_serialized_by_recovery_cursor_mutex(pool: PgPool) {
    let f = fixture(&pool).await;
    for id in ["conc-1", "conc-2", "conc-3"] {
        seed_terminal(&pool, &f, id, Some(123_456_789_000_000_000), None).await;
    }
    expire(&pool, &f).await;

    let g1 = f.gateway.clone();
    let g2 = f.gateway.clone();

    let (r1, r2) = tokio::join!(
        g1.reconcile_completed_streams(2),
        g2.reconcile_completed_streams(2),
    );
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert_eq!(r1.unwrap() + r2.unwrap(), 3);

    for id in ["conc-1", "conc-2", "conc-3"] {
        let charge = f.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(charge.state, "settled");
    }
}
