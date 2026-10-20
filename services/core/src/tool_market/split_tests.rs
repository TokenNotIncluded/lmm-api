//! Independent disposable PG installation of the unregistered second-batch
//! ledger extension. Canonical 17-component schema is untouched by these tests.
use super::*;
use crate::billing::ledger::Rejection;
struct SplitFixture {
    ledger: Ledger,
    buyer: i64,
    seller: i64,
    platform: i64,
}
async fn setup(pool: &PgPool) -> SplitFixture {
    crate::core_schema::init(pool).await.unwrap();
    sqlx::raw_sql(include_str!("ledger_split_capture_test.sql"))
        .execute(pool)
        .await
        .unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let mut ids = Vec::new();
    let ledger = Ledger::from_pool(pool.clone());
    for user in [7, 9, 11] {
        let s = identity.bootstrap_user(user, 1).await.unwrap().secret;
        let id = identity.authorize(&s).await.unwrap().owner_account_id;
        ledger.open_wallet(id).await.unwrap();
        ids.push(id);
    }
    SplitFixture {
        ledger,
        buyer: ids[0],
        seller: ids[1],
        platform: ids[2],
    }
}
async fn act(f: &SplitFixture, key: &str, action: Action) -> Outcome {
    f.ledger
        .execute(&Request {
            scope: "core-tool-market-credit".into(),
            operation_key: key.into(),
            actor_user_id: Some(7),
            reason: "Synthetic split capture fixture".into(),
            action,
        })
        .await
        .unwrap()
}
fn c(v: i64) -> Credits {
    Credits::try_from(v).unwrap()
}
async fn credit(f: &SplitFixture, who: i64, amount: i64, key: &str) -> i64 {
    let o = act(
        f,
        key,
        Action::Credit {
            account_id: who,
            amount_units: c(amount),
            payment_reference: format!("synthetic:{key}"),
        },
    )
    .await;
    match o {
        Outcome::Posted { journal_id, .. } => journal_id,
        _ => panic!("credit not posted"),
    }
}
async fn hold(f: &SplitFixture, amount: i64, key: &str) -> i64 {
    match act(
        f,
        key,
        Action::Reserve {
            account_id: f.buyer,
            amount_units: c(amount),
        },
    )
    .await
    {
        Outcome::Posted { journal_id, .. } => journal_id,
        _ => panic!("reserve not posted"),
    }
}
async fn debt(f: &SplitFixture, who: i64, amount: i64, key: &str) {
    let original = credit(f, who, amount, &format!("{key}-credit")).await;
    assert!(matches!(
        act(
            f,
            &format!("{key}-spent"),
            Action::Charge {
                account_id: who,
                amount_units: c(amount)
            }
        )
        .await,
        Outcome::Posted { .. }
    ));
    assert!(matches!(
        act(
            f,
            &format!("{key}-refund"),
            Action::RefundTopup {
                journal_id: original,
                amount_units: c(amount),
                refund_reference: format!("synthetic-debt-{key}")
            }
        )
        .await,
        Outcome::Posted { .. }
    ));
}
async fn balanced(pool: &PgPool) {
    let bad:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) x").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0);
}
#[sqlx::test(migrations = false)]
async fn split_hold_repays_both_debts_once_and_returns_unused(pool: PgPool) {
    let f = setup(&pool).await;
    credit(&f, f.buyer, 100, "buyer").await;
    debt(&f, f.seller, 20, "seller").await;
    debt(&f, f.platform, 7, "platform").await;
    let original = hold(&f, 100, "hold").await;
    let action = Action::SplitCommerceCapture {
        reservation_journal_id: original,
        seller_account_id: f.seller,
        platform_account_id: f.platform,
        amount_units: c(81),
        fee_basis_points: 2500,
    };
    let out = act(&f, "capture", action.clone()).await;
    assert!(matches!(out, Outcome::Posted { .. }));
    assert_eq!(act(&f, "capture", action).await, out);
    assert_eq!(f.ledger.balance(f.buyer).await.unwrap().available_units, 19);
    assert_eq!(f.ledger.balance(f.buyer).await.unwrap().reserved_units, 0);
    assert_eq!(
        f.ledger.balance(f.seller).await.unwrap().available_units,
        41
    );
    assert_eq!(f.ledger.balance(f.seller).await.unwrap().debt_units, 0);
    assert_eq!(
        f.ledger.balance(f.platform).await.unwrap().available_units,
        13
    );
    assert_eq!(f.ledger.balance(f.platform).await.unwrap().debt_units, 0);
    assert!(matches!(
        act(
            &f,
            "release-after",
            Action::Release {
                reservation_journal_id: original
            }
        )
        .await,
        Outcome::Rejected {
            code: Rejection::AlreadyFinalized
        }
    ));
    balanced(&pool).await;
}
#[sqlx::test(migrations = false)]
async fn shared_destination_and_self_seller_do_not_duplicate_entries(pool: PgPool) {
    let f = setup(&pool).await;
    credit(&f, f.buyer, 200, "buyer").await;
    debt(&f, f.seller, 20, "seller").await;
    let first = hold(&f, 100, "first-hold").await;
    assert!(matches!(
        act(
            &f,
            "first",
            Action::SplitCommerceCapture {
                reservation_journal_id: first,
                seller_account_id: f.seller,
                platform_account_id: f.seller,
                amount_units: c(81),
                fee_basis_points: 2500
            }
        )
        .await,
        Outcome::Posted { .. }
    ));
    assert_eq!(
        f.ledger.balance(f.seller).await.unwrap().available_units,
        61
    );
    assert_eq!(f.ledger.balance(f.seller).await.unwrap().debt_units, 0);
    let second = hold(&f, 100, "second-hold").await;
    assert!(matches!(
        act(
            &f,
            "second",
            Action::SplitCommerceCapture {
                reservation_journal_id: second,
                seller_account_id: f.buyer,
                platform_account_id: f.platform,
                amount_units: c(81),
                fee_basis_points: 2500
            }
        )
        .await,
        Outcome::Posted { .. }
    ));
    assert_eq!(f.ledger.balance(f.buyer).await.unwrap().available_units, 99);
    assert_eq!(
        f.ledger.balance(f.platform).await.unwrap().available_units,
        20
    );
    balanced(&pool).await;
}
#[sqlx::test(migrations = false)]
async fn zero_usage_releases_full_original_hold_and_excess_cannot_post(pool: PgPool) {
    let f = setup(&pool).await;
    credit(&f, f.buyer, 100, "buyer").await;
    let original = hold(&f, 100, "hold").await;
    assert!(matches!(
        act(
            &f,
            "excess",
            Action::SplitCommerceCapture {
                reservation_journal_id: original,
                seller_account_id: f.seller,
                platform_account_id: f.platform,
                amount_units: c(101),
                fee_basis_points: 1
            }
        )
        .await,
        Outcome::Rejected {
            code: Rejection::ExceedsRemaining
        }
    ));
    assert!(matches!(
        act(
            &f,
            "zero",
            Action::SplitCommerceCapture {
                reservation_journal_id: original,
                seller_account_id: f.seller,
                platform_account_id: f.platform,
                amount_units: c(0),
                fee_basis_points: 2500
            }
        )
        .await,
        Outcome::Posted { .. }
    ));
    assert_eq!(
        f.ledger.balance(f.buyer).await.unwrap().available_units,
        100
    );
    assert_eq!(f.ledger.balance(f.buyer).await.unwrap().reserved_units, 0);
    assert_eq!(f.ledger.balance(f.seller).await.unwrap().available_units, 0);
    balanced(&pool).await;
}
