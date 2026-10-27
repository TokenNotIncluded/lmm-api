//! Local fixtures verify the adapter only; they are not supplier acceptance.
use super::*;
use crate::relay::{
    Billing, Cancellation, ClientProtocol, Relay, RelayFuture, Report, Request, RequestContext,
    Reservation, Route, RouteProvider, Settlement, UpstreamProtocol,
};
use axum::{Json, Router, body::Body, http::StatusCode, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{net::TcpListener, sync::Notify, time::Instant};

// @context-fold begin input-count-fixture-tests
fn profile() -> ResponsesCountProfile {
    ResponsesCountProfile::new(
        "https://api.openai.com/v1",
        "gpt-4.1-2025-04-14".into(),
        1024,
        128,
        NonZeroU64::new(2000).unwrap(),
    )
    .unwrap()
}

fn body() -> Value {
    json!({
        "model":"gpt-4.1-2025-04-14", "store":false, "stream":false,
        "max_output_tokens":8, "instructions":"Complete instructions: \u{4e2d}\u{6587}\n",
        "input":[
            {"role":"system","content":"System text"},
            {"role":"developer","content":[{"type":"input_text","text":"Developer \u{1f980}"}]},
            {"role":"user","content":[{"type":"input_text","text":"Prior question"}]},
            {"role":"assistant","content":[{"type":"output_text","text":"Full prior answer"}]},
            {"role":"user","content":"Next question"}
        ]
    })
}

fn frozen(profile: &ResponsesCountProfile, body: &Value, limits: &Limits) -> FrozenCountRequest {
    project(
        profile,
        &Url::parse(GENERATION_URL).unwrap(),
        &serde_json::to_vec(body).unwrap(),
        limits,
    )
    .unwrap()
}

fn gate(duration: Duration) -> Gate {
    Gate {
        cancel: Cancellation::default(),
        deadline: Instant::now() + duration,
    }
}

#[test]
fn input_count_freezes_all_text_and_generation_bytes() {
    let body = body();
    let generation = serde_json::to_vec(&body).unwrap();
    let frozen = project(
        &profile(),
        &Url::parse(GENERATION_URL).unwrap(),
        &generation,
        &Limits::default(),
    )
    .unwrap();
    let projected: Value = serde_json::from_slice(&frozen.bytes).unwrap();
    assert_eq!(
        projected,
        json!({"model":body["model"],"input":body["input"],"instructions":body["instructions"]})
    );
    assert_eq!(
        frozen.wire_sha256,
        <[u8; 32]>::from(Sha256::digest(&generation))
    );
    assert_eq!(frozen.requested_output, 8);
    assert_eq!(serde_json::from_slice::<Value>(&generation).unwrap(), body);
}

#[test]
fn input_count_projects_final_encoder_output_including_full_history() {
    let request = Request::parse(
        ClientProtocol::Responses,
        &serde_json::to_vec(&json!({
            "model":"public-alias", "max_output_tokens":8,
            "instructions":"Complete instructions: 中\n",
            "input":[
                {"role":"developer","content":"Developer 🦀"},
                {"role":"user","content":"Prior question"},
                {"role":"assistant","content":[{"type":"output_text","text":"Prior answer"}]},
                {"role":"user","content":"Next question"}
            ]
        }))
        .unwrap(),
        &Limits::default(),
    )
    .unwrap();
    request.validate_exact_input_count().unwrap();
    let route = exact_route("gpt-4.1-2025-04-14");
    let body = request.upstream_body(&route).unwrap();
    let encoded: Value = serde_json::from_slice(&body).unwrap();
    let frozen = project(&profile(), &route.endpoint, &body, &Limits::default()).unwrap();
    let counted: Value = serde_json::from_slice(&frozen.bytes).unwrap();
    assert_eq!(counted["model"], "gpt-4.1-2025-04-14");
    assert_eq!(counted["input"], encoded["input"]);
    assert_eq!(counted["input"].as_array().unwrap().len(), 5);
    assert_eq!(
        counted["input"][0]["content"][0]["text"],
        "Complete instructions: 中\n"
    );
    assert_eq!(counted["input"][3]["content"][0]["text"], "Prior answer");
    assert_eq!(frozen.wire_sha256, <[u8; 32]>::from(Sha256::digest(&body)));
    assert_eq!(encoded["max_output_tokens"], 8);
    assert_eq!(encoded["store"], false);
    assert_eq!(encoded["stream"], false);
}

fn exact_route(model: &str) -> Route {
    Route::new(
        "synthetic-exact".into(),
        UpstreamProtocol::Responses,
        GENERATION_URL,
        model.into(),
        "synthetic-not-a-provider-key",
        1,
    )
    .unwrap()
    .with_count_plan(CountPlan::OpenAiResponsesExactV1(profile()))
}

fn fixture_route(url: &Url, key: &str) -> Route {
    Route::new(
        "synthetic-local-counter".into(),
        UpstreamProtocol::Responses,
        url.as_str(),
        "gpt-4.1-2025-04-14".into(),
        key,
        1,
    )
    .unwrap()
}

fn context() -> RequestContext {
    RequestContext {
        request_id: "synthetic_count_boundary".into(),
        actor_user_id: 1,
        key_id: Some(1),
        oauth_source: None,
    }
}

// This sentinel observes the relay/adaptor boundary only. Financial behavior is
// exercised by the separate real PostgreSQL reservation tests.
#[derive(Default)]
struct LegacyAdapter(AtomicUsize);
impl Billing for LegacyAdapter {
    fn reserve<'a>(
        &'a self,
        context: &'a RequestContext,
        _: &'a Route,
        _: &'a Request,
    ) -> RelayFuture<'a, Reservation> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(Reservation {
                id: context.request_id.clone(),
            })
        })
    }

    fn finalize<'a>(&'a self, _: &'a Reservation, _: &'a Report) -> RelayFuture<'a, ()> {
        panic!("no reservation may reach finalization in a rejected count test")
    }
}

