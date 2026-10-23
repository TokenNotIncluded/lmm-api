//! Independent HTTP rejection checks. Lazy pool: no database is contacted.
//! Positive authorization and ledger persistence require the separate PG tests.
use axum::{
    body::{Body, to_bytes},
    http::{HeaderName, HeaderValue, Request, StatusCode},
};
use lmm_core::wallet::{Store, http};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower::ServiceExt;

fn store() -> Store {
    Store::new(
        PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(20))
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1:1/unreachable")
            .unwrap(),
    )
}

// @mark wallet.header-rejection role=test
#[tokio::test]
async fn duplicate_host_and_user_credentials_fail_before_database_access() {
    let token = "a".repeat(32);
    for header_name in ["authorization", "x-lmm-user-credential"] {
        for bad_first in [false, true] {
            for second in ["invalid", ""] {
                let router = http::internal_router(store(), token.as_bytes().to_vec()).unwrap();
                let mut request = Request::post("/internal/wallet/v1/prepare")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-lmm-user-credential", "synthetic-session")
                    .body(Body::from(r#"{"amount_credits":1,"request_key":"independent-key-01","token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#))
                    .unwrap();
                let value = HeaderValue::from_str(second).unwrap();
                if bad_first {
                    let original = request.headers_mut().remove(header_name).unwrap();
                    request
                        .headers_mut()
                        .insert(HeaderName::from_static(header_name), value);
                    request
                        .headers_mut()
                        .append(HeaderName::from_static(header_name), original);
                } else {
                    request
                        .headers_mut()
                        .append(HeaderName::from_static(header_name), value);
                }
                let response = router.oneshot(request).await.unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::UNAUTHORIZED,
                    "{header_name}, bad_first={bad_first}"
                );
                assert_eq!(response.headers()["cache-control"], "no-store");
                let body = to_bytes(response.into_body(), 1024).await.unwrap();
                assert!(!String::from_utf8_lossy(&body).contains("synthetic-session"));
            }
        }
    }
}

// @mark wallet.header-rejection role=test
#[tokio::test]
async fn duplicate_origin_fails_before_cookie_or_database_checks() {
    for bad_first in [false, true] {
        let router = http::browser_router(store(), "https://wallet.example".into()).unwrap();
        let (first, second) = if bad_first {
            ("https://evil.example", "https://wallet.example")
        } else {
            ("https://wallet.example", "https://evil.example")
        };
        let mut request = Request::post("/core/v1/wallet/confirm/create/1")
            .header("content-type", "application/json")
            .header("origin", first)
            .body(Body::from(
                r#"{"csrf":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            ))
            .unwrap();
        request
            .headers_mut()
            .append("origin", HeaderValue::from_static(second));
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}
