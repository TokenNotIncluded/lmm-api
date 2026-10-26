//! Relay-local contracts. Routing and money implementations live outside this module.
use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use reqwest::{Url, header::HeaderValue};
use serde::Serialize;
use serde_json::{Value, json};

use crate::routing::{CacheCreationMode, CacheCreationPartition};

pub type RelayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RelayError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientProtocol {
    Chat,
    Responses,
    Anthropic,
    Gemini,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamProtocol {
    Chat,
    Responses,
    Anthropic,
    Gemini,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Route,
    Reserve,
    Connect,
    Headers,
    Read,
    Write,
    Total,
    Settle,
}

/// Public errors deliberately contain neither upstream bodies nor credentials/URLs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayError {
    Invalid(&'static str),
    Unsupported(&'static str),
    Protocol(&'static str),
    Limit(&'static str),
    Timeout(Phase),
    Cancelled,
    Draining,
    Busy,
    RouteUnavailable,
    BillingUnavailable,
    Unauthorized,
    Forbidden,
    NoFunds,
    Conflict,
    UpstreamStatus(u16),
    UpstreamConnection,
    Truncated,
    SettlementPending,
}

impl RelayError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_request",
            Self::Unsupported(_) => "unsupported_feature",
            Self::Protocol(_) | Self::Truncated => "invalid_upstream_response",
            Self::Limit(_) => "relay_limit_exceeded",
            Self::Timeout(_) => "relay_timeout",
            Self::Cancelled => "request_cancelled",
            Self::Draining => "relay_draining",
            Self::Busy => "relay_busy",
            Self::RouteUnavailable => "route_unavailable",
            Self::BillingUnavailable => "billing_unavailable",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NoFunds => "insufficient_funds",
            Self::Conflict => "request_conflict",
            Self::UpstreamStatus(_) => "upstream_error",
            Self::UpstreamConnection => "upstream_connection_error",
            Self::SettlementPending => "settlement_pending",
        }
    }
    pub fn status(&self) -> u16 {
        match self {
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NoFunds => 402,
            Self::Conflict => 409,
            Self::Invalid(_) | Self::Unsupported(_) => 400,
            Self::Busy => 429,
            Self::Limit(_) => 413,
            Self::Timeout(_) => 504,
            Self::Cancelled => 499,
            Self::Draining
            | Self::RouteUnavailable
            | Self::BillingUnavailable
            | Self::SettlementPending => 503,
            _ => 502,
        }
    }
    pub fn json(&self) -> Value {
        json!({"error":{"type":self.code(),"code":self.code(),"message":self.to_string()}})
    }
}
impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) | Self::Unsupported(s) | Self::Protocol(s) | Self::Limit(s) => {
                write!(f, "{}: {s}", self.code())
            }
            Self::Timeout(p) => write!(f, "relay_timeout: {p:?}"),
            Self::UpstreamStatus(s) => write!(f, "upstream_error: HTTP {s}"),
            _ => f.write_str(self.code()),
        }
    }
}
impl std::error::Error for RelayError {}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_in_flight: usize,
    pub queue_chunks: usize,
    pub chunk_bytes: usize,
    pub request_bytes: usize,
    pub event_bytes: usize,
    pub output_bytes: usize,
    pub upstream_bytes: usize,
    pub transport_chunk_bytes: usize,
    pub max_blocks: usize,
    pub tool_argument_bytes: usize,
    pub max_output_tokens: u32,
    pub connect_timeout: Duration,
    pub header_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub total_timeout: Duration,
    pub hook_timeout: Duration,
    /// Only connect failures, before an HTTP request is sent, may be retried.
    pub connect_retries: u8,
    pub retry_backoff: Duration,
    /// Anthropic Messages streaming keepalive. The first SSE comment waits one
    /// full interval, so a response that starts sooner is left unchanged.
    /// Other client protocols do not emit these comments.
    pub ping_interval: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_in_flight: 16,
            queue_chunks: 8,
            chunk_bytes: 16 * 1024,
            request_bytes: 2 * 1024 * 1024,
            event_bytes: 256 * 1024,
            output_bytes: 1024 * 1024,
            upstream_bytes: 64 * 1024 * 1024,
            transport_chunk_bytes: 1024 * 1024,
            max_blocks: 128,
            tool_argument_bytes: 256 * 1024,
            max_output_tokens: 65_536,
            connect_timeout: Duration::from_secs(5),
            header_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(300),
            hook_timeout: Duration::from_secs(10),
            connect_retries: 0,
            retry_backoff: Duration::from_millis(200),
            ping_interval: Duration::from_secs(10),
        }
    }
}
impl Limits {
    pub(crate) fn validate(&self) -> Result<(), RelayError> {
        let sizes = [
            self.max_in_flight,
            self.queue_chunks,
            self.chunk_bytes,
            self.request_bytes,
            self.event_bytes,
            self.output_bytes,
            self.upstream_bytes,
            self.transport_chunk_bytes,
            self.max_blocks,
            self.tool_argument_bytes,
        ];
        let times = [
            self.connect_timeout,
            self.header_timeout,
            self.read_timeout,
            self.write_timeout,
            self.total_timeout,
            self.hook_timeout,
            self.retry_backoff,
            self.ping_interval,
        ];
        if sizes.contains(&0)
            || times.iter().any(Duration::is_zero)
            || self.ping_interval < Duration::from_millis(1)
            || self.max_output_tokens == 0
            || self.max_in_flight > 4096
            || self.queue_chunks > 1024
            || self.chunk_bytes > 65_536
            || self.max_blocks > 1024
            || self.connect_retries > 2
            || self.output_bytes > 64 * 1024 * 1024
            || self.request_bytes > 16 * 1024 * 1024
            || self.event_bytes > self.output_bytes
            || self.tool_argument_bytes > self.output_bytes
            || self.transport_chunk_bytes > self.upstream_bytes
            || times.iter().any(|d| *d > Duration::from_secs(86_400))
        {
            return Err(RelayError::Invalid("invalid relay limits"));
        }
        Ok(())
    }
}

/// Construct this only after core identity verification. The billing adapter must
/// recheck the actor/key, account funding policy, budget and immutable price version.
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub request_id: String,
    pub actor_user_id: i64,
    pub key_id: Option<i64>,
    pub oauth_source: Option<crate::identity::grants::OAuthSource>,
}
impl RequestContext {
    pub(crate) fn validate(&self) -> Result<(), RelayError> {
        let source_valid = match (&self.key_id, &self.oauth_source) {
            (Some(key), None) => *key > 0,
            (None, Some(source)) => {
                source.user_id() == self.actor_user_id && source.validate_stored().is_ok()
            }
            _ => false,
        };
        if self.actor_user_id <= 0
            || !source_valid
            || self.request_id.is_empty()
            || self.request_id.len() > 128
            || !self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(RelayError::Invalid("invalid request identity"));
        }
        Ok(())
    }
}

/// An immutable route snapshot selected by task 04, not a URL from an HTTP caller.
/// `endpoint` is the complete operation URL. Gemini streaming uses `alt=sse`.
/// HTTPS is required except for explicit numeric loopback test/local upstreams.
#[derive(Clone)]
pub struct Route {
    pub id: String,
    pub protocol: UpstreamProtocol,
    pub model: String,
    pub price_version: u64,
    pub(crate) endpoint: Url,
    pub(crate) credential: HeaderValue,
    pub(crate) cache_creation_mode: Option<CacheCreationMode>,
    availability: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}
impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("id", &self.id)
            .field("protocol", &self.protocol)
            .field("price_version", &self.price_version)
            .finish_non_exhaustive()
    }
}
impl Route {
    pub fn new(
        id: String,
        protocol: UpstreamProtocol,
        endpoint: &str,
        model: String,
        api_key: &str,
        price_version: u64,
    ) -> Result<Self, RelayError> {
        let endpoint =
            Url::parse(endpoint).map_err(|_| RelayError::Invalid("invalid route URL"))?;
        let local = endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if id.is_empty()
            || id.len() > 128
            || model.is_empty()
            || model.len() > 256
            || price_version == 0
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
            || endpoint.host_str().is_none()
            || !(endpoint.scheme() == "https" || (endpoint.scheme() == "http" && local))
            || endpoint
                .query_pairs()
                .any(|(k, v)| protocol != UpstreamProtocol::Gemini || k != "alt" || v != "sse")
            || api_key.is_empty()
            || api_key.len() > 4096
        {
            return Err(RelayError::Invalid("invalid route configuration"));
        }
        let auth = if matches!(
            protocol,
            UpstreamProtocol::Chat | UpstreamProtocol::Responses
        ) {
            format!("Bearer {api_key}")
        } else {
            api_key.to_owned()
        };
        let mut credential = HeaderValue::from_str(&auth)
            .map_err(|_| RelayError::Invalid("invalid upstream credential"))?;
        credential.set_sensitive(true);
        Ok(Self {
            id,
            protocol,
            endpoint,
            model,
            credential,
            price_version,
            cache_creation_mode: None,
            availability: None,
        })
    }

    /// Captured from the selected frozen Core price, never from provider JSON.
    pub(crate) fn with_cache_creation_mode(mut self, mode: Option<CacheCreationMode>) -> Self {
        self.cache_creation_mode = mode;
        self
    }

    /// Server-owned snapshot guard, checked immediately before sending bytes.
    /// HTTP callers cannot construct or deserialize a route or this guard.
    pub fn with_availability_guard(mut self, guard: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        self.availability = Some(guard);
        self
    }

    pub(crate) fn is_available(&self) -> bool {
        self.availability.as_ref().is_none_or(|guard| guard())
    }
}

pub trait RouteProvider: Send + Sync + 'static {
    /// Read an already active Rust-owned snapshot. Do not call the Go extension.
    fn select<'a>(
        &'a self,
        context: &'a RequestContext,
        request: &'a super::Request,
    ) -> RelayFuture<'a, Route>;
}

/// A durable reservation handle, never a live SQL transaction/connection.
#[derive(Clone, Debug)]
pub struct Reservation {
    pub id: String,
}

/// Evidence prepared before exposing a streaming packet. A digest is an audit
/// commitment, not a token estimate or proof that the client received bytes.
/// Missing provider usage remains missing; partial usage is never final usage.
#[derive(Clone, Debug)]
pub struct OutputCheckpoint {
    pub sequence: u64,
    pub packet_sha256: [u8; 32],
    pub packet_bytes: usize,
    pub usage: Usage,
    pub terminal_observed: bool,
}

pub trait Billing: Send + Sync + 'static {
    /// Idempotent by context.request_id. Commit before returning. A timeout may
    /// mean the commit succeeded: durable recovery must resolve that ambiguity.
    fn reserve<'a>(
        &'a self,
        context: &'a RequestContext,
        route: &'a Route,
        request: &'a super::Request,
    ) -> RelayFuture<'a, Reservation>;
    /// Commit output/usage evidence before output is queued, including an empty
    /// packet for a validated non-stream terminal. Integrations must explicitly
    /// provide their durability policy; the default rejects the observation.
    fn checkpoint<'a>(
        &'a self,
        _: &'a Reservation,
        _: &'a OutputCheckpoint,
    ) -> RelayFuture<'a, ()> {
        Box::pin(async { Err(RelayError::SettlementPending) })
    }
    /// Persist settlement or a reconciliation obligation, idempotently. Missing
    /// usage or a disconnected upstream must NEVER be interpreted as zero cost.
    /// This hook runs even after the client drops its body. Do not wait for Go.
    fn finalize<'a>(
        &'a self,
        reservation: &'a Reservation,
        report: &'a Report,
    ) -> RelayFuture<'a, ()>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCreationDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_5m_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_1h_input_tokens: Option<u64>,
}

impl CacheCreationDetails {
    fn has_known_bucket(&self) -> bool {
        self.ephemeral_5m_input_tokens.is_some() || self.ephemeral_1h_input_tokens.is_some()
    }

    fn with_total(self, total: u64) -> Result<Self, RelayError> {
        let invalid = RelayError::Protocol("inconsistent cache creation usage");
        match (
            self.ephemeral_5m_input_tokens,
            self.ephemeral_1h_input_tokens,
        ) {
            (Some(five), Some(hour)) => {
                if five.checked_add(hour) != Some(total) {
                    return Err(invalid);
                }
                Ok(self)
            }
            (Some(five), None) => Ok(Self {
                ephemeral_1h_input_tokens: Some(total.checked_sub(five).ok_or(invalid)?),
                ..self
            }),
            (None, Some(hour)) => Ok(Self {
                ephemeral_5m_input_tokens: Some(total.checked_sub(hour).ok_or(invalid)?),
                ..self
            }),
            (None, None) => Ok(self),
        }
    }

    pub(crate) fn known_total(&self) -> Result<u64, RelayError> {
        self.ephemeral_5m_input_tokens
            .unwrap_or(0)
            .checked_add(self.ephemeral_1h_input_tokens.unwrap_or(0))
            .ok_or(RelayError::Protocol("usage overflow"))
    }

    pub(crate) fn total(&self) -> Result<Option<u64>, RelayError> {
        match (
            self.ephemeral_5m_input_tokens,
            self.ephemeral_1h_input_tokens,
        ) {
            (Some(five), Some(hour)) => five
                .checked_add(hour)
                .map(Some)
                .ok_or(RelayError::Protocol("usage overflow")),
            _ => Ok(None),
        }
    }
}

/// OpenAI's cache lifetimes are separate from Anthropic's 5m/1h counters.
/// Both are subsets of the reported aggregate, never additional input tokens.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCacheWriteDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_30m_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_12h_tokens: Option<u64>,
}

impl OpenAiCacheWriteDetails {
    fn has_known_bucket(&self) -> bool {
        self.ttl_30m_tokens.is_some() || self.ttl_12h_tokens.is_some()
    }