#[tokio::test]
async fn input_count_legacy_billing_cannot_silently_discard_prepared_evidence() {
    let billing = LegacyAdapter::default();
    let context = context();
    let limits = Limits::default();
    let request = Request::parse(
        ClientProtocol::Responses,
        br#"{"model":"alias","input":"hello","max_output_tokens":8}"#,
        &limits,
    )
    .unwrap();
    let route = exact_route("gpt-4.1-2025-04-14");
    let body = request.upstream_body(&route).unwrap();
    let counted = observation(
        &profile(),
        Sha256::digest(&body).into(),
        8,
        [2; 32],
        br#"{"object":"response.input_tokens","input_tokens":24}"#,
    )
    .unwrap();
    let ordinary = route.clone().with_count_plan(CountPlan::ContextMax);
    for (route, evidence) in [
        (&route, Some(&counted)),
        (&route, None),
        (&ordinary, Some(&counted)),
    ] {
        assert_eq!(
            billing
                .reserve_prepared(&context, route, &request, &body, evidence)
                .await
                .err(),
            Some(RelayError::BillingUnavailable)
        );
    }
    assert_eq!(billing.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        billing
            .reserve_prepared(&context, &ordinary, &request, &body, None)
            .await
            .unwrap()
            .id,
        context.request_id
    );
    assert_eq!(billing.0.load(Ordering::SeqCst), 1);
}

struct ExactRoutes(Route);
impl RouteProvider for ExactRoutes {
    fn select<'a>(&'a self, _: &'a RequestContext, _: &'a Request) -> RelayFuture<'a, Route> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

#[tokio::test]
async fn input_count_worker_rejects_before_reserve_or_generation_dispatch() {
    for (body, model, expected) in [
        (
            json!({"model":"alias","input":"hello","max_output_tokens":8,"stream":true}),
            "gpt-4.1-2025-04-14",
            RelayError::Unsupported("exact input count requires stateless text"),
        ),
        (
            json!({"model":"alias","input":"hello","max_output_tokens":8,"temperature":0}),
            "gpt-4.1-2025-04-14",
            RelayError::Unsupported("exact input count requires stateless text"),
        ),
        (
            json!({"model":"alias","input":"hello","max_output_tokens":129}),
            "gpt-4.1-2025-04-14",
            RelayError::Invalid("exact count generation binding mismatch"),
        ),
        (
            json!({"model":"alias","input":"hello","max_output_tokens":8}),
            "other-2025-04-14",
            RelayError::Invalid("exact count generation binding mismatch"),
        ),
    ] {
        let limits = Limits::default();
        let request = Request::parse(
            ClientProtocol::Responses,
            &serde_json::to_vec(&body).unwrap(),
            &limits,
        )
        .unwrap();
        let billing = Arc::new(LegacyAdapter::default());
        let relay = Relay::new(
            Arc::new(ExactRoutes(exact_route(model))),
            billing.clone(),
            limits,
        )
        .unwrap();
        let session = relay.start(request, context()).unwrap();
        assert_eq!(session.head.await.unwrap().err(), Some(expected.clone()));
        let completed = session.completion.await.unwrap();
        assert_eq!(completed.error, Some(expected));
        assert_eq!(completed.settlement, Settlement::NotReserved);
        assert!(!completed.report.upstream_attempted);
        assert_eq!(completed.report.attempts, 0);
        assert_eq!(billing.0.load(Ordering::SeqCst), 0);
        drop(session.body);
    }
}

