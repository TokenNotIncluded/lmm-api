//! Query-credential redaction for request logs.
//!
//! Adapted from QuantumNous/new-api `30748c65` (`SanitizeURLForLog` and the
//! access-log formatter). The deny list is exact, case-insensitive, and
//! whitespace-trimmed. Any other query key is kept so operators can still see
//! `alt`, `api-version`, `model`, `stream`, `client_id`, and similar values.
//!
//! A key is also denied when its normalized form contains `token`, `secret`,
//! or `signature` (`session_token`, `nottoken`, `client_secret`). `code` and
//! `state` are not on that list. Paths under `/api/oauth/` and `/oauth/` drop
//! the whole query, matching the upstream logger. `/api/oauth2/` does not.
//!
//! Access logs re-encode the query before masking, which drops invalid
//! percent-escapes and semicolon pairs. `sanitize_url_for_log` itself follows
//! `url.URL.Query`: if nothing sensitive remains, the original string is kept.

use axum::{extract::Request, middleware::Next, response::Response};

const MASK: &str = "***masked***";

const EXACT_KEYS: &[&str] = &[
    "key",
    "api_key",
    "api-key",
    "apikey",
    "x-api-key",
    "access_token",
    "refresh_token",
    "id_token",
    "token",
    "authorization",
    "auth",
    "client_secret",
    "secret",
    "password",
    "passwd",
    "signature",
    "sig",
    "awsaccesskeyid",
    "x-amz-credential",
    "x-amz-security-token",
    "x-amz-signature",
];

/// True when a decoded query key must not be written to a log.
pub fn is_sensitive_query_key(key: &str) -> bool {
    let normalized = key.trim().to_lowercase();
    EXACT_KEYS.contains(&normalized.as_str())
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("signature")
}

/// Mask sensitive query values in a URL. Non-sensitive queries, including
/// their original key order, are returned unchanged.
pub fn sanitize_url_for_log(raw: &str) -> String {
    if raw.is_empty() || has_control(raw) {
        return raw.to_owned();
    }
    let (base, fragment) = match split_once_char(raw, '#') {
        Some((base, fragment)) => (base, Some(fragment)),
        None => (raw, None),
    };
    let Some((prefix, query)) = split_once_char(base, '?') else {
        return raw.to_owned();
    };
    let mut pairs = parse_query(query);
    if pairs.is_empty() || !pairs.keys().any(|key| is_sensitive_query_key(key)) {
        return raw.to_owned();
    }
    for (key, values) in &mut pairs {
        if is_sensitive_query_key(key) {
            values.clear();
            values.push(MASK.to_owned());
        }
    }
    let mut out = format!("{prefix}?{}", encode_query(&pairs));
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(&escape(fragment, Escape::Fragment));
    }
    out
}

/// Request-target as it should appear in an access log.
///
/// OAuth callback paths lose the query. Other targets keep a re-encoded query
/// with sensitive values replaced by `***masked***` (`*` is percent-encoded).
pub fn logged_request_target(target: &str) -> String {
    let Some(parsed) = parse_request_target(target) else {
        return strip_query(target);
    };
    if parsed.decoded_path.starts_with("/api/oauth/") || parsed.decoded_path.starts_with("/oauth/")
    {
        return parsed.decoded_path;
    }
    if !parsed.had_query {
        return parsed.decoded_path;
    }
    let encoded = encode_query(&parse_query(&parsed.raw_query));
    let suffix = sanitize_url_for_log(&format!("?{encoded}"));
    format!("{}{suffix}", parsed.escaped_path)
}

/// One stderr line for a request whose target carries a query. The original
/// target is not included.
pub fn access_log_line(method: &str, status: u16, target: &str) -> Option<String> {
    if !target.contains('?') {
        return None;
    }
    let mut logged = logged_request_target(target);
    if logged.chars().count() > 1024 {
        logged = logged.chars().take(1024).collect();
    }
    let logged: String = logged
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    let method: String = method
        .chars()
        .filter(|c| !c.is_control())
        .take(16)
        .collect();
    Some(format!(
        "core_access method={method} status={status} target={logged}"
    ))
}

/// Outermost access note. Query-less requests stay quiet; credential-bearing
/// queries are masked before they reach stderr.
pub async fn log_redacted_request_target(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    let target = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_owned())
        .unwrap_or_default();
    let response = next.run(request).await;
    if let Some(line) = access_log_line(&method, response.status().as_u16(), &target) {
        eprintln!("{line}");
    }
    response
}

fn strip_query(target: &str) -> String {
    let path = split_once_char(target, '?')
        .map(|(path, _)| path)
        .unwrap_or(target);
    path.chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect()
}

struct RequestTarget {
    decoded_path: String,
    escaped_path: String,
    raw_query: String,
    had_query: bool,
}

