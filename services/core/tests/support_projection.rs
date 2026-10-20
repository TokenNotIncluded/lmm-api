//! Real Core identities, ledger reservations, and seller-only projections.
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        ledger::{Action, Ledger, Outcome, Request as LedgerRequest},
    },
    commerce::{CommerceStore, CreditPrice, MoneyCommand, OrderSnapshot, Price},
    core_schema,
    identity::IdentityStore,
    support_projection::{Error, Relationship, Request, SupportProjection},
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;
fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
async fn fixture(pool: &PgPool) -> (IdentityStore, String, String, String) {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let buyer = identity.bootstrap_user(7, 1).await.unwrap().secret;
    let seller = identity.bootstrap_user(9, 1).await.unwrap().secret;
    let outsider = identity.bootstrap_user(11, 1).await.unwrap().secret;
    (identity, buyer, seller, outsider)
}
fn query() -> Request {
    Request {
        seller_id: 9,
        buyer_id: None,
        relationships: vec![],
        query: String::new(),
        offset: 0,
        limit: 30,
    }
}
#[sqlx::test(migrations = false)]
async fn real_paid_orders_are_integer_credits_and_exclude_other_sellers(pool: PgPool) {
    let (identity, buyer, seller, outsider) = fixture(&pool).await;
    let ledger = Ledger::from_pool(pool.clone());
    let account = identity.authorize(&buyer).await.unwrap().owner_account_id;
    ledger.open_wallet(account).await.unwrap();
    let seller_account = identity.authorize(&seller).await.unwrap().owner_account_id;
    ledger.open_wallet(seller_account).await.unwrap();
    assert!(matches!(
        ledger
            .execute(&LedgerRequest {
                scope: "support-fixture".into(),
                operation_key: "synthetic-funds".into(),
                actor_user_id: Some(7),
                reason: "Synthetic support projection test".into(),
                action: Action::Credit {
                    account_id: account,
                    amount_units: Credits::try_from(500000).unwrap(),
                    payment_reference: "synthetic-support-fixture".into()
                }
            })
            .await
            .unwrap(),
        Outcome::Posted { .. }
    ));
    let price = Price::Credits(CreditPrice {
        amount_credits: 12345,
        unit: "credits".into(),
    });
    let shop = hash("synthetic-shop");
    let order = hash("synthetic-order");
    let command = MoneyCommand {
        key: hash(
            &serde_json::to_string(&serde_json::json!(["store-money-v1", shop, order, "pay"]))
                .unwrap(),
        ),
        kind: "pay".into(),
        shop_id: shop,
        order_id: order.clone(),
        buyer: Account {
            kind: AccountKind::Personal,
            id: 7,
        },
        seller: Account {
            kind: AccountKind::Personal,
            id: 9,
        },
        total: price.clone(),
        payment_ref: String::new(),
        settlement_ref: String::new(),
        order_snapshot: OrderSnapshot {
            variant_id: hash("synthetic-variant"),
            quantity: 1,
            unit_price: price,
            created_by: 7,
        },
    };
    let receipt = CommerceStore::from_pool(pool.clone())
        .execute(&buyer, &serde_json::to_vec(&vec![command]).unwrap())
        .await
        .unwrap();
    assert_eq!(receipt.outcome, "succeeded");
    let projection = SupportProjection::new(pool.clone());
    let rows = projection.customers(&seller, &query()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].buyer_id, 7);
    assert_eq!(rows[0].paid_quota, 12345);
    assert_eq!(rows[0].order_count, 1);
    assert_eq!(rows[0].last_order_id, order);
    assert_eq!(rows[0].display_name, "native_7");
    let mut other = query();
    other.seller_id = 11;
    assert!(
        projection
            .customers(&outsider, &other)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        projection.customers(&outsider, &query()).await,
        Err(Error::Forbidden)
    ));
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    projection.customers(&seller, &query()).await.unwrap();
    assert_eq!(
        journals,
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_billing.ledger_journals")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
}
#[sqlx::test(migrations = false)]
async fn conversation_only_names_search_pagination_and_revoked_session(pool: PgPool) {
    let (_identity, _buyer, seller, _outsider) = fixture(&pool).await;
    let projection = SupportProjection::new(pool.clone());
    let mut request = query();
    request.relationships = vec![
        Relationship {
            buyer_id: 7,
            conversation_id: "conversation-seven".into(),
        },
        Relationship {
            buyer_id: 11,
            conversation_id: "conversation-eleven".into(),
        },
    ];
    let rows = projection.customers(&seller, &request).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].buyer_id, 11);
    assert_eq!(rows[1].order_count, 0);
    assert_eq!(rows[1].paid_quota, 0);
    assert_eq!(rows[1].conversation_id, "conversation-seven");
    request.query = "native_7".into();
    assert_eq!(
        projection.customers(&seller, &request).await.unwrap().len(),
        1
    );
    request.query = "%".into();
    assert!(
        projection
            .customers(&seller, &request)
            .await
            .unwrap()
            .is_empty()
    );
    request.query = String::new();
    request.buyer_id = Some(7);
    assert_eq!(
        projection.customers(&seller, &request).await.unwrap()[0].buyer_id,
        7
    );
    request.query = String::new();
    request.buyer_id = None;
    request.limit = 1;
    request.offset = 1;
    assert_eq!(
        projection.customers(&seller, &request).await.unwrap()[0].buyer_id,
        7
    );
    sqlx::query(
        "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=9",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        projection.customers(&seller, &request).await,
        Err(Error::Unauthorized)
    ));
}

#[sqlx::test(migrations = false)]
async fn support_only_runtime_requires_reads_and_never_commerce_writer_grants(pool: PgPool) {
    let (_identity, _buyer, seller, _outsider) = fixture(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    let projection = SupportProjection::new(role.pool.clone());
    assert!(matches!(
        projection.check_readonly().await,
        Err(Error::Unavailable)
    ));
    role.optional("support_projection").await;
    projection.check_readonly().await.unwrap();
    lmm_core::runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    let writes: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('core_commerce.orders','INSERT,UPDATE,DELETE')",
    )
    .fetch_one(&role.pool)
    .await
    .unwrap();
    assert!(!writes);
    let mut request = query();
    request.relationships.push(Relationship {
        buyer_id: 7,
        conversation_id: "synthetic".into(),
    });
    assert_eq!(
        projection.customers(&seller, &request).await.unwrap().len(),
        1
    );
    sqlx::query(&format!(
        "REVOKE SELECT ON core_commerce.orders FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        projection.check_readonly().await,
        Err(Error::Unavailable)
    ));
    role.cleanup().await;
}
