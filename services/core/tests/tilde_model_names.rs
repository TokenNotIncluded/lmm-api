//! Existing literal `~` model labels retain exact native Key scope and prices.
//! Real disposable Core DB/ledger and counting local HTTP supplier; no paid calls.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    identity::{DashboardKeyInput, IdentityError},
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::*,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tower::ServiceExt;
#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
use model_fixture::{Provider, fixture};
const MODEL: &str = "~anthropic/claude-fable-latest";
const SIMILAR: &str = "anthropic/claude-fable-latest";
fn routing(provider: &Provider) -> Arc<Router> {
    let mut config: RoutingConfig = serde_json::from_value(json!({
        "version":1,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[],"aliases":[]
    })).unwrap();
    config.upstreams[0].endpoint = Endpoint::local_loopback(provider.endpoint.clone()).unwrap();
    let mut rules = Vec::new();
    for (i, model) in [MODEL, SIMILAR].into_iter().enumerate() {
        let id = i as u64 + 1;
        config.routes.push(serde_json::from_value(json!({
            "id":id,"model":model,"group":"allowed","enabled":true,"speeds":["standard"],
            "targets":[{"id":id,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
            "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}
        })).unwrap());
        rules.push(serde_json::from_value(json!({
            "id":id,"model":model,"group":"allowed","speed":"standard","billing_mode":"tokens",
            "rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
        })).unwrap());
    }
    Arc::new(
        Router::new(
            config,
            Arc::new(PriceSnapshot::compile(PriceConfig { version: 1, rules }).unwrap()),
        )
        .unwrap(),
    )
}
#[sqlx::test(migrations = false)]
async fn tilde_model_native_key_scope_route_and_ledger_remain_exact(pool: PgPool) {
    let (identity, ledger, session, _, _, account) = fixture(&pool, 10000).await;
    let provider = Provider::start(true).await;
    let router = routing(&provider);
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'tilde-model-test')").execute(&pool).await.unwrap();
    let identity = identity.with_dashboard_routing(router.clone());
    let input: DashboardKeyInput = serde_json::from_value(json!({
        "name":"tilde-exact","remain_quota":5000,"expired_time":-1,"unlimited_quota":false,
        "model_limits_enabled":true,"model_limits":MODEL,"group":"allowed"
    }))
    .unwrap();
    let created = identity
        .dashboard_create_key(&session, &input)
        .await
        .unwrap();
    let key = created["key"].as_str().unwrap();
    identity
        .authorize_model_request(key, MODEL, None, "allowed")
        .await
        .unwrap();
    assert!(matches!(
        identity
            .authorize_model_request(key, SIMILAR, None, "allowed")
            .await,
        Err(IdentityError::Forbidden)
    ));
    let gateway = ModelGateway::new(
        pool.clone(),
        router,
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-upstream-key".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 100,
            lease_seconds: 30,
        },
    )
    .unwrap();
    for (model, expected) in [(SIMILAR, StatusCode::FORBIDDEN), (MODEL, StatusCode::OK)] {
        let request = Request::builder().method("POST").uri("/v1/chat/completions")
            .header("authorization", format!("Bearer {key}")).header("content-type", "application/json")
            .header("idempotency-key", if model==MODEL {"tilde-authorized"} else {"similar-denied"})
            .body(Body::from(json!({"model":model,"messages":[{"role":"user","content":"hello"}],"max_tokens":20}).to_string())).unwrap();
        let response = gateway.router().oneshot(request).await.unwrap();
        let status = response.status();
        let id = response
            .headers()
            .get("x-request-id")
            .map(|v| v.to_str().unwrap().to_owned());
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1000000).await.unwrap())
                .unwrap();
        assert_eq!(status, expected, "{body}");
        if model == SIMILAR {
            assert_eq!(provider.hits.load(Ordering::SeqCst), 0);
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
        } else {
            let receipt: (String, String, i64) = sqlx::query_as(
                "SELECT price->>'model',state,settled FROM core_charging.requests WHERE id=$1",
            )
            .bind(id.unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(receipt, (MODEL.into(), "settled".into(), 49));
        }
    }
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9951, 0));
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT unnest(model_limits) FROM core_identity.api_key_settings WHERE name='tilde-exact'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(stored, [MODEL]);
}
