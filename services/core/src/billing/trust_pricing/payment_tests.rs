//! Native entitlement comes from the real signed payment/credit/refund pipeline.
use super::*;
use crate::{
    identity::IdentityStore,
    internal_rpc::pb,
    payments::{Channel, PaymentsStore},
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;
const SECRET: &str = "whsec_trust_pricing_synthetic_only";
const MERCHANT: &str = "acct_trust_pricing_synthetic";
fn mutation(key: &str) -> Option<pb::MutationContext> {
    Some(pb::MutationContext {
        idempotency_key: key.into(),
        expected_version: None,
    })
}
fn proof(
    intent: &pb::PaymentIntent,
    event_id: &str,
    refund: Option<(&str, i64)>,
) -> pb::ProviderEvidence {
    let metadata =
        json!({"lmm_core_intent_id":intent.intent_id.to_string(),"lmm_merchant":MERCHANT});
    let pi = format!("pi_{}", intent.intent_id);
    let object = match refund {
        Some((id, amount)) => {
            json!({"id":id,"object":"refund","status":"succeeded","amount":amount,"currency":"usd","payment_intent":pi,"metadata":metadata})
        }
        None => {
            json!({"id":pi,"object":"payment_intent","status":"succeeded","amount":intent.amount_minor,"amount_received":intent.amount_minor,"currency":"usd","livemode":false,"metadata":metadata})
        }
    };
    let value = json!({"id":event_id,"type":if refund.is_some(){"refund.updated"}else{"payment_intent.succeeded"},"livemode":false,"data":{"object":object}});
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = serde_json::to_vec(&value).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&payload);
    let signature = format!(
        "t={now},v1={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    pb::ProviderEvidence {
        payment: Some(pb::ExternalPaymentReference {
            provider: "stripe".into(),
            merchant: MERCHANT.into(),
            environment: "test".into(),
            transaction: pi,
        }),
        signed_payload: payload,
        signature: signature.into_bytes(),
        provider_event_id: event_id.into(),
        accepted_proof_id: String::new(),
    }
}
async fn ingress(
    store: &PaymentsStore,
    intent: &pb::PaymentIntent,
    mut evidence: pb::ProviderEvidence,
) -> pb::ProviderEvidence {
    evidence.accepted_proof_id = store
        .accept_proof(pb::AcceptProviderProofRequest {
            intent_id: intent.intent_id,
            evidence: Some(evidence.clone()),
        })
        .await
        .unwrap()
        .proof_id;
    evidence
}
async fn level(pool: &PgPool, user: i64) -> u8 {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let result = quote_in_transaction(&mut tx, user).await.unwrap();
    let level = result.evidence().level();
    tx.rollback().await.unwrap();
    level
}
#[sqlx::test]
async fn real_new_credit_and_partial_full_refund_recompute_entitlement(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(17, 0).await.unwrap();
    let account = identity
        .authorize(&session.secret)
        .await
        .unwrap()
        .owner_account_id;
    let store = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_test(
            1,
            MERCHANT.into(),
            SECRET.into(),
            10000,
        )],
    )
    .unwrap();
    let intent = store
        .prepare(
            &session.secret,
            pb::PrepareTopupRequest {
                mutation: mutation("trust_prepare_0001"),
                account_id: account,
                payment_channel_id: 1,
                currency: "USD".into(),
                amount_minor: 1000,
            },
        )
        .await
        .unwrap();
    assert_eq!(level(&pool, 17).await, 0);
    let evidence = ingress(
        &store,
        &intent,
        proof(&intent, "evt_trust_credit_0001", None),
    )
    .await;
    // An accepted proof alone must NOT grant paid eligibility or money.
    assert_eq!(level(&pool, 17).await, 0);
    let credit = store
        .credit(pb::CreditTopupRequest {
            mutation: mutation("trust_credit_0001"),
            intent_id: intent.intent_id,
            evidence: Some(evidence.clone()),
        })
        .await
        .unwrap();
    assert_eq!(credit.state, 1);
    assert_eq!(level(&pool, 17).await, 2);
    let replay = store
        .credit(pb::CreditTopupRequest {
            mutation: mutation("trust_credit_0001"),
            intent_id: intent.intent_id,
            evidence: Some(evidence),
        })
        .await
        .unwrap();
    assert_eq!(replay, credit);
    assert_eq!(level(&pool, 17).await, 2);
    let partial = ingress(
        &store,
        &intent,
        proof(
            &intent,
            "evt_trust_refund_0001",
            Some(("re_trust_partial", 1)),
        ),
    )
    .await;
    store
        .external_refund(pb::ExternalRefundRequest {
            mutation: mutation("trust_refund_partial_0001"),
            intent_id: intent.intent_id,
            evidence: Some(partial),
        })
        .await
        .unwrap();
    assert_eq!(level(&pool, 17).await, 1);
    let full = ingress(
        &store,
        &intent,
        proof(
            &intent,
            "evt_trust_refund_0002",
            Some(("re_trust_remaining", 999)),
        ),
    )
    .await;
    store
        .external_refund(pb::ExternalRefundRequest {
            mutation: mutation("trust_refund_full_0002"),
            intent_id: intent.intent_id,
            evidence: Some(full),
        })
        .await
        .unwrap();
    assert_eq!(level(&pool, 17).await, 0);
    // The price resolver never writes another money journal or counterfeit balance.
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(level(&pool, 17).await, 0);
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    // Company money belongs to that beneficiary, never the creator's personal entitlement.
    let team = identity.create_team(&session.secret).await.unwrap();
    let company = identity
        .list_teams(&session.secret, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == team)
        .unwrap()
        .account_id;
    let other = identity.bootstrap_user(18, 0).await.unwrap();
    let other_account = identity
        .authorize(&other.secret)
        .await
        .unwrap()
        .owner_account_id;
    for (key, who, beneficiary) in [
        ("company", session.secret.as_str(), company),
        ("other", other.secret.as_str(), other_account),
    ] {
        let operation = format!("trust_{key}_prepare_0001");
        let order = store
            .prepare(
                who,
                pb::PrepareTopupRequest {
                    mutation: mutation(&operation),
                    account_id: beneficiary,
                    payment_channel_id: 1,
                    currency: "USD".into(),
                    amount_minor: 1000,
                },
            )
            .await
            .unwrap();
        let evidence = ingress(
            &store,
            &order,
            proof(&order, &format!("evt_trust_{key}_paid"), None),
        )
        .await;
        store
            .credit(pb::CreditTopupRequest {
                mutation: mutation(&format!("trust_{key}_credit_0001")),
                intent_id: order.intent_id,
                evidence: Some(evidence),
            })
            .await
            .unwrap();
        assert_eq!(
            level(&pool, 17).await,
            0,
            "foreign/company money never grants creator personal discounts"
        );
    }
    assert_eq!(level(&pool, 18).await, 2);
}
