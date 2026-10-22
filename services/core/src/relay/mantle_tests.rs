use super::*;
use crate::relay::{Route, UpstreamProtocol};
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};

// AWS's published example credentials are public test data, never live keys.
const CREDENTIALS: &str = r#"{
  "access_key_id": "AKIAIOSFODNN7EXAMPLE",
  "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
}"#;
const ENDPOINT: &str = "https://bedrock-mantle.us-east-1.api.aws/anthropic/v1/messages";
const BODY: &[u8] = br#"{"model":"anthropic.claude-test","max_tokens":16,"messages":[{"role":"user","content":"hello"}]}"#;

fn at() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_440_938_160)
}

#[test]
fn endpoint_is_normalized_once_and_bound_to_region_host_and_operation() {
    for suffix in [
        "",
        "/",
        "/anthropic",
        "/anthropic/",
        "/anthropic/v1",
        "/anthropic/v1/",
        "/anthropic/v1/messages",
        "/anthropic/v1/messages/",
    ] {
        let base = format!("https://bedrock-mantle.us-east-1.api.aws{suffix}");
        validate_mantle_endpoint(&base, "us-east-1").unwrap();
        let signer = MantleSigner::new(&base, "us-east-1", CREDENTIALS).unwrap();
        assert_eq!(signer.endpoint().as_str(), ENDPOINT);
    }
    for value in [
        "http://bedrock-mantle.us-east-1.api.aws",
        "https://bedrock-mantle.us-east-1.api.aws.evil.test",
        "https://bedrock-mantle.us-east-1.api.aws:8443",
        "https://bedrock-mantle.eu-west-1.api.aws",
        "https://user@bedrock-mantle.us-east-1.api.aws",
        "https://bedrock-runtime.us-east-1.amazonaws.com",
        "https://bedrock-mantle.us-east-1.api.aws/anthropic/v1/messages/count_tokens",
        "https://bedrock-mantle.us-east-1.api.aws/v1/messages",
        "https://bedrock-mantle.us-east-1.api.aws/anthropic?region=other",
        "https://bedrock-mantle.us-east-1.api.aws/anthropic#ignored",
    ] {
        assert!(validate_mantle_endpoint(value, "us-east-1").is_err());
        assert!(
            validate_mantle_binding(value, "us-east-1", CREDENTIALS).is_err(),
            "{value}"
        );
    }
    for region in [
        "US-east-1",
        "us-east-1/path",
        "us--1",
        "us-east-",
        "us-east-a",
        "",
    ] {
        assert!(validate_mantle_endpoint(ENDPOINT, region).is_err());
        assert!(validate_mantle_binding(ENDPOINT, region, CREDENTIALS).is_err());
    }
}

#[test]
fn official_signer_matches_an_independently_calculated_fixed_request_vector() {
    // Fixed canonical request, 2015-08-30T12:36Z. Expected HMAC was computed
    // independently from the AWS SigV4 canonical-request specification; the
    // production implementation delegates all signing to aws-sigv4.
    let signer = MantleSigner::new(ENDPOINT, "us-east-1", CREDENTIALS).unwrap();
    let headers = signer
        .headers(signer.endpoint(), BODY, false, at())
        .unwrap();
    assert_eq!(headers["x-amz-date"], "20150830T123600Z");
    assert_eq!(
        headers["x-amz-content-sha256"],
        "fc467d3f7a09c48d32148bcaa6dcee6f71cc7f0810db4066239c38b01d00e076"
    );
    assert_eq!(
        headers[header::AUTHORIZATION],
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20150830/us-east-1/bedrock-mantle/aws4_request, SignedHeaders=accept;accept-encoding;anthropic-version;content-type;host;x-amz-content-sha256;x-amz-date, Signature=6c090f2205a7bd8fc8d0189f427f195efe694c19986a286a4ed7af8b91214648"
    );
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert!(headers[header::AUTHORIZATION].is_sensitive());
    assert!(headers.get("x-api-key").is_none());
    assert!(headers.get("x-amz-security-token").is_none());
}

#[test]
fn session_token_stream_and_exact_body_bytes_participate_in_the_signature() {
    let document=json!({"access_key_id":"AKIAIOSFODNN7EXAMPLE","secret_access_key":"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY","session_token":"public-test-session-token"}).to_string();
    let signer = MantleSigner::new(ENDPOINT, "us-east-1", &document).unwrap();
    let original = signer
        .headers(signer.endpoint(), BODY, false, at())
        .unwrap();
    assert_eq!(
        original["x-amz-security-token"],
        "public-test-session-token"
    );
    assert!(original["x-amz-security-token"].is_sensitive());
    assert!(
        original[header::AUTHORIZATION]
            .to_str()
            .unwrap()
            .contains("x-amz-security-token")
    );
    let stream = signer.headers(signer.endpoint(), BODY, true, at()).unwrap();
    assert_eq!(stream[header::ACCEPT], "text/event-stream");
    assert_ne!(
        stream[header::AUTHORIZATION],
        original[header::AUTHORIZATION]
    );
    let changed = [BODY, b" "].concat();
    let changed = signer
        .headers(signer.endpoint(), &changed, false, at())
        .unwrap();
    assert_ne!(
        changed["x-amz-content-sha256"],
        original["x-amz-content-sha256"]
    );
    assert_ne!(
        changed[header::AUTHORIZATION],
        original[header::AUTHORIZATION]
    );
    let later = signer
        .headers(
            signer.endpoint(),
            BODY,
            false,
            at() + Duration::from_secs(1),
        )
        .unwrap();
    assert_ne!(
        later[header::AUTHORIZATION],
        original[header::AUTHORIZATION]
    );
    let wrong =
        Url::parse("https://bedrock-mantle.eu-west-1.api.aws/anthropic/v1/messages").unwrap();
    assert!(signer.headers(&wrong, BODY, false, at()).is_err());
}