    fn known_total(self) -> Result<u64, RelayError> {
        self.ttl_30m_tokens
            .unwrap_or(0)
            .checked_add(self.ttl_12h_tokens.unwrap_or(0))
            .ok_or(RelayError::Protocol("usage overflow"))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Provider account charge, parsed exactly; absent is never a free receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_cost_atto_usd: Option<u128>,
    pub input_tokens: Option<u64>,
    /// Raw Anthropic input evidence for the generic contract; never a zero guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_details: Option<CacheCreationDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_cache_write_details: Option<OpenAiCacheWriteDetails>,
    /// Frozen Core interpretation; the provider's aggregate remains unmodified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_mode: Option<CacheCreationMode>,
    pub reasoning_output_tokens: Option<u64>,
}
impl Usage {
    pub(crate) fn merge(&mut self, next: Self) -> Result<(), RelayError> {
        if next.uncached_input_tokens.is_some() && next.cache_creation_mode.is_none() {
            return Err(RelayError::Protocol(
                "uncached input without cache creation mode",
            ));
        }
        if self.cache_creation_mode.is_some() || next.cache_creation_mode.is_some() {
            return self.merge_generic_creation(next);
        }
        fn advances(old: Option<u64>, next: Option<u64>) -> bool {
            next.is_some_and(|value| old.is_none_or(|old| value > old))
        }
        fn field(old: &mut Option<u64>, next: Option<u64>) -> Result<(), RelayError> {
            if let Some(n) = next {
                if old.is_some_and(|o| n < o) {
                    return Err(RelayError::Protocol("usage counter decreased"));
                }
                *old = Some(n);
            }
            Ok(())
        }
        let mut combined = *self;
        let input_advanced = advances(self.input_tokens, next.input_tokens);
        let creation_advanced = advances(
            self.cache_creation_input_tokens,
            next.cache_creation_input_tokens,
        );
        if input_advanced
            && self.cache_creation_input_tokens.is_some()
            && next.cache_creation_input_tokens.is_none()
        {
            return Err(RelayError::Protocol("missing current cache creation usage"));
        }
        let previous_details = self
            .cache_creation_details
            .map(|details| match self.cache_creation_input_tokens {
                Some(total) => details.with_total(total),
                None => Ok(details),
            })
            .transpose()?;
        let new_basis_with_details = (input_advanced || creation_advanced)
            && self
                .cache_creation_details
                .is_some_and(|details| details.has_known_bucket());
        let next_details = if new_basis_with_details {
            let total = next
                .cache_creation_input_tokens
                .ok_or(RelayError::Protocol("missing current cache creation usage"))?;
            let details = next
                .cache_creation_details
                .filter(CacheCreationDetails::has_known_bucket)
                .ok_or(RelayError::Protocol(
                    "missing current cache creation details",
                ))?;
            // A new measurement basis uses only this receipt's aggregate and
            // duration evidence. An old duration cannot fill a new snapshot.
            Some(details.with_total(total)?)
        } else {
            next.cache_creation_details
        };
        if let Some(cost) = next.upstream_cost_atto_usd
            && combined
                .upstream_cost_atto_usd
                .is_some_and(|old| cost < old)
        {
            return Err(RelayError::Protocol("upstream cost decreased"));
        }
        // Cost must belong to the latest provider usage receipt. In particular,
        // an initial zero must not make a later cost-less final usage free.
        combined.upstream_cost_atto_usd = next.upstream_cost_atto_usd;
        field(&mut combined.input_tokens, next.input_tokens)?;
        field(&mut combined.output_tokens, next.output_tokens)?;
        field(&mut combined.cached_input_tokens, next.cached_input_tokens)?;
        field(
            &mut combined.cache_creation_input_tokens,
            next.cache_creation_input_tokens,
        )?;
        if new_basis_with_details {
            combined.cache_creation_details = next_details;
        } else if let Some(details) = next_details {
            let combined_details = combined
                .cache_creation_details
                .get_or_insert_with(CacheCreationDetails::default);
            field(
                &mut combined_details.ephemeral_5m_input_tokens,
                details.ephemeral_5m_input_tokens,
            )?;
            field(
                &mut combined_details.ephemeral_1h_input_tokens,
                details.ephemeral_1h_input_tokens,
            )?;
        }
        field(
            &mut combined.reasoning_output_tokens,
            next.reasoning_output_tokens,
        )?;
        combined.openai_cache_write_details = self.merged_openai_cache_write_details(next)?;
        combined.validate()?;
        if let (Some(mut previous), Some(details)) =
            (previous_details, combined.cache_creation_details)
        {
            let current = match combined.cache_creation_input_tokens {
                Some(total) => details.with_total(total)?,
                None => details,
            };
            // A previously partial split can establish both cumulative lower
            // bounds when its aggregate is known. Check the inferred bucket too.
            field(
                &mut previous.ephemeral_5m_input_tokens,
                current.ephemeral_5m_input_tokens,
            )?;
            field(
                &mut previous.ephemeral_1h_input_tokens,
                current.ephemeral_1h_input_tokens,
            )?;
        }
        *self = combined;
        Ok(())
    }

    fn merge_generic_creation(&mut self, next: Self) -> Result<(), RelayError> {
        fn advances(old: Option<u64>, next: Option<u64>) -> bool {
            next.is_some_and(|value| old.is_none_or(|old| value > old))
        }
        fn field(old: &mut Option<u64>, next: Option<u64>) -> Result<(), RelayError> {
            if let Some(value) = next {
                if old.is_some_and(|old| value < old) {
                    return Err(RelayError::Protocol("usage counter decreased"));
                }
                *old = Some(value);
            }
            Ok(())
        }
        if self.cache_creation_mode != next.cache_creation_mode
            && *self != Self::default()
            && next != Self::default()
        {
            return Err(RelayError::Protocol("cache creation mode changed"));
        }
        self.validate()?;
        next.validate()?;
        let mut combined = *self;
        combined.cache_creation_mode = self.cache_creation_mode.or(next.cache_creation_mode);
        let input_advanced = advances(self.input_tokens, next.input_tokens);
        let uncached_advanced = advances(self.uncached_input_tokens, next.uncached_input_tokens);
        let read_advanced = advances(self.cached_input_tokens, next.cached_input_tokens);
        let raw_creation_advanced = advances(
            self.cache_creation_input_tokens,
            next.cache_creation_input_tokens,
        );
        if (input_advanced || uncached_advanced || read_advanced)
            && self.cache_creation_input_tokens.is_some()
            && next.cache_creation_input_tokens.is_none()
        {
            return Err(RelayError::Protocol("missing current cache creation usage"));
        }
        if let Some(cost) = next.upstream_cost_atto_usd
            && combined
                .upstream_cost_atto_usd
                .is_some_and(|old| cost < old)
        {
            return Err(RelayError::Protocol("upstream cost decreased"));
        }
        combined.upstream_cost_atto_usd = next.upstream_cost_atto_usd;
        field(&mut combined.input_tokens, next.input_tokens)?;
        field(
            &mut combined.uncached_input_tokens,
            next.uncached_input_tokens,
        )?;
        field(&mut combined.output_tokens, next.output_tokens)?;
        field(&mut combined.cached_input_tokens, next.cached_input_tokens)?;
        field(
            &mut combined.cache_creation_input_tokens,
            next.cache_creation_input_tokens,
        )?;
        if let Some(details) = next.cache_creation_details {
            let combined_details = combined
                .cache_creation_details
                .get_or_insert_with(CacheCreationDetails::default);
            field(
                &mut combined_details.ephemeral_5m_input_tokens,
                details.ephemeral_5m_input_tokens,
            )?;
            field(
                &mut combined_details.ephemeral_1h_input_tokens,
                details.ephemeral_1h_input_tokens,
            )?;
        }
        field(
            &mut combined.reasoning_output_tokens,
            next.reasoning_output_tokens,
        )?;
        let effective_advanced = advances(
            self.effective_cache_creation_input_tokens()?,
            combined.effective_cache_creation_input_tokens()?,
        );
        if (effective_advanced || read_advanced)
            && (self.input_tokens.is_some() || self.uncached_input_tokens.is_some())
            && (next.input_tokens.is_none()
                || (self.uncached_input_tokens.is_some() && next.uncached_input_tokens.is_none()))
        {
            return Err(RelayError::Protocol("missing current input usage"));
        }
        if (input_advanced
            || uncached_advanced
            || read_advanced
            || raw_creation_advanced
            || effective_advanced)
            && let Some(previous) = self
                .cache_creation_details
                .filter(CacheCreationDetails::has_known_bucket)
        {
            // Generic remainder cannot prove an absent TTL. Every formerly
            // known bucket needs fresh evidence on the new measurement basis.
            let current = next.cache_creation_details.ok_or(RelayError::Protocol(
                "missing current cache creation details",
            ))?;
            if next.cache_creation_input_tokens.is_none()
                || (previous.ephemeral_5m_input_tokens.is_some()
                    && current.ephemeral_5m_input_tokens.is_none())
                || (previous.ephemeral_1h_input_tokens.is_some()
                    && current.ephemeral_1h_input_tokens.is_none())
            {
                return Err(RelayError::Protocol(
                    "missing current cache creation details",
                ));
            }
        }
        combined.openai_cache_write_details = self.merged_openai_cache_write_details(next)?;
        combined.validate()?;
        *self = combined;
        Ok(())
    }

