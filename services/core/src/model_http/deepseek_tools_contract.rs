//! DeepSeek's native, non-streaming Search / Fetch receipt contract.
//! Reviewed against https://api-docs.deepseek.com/guides/search/ and the
//! /api/search/ and /api/fetch/ schemas. Upstream comparison: QuantumNous/new-api
//! 6cf0d90b5ff27f901697dddd61e0c28aba8efefc (AGPL-3.0); this is a Rust adaptation.
use crate::{
    relay::{RelayError, Usage, native_anthropic::unambiguous_json},
    routing::{SearchProvider, SearchUsage},
};
use serde_json::{Map, Value};

pub(super) const MAX_BODY: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Search,
    Fetch,
}
impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Fetch => "fetch",
        }
    }
    pub(super) fn provider(self) -> SearchProvider {
        match self {
            Self::Search => SearchProvider::DeepSeekSearchV1,
            Self::Fetch => SearchProvider::DeepSeekFetchV1,
        }
    }
}

pub(super) struct Request {
    pub kind: Kind,
    pub model: String,
    pub wire: Value,
    count: u64,
}

impl Request {
    pub(super) fn parse(kind: Kind, bytes: &[u8]) -> Result<Self, RelayError> {
        if bytes.len() > MAX_BODY {
            return Err(RelayError::Limit("DeepSeek tool request"));
        }
        let mut wire = unambiguous_json(bytes)
            .map_err(|_| RelayError::Invalid("ambiguous DeepSeek tool JSON"))?;
        let fields: &[&str] = match kind {
            Kind::Search => &[
                "model",
                "query",
                "count",
                "ori_question",
                "localization",
                "need_content",
            ],
            Kind::Fetch => &["model", "url"],
        };
        let object = object(&wire, fields)?;
        let model = match object.get("model") {
            Some(value) => text(value)?.to_owned(),
            None => format!("deepseek-{}", kind.name()),
        };
        if model.is_empty()
            || model.len() > 200
            || !model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-/:@~".contains(&b))
        {
            return Err(RelayError::Invalid("invalid native tool route model"));
        }
        let mut count = 0;
        match kind {
            Kind::Search => {
                query(&wire["query"])?;
                count = match object.get("count") {
                    Some(value) => value
                        .as_u64()
                        .filter(|n| (1..=50).contains(n))
                        .ok_or(RelayError::Invalid("invalid search result count"))?,
                    None => 10,
                };
                if let Some(value) = object.get("ori_question").filter(|v| !v.is_null()) {
                    query(value)?;
                }
                if let Some(value) = object.get("localization") {
                    let value = text(value)?;
                    if !value.is_empty() && value.parse::<std::net::IpAddr>().is_err() {
                        return Err(RelayError::Invalid("invalid search localization"));
                    }
                }
                if object.get("need_content").is_some_and(|v| !v.is_boolean()) {
                    return Err(RelayError::Invalid("invalid search content selection"));
                }
            }
            Kind::Fetch => web_url(text(&wire["url"])?)?,
        }
        // Native tools have no model parameter. This key is only a local route
        // selection and must never reach the provider as a different operation.
        wire.as_object_mut().unwrap().remove("model");
        Ok(Self {
            kind,
            model,
            wire,
            count,
        })
    }

    pub(super) fn response(&self, status: u16, bytes: &[u8]) -> Result<Observed, RelayError> {
        let output = unambiguous_json(bytes).map_err(|_| invalid_receipt())?;
        let calls = if status == 200 {
            match self.kind {
                Kind::Search => {
                    object(&output, &["results"]).map_err(|_| invalid_receipt())?;
                    let results = output["results"].as_array().ok_or_else(invalid_receipt)?;
                    if results.len() as u64 > self.count {
                        return Err(invalid_receipt());
                    }
                    for result in results {
                        validate_result(result)?;
                    }
                    u64::from(!results.is_empty())
                }
                Kind::Fetch => {
                    object(&output, &["title", "content"]).map_err(|_| invalid_receipt())?;
                    text(&output["title"]).map_err(|_| invalid_receipt())?;
                    u64::from(
                        !text(&output["content"])
                            .map_err(|_| invalid_receipt())?
                            .is_empty(),
                    )
                }
            }
        } else {
            // Only a complete documented error receipt proves a failed, free
            // request. A transport timeout or an arbitrary proxy error does not.
            if !matches!(
                status,
                400 | 401 | 402 | 403 | 404 | 408 | 413 | 429 | 500 | 504
            ) {
                return Err(invalid_receipt());
            }
            object(&output, &["error"]).map_err(|_| invalid_receipt())?;
            object(&output["error"], &["message", "type", "param", "code"])
                .map_err(|_| invalid_receipt())?;
            for field in ["message", "type", "code"] {
                text(&output["error"][field]).map_err(|_| invalid_receipt())?;
            }
            if output["error"].get("param") != Some(&Value::Null) {
                return Err(invalid_receipt());
            }
            0
        };
        Ok(Observed {
            output,
            upstream_error: (status != 200).then_some(status),
            usage: Usage {
                input_tokens: Some(0),
                output_tokens: Some(0),
                cached_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
                search: Some(SearchUsage {
                    provider: self.kind.provider(),
                    calls: Some(calls),
                    queries: None,
                    grounded_requests: None,
                    x_posts_fetched: None,
                    x_users_fetched: None,
                }),
                ..Usage::default()
            },
        })
    }
}

pub(super) struct Observed {
    pub output: Value,
    pub usage: Usage,
    pub upstream_error: Option<u16>,
}

