use super::*;
use axum::body::Body;
use tower::ServiceExt;

fn state() -> EdgeAccess {
    EdgeAccess {
        pool: sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap(),
        proof: Arc::new(vec![b'p'; 48]),
        limiter: Arc::new(Mutex::new(Limiter::default())),
    }
}
fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        PROOF_HEADER,
        HeaderValue::from_static("pppppppppppppppppppppppppppppppppppppppppppppppp"),
    );
    headers.insert(CLIENT_HEADER, HeaderValue::from_static("203.0.113.7"));
    headers
}
#[tokio::test]
async fn physical_peer_and_purpose_proof_are_both_required() {
    let state = state();
    let local = "127.0.0.1:10000".parse().unwrap();
    let remote = "198.51.100.8:10000".parse().unwrap();
    let mut h = headers();
    assert_eq!(
        edge_peer(&state, &h, local),
        Some("203.0.113.7".parse().unwrap())
    );
    assert_eq!(edge_peer(&state, &h, remote), None);
    h.remove(PROOF_HEADER);
    assert_eq!(edge_peer(&state, &h, local), None); // Go has no edge credential.
    h.insert(PROOF_HEADER, HeaderValue::from_static("wrong"));
    assert_eq!(edge_peer(&state, &h, local), None);
    h = headers();
    h.append(PROOF_HEADER, h[PROOF_HEADER].clone());
    assert_eq!(edge_peer(&state, &h, local), None);
    h = headers();
    for value in [
        "203.0.113.7, 127.0.0.1",
        "203.0.113.7:80",
        " 203.0.113.7",
        "fe80::1%eth0",
    ] {
        h.insert(CLIENT_HEADER, HeaderValue::from_str(value).unwrap());
        assert_eq!(edge_peer(&state, &h, local), None, "{value}");
    }
    assert_eq!(
        canonical_ip("::ffff:203.0.113.7"),
        canonical_ip("203.0.113.7")
    );
}
#[tokio::test]
async fn middleware_replaces_client_peer_preserves_transport_and_removes_edge_headers() {
    async fn inspect(request: Request) -> StatusCode {
        assert_eq!(
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .unwrap()
                .0
                .ip(),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        assert!(
            request
                .extensions()
                .get::<OriginalSocketPeer>()
                .unwrap()
                .0
                .ip()
                .is_loopback()
        );
        assert!(!request.headers().contains_key(PROOF_HEADER));
        assert!(!request.headers().contains_key(CLIENT_HEADER));
        StatusCode::NO_CONTENT
    }
    let app = state().protect(Router::new().route("/inspect", get(inspect)));
    let mut request = Request::builder()
        .uri("/inspect")
        .body(Body::empty())
        .unwrap();
    *request.headers_mut() = headers();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:10000".parse::<SocketAddr>().unwrap(),
    ));
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    let request = Request::builder()
        .uri("/inspect")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}
