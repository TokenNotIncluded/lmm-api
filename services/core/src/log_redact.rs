//! A conservative request-target log derived from QuantumNous/new-api
//! `30748c6558bc97de49ff74871cfec717e50f3322`.
//!
//! Only bounded `model`, `alt`, `api-version` and `stream` values are retained.
//! Every other query member, including its key, is omitted: OAuth codes,
//! state, unknown credential names and bare query tokens are never logged.
//! Parsing uses the existing reqwest::Url implementation. Invalid input loses
//! the complete target rather than returning unredacted bytes. URL userinfo
//! and fragments are always removed, and path escaping is preserved.

use axum::{extract::Request, http::Uri, middleware::Next, response::Response};
use reqwest::Url;

const MAX_URL_BYTES: usize = 16_384;
const MAX_TARGET_BYTES: usize = 1024;
const REDACTED: &str = "[redacted-target]";
const ORIGIN: &str = "https://redacted.invalid";

/// Known credential names; the logger also omits all unlisted query members.
pub fn is_sensitive_query_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    matches!(
        key.as_str(),
        "key"
            | "api_key"
            | "api-key"
            | "apikey"
            | "x-api-key"
            | "authorization"
            | "auth"
            | "password"
            | "passwd"
            | "sig"
            | "awsaccesskeyid"
            | "x-amz-credential"
            | "code"
            | "state"
    ) || key.contains("token")
        || key.contains("secret")
        || key.contains("signature")
}

/// Remove userinfo, fragments and all query members outside the finite
/// operational allowlist. Malformed or oversized input returns a fixed marker.
pub fn sanitize_url_for_log(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if !valid_input(raw) {
        return REDACTED.to_owned();
    }
    if raw.starts_with('?') {
        return sanitize(raw, Form::Query);
    }
    if raw.starts_with("//") {
        return sanitize(raw, Form::Network);
    }
    if raw.starts_with('/') {
        return sanitize(raw, Form::Target);
    }
    sanitize(raw, Form::Absolute)
}

/// Log an origin-form HTTP target. Unknown/credential query keys and values
/// are omitted on every path, including OAuth callbacks and unknown routes.
pub fn logged_request_target(target: &str) -> String {
    if target == "*" {
        return "*".to_owned();
    }
    if !target.starts_with('/') || !valid_input(target) {
        return REDACTED.to_owned();
    }
    sanitize(target, Form::Target)
}

/// A single bounded stderr line; ordinary query-less requests remain quiet.
pub fn access_log_line(method: &str, status: u16, target: &str) -> Option<String> {
    if !target.contains('?') {
        return None;
    }
    Some(format_line(method, status, &logged_request_target(target)))
}

/// Redact before moving the request into the handler. Only the already-safe
/// target is retained while waiting for the final response status.
pub async fn log_redacted_request_target(request: Request, next: Next) -> Response {
    let method = safe_method(request.method().as_str()).to_owned();
    let target = request.uri().path_and_query().and_then(|target| {
        request
            .uri()
            .query()
            .map(|_| logged_request_target(target.as_str()))
    });
    let response = next.run(request).await;
    if let Some(target) = target {
        eprintln!(
            "{}",
            format_line(&method, response.status().as_u16(), &target)
        );
    }
    response
}

fn safe_method(method: &str) -> &str {
    match method {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" | "CONNECT" | "TRACE" => {
            method
        }
        _ => "OTHER",
    }
}

fn format_line(method: &str, status: u16, target: &str) -> String {
    let method = safe_method(method);
    format!("core_access method={method} status={status} target={target}")
}

#[derive(Clone, Copy)]
enum Form {
    Target,
    Absolute,
    Network,
    Query,
}

fn valid_input(raw: &str) -> bool {
    if raw.len() > MAX_URL_BYTES || raw.chars().any(char::is_control) {
        return false;
    }
    let bytes = raw.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            if bytes.get(at + 1).is_none_or(|b| !b.is_ascii_hexdigit())
                || bytes.get(at + 2).is_none_or(|b| !b.is_ascii_hexdigit())
            {
                return false;
            }
            at += 3;
        } else {
            at += 1;
        }
    }
    true
}

fn sanitize(raw: &str, form: Form) -> String {
    let original_path = if matches!(form, Form::Target) {
        let Ok(uri) = raw.parse::<Uri>() else {
            return REDACTED.to_owned();
        };
        if raw.contains('#') {
            return REDACTED.to_owned();
        }
        Some(uri.path().to_owned())
    } else {
        None
    };
    let input = match form {
        Form::Target => format!("{ORIGIN}{raw}"),
        Form::Query => format!("{ORIGIN}/{raw}"),
        Form::Network => format!("https:{raw}"),
        Form::Absolute => raw.to_owned(),
    };
    let Ok(mut url) = Url::parse(&input) else {
        return REDACTED.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return REDACTED.to_owned();
    }
    if url.set_username("").is_err() || url.set_password(None).is_err() {
        return REDACTED.to_owned();
    }
    url.set_fragment(None);
    let mut pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, value)| allowed_pair(key, value))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    let target = match form {
        Form::Absolute => url.to_string(),
        Form::Network => url
            .as_str()
            .strip_prefix("https:")
            .unwrap_or(REDACTED)
            .to_owned(),
        Form::Target => {
            // Keep the actual escaped HTTP path, including dot segments;
            // the URL parser's normalization must not rewrite log evidence.
            let mut target = original_path
                .filter(|path| path.is_ascii())
                .unwrap_or_else(|| url.path().to_owned());
            if let Some(query) = url.query() {
                target.push('?');
                target.push_str(query);
            }
            target
        }
        Form::Query => url
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default(),
    };
    bounded(target)
}