#[test]
fn input_count_rejects_unreviewed_generation_projection_without_deleting_fields() {
    let profile = profile();
    let url = Url::parse(GENERATION_URL).unwrap();
    let limits = Limits::default();
    for (field, value) in [
        ("store", json!(true)),
        ("stream", json!(true)),
        ("model", json!("gpt-4.1")),
        ("max_output_tokens", json!(129)),
        ("instructions", Value::Null),
        ("tools", json!([])),
        ("previous_response_id", json!("prior")),
        ("conversation", json!("conversation")),
        ("background", json!(false)),
        ("prompt_cache_options", json!({})),
        ("text", json!({"format":{"type":"json_schema"}})),
        ("reasoning", json!({"effort":"low"})),
        ("temperature", json!(0)),
        ("input", json!([{"type":"item_reference","id":"prior"}])),
        (
            "input",
            json!([{"role":"user","content":[{"type":"input_image","image_url":"https://example.org/image"}]}]),
        ),
        (
            "input",
            json!([{"role":"user","content":[{"type":"output_text","text":"wrong role"}]}]),
        ),
    ] {
        let mut altered = body();
        altered[field] = value;
        assert!(
            project(
                &profile,
                &url,
                &serde_json::to_vec(&altered).unwrap(),
                &limits
            )
            .is_err(),
            "{field}"
        );
    }
    for field in ["store", "stream"] {
        let mut altered = body();
        altered.as_object_mut().unwrap().remove(field);
        assert!(
            project(
                &profile,
                &url,
                &serde_json::to_vec(&altered).unwrap(),
                &limits
            )
            .is_err()
        );
    }
    assert!(
        project(
            &profile,
            &Url::parse("https://third-party.example/v1/responses").unwrap(),
            &serde_json::to_vec(&body()).unwrap(),
            &limits
        )
        .is_err()
    );
}

#[test]
fn input_count_response_requires_object_integer_and_original_ceiling() {
    let profile = profile();
    for bytes in [
        r#"{}"#,
        r#"{"object":"response.input_tokens"}"#,
        r#"{"object":"other","input_tokens":1}"#,
        r#"{"object":"response.input_tokens","input_tokens":-1}"#,
        r#"{"object":"response.input_tokens","input_tokens":1.0}"#,
        r#"{"object":"response.input_tokens","input_tokens":true}"#,
        r#"{"object":"response.input_tokens","input_tokens":"1"}"#,
        r#"{"object":"response.input_tokens","input_tokens":1025}"#,
        r#"{"object":"response.input_tokens","input_tokens":18446744073709551616}"#,
    ] {
        assert!(observation(&profile, [1; 32], 8, [2; 32], bytes.as_bytes()).is_err());
    }
    for input_tokens in [0, 1024] {
        let bytes = serde_json::to_vec(
            &json!({"object":"response.input_tokens","input_tokens":input_tokens}),
        )
        .unwrap();
        let observed = observation(&profile, [1; 32], 8, [2; 32], &bytes).unwrap();
        assert_eq!(observed.input_tokens, input_tokens);
        assert_eq!(observed.api, "responses.input_tokens");
        assert_eq!(
            observed.response_sha256,
            <[u8; 32]>::from(Sha256::digest(&bytes))
        );
    }
}

