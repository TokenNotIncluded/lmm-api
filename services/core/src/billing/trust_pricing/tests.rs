use super::types::{Facts, evaluate};
use super::*;
use serde_json::json;
fn ratio(n: u64, d: u64) -> DiscountRatio {
    DiscountRatio::new(n, d).unwrap()
}
fn policy() -> TrustPolicy {
    let thresholds = [0, 500_000, 5_000_000, 25_000_000, 100_000_000];
    let discounts = [
        ratio(1, 1),
        ratio(1, 1),
        ratio(97, 100),
        ratio(94, 100),
        ratio(9, 10),
    ];
    TrustPolicy {
        version: 1,
        paid_activation_enabled: true,
        decay_period_days: 90,
        tiers: std::array::from_fn(|i| TrustTier {
            level: i as u8,
            min_paid_credits: thresholds[i],
            discount: discounts[i],
        }),
        admin_discount: ratio(9, 10),
        superadmin_discount: ratio(9, 10),
    }
}
fn facts() -> Facts {
    Facts {
        user_id: 7,
        role: "user".into(),
        basis_revision: 1,
        created_at: 1_700_000_000,
        activated_at: 0,
        last_api_at: 0,
        override_level: None,
        projection_known: true,
        net_paid_credits: 0,
        positive_orders: 0,
        last_paid_at: 0,
        evidence: json!({"synthetic":true}),
    }
}
fn quote(f: &Facts, at: i64) -> FrozenTrustPrice {
    evaluate(&policy(), 1, f, at).unwrap().evidence().clone()
}
#[test]
fn exact_ratios_reject_invalid_and_normalize() {
    assert_eq!(ratio(90, 100), ratio(9, 10));
    for (n, d) in [(0, 1), (1, 0), (2, 1), (1, 1_000_000_000_000_000_001)] {
        assert!(DiscountRatio::new(n, d).is_err());
    }
    assert!(
        serde_json::from_value::<DiscountRatio>(json!({"numerator":1,"denominator":0})).is_err()
    );
}
#[test]
fn threshold_and_activation_are_net_credits_not_balance_or_usd() {
    let mut f = facts();
    let at = f.created_at;
    for (n, level) in [
        (0, 0),
        (499_999, 0),
        (500_000, 1),
        (4_999_999, 1),
        (5_000_000, 2),
        (25_000_000, 3),
        (100_000_000, 4),
    ] {
        f.net_paid_credits = n;
        f.positive_orders = i64::from(n > 0);
        f.last_paid_at = if n > 0 { at } else { 0 };
        assert_eq!(quote(&f, at).level(), level);
    }
}
#[test]
fn activation_and_override_are_distinct() {
    let mut f = facts();
    let at = f.created_at;
    f.activated_at = at;
    assert_eq!(quote(&f, at).level(), 1);
    f.override_level = Some(0);
    assert_eq!(quote(&f, at).level(), 0);
    f.override_level = None;
    f.projection_known = false;
    assert_eq!(
        evaluate(&policy(), 1, &f, at).unwrap_err(),
        Error::Unavailable
    );
}
#[test]
fn override_and_current_role_do_not_decay_or_require_net_projection() {
    let mut f = facts();
    let at = f.created_at + 400 * 86400;
    f.projection_known = false;
    f.override_level = Some(4);
    assert_eq!(quote(&f, at).level(), 4);
    assert_eq!(quote(&f, at).next_decay_at(), None);
    f.role = "admin".into();
    assert_eq!(quote(&f, at).level(), 5);
    f.role = "superadmin".into();
    assert_eq!(quote(&f, at).level(), 6);
}
#[test]
fn ninety_day_boundaries_and_floor_are_exact() {
    let mut f = facts();
    f.net_paid_credits = 100_000_000;
    f.positive_orders = 1;
    f.last_paid_at = f.created_at;
    for (days, level) in [
        (89, 4),
        (90, 3),
        (179, 3),
        (180, 2),
        (269, 2),
        (270, 1),
        (1000, 1),
    ] {
        assert_eq!(quote(&f, f.created_at + days * 86400).level(), level);
    }
    assert_eq!(
        quote(&f, f.created_at).next_decay_at(),
        Some(f.created_at + 90 * 86400)
    );
}
#[test]
fn paid_refund_and_actual_activity_recompute_anchor() {
    let mut f = facts();
    let now = f.created_at + 100 * 86400;
    f.net_paid_credits = 25_000_000;
    f.positive_orders = 1;
    f.last_paid_at = f.created_at;
    assert_eq!(quote(&f, now).level(), 2);
    f.last_paid_at = now - 86400;
    assert_eq!(quote(&f, now).level(), 3);
    // Fully refund the latest order: resolver recomputes latest positive, not max-ever.
    f.last_paid_at = f.created_at;
    assert_eq!(quote(&f, now).level(), 2);
    f.last_api_at = now;
    assert_eq!(quote(&f, now).level(), 3);
    f.net_paid_credits = 0;
    f.positive_orders = 0;
    f.last_paid_at = 0;
    assert_eq!(quote(&f, now).level(), 0);
}
#[test]
fn disabling_paid_activation_preserves_explicit_console_activation() {
    let mut p = policy();
    p.paid_activation_enabled = false;
    let mut f = facts();
    f.net_paid_credits = 25_000_000;
    f.positive_orders = 1;
    f.last_paid_at = f.created_at;
    assert_eq!(
        evaluate(&p, 1, &f, f.created_at)
            .unwrap()
            .evidence()
            .level(),
        0
    );
    f.activated_at = f.created_at;
    assert_eq!(
        evaluate(&p, 1, &f, f.created_at)
            .unwrap()
            .evidence()
            .level(),
        3
    );
}
#[test]
fn zero_day_policy_does_not_decay_and_invalid_policy_is_rejected() {
    let mut p = policy();
    p.decay_period_days = 0;
    let mut f = facts();
    f.net_paid_credits = 5_000_000;
    f.positive_orders = 1;
    f.last_paid_at = f.created_at;
    assert_eq!(
        evaluate(&p, 1, &f, f.created_at + 1000 * 86400)
            .unwrap()
            .evidence()
            .level(),
        2
    );
    p.tiers[2].min_paid_credits = 500_000;
    assert_eq!(p.validate(), Err(Error::Invalid));
}
#[test]
fn frozen_receipt_is_stable_and_cannot_mint_new_authority() {
    let f = facts();
    let a = quote(&f, f.created_at);
    let encoded = serde_json::to_vec(&a).unwrap();
    let b: FrozenTrustPrice = serde_json::from_slice(&encoded).unwrap();
    b.validate().unwrap();
    assert_eq!(a, b);
    let mut v = serde_json::to_value(&a).unwrap();
    v["discount"] = json!({"numerator":2,"denominator":1});
    assert!(serde_json::from_value::<FrozenTrustPrice>(v).is_err());
    assert_eq!(quote(&f, f.created_at).discount(), ratio(1, 1));
}
#[test]
fn future_timestamps_and_inconsistent_net_evidence_are_not_zero() {
    let mut f = facts();
    f.last_paid_at = f.created_at + 1;
    assert_eq!(
        evaluate(&policy(), 1, &f, f.created_at).unwrap_err(),
        Error::Invalid
    );
    f.last_paid_at = 0;
    f.net_paid_credits = 1;
    assert_eq!(
        evaluate(&policy(), 1, &f, f.created_at).unwrap_err(),
        Error::Invalid
    );
}

