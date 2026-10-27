//! Administrator-only channel reads from the same published snapshot as relay.
//! Internal endpoints and credential references must never become HTTP fields.
//! Channel mutation requires durable publication and is deliberately unsupported.

use std::{collections::BTreeSet, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    history_read::{HistoryError, HistoryStore, ReadRoute},
    routing::{Protocol, RoutingConfig},
};

#[derive(Clone)]
struct ChannelState {
    identity: HistoryStore,
    routing: Arc<crate::routing::Router>,
}

/// Pass `gateway.dashboard_routing()` so publication updates appear on each read.
pub fn router(pool: PgPool, routing: Arc<crate::routing::Router>) -> Router {
    Router::new()
        .route("/api/channel", get(list).post(unsupported))
        .route("/api/channel/", get(list).put(unsupported))
        .route("/api/channel/search", get(list))
        .route("/api/channel/ops", get(ops))
        .route("/api/channel/{id}", get(detail).delete(unsupported))
        .route("/api/channel/{id}/status", any(unsupported))
        .route("/api/channel/{id}/key", any(unsupported))
        .route("/api/channel/test/{id}", any(unsupported))
        .route("/api/channel/update_balance/{id}", any(unsupported))
        .route("/api/channel/fetch_models/{id}", any(unsupported))
        .route("/api/channel/copy/{id}", any(unsupported))
        .route("/api/channel/status/batch", any(unsupported))
        .route("/api/channel/batch/tag", any(unsupported))
        .route("/api/channel/multi_key/manage", any(unsupported))
        .route("/api/channel/{id}/codex/{*operation}", any(unsupported))
        .with_state(ChannelState {
            identity: HistoryStore::new(pool),
            routing,
        })
        .layer(middleware::from_fn(no_store))
}

#[derive(Default, Deserialize)]
struct Params {
    p: Option<usize>,
    page_size: Option<usize>,
    keyword: Option<String>,
    group: Option<String>,
    model: Option<String>,
    status: Option<String>,
    #[serde(rename = "type")]
    channel_type: Option<u16>,
    sort_by: Option<String>,
    sort_order: Option<String>,
    id_sort: Option<bool>,
    tag_mode: Option<bool>,
}

/// Explicit allowlist: never serialize UpstreamConfig or ReleaseDocument.
#[derive(Serialize)]
struct Channel {
    id: u64,
    name: String,
    name_source: &'static str,
    #[serde(rename = "type")]
    channel_type: u16,
    type_source: &'static str,
    protocol: Protocol,
    status: u8,
    models: String,
    group: String,
    priority: Option<u16>,
    read_only: bool,
    metadata_source: &'static str,
}

fn channels(config: &RoutingConfig) -> Vec<Channel> {
    // Only explicitly selected, non-secret routing fields enter the response.
    // @context-fold begin channel-snapshot-projection
    config
        .upstreams
        .iter()
        .map(|upstream| {
            let mut models = BTreeSet::new();
            let mut groups = BTreeSet::new();
            let mut priority = None;
            for route in &config.routes {
                if !route.enabled
                    || !config
                        .groups
                        .iter()
                        .any(|group| group.enabled && group.name == route.group)
                {
                    continue;
                }
                for target in &route.targets {
                    if target.upstream_id == upstream.id && target.enabled {
                        models.insert(route.model.as_str());
                        groups.insert(route.group.as_str());
                        priority =
                            Some(priority.map_or(target.priority, |current: u16| {
                                current.min(target.priority)
                            }));
                    }
                }
            }
            Channel {
                id: upstream.id,
                name: format!("Channel {}", upstream.id),
                name_source: "generated_from_upstream_id",
                channel_type: match upstream.protocol {
                    Protocol::OpenAiChat | Protocol::OpenAiResponses => 1,
                    Protocol::AnthropicMessages => 14,
                    Protocol::Gemini => 24,
                    Protocol::SystemOne => 0,
                },
                type_source: "derived_from_protocol_not_legacy_provider",
                protocol: upstream.protocol,
                status: if upstream.enabled { 1 } else { 2 },
                models: models.into_iter().collect::<Vec<_>>().join(","),
                group: groups.into_iter().collect::<Vec<_>>().join(","),
                priority,
                read_only: true,
                metadata_source: "active_routing_snapshot",
            }
        })
        .collect()
    // @context-fold end channel-snapshot-projection
}