fn allowed_pair(key: &str, value: &str) -> bool {
    if is_sensitive_query_key(key) {
        return false;
    }
    match key {
        "alt" => matches!(value, "json" | "sse" | "media"),
        "stream" => matches!(value, "true" | "false" | "0" | "1"),
        "model" => {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        }
        "api-version" => {
            let bytes = value.as_bytes();
            bytes.len() >= 10
                && matches!(bytes.len(), 10 | 18)
                && bytes[..10].iter().enumerate().all(|(index, b)| {
                    if matches!(index, 4 | 7) {
                        *b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                })
                && (bytes.len() == 10 || &value[10..] == "-preview")
        }
        _ => false,
    }
}

fn bounded(mut target: String) -> String {
    const SUFFIX: &str = "...[truncated]";
    if target.len() <= MAX_TARGET_BYTES {
        return target;
    }
    let mut end = MAX_TARGET_BYTES - SUFFIX.len();
    while !target.is_char_boundary(end) {
        end -= 1;
    }
    if target.as_bytes()[end - 1] == b'%' {
        end -= 1;
    } else if target.as_bytes()[end - 2] == b'%' {
        end -= 2;
    }
    target.truncate(end);
    target.push_str(SUFFIX);
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_unknown_members_are_omitted_on_every_path() {
        for path in [
            "/v1/models",
            "/oauth",
            "/oauth2/callback",
            "/core/v1/auth/oauth/google/callback",
            "/unknown",
        ] {
            let target = format!(
                "{path}?key=synthetic-key&code=synthetic-code&state=synthetic-state&unknown=synthetic-unknown&synthetic-bare-token&model=gpt-5&alt=sse"
            );
            for status in [200, 401, 404, 500] {
                let line = access_log_line("GET", status, &target).unwrap();
                assert!(!line.contains("synthetic"), "{line}");
                assert!(line.contains("alt=sse&model=gpt-5"), "{line}");
            }
        }
        assert!(access_log_line("GET", 200, "/health/live").is_none());
    }

    #[test]
    fn absolute_urls_drop_userinfo_fragments_and_invalid_input() {
        assert_eq!(
            sanitize_url_for_log(
                "https://synthetic-user:synthetic-pass@example.test/v1?key=synthetic-key&stream=true#synthetic-fragment"
            ),
            "https://example.test/v1?stream=true"
        );
        for raw in [
            "https://example.test/?key=synthetic%ZZ",
            "/v1?key=synthetic\n",
            "/v1?model=synthetic%",
            "not a url?key=synthetic",
        ] {
            assert_eq!(sanitize_url_for_log(raw), REDACTED);
        }
        assert_eq!(sanitize_url_for_log("?state=synthetic"), "");
        assert_eq!(
            sanitize_url_for_log("//user:synthetic@example.test/v1?password=synthetic"),
            "//example.test/v1"
        );
    }

    #[test]
    fn operational_values_are_bounded_and_path_remains_escaped() {
        assert_eq!(
            logged_request_target(
                "/core/v1/auth/oauth/google/callback%20%E2%80%A8?code=synthetic&state=synthetic&api-version=2024-02-01"
            ),
            "/core/v1/auth/oauth/google/callback%20%E2%80%A8?api-version=2024-02-01"
        );
        assert_eq!(
            logged_request_target("//v1%2Fmodels?alt=sse"),
            "//v1%2Fmodels?alt=sse"
        );
        assert_eq!(
            logged_request_target(
                "/v1?model=one%26key%3Dsynthetic&stream=maybe&alt=other&client_id=synthetic"
            ),
            "/v1"
        );
        let line =
            access_log_line("GET", 401, &format!("/{}?code=synthetic", "a".repeat(2000))).unwrap();
        assert!(line.ends_with("...[truncated]"));
        assert!(line.len() <= MAX_TARGET_BYTES + 64);
        assert!(!line.contains("synthetic"));
        for separator in ['\u{2028}', '\u{2029}'] {
            let line =
                access_log_line("GET", 200, &format!("/oauth/{separator}?code=synthetic")).unwrap();
            assert!(!line.contains(separator));
            assert!(!line.contains("synthetic"));
        }
    }
}