#[test]
fn ordered_rules_and_unknown_country_preserve_fail_closed_semantics() {
    let policy =
        Policy::parse("dip(192.0.2.7) -> direct\ndip(geoip:cn) -> reject\nfallback: direct")
            .unwrap();
    let except = "192.0.2.7".parse().unwrap();
    let other = "198.51.100.7".parse().unwrap();
    assert!(policy.evaluate(except, None).unwrap() == Action::Direct);
    assert!(policy.evaluate(other, Some("CN")).unwrap() == Action::Reject);
    assert!(policy.evaluate(other, Some("US")).unwrap() == Action::Direct);
    assert!(policy.evaluate(other, None).is_err());
    for invalid in [
        "",
        "dip(geoip:china) -> direct",
        "dip(192.0.2.0/24) -> proxy",
        "domain(example.com) -> direct",
        "dip(192.0.2.1) && dip(geoip:cn) -> direct",
        "fallback: direct\nfallback: reject",
    ] {
        assert!(Policy::parse(invalid).is_err(), "{invalid}");
    }
    assert!(Policy::parse(&"x".repeat(16385)).is_err());
}
#[test]
fn country_and_native_bearer_are_unambiguous() {
    let mut h = HeaderMap::new();
    assert!(country(&h).unwrap().is_none());
    h.insert("x-lmm-cn-source", HeaderValue::from_static("1"));
    assert_eq!(country(&h).unwrap().as_deref(), Some("CN"));
    h.insert("x-lmm-edge-country", HeaderValue::from_static("us"));
    assert_eq!(country(&h).unwrap().as_deref(), Some("US"));
    h.append("x-lmm-edge-country", HeaderValue::from_static("cn"));
    assert!(country(&h).is_err());
    h.clear();
    h.insert(
        "authorization",
        HeaderValue::from_static("Bearer sk-opaque-group"),
    );
    assert_eq!(bearer(&h), Some("sk-opaque-group"));
    h.append("authorization", HeaderValue::from_static("Bearer second"));
    assert!(bearer(&h).is_none());
}
#[test]
fn explicit_opt_in_and_frozen_eligibility_are_separate_requirements() {
    for invalid in [
        r#"{"allow_key_bypass_ip_policy":"true"}"#,
        r#"{"allow_key_bypass_ip_policy":1}"#,
        r#"{"allow_key_bypass_ip_policy":true,"allow_key_bypass_ip_policy":false}"#,
    ] {
        assert!(serde_json::from_str::<OptIn>(invalid).is_err());
    }
    assert!(
        !serde_json::from_str::<OptIn>(r#"{"developer_access":true}"#)
            .unwrap()
            .allow_key_bypass_ip_policy
    );
    let facts = json!({"version":1,"users":[{"user_id":7,"minimum_trust_level":1,"basis":"source_console_activation","source_facts_sha256":"a".repeat(64)}]});
    let eligibility: Eligibility = serde_json::from_value(facts.clone()).unwrap();
    assert!(eligibility.allows(7));
    assert!(!eligibility.allows(8));
    let mut bad = facts.clone();
    bad["users"][0]["minimum_trust_level"] = json!(0);
    assert!(
        !serde_json::from_value::<Eligibility>(bad)
            .unwrap()
            .allows(7)
    );
    let mut bad = facts.clone();
    bad["users"][0]["basis"] = json!("developer_access");
    assert!(serde_json::from_value::<Eligibility>(bad).is_err());
    let mut bad = facts.clone();
    bad["users"]
        .as_array_mut()
        .unwrap()
        .push(facts["users"][0].clone());
    assert!(
        !serde_json::from_value::<Eligibility>(bad)
            .unwrap()
            .allows(7)
    );
}
#[test]
fn failed_bypass_is_bounded_and_success_is_not_cached() {
    let limiter = Arc::new(Mutex::new(Limiter::default()));
    let ip = "192.0.2.1".parse().unwrap();
    for _ in 0..10 {
        drop(Reservation::take(limiter.clone(), ip).unwrap());
    }
    assert!(Reservation::take(limiter.clone(), ip).is_none());
    let other = "192.0.2.2".parse().unwrap();
    for _ in 0..1000 {
        Reservation::take(limiter.clone(), other).unwrap().success();
    }
    let state = limiter.lock().unwrap();
    assert_eq!(state.global.failures, 10);
    assert_eq!(state.global.active, 0);
    assert_eq!(state.sources[&other].failures, 0);
}
#[test]
fn error_response_contract_conceals_api_and_has_bounded_negotiation() {
    for api in [
        "/api",
        "/api/user/self",
        "/v1/chat/completions",
        "/typesafe/v1/system-one",
        "/core/v2/admin/abuse/users",
        "/x/mj/a",
    ] {
        assert!(concealed(api), "{api}");
    }
    for page in ["/", "/console", "/oauth/mj", "/static/mj"] {
        assert!(!concealed(page), "{page}");
    }
    assert!(wants_json("text/html, application/problem+json;q=0.8"));
    assert!(!wants_json("application/json;q=0"));
    assert!(!wants_json("application/json;q=garbage"));
    let page = render_error(true, "safe-request-id");
    assert!(!page.contains("{{."));
    assert!(page.contains("safe-request-id"));
}
#[tokio::test]
async fn error_endpoint_requires_authenticated_internal_marker_and_preserves_451_404() {
    let app = state().protect(Router::new());
    for (path, status) in [
        ("/console", StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS),
        ("/api/user/self", StatusCode::NOT_FOUND),
    ] {
        let mut request = Request::builder()
            .uri("/internal/errors/access-policy")
            .body(Body::empty())
            .unwrap();
        *request.headers_mut() = headers();
        request.headers_mut().insert(
            "x-lmm-internal-error",
            HeaderValue::from_static("access-policy"),
        );
        request
            .headers_mut()
            .insert("x-lmm-access-policy", HeaderValue::from_static("denied"));
        request
            .headers_mut()
            .insert("x-lmm-original-uri", HeaderValue::from_static(path));
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:10000".parse::<SocketAddr>().unwrap(),
        ));
        assert_eq!(app.clone().oneshot(request).await.unwrap().status(), status);
    }
}
