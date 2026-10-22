//! Exact deployed Unicode groups through real Key authorization and the ledger.
//! All credentials/database/provider are isolated synthetic fixture resources.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    identity::DashboardKeyInput,
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
#[path = "common/model_fixture.rs"]
mod model_fixture;
use model_fixture::{Provider, fixture};

fn routing(provider: &Provider, group: &str) -> Arc<Router> {
    let prices = PriceSnapshot::compile(PriceConfig {
        version: 1,
        rules: vec![PriceRule {
            embedding: None,
            search: None,
            billing_mode: BillingMode::Tokens,
            tiered: None,
            id: 1,
            model: "local-model".into(),
            group: group.into(),
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
    Arc::new(
        Router::new(
            RoutingConfig {
                version: 1,
                groups: vec![ModelGroup {
                    name: group.into(),
                    enabled: true,
                }],
                upstreams: vec![UpstreamConfig {
                    id: 1,
                    endpoint: Endpoint::local_loopback(provider.endpoint.clone()).unwrap(),
                    protocol: Protocol::OpenAiChat,
                    credential: CredentialRef { id: 7, revision: 1 },
                    enabled: true,
                }],
                routes: vec![ModelRoute {
                    id: 1,
                    model: "local-model".into(),
                    group: group.into(),
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
                    failover: FailoverPolicy::default(),
                }],
                aliases: vec![],
            },
            Arc::new(prices),
        )
        .unwrap(),
    )
}
#[sqlx::test(migrations = false)]
async fn unicode_group_key_api_real_route_charge_and_no_implicit_alias(pool: PgPool) {
    let (identity, _ledger, session, _old_key, _old_id, _account) = fixture(&pool, 10000).await;
    let provider = Provider::start(true).await;
    for (index, group) in ["国产", "官key", "国产[Kimi/Deepseek/GLM]"]
        .into_iter()
        .enumerate()
    {
        let routes = routing(&provider, group);
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,$1,TRUE,'unicode-group-test')").bind(group).execute(&pool).await.unwrap();
        let identity = identity.clone().with_dashboard_routing(routes.clone());
        let input:DashboardKeyInput=serde_json::from_value(json!({"name":format!("unicode-{index}"),"remain_quota":5000,"expired_time":-1,"unlimited_quota":false,"model_limits_enabled":true,"model_limits":"local-model","group":group})).unwrap();
        let created = identity
            .dashboard_create_key(&session, &input)
            .await
            .unwrap();
        let key = created["key"].as_str().unwrap();
        assert_eq!(
            identity.api_key_routing_groups(key).await.unwrap(),
            vec![group.to_owned()]
        );
        assert!(matches!(
            identity
                .authorize_model_request(key, "local-model", None, &format!("{group}-other"))
                .await,
            Err(lmm_core::identity::IdentityError::Forbidden)
        ));

        // Prefix/suffix whitespace is never silently converted to a granted group.
        for wrong in [format!(" {group}"), format!("{group} ")] {
            let mut bad:DashboardKeyInput=serde_json::from_value(json!({"name":"bad","remain_quota":100,"expired_time":-1,"unlimited_quota":false,"group":wrong})).unwrap();
            bad.one_time_reveal = Some(true);
            assert!(identity.dashboard_create_key(&session, &bad).await.is_err());
        }
        let gateway = ModelGateway::new(
            pool.clone(),
            routes,
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
        let request=Request::builder().method("POST").uri("/v1/chat/completions").header("authorization",format!("Bearer {key}")).header("content-type","application/json").header("idempotency-key",format!("unicode-group-{index}")).body(Body::from(json!({"model":"local-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20}).to_string())).unwrap();
        let response = gateway.router().oneshot(request).await.unwrap();
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap())
                .unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        let receipt: (String, String, i64) = sqlx::query_as(
            "SELECT price->>'group',state,settled FROM core_charging.requests WHERE id=$1",
        )
        .bind(request_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(receipt, (group.to_owned(), "settled".into(), 49));
    }
    assert_eq!(provider.hits.load(Ordering::SeqCst), 3);
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) invalid").fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
}