fn page(config: &RoutingConfig, params: Params) -> Result<Value, &'static str> {
    let current_page = params.p.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(10);
    if current_page == 0 || !(1..=1000).contains(&page_size) {
        return Err("Invalid channel pagination");
    }
    if params.tag_mode == Some(true) {
        return Err("Legacy channel tags are unavailable in the routing snapshot");
    }
    let sort_by = params.sort_by.as_deref().unwrap_or("id");
    if !matches!(sort_by, "id" | "name" | "priority") {
        return Err("Only id, name and priority sorting are available");
    }
    let sort_order = params.sort_order.as_deref().unwrap_or("asc");
    if !matches!(sort_order, "asc" | "desc") {
        return Err("Invalid channel sort order");
    }
    let status = match params.status.as_deref().filter(|v| !v.is_empty()) {
        None | Some("0") | Some("all") => None,
        Some("1" | "enabled") => Some(1),
        Some("2" | "disabled") => Some(2),
        _ => return Err("Only configured enabled or disabled status is available"),
    };
    let mut items = channels(config);
    items.retain(|channel| {
        params.keyword.as_ref().is_none_or(|keyword| {
            let keyword = keyword.to_lowercase();
            channel.name.to_lowercase().contains(&keyword)
                || channel.id.to_string() == keyword
                || channel.models.to_lowercase().contains(&keyword)
        }) && params.group.as_ref().is_none_or(|group| {
            group.is_empty() || channel.group.split(',').any(|value| value == group)
        }) && params.model.as_ref().is_none_or(|model| {
            model.is_empty() || channel.models.split(',').any(|value| value == model)
        }) && status.is_none_or(|status| channel.status == status)
    });
    let mut type_counts = std::collections::BTreeMap::<u16, usize>::new();
    for channel in &items {
        *type_counts.entry(channel.channel_type).or_default() += 1;
    }
    items.retain(|channel| {
        params
            .channel_type
            .filter(|value| *value != 0)
            .is_none_or(|kind| channel.channel_type == kind)
    });
    items.sort_by(|a, b| {
        let order = match sort_by {
            "name" => a.name.cmp(&b.name),
            "priority" => a.priority.cmp(&b.priority),
            _ => a.id.cmp(&b.id),
        };
        order.then(a.id.cmp(&b.id))
    });
    if sort_order == "desc" || (params.sort_order.is_none() && params.id_sort == Some(true)) {
        items.reverse();
    }
    let total = items.len();
    let offset = current_page
        .checked_sub(1)
        .and_then(|page| page.checked_mul(page_size))
        .ok_or("Invalid channel pagination")?;
    let items = items
        .into_iter()
        .skip(offset)
        .take(page_size)
        .collect::<Vec<_>>();
    Ok(json!({
        "success": true,
        "data": {
            "items": items,
            "total": total,
            "page": current_page,
            "page_size": page_size,
            "type_counts": type_counts,
            "read_only": true,
            "config_version": config.version,
            "status_source": "configured_enabled_flag",
            "unavailable_fields": ["legacy_name", "legacy_type", "balance", "used_quota", "response_time", "test_time", "tag"]
        }
    }))
}

async fn admit(state: &ChannelState, headers: &HeaderMap) -> Result<(), Response> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(auth_error(HistoryError::Unauthorized));
    }
    let secret = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, secret)| scheme.eq_ignore_ascii_case("Bearer") && !secret.is_empty())
        .map(|(_, secret)| secret)
        .ok_or_else(|| auth_error(HistoryError::Unauthorized))?;
    state
        .identity
        .authenticate_category(secret, ReadRoute::Administrator)
        .await
        .map_err(auth_error)
}

fn auth_error(error: HistoryError) -> Response {
    let (status, code, message) = match error {
        HistoryError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "AUTH_UNAUTHORIZED",
            "Authentication is required",
        ),
        HistoryError::Forbidden => (
            StatusCode::FORBIDDEN,
            "CHANNEL_FORBIDDEN",
            "Administrator session is required",
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "CHANNEL_READ_UNAVAILABLE",
            "Channel read authentication is unavailable",
        ),
    };
    error_response(status, code, message)
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"success": false, "code": code, "message": message})),
    )
        .into_response()
}

