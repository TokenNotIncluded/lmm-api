use crate::log_redact::{access_log_line, sanitize_url_for_log};

fn log_target(target: &str) -> String {
    access_log_line("GET", 401, target)
        .expect("a query-bearing request still has an access diagnostic")
}

fn assert_no_secret(output: &str, secrets: &[&str]) {
    for secret in secrets {
        assert!(
            !output.contains(secret),
            "credential {secret:?} escaped redaction: {output}"
        );
    }
    let target = output
        .split_once(" target=")
        .map_or(output, |(_, target)| target);
    let absolute = if target.starts_with('/') {
        format!("https://example.invalid{target}")
    } else {
        target.to_owned()
    };
    if let Ok(url) = reqwest::Url::parse(&absolute) {
        for (key, value) in url.query_pairs() {
            for secret in secrets {
                assert!(
                    !key.contains(secret) && !value.contains(secret),
                    "credential remained after normal URL decoding: {output}"
                );
            }
        }
    }
}

fn assert_single_line(output: &str) {
    assert!(
        !output
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        "log output contains a line/control separator: {output:?}"
    );
}

#[test]
fn independent_966_actual_and_other_oauth_paths_never_log_code_or_state() {
    for path in [
        "/core/v1/auth/oauth/google/callback",
        "/api/oauth2/authorize",
        "/oauth/callback",
        "/oauth",
        "/api/oauth",
        "/unrecognized-login/callback",
    ] {
        for status in [200, 401, 404, 500] {
            let target = format!("{path}?code=synthetic-code-966&state=synthetic-state-966");
            let line = access_log_line("GET", status, &target).expect("access diagnostic");
            assert_no_secret(&line, &["synthetic-code-966", "synthetic-state-966"]);
            assert_single_line(&line);
        }
    }
}

#[test]
fn independent_966_decoded_and_duplicate_credential_values_are_hidden() {
    let line = log_target(
        "/v1/models?%6bey=synthetic-first-966&KEY=synthetic-second-966&api%5Fkey=synthetic-third-966&+token+=synthetic-fourth-966&authorization=synthetic-fifth-966",
    );
    assert_no_secret(
        &line,
        &[
            "synthetic-first-966",
            "synthetic-second-966",
            "synthetic-third-966",
            "synthetic-fourth-966",
            "synthetic-fifth-966",
        ],
    );
}

#[test]
fn independent_966_unknown_query_values_and_bare_query_credentials_are_hidden() {
    for target in [
        "/v1?credential=synthetic-credential-966",
        "/v1?authz=synthetic-authz-966",
        "/v1?unrecognized=synthetic-value-966",
        "/v1?synthetic-bare-bearer-966",
        "/v1?synthetic-secret-key-966=unused",
    ] {
        let line = log_target(target);
        assert_no_secret(
            &line,
            &[
                "synthetic-credential-966",
                "synthetic-authz-966",
                "synthetic-value-966",
                "synthetic-bare-bearer-966",
                "synthetic-secret-key-966",
            ],
        );
    }
}

#[test]
fn independent_966_invalid_encoding_and_parse_failures_do_not_return_credentials() {
    for target in [
        "/v1?key=synthetic-bad-966%ZZ",
        "/v1?credential=synthetic-bad-966%",
        "/v1?%ZZ=synthetic-bad-966",
        "/v1?key=synthetic-bad-966;alt=sse",
        "/bad%ZZ?key=synthetic-bad-966",
        "/v1?key=synthetic-bad-966%FF",
    ] {
        assert_no_secret(&log_target(target), &["synthetic-bad-966"]);
        let absolute = format!("https://example.invalid{target}");
        assert_no_secret(&sanitize_url_for_log(&absolute), &["synthetic-bad-966"]);
    }
    for malformed in [
        "https://example.invalid/v1?key=synthetic-bad-966\n",
        "not-a-valid-url synthetic-bad-966",
    ] {
        let output = sanitize_url_for_log(malformed);
        assert_no_secret(&output, &["synthetic-bad-966"]);
        assert_single_line(&output);
    }
}

#[test]
fn independent_966_absolute_url_userinfo_and_fragment_are_not_log_data() {
    let output = sanitize_url_for_log(
        "https://synthetic-user-966:synthetic-password-966@example.invalid/v1?alt=sse#access_token=synthetic-fragment-966",
    );
    assert_no_secret(
        &output,
        &[
            "synthetic-user-966",
            "synthetic-password-966",
            "synthetic-fragment-966",
        ],
    );
    if let Ok(url) = reqwest::Url::parse(&output) {
        assert!(url.username().is_empty());
        assert!(url.password().is_none());
        assert!(url.fragment().is_none());
    }
    assert_single_line(&output);
}

#[test]
fn independent_966_paths_and_methods_cannot_inject_a_log_record() {
    for target in [
        "/oauth/a%0D%0Ainjected?key=synthetic-key-966",
        "/oauth/a%E2%80%A8injected?code=synthetic-key-966",
        "/oauth/a%E2%80%A9injected?code=synthetic-key-966",
        "/v1/a%20b?key=synthetic-key-966",
    ] {
        let output = access_log_line("GET\r\ninjected", 404, target).expect("access diagnostic");
        assert_single_line(&output);
        assert_no_secret(&output, &["synthetic-key-966"]);
        assert!(!output.contains("a b"));
    }
    let output = sanitize_url_for_log(
        "https://example.invalid/oauth/a%E2%80%A8injected?code=synthetic-key-966",
    );
    assert_single_line(&output);
    assert_no_secret(&output, &["synthetic-key-966"]);
}