fn parse_request_target(target: &str) -> Option<RequestTarget> {
    if target.is_empty() || has_control(target) {
        return None;
    }
    // Request-targets are parsed like Go's ParseRequestURI: `#` stays in the
    // query because the request line has no fragment.
    let (path_raw, raw_query, had_query) = match split_once_char(target, '?') {
        Some((path, query)) => (path, query.to_owned(), true),
        None => (target, String::new(), false),
    };
    if path_raw != "*" && !path_raw.starts_with('/') {
        return None;
    }
    let decoded = unescape(path_raw, Escape::Path)?;
    let escaped_path = escaped_path(path_raw, &decoded);
    Some(RequestTarget {
        decoded_path: decoded,
        escaped_path,
        raw_query,
        had_query,
    })
}

fn escaped_path(raw: &str, decoded: &str) -> String {
    if raw != "*"
        && valid_encoded(raw, Escape::Path)
        && unescape(raw, Escape::Path).as_deref() == Some(decoded)
    {
        let escaped = escape(decoded, Escape::Path);
        if escaped != raw {
            return raw.to_owned();
        }
    }
    if decoded == "*" {
        return "*".to_owned();
    }
    escape(decoded, Escape::Path)
}

fn parse_query(query: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut pairs = std::collections::BTreeMap::new();
    if query.is_empty() {
        return pairs;
    }
    for segment in query.split('&') {
        if segment.is_empty() || segment.contains(';') {
            continue;
        }
        let (key, value) = segment.split_once('=').unwrap_or((segment, ""));
        let (Some(key), Some(value)) =
            (unescape(key, Escape::Query), unescape(value, Escape::Query))
        else {
            continue;
        };
        pairs.entry(key).or_default().push(value);
    }
    pairs
}