async fn list(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    params: Result<Query<Params>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if let Err(response) = admit(&state, &headers).await {
        return response;
    }
    let params = match params {
        Ok(Query(params)) => params,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "CHANNEL_QUERY_INVALID",
                "Invalid channel query",
            );
        }
    };
    let document = match state.routing.catalog_document() {
        Ok(document) => document,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "CHANNEL_READ_UNAVAILABLE",
                "Active routing snapshot is unavailable",
            );
        }
    };
    match page(&document.routing, params) {
        Ok(value) => Json(value).into_response(),
        Err(message) => error_response(StatusCode::BAD_REQUEST, "CHANNEL_QUERY_INVALID", message),
    }
}

async fn detail(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Response {
    if let Err(response) = admit(&state, &headers).await {
        return response;
    }
    match state.routing.catalog_document() {
        Ok(document) => match channels(&document.routing)
            .into_iter()
            .find(|channel| channel.id == id)
        {
            Some(channel) => Json(json!({"success": true, "data": channel})).into_response(),
            None => error_response(
                StatusCode::NOT_FOUND,
                "CHANNEL_NOT_FOUND",
                "Channel does not exist in the active snapshot",
            ),
        },
        Err(_) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "CHANNEL_READ_UNAVAILABLE",
            "Active routing snapshot is unavailable",
        ),
    }
}

async fn ops(State(state): State<ChannelState>, headers: HeaderMap) -> Response {
    if let Err(response) = admit(&state, &headers).await {
        return response;
    }
    Json(json!({
        "success": true,
        "data": {
            "read_only": true,
            "source": "active_routing_snapshot",
            "retry_times": null,
            "capabilities": {
                "list": true, "search": true, "read": true,
                "create": false, "update": false, "delete": false,
                "test": false, "balance": false, "fetch_models": false,
                "copy": false, "key": false
            },
            "reason": "Channel writes require durable Core configuration publication"
        }
    }))
    .into_response()
}

async fn unsupported(State(state): State<ChannelState>, headers: HeaderMap) -> Response {
    if let Err(response) = admit(&state, &headers).await {
        return response;
    }
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "CHANNEL_OPERATION_UNAVAILABLE",
        "This channel operation is unavailable; only snapshot reads are supported",
    )
}