    fn merged_openai_cache_write_details(
        &self,
        next: Self,
    ) -> Result<Option<OpenAiCacheWriteDetails>, RelayError> {
        let previous = self.openai_cache_write_details;
        let new_basis = [
            (self.input_tokens, next.input_tokens),
            (self.uncached_input_tokens, next.uncached_input_tokens),
            (self.cached_input_tokens, next.cached_input_tokens),
            (
                self.cache_creation_input_tokens,
                next.cache_creation_input_tokens,
            ),
        ]
        .into_iter()
        .any(|(old, new)| new.is_some_and(|value| old.is_none_or(|old| value > old)));
        if new_basis && let Some(old) = previous.filter(OpenAiCacheWriteDetails::has_known_bucket) {
            let current = next.openai_cache_write_details.ok_or(RelayError::Protocol(
                "missing current OpenAI cache write details",
            ))?;
            if next.cache_creation_input_tokens.is_none()
                || (old.ttl_30m_tokens.is_some() && current.ttl_30m_tokens.is_none())
                || (old.ttl_12h_tokens.is_some() && current.ttl_12h_tokens.is_none())
            {
                return Err(RelayError::Protocol(
                    "missing current OpenAI cache write details",
                ));
            }
        }
        let Some(current) = next.openai_cache_write_details else {
            return Ok(previous);
        };
        let mut combined = previous.unwrap_or_default();
        for (old, new) in [
            (&mut combined.ttl_30m_tokens, current.ttl_30m_tokens),
            (&mut combined.ttl_12h_tokens, current.ttl_12h_tokens),
        ] {
            if let Some(value) = new {
                if old.is_some_and(|old| value < old) {
                    return Err(RelayError::Protocol("usage counter decreased"));
                }
                *old = Some(value);
            }
        }
        // A known aggregate and 12h count also bound ordinary creation. Do not
        // let later TTL refinement erase that cumulative billable quantity.
        if let (Some(old_total), Some(old_long), Some(total), Some(long)) = (
            self.cache_creation_input_tokens,
            previous.and_then(|details| details.ttl_12h_tokens),
            next.cache_creation_input_tokens
                .or(self.cache_creation_input_tokens),
            combined.ttl_12h_tokens,
        ) {
            let old_short = old_total.checked_sub(old_long).ok_or(RelayError::Protocol(
                "inconsistent OpenAI cache write usage",
            ))?;
            let short = total.checked_sub(long).ok_or(RelayError::Protocol(
                "inconsistent OpenAI cache write usage",
            ))?;
            if short < old_short {
                return Err(RelayError::Protocol("usage counter decreased"));
            }
        }
        Ok(Some(combined))
    }

    /// Effective creation is derived from real aggregate and TTL evidence.
    /// Generic mode never establishes an aggregate from TTLs alone.
    pub(crate) fn effective_cache_creation_input_tokens(&self) -> Result<Option<u64>, RelayError> {
        match (self.cache_creation_mode, self.cache_creation_input_tokens) {
            (Some(CacheCreationMode::GenericRemainderV1), Some(reported)) => {
                let details = self.cache_creation_details.unwrap_or_default();
                CacheCreationPartition::from_reported(
                    reported,
                    details.ephemeral_5m_input_tokens,
                    details.ephemeral_1h_input_tokens,
                )
                .map(|partition| Some(partition.total()))
                .map_err(|_| RelayError::Protocol("invalid cache creation usage"))
            }
            _ => Ok(self.cache_creation_input_tokens),
        }
    }

    /// Validate both live observations and receipts restored after a restart.
    /// Deserialization alone does not establish counter consistency.
    pub(crate) fn validate(&self) -> Result<(), RelayError> {
        if self.uncached_input_tokens.is_some() && self.cache_creation_mode.is_none() {
            return Err(RelayError::Protocol(
                "uncached input without cache creation mode",
            ));
        }
        if let (Some(i), Some(o)) = (self.input_tokens, self.output_tokens) {
            i.checked_add(o)
                .ok_or(RelayError::Protocol("usage overflow"))?;
        }
        // Input includes both cache categories; output includes reasoning.
        // Missing totals stay unknown. Present subsets are lower bounds, never
        // evidence that an absent counter is zero.
        let known_creation = self
            .cache_creation_details
            .map(|details| {
                let known = details.known_total()?;
                if self.cache_creation_mode.is_none()
                    && let Some(total) = self.cache_creation_input_tokens
                    && (known > total || details.total()?.is_some_and(|sum| sum != total))
                {
                    return Err(RelayError::Protocol("inconsistent cache creation usage"));
                }
                Ok(known)
            })
            .transpose()?
            .unwrap_or(0);
        let openai_creation = self
            .openai_cache_write_details
            .map(|details| {
                let known = details.known_total()?;
                if known_creation != 0 {
                    return Err(RelayError::Protocol("mixed cache write lifetimes"));
                }
                if let Some(total) = self.cache_creation_input_tokens
                    && (known > total
                        || (details.ttl_30m_tokens.is_some()
                            && details.ttl_12h_tokens.is_some()
                            && known != total))
                {
                    return Err(RelayError::Protocol(
                        "inconsistent OpenAI cache write usage",
                    ));
                }
                Ok(known)
            })
            .transpose()?
            .unwrap_or(0);
        let known_creation = known_creation.max(openai_creation);
        let cached = self
            .cached_input_tokens
            .unwrap_or(0)
            .checked_add(
                self.effective_cache_creation_input_tokens()?
                    .unwrap_or(known_creation),
            )
            .ok_or(RelayError::Protocol("usage overflow"))?;
        if self.input_tokens.is_some_and(|total| cached > total) {
            return Err(RelayError::Protocol("cache usage exceeds input total"));
        }
        if let Some(uncached) = self.uncached_input_tokens {
            let lower = uncached
                .checked_add(cached)
                .ok_or(RelayError::Protocol("usage overflow"))?;
            if let Some(input) = self.input_tokens
                && (lower > input || (self.cache_creation_input_tokens.is_some() && lower != input))
            {
                return Err(RelayError::Protocol("inconsistent input usage"));
            }
        }
        if let (Some(total), Some(reasoning)) = (self.output_tokens, self.reasoning_output_tokens)
            && reasoning > total
        {
            return Err(RelayError::Protocol("reasoning usage exceeds output total"));
        }
        Ok(())
    }

    pub(crate) fn wire(&self, client: ClientProtocol) -> Value {
        if *self == Self::default() {
            return Value::Null;
        }
        let mut out = serde_json::Map::new();
        let (input, output) = match client {
            ClientProtocol::Chat => ("prompt_tokens", "completion_tokens"),
            ClientProtocol::Responses => ("input_tokens", "output_tokens"),
            // Native usage has different disjoint counters and is rendered by
            // the native leaf encoders; it never uses the OpenAI wire shape.
            ClientProtocol::Anthropic | ClientProtocol::Gemini => return Value::Null,
        };
        if let Some(v) = self.input_tokens {
            out.insert(input.into(), json!(v));
        }
        if let Some(v) = self.output_tokens {
            out.insert(output.into(), json!(v));
        }
        if let (Some(i), Some(o)) = (self.input_tokens, self.output_tokens)
            && let Some(total) = i.checked_add(o)
        {
            out.insert("total_tokens".into(), json!(total));
        }
        if let Some(v) = self.cached_input_tokens {
            out.insert(format!("{input}_details"), json!({"cached_tokens":v}));
        }
        if let Some(details) = self.openai_cache_write_details {
            let input_details = out
                .entry(format!("{input}_details"))
                .or_insert_with(|| json!({}));
            if let Some(total) = self.cache_creation_input_tokens {
                input_details["cache_write_tokens"] = json!(total);
            }
            input_details["cache_write_tokens_details"] = json!(details);
        }
        if let Some(v) = self.reasoning_output_tokens {
            out.insert(format!("{output}_details"), json!({"reasoning_tokens":v}));
        }
        if let Some(v) = self.cache_creation_input_tokens {
            out.insert("cache_creation_input_tokens".into(), json!(v));
        }
        if let Some(details) = self.cache_creation_details {
            out.insert("cache_creation_details".into(), json!(details));
        }
        Value::Object(out)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    /// Gemini Long Decode hit the provider hop limit and can resume. OpenAI
    /// clients see `length`; native Gemini keeps `CONTINUATION`.
    Continuation,
    ToolCalls,
    ContentFilter,
}
impl FinishReason {
    pub(crate) fn chat(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length | Self::Continuation => "length",
            Self::ToolCalls => "tool_calls",
            Self::ContentFilter => "content_filter",
        }
    }

    pub(crate) fn truncates_output(self) -> bool {
        matches!(
            self,
            Self::Length | Self::Continuation | Self::ContentFilter
        )
    }