#[test]
fn secret_document_is_strict_redacted_and_never_an_api_key_fallback() {
    for bad in [
        r#"{"access_key_id":"AKIAIOSFODNN7EXAMPLE","secret_access_key":"first","secret_access_key":"second"}"#,
        r#"{"access_key_id":"AKIAIOSFODNN7EXAMPLE","secret_access_key":"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY","region":"us-east-1"}"#,
        r#"{"access_key_id":"AKIAIOSFODNN7EXAMPLE","secret_access_key":"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY","session_token":"line\nbreak"}"#,
        "Bearer ordinary-api-key",
    ] {
        assert!(validate_mantle_binding(ENDPOINT, "us-east-1", bad).is_err());
        assert!(mantle_credential_reuses(bad, "unrelated").is_err());
        assert!(mantle_credential_fingerprints(bad).is_err());
    }
    let route = Route::new_mantle(
        "aws-test".into(),
        ENDPOINT,
        "anthropic.claude-test".into(),
        "us-east-1",
        CREDENTIALS,
        1,
    )
    .unwrap();
    assert_eq!(route.protocol, UpstreamProtocol::Anthropic);
    assert!(route.credential.as_bytes().is_empty());
    let debug = format!("{route:?} {:?}", route.mantle.as_ref().unwrap());
    assert!(!debug.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(!debug.contains("EXAMPLEKEY"));
    assert!(!debug.contains("access_key_id"));
    assert!(route.clone().with_deepseek_parameters().is_err());
    assert!(
        route
            .with_search_contract(
                crate::routing::SearchProvider::AnthropicMessagesV1,
                crate::routing::SearchLimits {
                    calls: Some(1),
                    ..Default::default()
                }
            )
            .is_err()
    );
}

#[test]
fn credential_reuse_checks_every_parsed_field_without_matching_prefixes_or_absence() {
    let document = json!({
        "access_key_id":"AKIAIOSFODNN7EXAMPLE",
        "secret_access_key":"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "session_token":"public-test-session-token"
    })
    .to_string();
    for candidate in [
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "public-test-session-token",
    ] {
        assert!(mantle_credential_reuses(&document, candidate).unwrap());
    }
    for candidate in [
        "",
        "AKIAIOSFODNN7EXAMPL",
        "xJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "public-test-session-toke",
        "public-test-session-token-suffix",
        "different-private-credential",
    ] {
        assert!(!mantle_credential_reuses(&document, candidate).unwrap());
    }
    assert!(!mantle_credential_reuses(CREDENTIALS, "").unwrap());
    assert!(!mantle_credential_reuses(CREDENTIALS, "public-test-session-token").unwrap());
    // JSON escaping must not disguise equality from the isolation check.
    let escaped = CREDENTIALS.replace("EXAMPLE", "\\u0045XAMPLE");
    assert_eq!(mantle_credential_fingerprints(&document).unwrap().len(), 3);
    let fingerprints = mantle_credential_fingerprints(CREDENTIALS).unwrap();
    assert_eq!(fingerprints.len(), 2);
    assert_eq!(
        fingerprints,
        mantle_credential_fingerprints(&escaped).unwrap()
    );
    assert!(mantle_credential_reuses(&escaped, "AKIAIOSFODNN7EXAMPLE").unwrap());
    assert!(
        mantle_credential_reuses(&escaped, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY").unwrap()
    );
}

#[test]
fn signer_cannot_emit_credentials_to_an_enabled_trace_subscriber() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Counter(Arc<AtomicUsize>);
    impl tracing::Subscriber for Counter {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
            Some(tracing::level_filters::LevelFilter::TRACE)
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, _: &tracing::Event<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
    let events = Arc::new(AtomicUsize::new(0));
    tracing::subscriber::with_default(Counter(events.clone()), || {
        tracing::info!("before signing");
        let document = json!({
            "access_key_id":"AKIAIOSFODNN7EXAMPLE",
            "secret_access_key":"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "session_token":"public-test-session-token"
        })
        .to_string();
        let signer = MantleSigner::new(ENDPOINT, "us-east-1", &document).unwrap();
        signer
            .headers(signer.endpoint(), BODY, false, at())
            .unwrap();
        tracing::info!("after signing");
    });
    assert_eq!(events.load(Ordering::Relaxed), 2);
}