#[test]
fn independent_966_useful_bounded_operational_values_remain_visible() {
    let output = log_target(
        "/v1/models?model=gpt-test-966&alt=sse&api-version=2024-02-01&stream=true&key=synthetic-key-966",
    );
    for operational in ["gpt-test-966", "sse", "2024-02-01", "true"] {
        assert!(
            output.contains(operational),
            "lost operational value: {output}"
        );
    }
    assert_no_secret(&output, &["synthetic-key-966"]);
    assert!(output.starts_with("core_access method=GET status=401 target="));
    assert!(access_log_line("GET", 200, "/health/live").is_none());
}

#[test]
fn independent_966_truncation_is_bounded_after_redaction() {
    let target = format!(
        "/{}?key=synthetic-tail-key-966&unrecognized=synthetic-tail-value-966",
        "中".repeat(800)
    );
    let line = log_target(&target);
    assert_no_secret(
        &line,
        &["synthetic-tail-key-966", "synthetic-tail-value-966"],
    );
    assert_single_line(&line);
    // The original target contract was 1024 characters. A byte-bounded target
    // plus its fixed method/status prefix fits within this conservative margin.
    assert!(line.len() <= 1152, "unbounded log bytes: {}", line.len());
}

#[test]
fn independent_966_real_middleware_redacts_loopback_access_logs() {
    const CHILD_ENV: &str = "LMM_TEST_20261027_PR966_MIDDLEWARE_CHILD";
    const CHILD_MARKER: &str = "real-axum-loopback-fixture";
    const COMPLETED: &str = "independent_966_real_middleware_http_requests_completed";

    if std::env::var(CHILD_ENV).as_deref() == Ok(CHILD_MARKER) {
        use axum::{Router, http::StatusCode, middleware, routing::get};
        use std::time::Duration;

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                    .await
                    .unwrap();
                let address = listener.local_addr().unwrap();
                let app = Router::new()
                    .route(
                        "/core/v1/auth/oauth/google/callback",
                        get(|| async { (StatusCode::UNAUTHORIZED, "fixture callback denied") }),
                    )
                    .route("/v1/fixture-ok", get(|| async { "fixture success" }))
                    .route("/health/live", get(|| async { "fixture ready" }))
                    .layer(middleware::from_fn(
                        crate::log_redact::log_redacted_request_target,
                    ));
                let (shutdown, stopped) = tokio::sync::oneshot::channel::<()>();
                let server = tokio::spawn(async move {
                    axum::serve(listener, app)
                        .with_graceful_shutdown(async move {
                            stopped.await.unwrap();
                        })
                        .await
                        .unwrap();
                });
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(3))
                    .build()
                    .unwrap();
                for (path, status, body) in [
                    (
                        "/core/v1/auth/oauth/google/callback?code=synthetic-runtime-code-401&state=synthetic-runtime-state-401&unknown=synthetic-runtime-value-401&synthetic-runtime-key-401=unused&model=gpt-runtime-966",
                        StatusCode::UNAUTHORIZED,
                        Some("fixture callback denied"),
                    ),
                    (
                        "/unregistered-966?code=synthetic-runtime-code-404&state=synthetic-runtime-state-404&unknown=synthetic-runtime-value-404&synthetic-runtime-key-404=unused&model=gpt-runtime-966",
                        StatusCode::NOT_FOUND,
                        None,
                    ),
                    (
                        "/v1/fixture-ok?code=synthetic-runtime-code-200&state=synthetic-runtime-state-200&unknown=synthetic-runtime-value-200&synthetic-runtime-key-200=unused&model=gpt-runtime-966",
                        StatusCode::OK,
                        Some("fixture success"),
                    ),
                    ("/health/live", StatusCode::OK, Some("fixture ready")),
                ] {
                    let response = client
                        .get(format!("http://{address}{path}"))
                        .header(reqwest::header::CONNECTION, "close")
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(response.status(), status);
                    let actual_body = response.text().await.unwrap();
                    if let Some(expected_body) = body {
                        assert_eq!(actual_body, expected_body);
                    }
                }
                drop(client);
                shutdown.send(()).unwrap();
                tokio::time::timeout(Duration::from_secs(3), server)
                    .await
                    .unwrap()
                    .unwrap();
            });
        println!("\n{COMPLETED}");
        return;
    }

    // Libtest omits the crate name from its exact test path. The completion
    // marker below also prevents a mistyped filter from yielding a false pass.
    let full_name = concat!(
        module_path!(),
        "::independent_966_real_middleware_redacts_loopback_access_logs"
    );
    let (_, exact_name) = full_name.split_once("::").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", exact_name, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env(CHILD_ENV, CHILD_MARKER)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        output.status.success(),
        "middleware child failed: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert!(
        stdout.lines().any(|line| line == COMPLETED),
        "child did not execute the HTTP fixture:\n{stdout}\n{stderr}"
    );
    assert_no_secret(&stderr, &["synthetic-runtime-"]);
    let lines: Vec<_> = stderr
        .lines()
        .filter(|line| line.starts_with("core_access "))
        .collect();
    assert_eq!(
        lines,
        [
            "core_access method=GET status=401 target=/core/v1/auth/oauth/google/callback?model=gpt-runtime-966",
            "core_access method=GET status=404 target=/unregistered-966?model=gpt-runtime-966",
            "core_access method=GET status=200 target=/v1/fixture-ok?model=gpt-runtime-966",
        ],
        "query-less health request must remain silent; each real query request needs its final status"
    );
    for line in lines {
        assert_single_line(line);
    }
    eprintln!("captured real middleware child stderr:\n{stderr}");
}