    /// Responses incomplete_details. Continuation is a length hop, not a filter.
    pub(crate) fn responses_incomplete_reason(self) -> &'static str {
        match self {
            Self::ContentFilter => "content_filter",
            Self::Length | Self::Continuation => "max_output_tokens",
            Self::Stop | Self::ToolCalls => "stop",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum BlockKind {
    Text,
    Refusal,
    Thinking,
    Tool { id: String, name: String },
}
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Start(String),
    Block {
        index: usize,
        kind: BlockKind,
    },
    Delta {
        index: usize,
        text: String,
    },
    Signature {
        index: usize,
        provider: UpstreamProtocol,
        value: String,
    },
    EndBlock(usize),
    Usage(Usage),
    StopSequence(String),
    /// Opaque Gemini Long Decode token from `candidates[].continuationToken`.
    /// Empty values are never emitted, so marshal does not invent the field.
    ContinuationToken(String),
    Finish(FinishReason),
    Done,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Cancelled,
    TimedOut,
    Failed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Settlement {
    NotReserved,
    Confirmed,
    Pending,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub request_id: String,
    pub route_id: Option<String>,
    pub price_version: Option<u64>,
    pub outcome: Outcome,
    pub usage: Usage,
    pub finish_reason: Option<FinishReason>,
    /// A validated protocol terminal boundary was decoded, independently of
    /// whether the downstream still accepts output.
    pub provider_complete: bool,
    /// True as soon as send starts. A connection error is not proof of no cost.
    pub upstream_attempted: bool,
    pub attempts: u8,
    pub upstream_status: Option<u16>,
    pub upstream_bytes: usize,
    pub emitted_bytes: usize,
}
#[derive(Clone, Debug)]
pub struct Completion {
    pub report: Report,
    pub settlement: Settlement,
    pub error: Option<RelayError>,
}
#[derive(Clone, Copy, Debug)]
pub struct ResponseHead {
    pub status: u16,
    pub content_type: &'static str,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainReport {
    pub cancelled: usize,
    pub remaining: usize,
}

#[cfg(test)]
mod a07_usage_tests {
    use super::*;

    fn creation(five: Option<u64>, hour: Option<u64>) -> CacheCreationDetails {
        CacheCreationDetails {
            ephemeral_5m_input_tokens: five,
            ephemeral_1h_input_tokens: hour,
        }
    }

    #[test]
    fn cache_creation_details_keep_legacy_receipt_bytes_and_missing_dimensions() {
        let bytes = br#"{"input_tokens":10,"output_tokens":4,"cached_input_tokens":3,"cache_creation_input_tokens":2,"reasoning_output_tokens":1}"#;
        let value: Usage = serde_json::from_slice(bytes).unwrap();
        assert_eq!(value.cache_creation_details, None);
        assert_eq!(serde_json::to_vec(&value).unwrap(), bytes);
        assert_eq!(
            serde_json::to_value(creation(None, None)).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(creation(Some(0), None)).unwrap(),
            json!({"ephemeral_5m_input_tokens":0})
        );
        for invalid in [
            json!({"ephemeral_5m_input_tokens":-1}),
            json!({"ephemeral_1h_input_tokens":"1"}),
            json!({"other_duration_input_tokens":1}),
        ] {
            assert!(serde_json::from_value::<CacheCreationDetails>(invalid).is_err());
        }
    }

    #[test]
    fn cache_creation_splits_are_checked_against_aggregate_and_input() {
        for (details, aggregate, input, valid) in [
            (creation(Some(2), Some(3)), Some(5), Some(9), true),
            (creation(Some(2), Some(3)), Some(6), Some(9), false),
            (creation(Some(2), None), Some(5), Some(9), true),
            (creation(None, Some(6)), Some(5), Some(9), false),
            (creation(Some(2), None), None, Some(5), false),
            (creation(Some(2), None), None, Some(6), true),
            (creation(Some(u64::MAX), Some(1)), None, None, false),
        ] {
            let value = Usage {
                input_tokens: input,
                cached_input_tokens: Some(4),
                cache_creation_input_tokens: aggregate,
                cache_creation_details: Some(details),
                ..Usage::default()
            };
            assert_eq!(value.validate().is_ok(), valid, "{value:?}");
        }
    }

    #[test]
    fn cache_creation_merge_preserves_unknowns_and_checks_combined_counters_atomically() {
        let mut value = Usage {
            input_tokens: Some(20),
            cache_creation_input_tokens: Some(5),
            cache_creation_details: Some(creation(Some(2), Some(3))),
            ..Usage::default()
        };
        value
            .merge(Usage {
                output_tokens: Some(1),
                cache_creation_details: Some(creation(None, None)),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(
            value.cache_creation_details,
            Some(creation(Some(2), Some(3)))
        );
        for next in [
            Usage {
                cache_creation_details: Some(creation(Some(1), None)),
                ..Usage::default()
            },
            Usage {
                cache_creation_details: Some(creation(None, Some(2))),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(7),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(6),
                cache_creation_details: Some(creation(Some(4), None)),
                ..Usage::default()
            },
        ] {
            let before = value;
            assert!(value.merge(next).is_err());
            assert_eq!(value, before);
        }
        value
            .merge(Usage {
                cache_creation_input_tokens: Some(7),
                cache_creation_details: Some(creation(Some(4), None)),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(
            value.cache_creation_details,
            Some(creation(Some(4), Some(3)))
        );
        let mut partial = Usage::default();
        partial
            .merge(Usage {
                cache_creation_details: Some(creation(Some(2), None)),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(partial.cache_creation_input_tokens, None);
        assert_eq!(
            partial.cache_creation_details,
            Some(creation(Some(2), None))
        );
        partial
            .merge(Usage {
                cache_creation_details: Some(creation(None, Some(3))),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(
            partial.cache_creation_details,
            Some(creation(Some(2), Some(3)))
        );
        assert_eq!(partial.cache_creation_input_tokens, None);
    }

    #[test]
    fn advancing_input_requires_current_cache_creation_evidence() {
        for previous_input in [None, Some(0)] {
            let mut value = Usage {
                input_tokens: previous_input,
                output_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
                ..Usage::default()
            };
            let before = value;
            assert!(matches!(
                value.merge(Usage {
                    input_tokens: Some(100),
                    output_tokens: Some(1),
                    ..Usage::default()
                }),
                Err(RelayError::Protocol(_))
            ));
            assert_eq!(value, before);
            value
                .merge(Usage {
                    input_tokens: Some(100),
                    output_tokens: Some(1),
                    cache_creation_input_tokens: Some(0),
                    ..Usage::default()
                })
                .unwrap();
            assert_eq!(value.input_tokens, Some(100));
            assert_eq!(value.cache_creation_input_tokens, Some(0));
        }
    }

    #[test]
    fn output_only_and_unchanged_input_keep_anthropic_initial_cache_evidence() {
        let mut value = Usage {
            input_tokens: Some(100),
            output_tokens: Some(0),
            cached_input_tokens: Some(10),
            cache_creation_input_tokens: Some(7),
            cache_creation_details: Some(creation(Some(2), Some(5))),
            ..Usage::default()
        };
        for input_tokens in [None, Some(100)] {
            value
                .merge(Usage {
                    input_tokens,
                    output_tokens: Some(value.output_tokens.unwrap() + 1),
                    ..Usage::default()
                })
                .unwrap();
            assert_eq!(value.input_tokens, Some(100));
            assert_eq!(value.cache_creation_input_tokens, Some(7));
            assert_eq!(
                value.cache_creation_details,
                Some(creation(Some(2), Some(5)))
            );
        }
    }

    #[test]
    fn new_cache_basis_requires_fresh_duration_evidence_and_preserves_atomicity() {
        let initial = Usage {
            input_tokens: Some(100),
            cache_creation_input_tokens: Some(7),
            cache_creation_details: Some(creation(None, Some(5))),
            ..Usage::default()
        };
        for next in [
            Usage {
                cache_creation_input_tokens: Some(8),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(8),
                cache_creation_details: Some(creation(None, None)),
                ..Usage::default()
            },
            Usage {
                input_tokens: Some(101),
                cache_creation_input_tokens: Some(7),
                ..Usage::default()
            },
            Usage {
                input_tokens: Some(101),
                cache_creation_details: Some(creation(None, Some(5))),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(8),
                cache_creation_details: Some(creation(None, Some(7))),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(8),
                cache_creation_details: Some(creation(Some(4), None)),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(8),
                cache_creation_details: Some(creation(None, Some(9))),
                ..Usage::default()
            },
        ] {
            let mut value = initial;
            assert!(
                matches!(value.merge(next), Err(RelayError::Protocol(_))),
                "{next:?}"
            );
            assert_eq!(value, initial);
        }
        let mut value = initial;
        value
            .merge(Usage {
                input_tokens: Some(101),
                cache_creation_input_tokens: Some(8),
                cache_creation_details: Some(creation(None, Some(6))),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(
            value.cache_creation_details,
            Some(creation(Some(2), Some(6)))
        );
        value
            .merge(Usage {
                cache_creation_input_tokens: Some(9),
                cache_creation_details: Some(creation(Some(3), None)),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(
            value.cache_creation_details,
            Some(creation(Some(3), Some(6)))
        );
    }

    #[test]
    fn inferred_cache_duration_cannot_decrease_even_without_a_new_total() {
        let initial = Usage {
            input_tokens: Some(100),
            cache_creation_input_tokens: Some(7),
            cache_creation_details: Some(creation(None, Some(5))),
            ..Usage::default()
        };
        for total in [None, Some(7)] {
            let mut value = initial;
            assert!(
                value
                    .merge(Usage {
                        cache_creation_input_tokens: total,
                        cache_creation_details: Some(creation(None, Some(6))),
                        ..Usage::default()
                    })
                    .is_err()
            );
            assert_eq!(value, initial);
        }
    }

    #[test]
    fn impossible_cache_subsets_are_rejected_without_partial_update() {
        let initial = Usage {
            input_tokens: Some(10),
            output_tokens: Some(3),
            ..Usage::default()
        };
        for next in [
            Usage {
                cached_input_tokens: Some(11),
                ..Usage::default()
            },
            Usage {
                cache_creation_input_tokens: Some(11),
                ..Usage::default()
            },
            Usage {
                cached_input_tokens: Some(6),
                cache_creation_input_tokens: Some(5),
                ..Usage::default()
            },
        ] {
            let mut value = initial;
            assert!(matches!(value.merge(next), Err(RelayError::Protocol(_))));
            assert_eq!(value, initial);
        }
    }

    #[test]
    fn reasoning_is_a_subset_of_output_and_failed_merge_is_atomic() {
        let mut value = Usage {
            input_tokens: Some(2),
            output_tokens: Some(3),
            ..Usage::default()
        };
        let before = value;
        assert!(matches!(
            value.merge(Usage {
                input_tokens: Some(4),
                reasoning_output_tokens: Some(4),
                ..Usage::default()
            }),
            Err(RelayError::Protocol(_))
        ));
        assert_eq!(value, before);
    }

    #[test]
    fn unknown_totals_remain_unknown_until_their_own_evidence_arrives() {
        let mut value = Usage::default();
        value
            .merge(Usage {
                cached_input_tokens: Some(4),
                cache_creation_input_tokens: Some(3),
                reasoning_output_tokens: Some(2),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(value.input_tokens, None);
        assert_eq!(value.output_tokens, None);
        let before = value;
        assert!(
            value
                .merge(Usage {
                    input_tokens: Some(6),
                    ..Usage::default()
                })
                .is_err()
        );
        assert_eq!(value, before);
        value
            .merge(Usage {
                input_tokens: Some(7),
                output_tokens: Some(2),
                cache_creation_input_tokens: Some(3),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(value.cached_input_tokens, Some(4));
        assert_eq!(value.cache_creation_input_tokens, Some(3));
        assert_eq!(value.reasoning_output_tokens, Some(2));
    }

    #[test]
    fn subsets_from_separate_frames_are_checked_together() {
        let mut value = Usage {
            input_tokens: Some(10),
            cached_input_tokens: Some(6),
            ..Usage::default()
        };
        let before = value;
        assert!(
            value
                .merge(Usage {
                    cache_creation_input_tokens: Some(5),
                    ..Usage::default()
                })
                .is_err()
        );
        assert_eq!(value, before);
        value
            .merge(Usage {
                input_tokens: Some(11),
                cache_creation_input_tokens: Some(5),
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(value.input_tokens, Some(11));
    }

    #[test]
    fn overflow_is_rejected_even_when_total_is_unknown() {
        let mut value = Usage {
            cached_input_tokens: Some(u64::MAX),
            ..Usage::default()
        };
        let before = value;
        assert!(
            value
                .merge(Usage {
                    cache_creation_input_tokens: Some(1),
                    ..Usage::default()
                })
                .is_err()
        );
        assert_eq!(value, before);
    }

    #[test]
    fn zero_boundary_repeated_receipt_and_missing_fields_are_valid() {
        let zero = Usage {
            upstream_cost_atto_usd: None,
            uncached_input_tokens: None,
            input_tokens: Some(0),
            output_tokens: Some(0),
            cached_input_tokens: Some(0),
            cache_creation_input_tokens: Some(0),
            cache_creation_details: None,
            cache_creation_mode: None,
            openai_cache_write_details: None,
            reasoning_output_tokens: Some(0),
        };
        let mut value = Usage::default();
        value.merge(zero).unwrap();
        value.merge(zero).unwrap();
        value.merge(Usage::default()).unwrap();
        assert_eq!(value, zero);
    }

    #[test]
    fn decreasing_receipt_and_total_overflow_still_leave_state_unchanged() {
        let mut value = Usage {
            input_tokens: Some(2),
            ..Usage::default()
        };
        let before = value;
        for next in [
            Usage {
                input_tokens: Some(1),
                ..Usage::default()
            },
            Usage {
                output_tokens: Some(u64::MAX),
                ..Usage::default()
            },
        ] {
            assert!(value.merge(next).is_err());
            assert_eq!(value, before);
        }
    }

    #[test]
    fn restored_receipts_need_validation_even_without_a_merge() {
        let value: Usage = serde_json::from_value(serde_json::json!({
            "input_tokens": 2,
            "output_tokens": 3,
            "reasoning_output_tokens": 4,
        }))
        .unwrap();
        assert!(value.validate().is_err());
        assert!(
            serde_json::from_value::<Usage>(serde_json::json!({
                "input_tokens": 2,
                "output_tokens": 3,
                "unknown_dimension": 1,
            }))
            .is_err()
        );
    }
}

#[cfg(test)]
mod openai_cache_write_tests {
    use super::*;

    fn measured(total: Option<u64>, short: Option<u64>, long: Option<u64>) -> Usage {
        Usage {
            input_tokens: Some(100),
            cache_creation_input_tokens: total,
            openai_cache_write_details: Some(OpenAiCacheWriteDetails {
                ttl_30m_tokens: short,
                ttl_12h_tokens: long,
            }),
            ..Usage::default()
        }
    }

    #[test]
    fn old_receipts_keep_their_shape_and_new_receipts_round_trip() {
        let old = Usage {
            input_tokens: Some(100),
            cache_creation_input_tokens: Some(5),
            ..Usage::default()
        };
        let old_json = serde_json::to_value(old).unwrap();
        assert!(old_json.get("openai_cache_write_details").is_none());
        assert_eq!(serde_json::from_value::<Usage>(old_json).unwrap(), old);
        let new = measured(Some(5), Some(2), Some(3));
        let bytes = serde_json::to_vec(&new).unwrap();
        let restored: Usage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, new);
        assert!(restored.validate().is_ok());
        let mut invalid = serde_json::to_value(new).unwrap();
        invalid["openai_cache_write_details"]["ttl_1h_tokens"] = json!(1);
        assert!(serde_json::from_value::<Usage>(invalid).is_err());
    }

    #[test]
    fn known_lifetimes_validate_without_creating_missing_evidence() {
        for value in [
            measured(Some(5), None, Some(3)),
            measured(Some(5), Some(2), None),
            measured(Some(5), None, None),
            measured(None, Some(2), Some(3)),
            measured(None, None, Some(3)),
            measured(Some(0), Some(0), Some(0)),
        ] {
            let before = value;
            assert!(value.validate().is_ok());
            assert_eq!(value, before);
        }
        for value in [
            measured(Some(5), None, Some(6)),
            measured(Some(5), Some(2), Some(4)),
            measured(Some(5), Some(2), Some(2)),
            measured(Some(0), None, Some(1)),
            measured(None, Some(u64::MAX), Some(1)),
            measured(None, None, Some(101)),
            Usage {
                cached_input_tokens: Some(98),
                ..measured(None, None, Some(3))
            },
        ] {
            assert!(value.validate().is_err(), "{value:?}");
        }
    }

    #[test]
    fn anthropic_lifetimes_cannot_be_combined_with_openai_lifetimes() {
        for mode in [None, Some(CacheCreationMode::GenericRemainderV1)] {
            let mixed = Usage {
                cache_creation_details: Some(CacheCreationDetails {
                    ephemeral_5m_input_tokens: Some(2),
                    ephemeral_1h_input_tokens: Some(3),
                }),
                cache_creation_mode: mode,
                ..measured(Some(5), Some(2), Some(3))
            };
            assert!(mixed.validate().is_err());
        }
    }

    #[test]
    fn cumulative_merges_preserve_unknown_zero_and_repeated_observations() {
        let first = measured(Some(5), None, Some(3));
        let mut value = Usage::default();
        value.merge(first).unwrap();
        value.merge(first).unwrap();
        value.merge(Usage::default()).unwrap();
        assert_eq!(value, first);
        value.merge(measured(Some(6), None, Some(4))).unwrap();
        assert_eq!(
            value.openai_cache_write_details.unwrap().ttl_30m_tokens,
            None
        );
        assert_eq!(
            value.openai_cache_write_details.unwrap().ttl_12h_tokens,
            Some(4)
        );
        let mut zero = Usage::default();
        zero.merge(measured(Some(0), None, Some(0))).unwrap();
        assert_eq!(
            zero.openai_cache_write_details.unwrap().ttl_12h_tokens,
            Some(0)
        );
        assert_ne!(zero.openai_cache_write_details, None);
    }

    #[test]
    fn changing_the_measurement_basis_requires_fresh_known_lifetimes() {
        for mode in [None, Some(CacheCreationMode::GenericRemainderV1)] {
            let initial = Usage {
                cache_creation_mode: mode,
                ..measured(Some(5), Some(2), Some(3))
            };
            for next in [
                Usage {
                    cache_creation_input_tokens: Some(6),
                    openai_cache_write_details: None,
                    ..initial
                },
                measured(Some(6), None, Some(4)),
                measured(Some(6), Some(3), None),
                measured(None, Some(3), Some(3)),
                measured(Some(6), Some(1), Some(5)),
                measured(Some(6), Some(4), Some(2)),
                Usage {
                    input_tokens: Some(101),
                    openai_cache_write_details: None,
                    ..initial
                },
            ] {
                let next = Usage {
                    cache_creation_mode: mode,
                    ..next
                };
                let mut value = initial;
                assert!(value.merge(next).is_err(), "{next:?}");
                assert_eq!(value, initial, "a rejected observation must be atomic");
            }
            let mut value = initial;
            value
                .merge(Usage {
                    cache_creation_mode: mode,
                    ..measured(Some(6), Some(3), Some(3))
                })
                .unwrap();
            assert_eq!(value.cache_creation_input_tokens, Some(6));
        }
    }

    #[test]
    fn a_later_lifetime_cannot_reduce_the_ordinary_creation_remainder() {
        let initial = measured(Some(5), None, Some(3));
        for next in [
            measured(Some(5), None, Some(4)),
            measured(None, None, Some(4)),
            measured(Some(6), None, Some(5)),
        ] {
            let mut value = initial;
            assert!(value.merge(next).is_err());
            assert_eq!(value, initial);
        }
    }

    #[test]
    fn openai_wire_uses_the_provider_shape_without_internal_markers() {
        let value = Usage {
            cached_input_tokens: Some(7),
            ..measured(Some(5), None, Some(3))
        };
        for (client, input) in [
            (ClientProtocol::Chat, "prompt_tokens_details"),
            (ClientProtocol::Responses, "input_tokens_details"),
        ] {
            let wire = value.wire(client);
            assert_eq!(wire[input]["cached_tokens"], 7);
            assert_eq!(wire[input]["cache_write_tokens"], 5);
            assert_eq!(
                wire[input]["cache_write_tokens_details"],
                json!({"ttl_12h_tokens":3})
            );
            assert_eq!(wire["cache_creation_input_tokens"], 5);
            assert!(wire.get("openai_cache_write_details").is_none());
            assert!(wire.get("cache_creation_mode").is_none());
        }
        assert_eq!(value.wire(ClientProtocol::Anthropic), Value::Null);
        assert_eq!(value.wire(ClientProtocol::Gemini), Value::Null);
    }
}

#[cfg(test)]
mod generic_creation_tests {
    use super::*;

    fn measured(
        input: Option<u64>,
        aggregate: Option<u64>,
        five: Option<u64>,
        hour: Option<u64>,
    ) -> Usage {
        Usage {
            input_tokens: input,
            cache_creation_input_tokens: aggregate,
            cache_creation_details: (five.is_some() || hour.is_some()).then_some(
                CacheCreationDetails {
                    ephemeral_5m_input_tokens: five,
                    ephemeral_1h_input_tokens: hour,
                },
            ),
            cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
            ..Usage::default()
        }
    }

    #[test]
    fn generic_effective_total_preserves_raw_aggregate_and_unknown_ttls() {
        let value = Usage {
            cached_input_tokens: Some(20),
            ..measured(Some(37), Some(2), Some(3), Some(4))
        };
        assert!(value.validate().is_ok());
        assert_eq!(value.cache_creation_input_tokens, Some(2));
        assert_eq!(
            value.effective_cache_creation_input_tokens().unwrap(),
            Some(7)
        );
        assert!(
            Usage {
                input_tokens: Some(26),
                ..value
            }
            .validate()
            .is_err()
        );
        assert!(
            Usage {
                cache_creation_mode: None,
                ..value
            }
            .validate()
            .is_err()
        );
        let aggregate_only = measured(Some(10), Some(10), None, None);
        assert_eq!(
            aggregate_only
                .effective_cache_creation_input_tokens()
                .unwrap(),
            Some(10)
        );
        assert_eq!(aggregate_only.cache_creation_details, None);
        let no_aggregate = measured(None, None, Some(0), Some(0));
        assert_eq!(
            no_aggregate
                .effective_cache_creation_input_tokens()
                .unwrap(),
            None
        );
        let no_aggregate = measured(None, None, Some(2), Some(3));
        assert_eq!(
            no_aggregate
                .effective_cache_creation_input_tokens()
                .unwrap(),
            None
        );
    }

    #[test]
    fn generic_raw_decrease_cannot_hide_behind_unchanged_effective_total() {
        let mut value = measured(Some(100), Some(5), Some(10), Some(0));
        let before = value;
        assert!(
            value
                .merge(measured(Some(100), Some(4), Some(10), Some(0)))
                .is_err()
        );
        assert_eq!(value, before);
    }

    #[test]
    fn generic_classification_can_reduce_remainder_without_inventing_ttls() {
        let mut value = measured(Some(100), Some(10), None, None);
        value.merge(measured(None, None, Some(2), Some(3))).unwrap();
        assert_eq!(value.cache_creation_input_tokens, Some(10));
        assert_eq!(
            value.cache_creation_details,
            Some(CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(2),
                ephemeral_1h_input_tokens: Some(3),
            })
        );
        assert_eq!(
            value.effective_cache_creation_input_tokens().unwrap(),
            Some(10)
        );
        let mut value = measured(Some(100), Some(7), None, Some(5));
        value
            .merge(measured(Some(101), Some(8), None, Some(7)))
            .unwrap();
        assert_eq!(
            value
                .cache_creation_details
                .unwrap()
                .ephemeral_5m_input_tokens,
            None
        );
        assert_eq!(
            value.effective_cache_creation_input_tokens().unwrap(),
            Some(8)
        );
    }

    #[test]
    fn generic_new_basis_requires_all_previously_known_ttls() {
        let initial = measured(Some(100), Some(7), Some(2), Some(5));
        for next in [
            measured(Some(101), Some(8), None, Some(7)),
            measured(Some(101), Some(8), Some(3), None),
            measured(Some(101), Some(8), None, None),
            measured(Some(101), None, Some(3), Some(5)),
            measured(Some(101), Some(8), Some(1), Some(7)),
        ] {
            let mut value = initial;
            assert!(value.merge(next).is_err(), "{next:?}");
            assert_eq!(value, initial);
        }
        let mut value = initial;
        value
            .merge(measured(Some(101), Some(8), Some(2), Some(6)))
            .unwrap();
        assert_eq!(value.cache_creation_input_tokens, Some(8));
    }

    #[test]
    fn generic_effective_growth_needs_fresh_inclusive_input() {
        let initial = measured(Some(100), Some(2), Some(3), Some(4));
        let mut value = initial;
        assert!(
            value
                .merge(measured(None, Some(2), Some(3), Some(5)))
                .is_err()
        );
        assert_eq!(value, initial);
        value
            .merge(measured(Some(101), Some(2), Some(3), Some(5)))
            .unwrap();
        assert_eq!(
            value.effective_cache_creation_input_tokens().unwrap(),
            Some(8)
        );
    }

    #[test]
    fn generic_new_input_cannot_reuse_old_zero_creation() {
        for input in [None, Some(0)] {
            let initial = measured(input, Some(0), None, None);
            let mut value = initial;
            assert!(
                value
                    .merge(Usage {
                        output_tokens: Some(1),
                        ..measured(Some(100), None, None, None)
                    })
                    .is_err()
            );
            assert_eq!(value, initial);
        }
    }

    #[test]
    fn generic_fixed_basis_output_only_preserves_same_request_evidence() {
        let mut value = measured(Some(100), Some(10), Some(2), None);
        value
            .merge(Usage {
                output_tokens: Some(1),
                ..measured(None, None, None, None)
            })
            .unwrap();
        value
            .merge(Usage {
                output_tokens: Some(2),
                ..measured(Some(100), None, None, None)
            })
            .unwrap();
        value.merge(Usage::default()).unwrap();
        assert_eq!(value.output_tokens, Some(2));
        assert_eq!(value.cache_creation_input_tokens, Some(10));
        assert_eq!(
            value
                .cache_creation_details
                .unwrap()
                .ephemeral_1h_input_tokens,
            None
        );
    }

    #[test]
    fn generic_mode_cannot_relabel_an_existing_receipt() {
        let generic = measured(Some(100), Some(10), None, None);
        let strict = Usage {
            cache_creation_mode: None,
            ..generic
        };
        for (initial, next) in [(generic, strict), (strict, generic)] {
            let mut value = initial;
            assert!(value.merge(next).is_err());
            assert_eq!(value, initial);
        }
        let mut value = Usage::default();
        value.merge(generic).unwrap();
        assert_eq!(value, generic);
    }

    #[test]
    fn generic_overflow_and_cache_bounds_leave_merge_unchanged() {
        let initial = measured(Some(10), Some(2), None, None);
        for next in [
            measured(Some(10), Some(2), Some(u64::MAX), Some(1)),
            Usage {
                cached_input_tokens: Some(u64::MAX),
                ..measured(None, Some(1), None, None)
            },
            Usage {
                output_tokens: Some(u64::MAX),
                ..measured(None, None, None, None)
            },
        ] {
            let mut value = initial;
            assert!(value.merge(next).is_err());
            assert_eq!(value, initial);
        }
    }

    #[test]
    fn generic_mode_roundtrips_durably_and_never_enters_client_wire() {
        let value = measured(Some(37), Some(2), Some(3), Some(4));
        let durable = serde_json::to_value(value).unwrap();
        assert_eq!(durable["cache_creation_mode"], "generic_remainder_v1");
        assert_eq!(
            serde_json::to_string(&Usage::default()).unwrap(),
            r#"{"input_tokens":null,"output_tokens":null,"cached_input_tokens":null,"cache_creation_input_tokens":null,"reasoning_output_tokens":null}"#
        );
        assert_eq!(serde_json::from_value::<Usage>(durable).unwrap(), value);
        for protocol in [ClientProtocol::Chat, ClientProtocol::Responses] {
            let wire = value.wire(protocol);
            assert_eq!(wire["cache_creation_input_tokens"], 2);
            assert!(wire.get("cache_creation_mode").is_none());
        }
        assert!(
            serde_json::to_value(Usage::default())
                .unwrap()
                .get("cache_creation_mode")
                .is_none()
        );
        assert!(serde_json::from_value::<Usage>(json!({"input_tokens":null,"output_tokens":null,"cached_input_tokens":null,"cache_creation_input_tokens":null,"reasoning_output_tokens":null,"cache_creation_mode":"unreviewed"})).is_err());
    }
}

#[cfg(test)]
mod generic_uncached_input_tests {
    use super::*;

    fn initial() -> Usage {
        Usage {
            input_tokens: Some(100),
            uncached_input_tokens: Some(70),
            cached_input_tokens: Some(20),
            cache_creation_input_tokens: Some(10),
            cache_creation_details: Some(CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(2),
                ephemeral_1h_input_tokens: Some(3),
            }),
            cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
            ..Usage::default()
        }
    }

    #[test]
    fn generic_read_growth_needs_current_raw_input_and_ttl_evidence() {
        let before = initial();
        let mut value = before;
        let next = Usage {
            input_tokens: None,
            uncached_input_tokens: None,
            cached_input_tokens: Some(30),
            output_tokens: Some(1),
            ..before
        };
        assert!(value.merge(next).is_err());
        assert_eq!(value, before);
        let next = Usage {
            input_tokens: Some(110),
            cached_input_tokens: Some(30),
            output_tokens: Some(1),
            ..before
        };
        value.merge(next).unwrap();
        assert_eq!(value.uncached_input_tokens, Some(70));
        assert_eq!(value.input_tokens, Some(110));
    }

    #[test]
    fn generic_raw_input_is_monotone_and_cannot_advance_without_current_creation() {
        let before = initial();
        for next in [
            Usage {
                input_tokens: Some(99),
                uncached_input_tokens: Some(69),
                ..before
            },
            Usage {
                input_tokens: None,
                uncached_input_tokens: Some(71),
                cache_creation_input_tokens: None,
                ..before
            },
            Usage {
                input_tokens: Some(101),
                uncached_input_tokens: Some(71),
                cache_creation_details: None,
                ..before
            },
        ] {
            let mut value = before;
            assert!(value.merge(next).is_err());
            assert_eq!(value, before);
        }
    }

    #[test]
    fn generic_raw_input_is_retained_when_aggregate_absent_without_becoming_canonical() {
        let first = Usage {
            uncached_input_tokens: Some(10),
            cached_input_tokens: Some(20),
            cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
            ..Usage::default()
        };
        let mut value = Usage::default();
        value.merge(first).unwrap();
        value
            .merge(Usage {
                output_tokens: Some(1),
                cache_creation_mode: first.cache_creation_mode,
                ..Usage::default()
            })
            .unwrap();
        assert_eq!(value.uncached_input_tokens, Some(10));
        assert_eq!(value.input_tokens, None);
        assert_eq!(value.cache_creation_input_tokens, None);
        assert_eq!(
            serde_json::from_value::<Usage>(serde_json::to_value(value).unwrap()).unwrap(),
            value
        );
        for protocol in [ClientProtocol::Chat, ClientProtocol::Responses] {
            assert!(value.wire(protocol).get("uncached_input_tokens").is_none());
        }
    }

    #[test]
    fn generic_restored_input_must_match_raw_input_read_and_effective_creation() {
        let before = initial();
        let mut empty = Usage::default();
        assert!(
            empty
                .merge(Usage {
                    uncached_input_tokens: Some(1),
                    ..Usage::default()
                })
                .is_err()
        );
        assert_eq!(empty, Usage::default());
        assert!(before.validate().is_ok());
        assert!(
            Usage {
                input_tokens: Some(101),
                ..before
            }
            .validate()
            .is_err()
        );
        assert!(
            Usage {
                cache_creation_mode: None,
                ..before
            }
            .validate()
            .is_err()
        );
        assert!(
            Usage {
                input_tokens: None,
                uncached_input_tokens: Some(u64::MAX),
                ..before
            }
            .validate()
            .is_err()
        );
    }
}