fn validate_result(value: &Value) -> Result<(), RelayError> {
    let fields = object(
        value,
        &[
            "rerank_score",
            "title",
            "url",
            "snippet",
            "summary",
            "content",
            "site_name",
            "site_icon",
            "date",
            "authority",
        ],
    )
    .map_err(|_| invalid_receipt())?;
    for field in ["title", "snippet", "summary", "content"] {
        text(&value[field]).map_err(|_| invalid_receipt())?;
    }
    web_url(text(&value["url"]).map_err(|_| invalid_receipt())?).map_err(|_| invalid_receipt())?;
    if !value["rerank_score"]
        .as_f64()
        .is_some_and(|v| v.is_finite() && (0.0..=3.0).contains(&v))
        || !value["authority"]
            .as_i64()
            .is_some_and(|v| (-1..=4).contains(&v))
    {
        return Err(invalid_receipt());
    }
    for field in ["site_name", "site_icon", "date"] {
        let value = fields.get(field).ok_or_else(invalid_receipt)?;
        if !value.is_null() {
            let value = text(value).map_err(|_| invalid_receipt())?;
            if field == "site_icon" && !value.is_empty() {
                web_url(value).map_err(|_| invalid_receipt())?;
            } else if field == "date" && chrono::DateTime::parse_from_rfc3339(value).is_err() {
                return Err(invalid_receipt());
            }
        }
    }
    Ok(())
}
fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, RelayError> {
    let object = value
        .as_object()
        .ok_or(RelayError::Invalid("expected native tool object"))?;
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(RelayError::Unsupported("unsupported native tool field"));
    }
    Ok(object)
}
fn text(value: &Value) -> Result<&str, RelayError> {
    value
        .as_str()
        .ok_or(RelayError::Invalid("expected native tool string"))
}
fn query(value: &Value) -> Result<(), RelayError> {
    let value = text(value)?;
    if value.trim().is_empty()
        || value.chars().count() > 128
        || value.chars().any(|c| c.is_control() && !c.is_whitespace())
    {
        return Err(RelayError::Invalid("invalid search query"));
    }
    Ok(())
}
fn web_url(value: &str) -> Result<(), RelayError> {
    let url = reqwest::Url::parse(value).map_err(|_| RelayError::Invalid("invalid tool URL"))?;
    if value.len() > 8192
        || value.chars().any(char::is_control)
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(RelayError::Invalid("invalid tool URL"));
    }
    Ok(())
}
fn invalid_receipt() -> RelayError {
    RelayError::Protocol("invalid or unknown native tool receipt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request(kind: Kind, value: Value) -> Result<Request, RelayError> {
        Request::parse(kind, &serde_json::to_vec(&value).unwrap())
    }
    fn result() -> Value {
        json!({"rerank_score":2.4,"title":"synthetic","url":"https://example.test/page","snippet":"summary","summary":"summary","content":"","site_name":null,"site_icon":null,"date":null,"authority":-1})
    }
    #[test]
    fn native_request_bounds_and_route_only_model_are_enforced() {
        let search = request(Kind::Search, json!({"model":"local-search","query":"site:example.test 猫","count":50,"need_content":true})).unwrap();
        assert_eq!(search.model, "local-search");
        assert!(search.wire.get("model").is_none());
        assert_eq!(
            request(Kind::Fetch, json!({"url":"https://example.test/page"}))
                .unwrap()
                .model,
            "deepseek-fetch"
        );
        for invalid in [
            json!({"query":" "}),
            json!({"query":"a".repeat(129)}),
            json!({"query":"x","count":1.5}),
            json!({"query":"x","count":0}),
            json!({"query":"x","count":51}),
            json!({"query":"x","stream":true}),
            json!({"query":"x","localization":"example.test"}),
        ] {
            assert!(request(Kind::Search, invalid).is_err());
        }
        for url in [
            "file:///tmp/example",
            "https://user:secret@example.test/page",
            "https://",
            "http://example.test/\n",
        ] {
            assert!(request(Kind::Fetch, json!({"url":url})).is_err());
        }
        assert!(Request::parse(Kind::Search, br#"{"query":"x","query":"y"}"#).is_err());
    }
    #[test]
    fn returned_results_are_one_billable_call_and_empty_is_explicit_zero() {
        let request = request(Kind::Search, json!({"query":"x","count":2})).unwrap();
        for (results, calls) in [(json!([]), 0), (json!([result(), result()]), 1)] {
            let observed = request
                .response(
                    200,
                    &serde_json::to_vec(&json!({"results":results})).unwrap(),
                )
                .unwrap();
            assert_eq!(observed.usage.search.unwrap().calls, Some(calls));
            assert_eq!(observed.usage.input_tokens, Some(0));
        }
        for value in [
            json!({}),
            json!({"results":null}),
            json!({"results":[{}]}),
            json!({"results":[result(),result(),result()]}),
        ] {
            assert!(
                request
                    .response(200, &serde_json::to_vec(&value).unwrap())
                    .is_err()
            );
        }
    }
    #[test]
    fn fetch_requires_real_content_and_known_error_receipts_can_settle_zero() {
        let request = request(Kind::Fetch, json!({"url":"https://example.test/page"})).unwrap();
        for (content, calls) in [("", 0), ("# Page", 1)] {
            let receipt = request
                .response(
                    200,
                    &serde_json::to_vec(&json!({"title":"","content":content})).unwrap(),
                )
                .unwrap();
            assert_eq!(receipt.usage.search.unwrap().calls, Some(calls));
        }
        assert!(
            request
                .response(200, br#"{"title":"missing content"}"#)
                .is_err()
        );
        let error = br#"{"error":{"message":"not available","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}"#;
        let receipt = request.response(404, error).unwrap();
        assert_eq!(receipt.upstream_error, Some(404));
        assert_eq!(receipt.usage.search.unwrap().calls, Some(0));
        assert!(request.response(200, error).is_err());
        assert!(request.response(502, b"bad gateway").is_err());
    }
}