#[sqlx::test]
async fn native_origin_fence_and_owner_only_basis(pool: sqlx::PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let key = identity.bootstrap_user(7, 0).await.unwrap();
    let actor = identity.authorize(&key.secret).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let fresh = quote_in_transaction(&mut tx, actor.user_id).await.unwrap();
    assert_eq!(fresh.discount(), ratio(1, 1));
    tx.rollback().await.unwrap();
    // An owner fence that marks this as pre-existing must not silently grant zero legacy history.
    sqlx::query("UPDATE core_trust_pricing.policies SET native_user_floor=8")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        quote_in_transaction(&mut tx, 7).await.unwrap_err(),
        Error::Unavailable
    );
    tx.rollback().await.unwrap();
    // No anonymous/public grant can mutate pricing facts.
    let public: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_class c CROSS JOIN LATERAL aclexplode(COALESCE(c.relacl,acldefault('r',c.relowner))) a WHERE c.oid='core_trust_pricing.user_bases'::regclass AND a.grantee=0 AND a.privilege_type IN ('INSERT','UPDATE','DELETE','TRUNCATE'))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!public);
}
#[sqlx::test]
async fn imported_net_orders_and_full_refund_change_anchor(pool: sqlx::PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    identity.bootstrap_user(9, 0).await.unwrap();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_trust_pricing.user_bases VALUES(9,$1,0,0,NULL,TRUE,2,1,$2)")
        .bind(now - 110 * 86400)
        .bind(vec![1_u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    for (id, net, at) in [(1, 25_000_000, now - 100 * 86400), (2, 1, now - 86400)] {
        sqlx::query("INSERT INTO core_trust_pricing.legacy_paid_orders VALUES($1,9,$2,$3,1,$4)")
            .bind(id as i64)
            .bind(net as i64)
            .bind(at)
            .bind(vec![2_u8; 32])
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        quote_in_transaction(&mut tx, 9)
            .await
            .unwrap()
            .evidence()
            .level(),
        3
    );
    tx.rollback().await.unwrap();
    // Synthetic owner-import correction changes entitlement only, no money journal.
    sqlx::query("UPDATE core_trust_pricing.legacy_paid_orders SET net_credits=0,revision=2 WHERE source_order_id=2").execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        quote_in_transaction(&mut tx, 9)
            .await
            .unwrap()
            .evidence()
            .level(),
        2
    );
    tx.rollback().await.unwrap();
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(journals, 0);
}
#[test]
fn measured_activity_is_independent_of_price_http_status_or_outcome() {
    use crate::billing::charging::Usage;
    assert!(!counted_api_activity(Usage::default()).unwrap());
    assert!(
        counted_api_activity(Usage {
            input: 1,
            ..Usage::default()
        })
        .unwrap()
    );
    assert!(
        counted_api_activity(Usage {
            output: 1,
            ..Usage::default()
        })
        .unwrap()
    );
    assert!(
        counted_api_activity(Usage {
            upstream_cost_atto_usd: Some(1),
            ..Usage::default()
        })
        .unwrap()
    );
    assert!(
        counted_api_activity(Usage {
            input: -1,
            ..Usage::default()
        })
        .is_err()
    );
}

#[test]
fn production_assembly_enables_owner_price_before_serving_or_assistant_adapters() {
    let main = include_str!("../../main.rs");
    let check = main
        .find("lmm_core::core_schema::check(&pool).await?;")
        .unwrap();
    let enable = main
        .find("model_gateway = configured_models.map(|gateway| gateway.with_trust_pricing());")
        .unwrap();
    let assistant = main
        .find("config::assistant_rpc(url, model_gateway.as_ref(), &token)")
        .unwrap();
    assert!(check < enable && enable < assistant);
    assert_eq!(main.matches("configured_models.map(").count(), 1);
    let grant = include_str!("../../../../../deployment/postgres/grant-core-runtime.sql");
    assert!(grant.contains("core_trust_pricing"));
}
