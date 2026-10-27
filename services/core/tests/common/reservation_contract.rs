//! Owner-only v1 proof fixtures for recovery readers in disposable SQLx databases.
//!
//! These existing integration tests seed holds through the public charging API.
//! This helper adds synthetic durable preparation evidence before dispatch so
//! their recovery, cache rejection and cursor assertions still exercise the R1
//! reader. It is not a migration, an admission API, or evidence that reservation
//! and proof were committed atomically. The real prepared-admission tests cover
//! that transaction separately. No production request is repaired by this code.
use lmm_core::{
    billing::charging::{Price, Usage},
    routing::CredentialRef,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};

// Keep the v1 typed serialization order explicit. Hashing a JSONB value would
// sort object keys and would not reproduce the product's typed digest.
#[derive(Serialize)]
struct BindingV1 {
    route_id: String,
    target_id: u64,
    upstream_id: u64,
    protocol: String,
    model_snapshot: String,
    generation_url: String,
    credential_ref: CredentialRef,
    payload_policy_sha256: String,
    provider_contract_revision: String,
    projection_revision: String,
    quote_version: u64,
}

#[derive(Serialize)]
struct BoundsV1 {
    model_input_max: u64,
    input: u64,
    requested_output: u64,
    output: u64,
}

#[derive(Serialize)]
struct SemanticsV1 {
    input: String,
    output: String,
    missing_cached_tokens_is_zero: bool,
    missing_cache_creation_tokens_is_zero: bool,
}

#[derive(Serialize)]
struct PreparedFieldsV1 {
    kind: String,
    version: u32,
    mode: String,
    request_fingerprint: String,
    binding: BindingV1,
    wire_sha256: String,
    counter: Option<()>,
    bounds: BoundsV1,
    semantics: SemanticsV1,
}

fn typed_digest(value: &impl Serialize) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

async fn unchanged_liability(connection: &mut PgConnection, id: &str) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(\
         'request',(SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1),\
         'holds',(SELECT coalesce(jsonb_agg(to_jsonb(h) ORDER BY position),'[]') FROM core_charging.ledger_holds h WHERE request_id=$1),\
         'events',(SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY event_id),'[]') FROM core_charging.usage_events e WHERE request_id=$1 AND event_id NOT IN ('reservation-contract:v1','reservation-admission:v1')),\
         'commands',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY operation_id),'[]') FROM core_charging.ledger_commands c WHERE request_id=$1),\
         'checks',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY request_revision,budget_id),'[]') FROM core_charging.budget_checks b WHERE request_id=$1),\
         'balances',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id),'[]') FROM core_billing.balance_state b),\
         'journals',(SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY id),'[]') FROM core_billing.ledger_journals j),\
         'entries',(SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY journal_id,ledger_account_id),'[]') FROM core_billing.ledger_entries e),\
         'operations',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY scope,operation_key),'[]') FROM core_billing.ledger_operations o))",
    )
    .bind(id)
    .fetch_one(connection)
    .await
    .unwrap()
}

pub async fn install_synthetic_recovery_contract(pool: &PgPool, id: &str, input: u64, output: u64) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let (fingerprint, spec, price_json, reserved, state): (String, String, Value, i64, String) =
        sqlx::query_as(
            "SELECT encode(fingerprint,'hex'),encode(spec_digest,'hex'),price,reserved,state \
             FROM core_charging.requests WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(state, "reserved", "install only before synthetic dispatch");
    let prior_proofs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 \
         AND event_id IN ('reservation-contract:v1','reservation-admission:v1')",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(prior_proofs, 0, "never replace existing admission evidence");
    let before = unchanged_liability(&mut tx, id).await;
    let price: Price = serde_json::from_value(price_json).unwrap();
    assert!(price.tiered.is_none() && price.image.is_none() && price.user_trust.is_none());
    assert!(price.input_per_million >= price.cached_per_million);
    assert!(input > 0 && output > 0);
    // These existing fixtures use either simple token rates or a one-USD
    // upstream-cost reserve. Their evidence must describe that same ceiling.
    assert_eq!(
        price
            .cost(Usage {
                input: i64::try_from(input).unwrap(),
                cached: 0,
                output: i64::try_from(output).unwrap(),
                upstream_cost_atto_usd: price
                    .upstream_cost_group_ppm
                    .map(|_| 1_000_000_000_000_000_000),
                cache_creation: None,
                image: None,
            })
            .unwrap(),
        reserved,
        "synthetic bounds must preserve the original financial ceiling"
    );
    let fields = PreparedFieldsV1 {
        kind: "model_reservation_contract".into(),
        version: 1,
        mode: "context_max".into(),
        request_fingerprint: fingerprint,
        binding: BindingV1 {
            route_id: "synthetic-recovery-route-v1".into(),
            target_id: 1,
            upstream_id: 1,
            protocol: "chat".into(),
            model_snapshot: price.model.clone(),
            generation_url: "https://synthetic.invalid/v1/chat/completions".into(),
            credential_ref: CredentialRef { id: 1, revision: 1 },
            payload_policy_sha256: typed_digest(&"synthetic-recovery-payload"),
            provider_contract_revision: "synthetic-recovery-provider-v1".into(),
            projection_revision: "context-max-v1".into(),
            quote_version: 1,
        },
        wire_sha256: typed_digest(&("synthetic-recovery-wire-v1", id)),
        counter: None,
        bounds: BoundsV1 {
            model_input_max: input,
            input,
            requested_output: output,
            output,
        },
        semantics: SemanticsV1 {
            input: "inclusive".into(),
            output: "includes_reasoning".into(),
            missing_cached_tokens_is_zero: false,
            missing_cache_creation_tokens_is_zero: false,
        },
    };
    let prepared_sha256 = typed_digest(&fields);
    let frozen_price_sha256 = typed_digest(&price);
    let mut proof = serde_json::to_value(&fields).unwrap();
    proof["prepared_sha256"] = json!(prepared_sha256);
    proof["admission_spec_sha256"] = json!(spec);
    proof["frozen_price_sha256"] = json!(frozen_price_sha256);
    let marker = json!({
        "kind":"model_reservation_admission",
        "version":1,
        "prepared_sha256":prepared_sha256,
        "admission_spec_sha256":spec,
        "frozen_price_sha256":frozen_price_sha256,
    });
    let inserted = sqlx::query(
        "INSERT INTO core_charging.usage_events(request_id,event_id,detail) \
         VALUES ($1,'reservation-contract:v1',$2),($1,'reservation-admission:v1',$3)",
    )
    .bind(id)
    .bind(&proof)
    .bind(&marker)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(inserted.rows_affected(), 2);
    assert_eq!(unchanged_liability(&mut tx, id).await, before);
    tx.commit().await.unwrap();
}