async fn no_store(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{
        CredentialRef, FailoverPolicy, ModelGroup, ModelRoute, PayloadPolicy, PriceConfig,
        PriceRule, PriceSnapshot, RouteTarget, Speed, TokenRates, UpstreamConfig,
    };
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    fn fixture() -> (RoutingConfig, PriceConfig) {
        let config = RoutingConfig {
            version: 1,
            groups: vec![ModelGroup {
                name: "default".into(),
                enabled: true,
            }],
            upstreams: (1..=21)
                .map(|id| UpstreamConfig {
                    id,
                    endpoint: "https://private-upstream.invalid/v1"
                        .to_owned()
                        .try_into()
                        .unwrap(),
                    protocol: Protocol::OpenAiChat,
                    credential: CredentialRef {
                        id: 999_000 + id,
                        revision: 53,
                    },
                    enabled: true,
                })
                .collect(),
            routes: vec![ModelRoute {
                id: 1,
                model: "model-a".into(),
                group: "default".into(),
                enabled: true,
                speeds: vec![Speed::Standard],
                targets: vec![RouteTarget {
                    id: 1,
                    upstream_id: 1,
                    upstream_model: "private-model-name".into(),
                    enabled: true,
                    priority: 3,
                    weight: 1,
                    speeds: vec![Speed::Standard],
                    payload: PayloadPolicy::Adapt,
                    timeout_ms: 30000,
                }],
                failover: FailoverPolicy::default(),
            }],
            aliases: vec![],
        };
        let prices = PriceConfig {
            version: 1,
            rules: vec![PriceRule {
                billing_mode: crate::routing::BillingMode::Tokens,
                tiered: None,
                id: 1,
                model: "model-a".into(),
                group: "default".into(),
                speed: Speed::Standard,
                rates: TokenRates {
                    input: 1,
                    output: 2,
                    cache_read: 0,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: 1_000_000,
                speed_multiplier_ppm: 1_000_000,
            }],
        };
        (config, prices)
    }

    fn serving(config: RoutingConfig, prices: PriceConfig) -> Arc<crate::routing::Router> {
        Arc::new(
            crate::routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        )
    }

    #[test]
    fn lists_21_unique_upstreams_and_projects_only_safe_metadata() {
        let (config, _) = fixture();
        let value = page(
            &config,
            Params {
                page_size: Some(100),
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(value["data"]["total"], 21);
        let rows = value["data"]["items"].as_array().unwrap();
        assert_eq!(rows.len(), 21);
        assert_eq!(rows[0]["id"], 1);
        assert_eq!(rows[0]["models"], "model-a");
        assert_eq!(rows[0]["group"], "default");
        assert_eq!(rows[0]["priority"], 3);
        let encoded = serde_json::to_string(&value).unwrap();
        for forbidden in [
            "private-upstream",
            "private-model-name",
            "credential",
            "999001",
            "endpoint",
            "base_url",
            "\"key\"",
        ] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn search_filter_pagination_and_sort_match_configured_values() {
        let (mut config, _) = fixture();
        config.upstreams[20].enabled = false;
        config.upstreams[19].protocol = Protocol::AnthropicMessages;
        let value = page(
            &config,
            Params {
                p: Some(2),
                page_size: Some(10),
                sort_order: Some("desc".into()),
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(value["data"]["total"], 21);
        assert_eq!(value["data"]["items"][0]["id"], 11);
        let value = page(
            &config,
            Params {
                group: Some("default".into()),
                model: Some("model-a".into()),
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(value["data"]["total"], 1);
        let value = page(
            &config,
            Params {
                status: Some("disabled".into()),
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(value["data"]["items"][0]["id"], 21);
        let value = page(
            &config,
            Params {
                channel_type: Some(14),
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(value["data"]["items"][0]["id"], 20);
        let value = page(
            &config,
            Params {
                keyword: Some("21".into()),
                ..Params::default()
            },
        )
        .unwrap();
        assert_eq!(value["data"]["total"], 1);
        assert!(
            page(
                &config,
                Params {
                    p: Some(0),
                    ..Params::default()
                }
            )
            .is_err()
        );
        assert!(
            page(
                &config,
                Params {
                    tag_mode: Some(true),
                    ..Params::default()
                }
            )
            .is_err()
        );
        assert!(
            page(
                &config,
                Params {
                    sort_by: Some("balance".into()),
                    ..Params::default()
                }
            )
            .is_err()
        );
    }

    async fn request(app: Router, path: &str, secret: Option<&str>, method: &str) -> Response {
        let mut request = Request::builder().uri(path).method(method);
        if let Some(secret) = secret {
            request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        app.oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn all_channel_reads_require_authentication_and_prevent_caching() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let (config, prices) = fixture();
        let app = router(pool, serving(config, prices));
        for path in [
            "/api/channel",
            "/api/channel/",
            "/api/channel/search",
            "/api/channel/ops",
            "/api/channel/1",
        ] {
            let response = request(app.clone(), path, None, "GET").await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }

    #[sqlx::test(migrations = false)]
    async fn admin_reads_live_snapshot_and_writes_never_claim_success(pool: PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        let user = identity.bootstrap_user(7, 1).await.unwrap();
        let admin = identity.bootstrap_user(8, 5).await.unwrap();
        let (config, prices) = fixture();
        let serving = serving(config.clone(), prices.clone());
        let app = router(pool, serving.clone());
        let response = request(app.clone(), "/api/channel", Some(&user.secret), "GET").await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        for (path, method, expected) in [
            ("/api/channel", "GET", StatusCode::OK),
            ("/api/channel/search?keyword=21", "GET", StatusCode::OK),
            ("/api/channel/ops", "GET", StatusCode::OK),
            ("/api/channel/1", "GET", StatusCode::OK),
            ("/api/channel/99", "GET", StatusCode::NOT_FOUND),
            ("/api/channel", "POST", StatusCode::NOT_IMPLEMENTED),
            ("/api/channel/", "PUT", StatusCode::NOT_IMPLEMENTED),
            ("/api/channel/1", "DELETE", StatusCode::NOT_IMPLEMENTED),
        ] {
            let response = request(app.clone(), path, Some(&admin.secret), method).await;
            assert_eq!(response.status(), expected, "{method} {path}");
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            if path == "/api/channel/ops" {
                assert_eq!(value["data"]["read_only"], true);
                assert_eq!(value["data"]["capabilities"]["update"], false);
            }
        }
        let mut next = config;
        next.version = 2;
        next.upstreams[0].enabled = false;
        let mut prices = prices;
        prices.version = 2;
        serving
            .publish(
                serving.version().unwrap(),
                next,
                Arc::new(PriceSnapshot::compile(prices).unwrap()),
            )
            .unwrap();
        let response = request(app, "/api/channel/1", Some(&admin.secret), "GET").await;
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(value["data"]["status"], 2);
    }
}