fn encode_query(pairs: &std::collections::BTreeMap<String, Vec<String>>) -> String {
    let mut out = String::new();
    for (key, values) in pairs {
        let key = escape(key, Escape::Query);
        for value in values {
            if !out.is_empty() {
                out.push('&');
            }
            out.push_str(&key);
            out.push('=');
            out.push_str(&escape(value, Escape::Query));
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    Path,
    Query,
    Fragment,
}

fn has_control(value: &str) -> bool {
    value.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
}

fn split_once_char(value: &str, separator: char) -> Option<(&str, &str)> {
    value.split_once(separator)
}

fn should_escape(byte: u8, mode: Escape) -> bool {
    if byte.is_ascii_alphanumeric() {
        return false;
    }
    if matches!(byte, b'-' | b'_' | b'.' | b'~') {
        return false;
    }
    if matches!(
        byte,
        b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'?' | b'@'
    ) {
        return match mode {
            Escape::Path => byte == b'?',
            Escape::Query => true,
            Escape::Fragment => false,
        };
    }
    if mode == Escape::Fragment && matches!(byte, b'!' | b'(' | b')' | b'*') {
        return false;
    }
    true
}

fn escape(value: &str, mode: Escape) -> String {
    let bytes = value.as_bytes();
    if !bytes
        .iter()
        .any(|byte| should_escape(*byte, mode) || (*byte == b' ' && mode == Escape::Query))
    {
        return value.to_owned();
    }
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        if *byte == b' ' && mode == Escape::Query {
            out.push('+');
        } else if should_escape(*byte, mode) {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        } else {
            out.push(*byte as char);
        }
    }
    out
}

fn unescape(value: &str, mode: Escape) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return None;
                }
                let high = from_hex(bytes[index + 1])?;
                let low = from_hex(bytes[index + 2])?;
                out.push((high << 4) | low);
                index += 3;
            }
            b'+' if mode == Escape::Query => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn valid_encoded(value: &str, mode: Escape) -> bool {
    value.bytes().all(|byte| {
        matches!(
            byte,
            b'!' | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
                | b'['
                | b']'
                | b'%'
        ) || !should_escape(byte, mode)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_log_matches_upstream_query_redaction_goldens() {
        let cases = [
            (
                "/v1/models?%6bey=first&key=second&API-Key=third&session_token=fourth&alt=sse",
                "/v1/models?API-Key=%2A%2A%2Amasked%2A%2A%2A&alt=sse&key=%2A%2A%2Amasked%2A%2A%2A&session_token=%2A%2A%2Amasked%2A%2A%2A",
            ),
            (
                "/v1/models%23test%25?key=#secret&alt=#sse",
                "/v1/models%23test%25?alt=%23sse&key=%2A%2A%2Amasked%2A%2A%2A",
            ),
            (
                "//v1%2Fmodels?key=secret",
                "//v1%2Fmodels?key=%2A%2A%2Amasked%2A%2A%2A",
            ),
            (
                "/v1/models?key=bad%ZZ&%ZZ=bad&token=bad;other=true&alt=sse",
                "/v1/models?alt=sse",
            ),
            ("/v1/models?key=bad%ZZ", "/v1/models?"),
            (
                "/v1/models?alt=sse&api-version=2024-02-01",
                "/v1/models?alt=sse&api-version=2024-02-01",
            ),
            ("/v1/models", "/v1/models"),
            (
                "/%61pi/oauth/test?code=secret&state=secret",
                "/api/oauth/test",
            ),
            ("/oauth/callback?code=abc&state=xyz", "/oauth/callback"),
            (
                "/v1?+key+=secret&model=gpt",
                "/v1?+key+=%2A%2A%2Amasked%2A%2A%2A&model=gpt",
            ),
            (
                "/v1?author=ada&authentication=no&auth=yes&alt=sse",
                "/v1?alt=sse&auth=%2A%2A%2Amasked%2A%2A%2A&authentication=no&author=ada",
            ),
            (
                "/api/oauth2/authorize?client_id=public&code=secret",
                "/api/oauth2/authorize?client_id=public&code=secret",
            ),
            (
                "/v1/models?api_key=k&apikey=k2&x-api-key=k3&refresh_token=r&id_token=i&password=p&passwd=p2&sig=s&signature=s2&awsaccesskeyid=a&x-amz-security-token=t&model=keep",
                "/v1/models?api_key=%2A%2A%2Amasked%2A%2A%2A&apikey=%2A%2A%2Amasked%2A%2A%2A&awsaccesskeyid=%2A%2A%2Amasked%2A%2A%2A&id_token=%2A%2A%2Amasked%2A%2A%2A&model=keep&passwd=%2A%2A%2Amasked%2A%2A%2A&password=%2A%2A%2Amasked%2A%2A%2A&refresh_token=%2A%2A%2Amasked%2A%2A%2A&sig=%2A%2A%2Amasked%2A%2A%2A&signature=%2A%2A%2Amasked%2A%2A%2A&x-amz-security-token=%2A%2A%2Amasked%2A%2A%2A&x-api-key=%2A%2A%2Amasked%2A%2A%2A",
            ),
        ];
        for (target, expected) in cases {
            let logged = logged_request_target(target);
            assert_eq!(logged, expected, "{target}");
            for secret in ["first", "second", "third", "fourth", "sk-secret", "#secret"] {
                if target.contains(secret) && !expected.contains(secret) {
                    assert!(!logged.contains(secret), "{target} leaked {secret}");
                }
            }
        }
    }

    #[test]
    fn sanitize_url_for_log_masks_keys_and_keeps_operational_queries() {
        let masked = sanitize_url_for_log(
            "https://example.test/v1beta/models/gemini:streamGenerateContent?alt=sse&key=sk-secret&access_token=ya29-secret&api-version=2024-02-01",
        );
        assert_eq!(
            masked,
            "https://example.test/v1beta/models/gemini:streamGenerateContent?access_token=%2A%2A%2Amasked%2A%2A%2A&alt=sse&api-version=2024-02-01&key=%2A%2A%2Amasked%2A%2A%2A"
        );
        assert!(!masked.contains("sk-secret"));
        assert!(!masked.contains("ya29-secret"));
        let aws = sanitize_url_for_log(
            "https://example.test/path?X-Amz-Credential=credential&X-Amz-Signature=signature&session_token=session&client_secret=secret&model=gpt-test",
        );
        assert!(!aws.contains("=credential"));
        assert!(!aws.contains("=signature"));
        assert!(!aws.contains("=session"));
        assert!(!aws.contains("=secret"));
        assert!(aws.contains("X-Amz-Credential=%2A%2A%2Amasked%2A%2A%2A"));
        assert!(aws.contains("model=gpt-test"));
        let kept = "https://example.test/v1/chat/completions?api-version=2024-02-01&alt=sse";
        assert_eq!(sanitize_url_for_log(kept), kept);
        assert_eq!(
            sanitize_url_for_log("https://user:pass@example.test/v1?key=sek&alt=sse"),
            "https://user:pass@example.test/v1?alt=sse&key=%2A%2A%2Amasked%2A%2A%2A"
        );
        assert_eq!(
            sanitize_url_for_log("https://example.test/v1?key=bad%ZZ"),
            "https://example.test/v1?key=bad%ZZ"
        );
        assert_eq!(sanitize_url_for_log("not a url %%"), "not a url %%");
        assert_eq!(sanitize_url_for_log(""), "");
    }

    #[test]
    fn access_log_line_omits_plaintext_credentials_and_keeps_model() {
        let line = access_log_line(
            "GET",
            401,
            "/v1/models?key=sk-secret&token=plain-token&api_key=abc&access_token=def&secret=ghi&model=gpt-test&alt=sse",
        )
        .expect("query is logged");
        for secret in ["sk-secret", "plain-token", "abc", "def", "ghi"] {
            assert!(!line.contains(secret), "{line}");
        }
        assert!(line.contains("model=gpt-test"));
        assert!(line.contains("alt=sse"));
        assert!(line.contains("%2A%2A%2Amasked%2A%2A%2A"));
        assert!(line.starts_with("core_access method=GET status=401 target="));
        assert!(access_log_line("GET", 200, "/health/live").is_none());
    }
}