#[tokio::test]
async fn input_count_context_max_never_calls_counter() {
    let gate = gate(Duration::ZERO);
    gate.cancel.cancel();
    assert!(
        execute(
            &fixture_route(&Url::parse("http://127.0.0.1:1").unwrap(), "synthetic"),
            b"not a count request",
            &Limits::default(),
            &ProviderTransport::new(1),
            &gate
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn input_count_real_local_exchange_binds_complete_projection_and_response() {
    let seen = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses/input_tokens",
        post(move |headers: axum::http::HeaderMap, bytes: Bytes| {
            assert_eq!(
                headers[header::AUTHORIZATION],
                "Bearer synthetic-counter-only"
            );
            capture.lock().unwrap().push(bytes.to_vec());
            async { Json(json!({"object":"response.input_tokens","input_tokens":24})) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/responses/input_tokens",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let profile = profile();
    let limits = Limits::default();
    let body = body();
    let frozen = frozen(&profile, &body, &limits);
    let expected_count_bytes = frozen.bytes.clone();
    let expected_wire = frozen.wire_sha256;
    let observed = count_once(
        &profile,
        &url,
        &fixture_route(&url, "synthetic-counter-only"),
        frozen,
        &limits,
        &ProviderTransport::new(1),
        &gate(Duration::from_secs(5)),
    )
    .await
    .unwrap();
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [expected_count_bytes.as_ref()]
    );
    assert_eq!(observed.input_tokens, 24);
    assert_eq!(observed.requested_output, 8);
    assert_eq!(observed.wire_sha256, expected_wire);
    assert_eq!(
        observed.request_sha256,
        <[u8; 32]>::from(Sha256::digest(&expected_count_bytes))
    );
    assert_eq!(observed.projection_revision, PROJECTION_REVISION);
    server.abort();
}

#[tokio::test]
async fn input_count_status_and_body_cap_fail_without_observation() {
    for (status, bytes) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":"synthetic"}"#.to_owned(),
        ),
        (StatusCode::OK, " ".repeat(129)),
    ] {
        let app = Router::new().route(
            "/count",
            post(move || {
                let bytes = bytes.clone();
                async move { (status, [(header::CONTENT_TYPE, "application/json")], bytes) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/count", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let limits = Limits {
            event_bytes: 128,
            ..Limits::default()
        };
        let profile = profile();
        let error = count_once(
            &profile,
            &url,
            &fixture_route(&url, "synthetic"),
            frozen(&profile, &body(), &limits),
            &limits,
            &ProviderTransport::new(1),
            &gate(Duration::from_secs(5)),
        )
        .await
        .err()
        .unwrap();
        if status == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(error, RelayError::UpstreamStatus(429));
        } else {
            assert_eq!(error, RelayError::Limit("input count response"));
        }
        server.abort();
    }
}

#[tokio::test]
async fn input_count_revoked_route_is_checked_before_and_after_connection_preparation() {
    // A real local listener must receive no HTTP bytes. The second guard check
    // deterministically models withdrawal during the awaited prepare operation.
    let hits = Arc::new(AtomicUsize::new(0));
    let requests = hits.clone();
    let app = Router::new().route(
        "/count",
        post(move || {
            requests.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"object":"response.input_tokens","input_tokens":24})) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/count", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for initially_available in [false, true] {
        let checks = Arc::new(AtomicUsize::new(0));
        let observed = checks.clone();
        let route = fixture_route(&url, "synthetic").with_availability_guard(Arc::new(move || {
            let first = observed.fetch_add(1, Ordering::SeqCst) == 0;
            initially_available && first
        }));
        let limits = Limits::default();
        let profile = profile();
        let result = count_once(
            &profile,
            &url,
            &route,
            frozen(&profile, &body(), &limits),
            &limits,
            &ProviderTransport::new(1),
            &gate(Duration::from_secs(5)),
        )
        .await;
        assert_eq!(result.err(), Some(RelayError::RouteUnavailable));
        assert_eq!(
            checks.load(Ordering::SeqCst),
            if initially_available { 2 } else { 1 }
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
    server.abort();
}

#[tokio::test]
async fn input_count_header_wait_obeys_count_timeout_total_deadline_and_cancellation() {
    for mode in ["count_timeout", "total_deadline", "cancel"] {
        let started = Arc::new(Notify::new());
        let notify = started.clone();
        let app = Router::new().route(
            "/count",
            post(move || {
                notify.notify_one();
                async { std::future::pending::<Body>().await }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/count", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let limits = Limits::default();
        let mut profile = profile();
        let gate = gate(if mode == "total_deadline" {
            Duration::from_millis(40)
        } else {
            Duration::from_secs(5)
        });
        if mode == "count_timeout" {
            profile.count_timeout = Duration::from_millis(40);
        }
        if mode == "total_deadline" {
            profile.count_timeout = Duration::from_millis(u64::MAX);
        }
        let cancel = gate.cancel.clone();
        let cancel_task = tokio::spawn(async move {
            started.notified().await;
            if mode == "cancel" {
                cancel.cancel();
            }
        });
        let error = count_once(
            &profile,
            &url,
            &fixture_route(&url, "synthetic"),
            frozen(&profile, &body(), &limits),
            &limits,
            &ProviderTransport::new(1),
            &gate,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(
            error,
            match mode {
                "count_timeout" => RelayError::Timeout(Phase::Headers),
                "total_deadline" => RelayError::Timeout(Phase::Total),
                _ => RelayError::Cancelled,
            }
        );
        cancel_task.abort();
        server.abort();
    }
}
// @context-fold end input-count-fixture-tests
