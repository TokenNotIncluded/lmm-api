//! Native, non-chat supplier operations use the same Core identity, immutable
//! prices, durable holds and receipt checkpoints as model traffic.
#[path = "deepseek_tools_contract.rs"]
mod deepseek;
#[path = "ali_embedding_contract.rs"]
mod embedding;

use super::*;
use crate::{
    billing::charging::{self, Outcome, ReservationBasis},
    routing::{
        BillingMode, CreditRate, PayloadPolicy, PersistedEmbeddingPrice, PersistedSearchPrice,
        Protocol, ReplayPermission, RouteRequest, SearchBillingUnit,
    },
};
use axum::{body::to_bytes, extract::Request};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(super) struct Host {
    gate: Arc<Semaphore>,
    parallelism: usize,
    transport: relay::provider_network::ProviderTransport,
}
impl Host {
    pub(super) fn new(limits: &Limits) -> Self {
        let parallelism = limits.max_in_flight.min(2);
        Self {
            gate: Arc::new(Semaphore::new(parallelism)),
            parallelism,
            transport: relay::provider_network::ProviderTransport::new(parallelism),
        }
    }
    pub(super) async fn drain(&self, grace: Duration) -> usize {
        self.gate.close();
        let deadline = tokio::time::Instant::now() + grace;
        while self.gate.available_permits() < self.parallelism
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.parallelism
            .saturating_sub(self.gate.available_permits())
    }
}

pub(super) fn router(gateway: ModelGateway) -> Router {
    Router::new()
        .route("/v1/tools/search", post(search))
        .route("/v1/tools/fetch", post(fetch))
        .route("/v1/embeddings", post(embeddings))
        .route(
            "/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding",
            post(native_embeddings),
        )
        .layer(axum::middleware::map_response(no_store))
        .with_state(gateway)
}
async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
type Peer = Result<ConnectInfo<SocketAddr>, ExtensionRejection>;
async fn search(State(gateway): State<ModelGateway>, peer: Peer, request: Request) -> Response {
    handle(gateway, peer, request, Entry::Tool(deepseek::Kind::Search)).await
}
async fn fetch(State(gateway): State<ModelGateway>, peer: Peer, request: Request) -> Response {
    handle(gateway, peer, request, Entry::Tool(deepseek::Kind::Fetch)).await
}
async fn embeddings(State(gateway): State<ModelGateway>, peer: Peer, request: Request) -> Response {
    handle(
        gateway,
        peer,
        request,
        Entry::Embedding(embedding::Client::OpenAi),
    )
    .await
}
async fn native_embeddings(
    State(gateway): State<ModelGateway>,
    peer: Peer,
    request: Request,
) -> Response {
    handle(
        gateway,
        peer,
        request,
        Entry::Embedding(embedding::Client::Native),
    )
    .await
}

#[derive(Clone, Copy)]
enum Entry {
    Tool(deepseek::Kind),
    Embedding(embedding::Client),
}
enum NativeRequest {
    Tool(deepseek::Request),
    Embedding(embedding::Request),
}
enum Bound {
    Tool,
    Embedding(embedding::Bound),
}
impl NativeRequest {
    fn model(&self) -> &str {
        match self {
            Self::Tool(v) => &v.model,
            Self::Embedding(v) => &v.model,
        }
    }
    fn protocol(&self) -> Protocol {
        match self {
            Self::Tool(_) => Protocol::DeepSeekTools,
            Self::Embedding(_) => Protocol::AliMultimodalEmbedding,
        }
    }
    fn response(
        &self,
        bound: &Bound,
        status: u16,
        body: &[u8],
    ) -> Result<(Value, relay::Usage, Option<u16>), RelayError> {
        match (self, bound) {
            (Self::Tool(request), Bound::Tool) => {
                let observed = request.response(status, body)?;
                Ok((observed.output, observed.usage, observed.upstream_error))
            }
            (Self::Embedding(request), Bound::Embedding(bound)) => {
                if status != 200 {
                    return Err(RelayError::UpstreamStatus(status));
                }
                let observed = request.response(bound, body)?;
                Ok((observed.output, observed.usage, None))
            }
            _ => Err(RelayError::SettlementPending),
        }
    }
}

async fn handle(gateway: ModelGateway, peer: Peer, request: Request, entry: Entry) -> Response {
    let headers = request.headers().clone();
    if oauth_header(&headers) {
        return public_error(RelayError::Forbidden);
    }
    if request.uri().query().is_some()
        || ["x-api-key", "x-goog-api-key", "x-auth-session"]
            .iter()
            .any(|h| headers.contains_key(*h))
        || headers.contains_key(header::CONTENT_ENCODING)
    {
        return public_error(RelayError::Invalid(
            "unsupported native operation credential or encoding",
        ));
    }
    let content_type = match one_header(&headers, header::CONTENT_TYPE.as_str()) {
        Ok(Some(value)) => value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase(),
        _ => return public_error(RelayError::Invalid("missing native operation content type")),
    };
    if content_type != "application/json" {
        return public_error(RelayError::Unsupported("native operation requires JSON"));
    }
    let peer = peer.ok().map(|p| p.0.ip());
    if let Some(honeypot) = &gateway.honeypot {
        match honeypot.models(&headers, peer).await {
            Ok(Some(_)) => {
                return public_error(RelayError::Unsupported(
                    "native operation honeypot policy is not configured",
                ));
            }
            Ok(None) => (),
            Err(error) => return public_error(error),
        }
    }
    let (principal, secret, groups) = match gateway.authenticate(&headers).await {
        Ok(value) => value,
        Err(error) => return public_error(error),
    };
    let Some(key_id) = principal.key_id() else {
        return public_error(RelayError::Forbidden);
    };
    let permit = match gateway.native_operations.gate.clone().try_acquire_owned() {
        Ok(value) => value,
        Err(_) => return public_error(RelayError::Busy),
    };
    let cap = match entry {
        Entry::Tool(_) => deepseek::MAX_BODY,
        Entry::Embedding(_) => embedding::MAX_BODY,
    }
    .min(gateway.limits.request_bytes);
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > cap)
    {
        return public_error(RelayError::Limit("native operation body"));
    }
    let bytes = match tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(request.into_body(), cap),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return public_error(RelayError::Limit("native operation body")),
        Err(_) => return public_error(RelayError::Timeout(relay::Phase::Read)),
    };
    // This detached Core worker owns its admission slot through settlement even
    // when the caller disconnects before receiving its terminal acknowledgement.
    match tokio::spawn(process(
        gateway, principal, secret, groups, peer, key_id, headers, entry, bytes, permit,
    ))
    .await
    {
        Ok(response) => response,
        Err(_) => public_error(RelayError::SettlementPending),
    }
}

#[allow(clippy::too_many_arguments)]
async fn process(
    gateway: ModelGateway,
    principal: ModelIdentity,
    secret: String,
    groups: Vec<String>,
    peer: Option<IpAddr>,
    key_id: i64,
    headers: HeaderMap,
    entry: Entry,
    bytes: Bytes,
    permit: OwnedSemaphorePermit,
) -> Response {
    let body_hash: [u8; 32] = Sha256::digest(&bytes).into();
    let request = match tokio::task::spawn_blocking(move || match entry {
        Entry::Tool(kind) => deepseek::Request::parse(kind, &bytes).map(NativeRequest::Tool),
        Entry::Embedding(client) => {
            embedding::Request::parse(client, &bytes).map(NativeRequest::Embedding)
        }
    })
    .await
    {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => return public_error(error),
        Err(_) => return public_error(RelayError::Invalid("native operation parsing failed")),
    };
    let id = match request_id(key_id, &headers) {
        Ok(id) => id,
        Err(error) => return public_error(error),
    };
    let response_id = id.clone();
    let response = match execute(
        gateway, principal, secret, groups, peer, request, id, body_hash, permit,
    )
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => public_error(error),
    };
    with_request_id(response, &response_id)
}

struct Active {
    bridge: Arc<Bridge>,
    id: String,
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut active) = self.bridge.active.lock() {
            active.remove(&self.id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    gateway: ModelGateway,
    principal: ModelIdentity,
    secret: String,
    groups: Vec<String>,
    peer: Option<IpAddr>,
    request: NativeRequest,
    id: String,
    body_hash: [u8; 32],
    _permit: OwnedSemaphorePermit,
) -> Result<Value, RelayError> {
    if gateway
        .bridge
        .charging
        .resolve(&id)
        .await
        .map_err(billing::error)?
        .is_some()
    {
        return Err(RelayError::Conflict);
    }
    let mut selected = None;
    for group in groups {
        if let Ok(plan) = gateway.bridge.routing.route(RouteRequest {
            model: request.model(),
            group: &group,
            speed: None,
            protocol: request.protocol(),
            replay: ReplayPermission::Denied,
        }) {
            gateway
                .authorize_request(&principal, &secret, plan.canonical_model(), &group, peer)
                .await
                .map_err(identity_error)?;
            selected = Some((group, plan));
            break;
        }
    }
    let (group, mut plan) = selected.ok_or(RelayError::RouteUnavailable)?;
    let quote = plan
        .quote()
        .with_credit_rate(CreditRate::new(1, 500_000).map_err(|_| RelayError::BillingUnavailable)?);
    let token_rates = quote.price().rates();
    if quote.price().billing_mode() != BillingMode::Tokens
        || quote.price().tiered().is_some()
        || [
            token_rates.input,
            token_rates.output,
            token_rates.cache_read,
            token_rates.cache_write,
            token_rates.request,
        ]
        .into_iter()
        .any(|rate| rate != 0)
    {
        return Err(RelayError::Unsupported(
            "native operations require their explicit independent tariff",
        ));
    }
    let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
    if attempt.protocol() != request.protocol()
        || attempt.payload_policy() != PayloadPolicy::Adapt
        || !attempt.is_available()
        || u128::from(attempt.timeout_ms()) < gateway.limits.total_timeout.as_millis()
    {
        return Err(RelayError::RouteUnavailable);
    }
    let mut price = routing::credit_price(quote.price(), plan.canonical_model(), &group)?;
    let (group_multiplier_ppm, speed_multiplier_ppm) = quote.price().multipliers_ppm();
    let (input_limit, wire, bound) = match &request {
        NativeRequest::Tool(request) => {
            if quote.price().embedding().is_some() {
                return Err(RelayError::Unsupported(
                    "native tools cannot use embedding prices",
                ));
            }
            let contract = gateway.bridge.provider_policy.search_contract(&attempt)?;
            let tariff = quote
                .price()
                .search()
                .ok_or(RelayError::Unsupported("native tool tariff is missing"))?;
            if contract.provider != request.kind.provider()
                || tariff.provider != contract.provider
                || tariff.unit != SearchBillingUnit::Call
                || contract.limits.calls != Some(1)
            {
                return Err(RelayError::Unsupported(
                    "native tool operation and price do not match",
                ));
            }
            price.search = Some(PersistedSearchPrice {
                tariff,
                limits: contract.limits,
                supplier_contract_revision: contract.contract_revision.clone(),
                group_multiplier_ppm,
                speed_multiplier_ppm,
                credits_per_usd: quote.conversion().credits_per_usd(),
            });
            (0_u64, request.wire.clone(), Bound::Tool)
        }
        NativeRequest::Embedding(request) => {
            if quote.price().search().is_some() {
                return Err(RelayError::Unsupported(
                    "embedding prices cannot include search charges",
                ));
            }
            let (contract, input_limit) = gateway
                .bridge
                .provider_policy
                .embedding_contract(&attempt)?;
            let tariff = quote
                .price()
                .embedding()
                .ok_or(RelayError::Unsupported("embedding tariff is missing"))?;
            if tariff.provider != contract.provider {
                return Err(RelayError::Unsupported(
                    "embedding price and receipt contract do not match",
                ));
            }
            let bound = request.bind(contract, input_limit)?;
            price.embedding = Some(PersistedEmbeddingPrice {
                tariff,
                input_token_limit: u64::from(input_limit),
                provider_contract_revision: contract.contract_revision.clone(),
                group_multiplier_ppm,
                speed_multiplier_ppm,
                credits_per_usd: quote.conversion().credits_per_usd(),
            });
            (
                u64::from(input_limit),
                request.outbound(attempt.upstream_model(), contract),
                Bound::Embedding(bound),
            )
        }
    };
    price.validate().map_err(billing::error)?;
    let basis = ReservationBasis::Tokens {
        input: input_limit,
        output: 0,
    };
    let reserve = price.reservation_ceiling(basis).map_err(billing::error)?;
    let reference = attempt.credential_ref();
    let credential = gateway
        .bridge
        .credentials
        .get(&(reference.id, reference.revision))
        .ok_or(RelayError::RouteUnavailable)?
        .clone();
    let endpoint = endpoint(attempt.endpoint(), &request)?;
    let connection = tokio::time::timeout(
        gateway.limits.total_timeout,
        gateway
            .native_operations
            .transport
            .prepare(&endpoint, &gateway.limits),
    )
    .await
    .map_err(|_| RelayError::RouteUnavailable)??;
    let encoded = serde_json::to_vec(&wire)
        .map_err(|_| RelayError::Invalid("native operation outbound body"))?;
    if encoded.len() > embedding::MAX_BODY.min(gateway.limits.request_bytes) {
        return Err(RelayError::Limit("native operation outbound body"));
    }
    let upstream = connection
        .client()
        .post(endpoint)
        .bearer_auth(&credential)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
        .header(header::ACCEPT_ENCODING, "identity")
        .body(encoded)
        .build()
        .map_err(|_| RelayError::Invalid("native operation outbound request"))?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"lmm-native-operations-v1\0");
    fingerprint.update(group.as_bytes());
    fingerprint.update([0]);
    fingerprint.update(match &request {
        NativeRequest::Tool(request) => request.kind.name(),
        NativeRequest::Embedding(_) => "embedding",
    });
    fingerprint.update(body_hash);
    let fingerprint: [u8; 32] = fingerprint.finalize().into();
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    let selected = routing::Selected {
        route_id: format!(
            "r{}-t{}-u{}",
            attempt.route_id(),
            attempt.target_id(),
            attempt.upstream_id()
        ),
        attempt: attempt.clone(),
        price: price.clone(),
        quote,
        input_token_ceiling: input_limit as i64,
    };
    {
        let mut active = gateway
            .bridge
            .active
            .lock()
            .map_err(|_| RelayError::BillingUnavailable)?;
        if active.contains_key(&id) {
            return Err(RelayError::Conflict);
        }
        active.insert(
            id.clone(),
            Operation {
                secret: secret.clone(),
                group,
                worker,
                fingerprint,
                selected: Some(selected),
                output_token_ceiling: 0,
                peer,
                oauth_issuer: None,
            },
        );
    }
    let _active = Active {
        bridge: gateway.bridge.clone(),
        id: id.clone(),
    };
    let spec = charging::Request {
        id: id.clone(),
        fingerprint,
        price,
        reserve,
        lease_seconds: gateway.bridge.policy.lease_seconds,
        maximum_seconds: gateway.bridge.policy.lease_seconds,
    };
    let charge = billing::retry(|| {
        gateway
            .bridge
            .charging
            .reserve_model_with_context(&secret, &spec, &worker, peer, basis)
    })
    .await
    .map_err(billing::error)?;
    if charge.actor_user_id != principal.user_id() || charge.key_id != principal.key_id() {
        return Err(RelayError::Forbidden);
    }
    gateway.bridge.freeze_selected_price(&id, &charge)?;
    if !attempt.is_available() {
        let _ = billing::retry(|| gateway.bridge.charging.release(&id, &worker)).await;
        return Err(RelayError::RouteUnavailable);
    }
    if !billing::retry(|| gateway.bridge.charging.start(&id, &worker))
        .await
        .map_err(billing::error)?
    {
        return Err(RelayError::Conflict);
    }
    let result = tokio::time::timeout(gateway.limits.total_timeout, async {
        let mut response = tokio::time::timeout(
            gateway.limits.header_timeout,
            connection.client().execute(upstream),
        )
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)?;
        if !connection.matches_peer(response.remote_addr())
            || response.headers().contains_key(header::CONTENT_ENCODING)
            || !response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(';')
                        .next()
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
                })
        {
            return Err(RelayError::SettlementPending);
        }
        let status = response.status().as_u16();
        let cap = gateway
            .limits
            .upstream_bytes
            .min(gateway.limits.output_bytes);
        if response.content_length().is_some_and(|n| n > cap as u64) {
            return Err(RelayError::SettlementPending);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout(gateway.limits.read_timeout, response.chunk())
            .await
            .map_err(|_| RelayError::SettlementPending)?
            .map_err(|_| RelayError::SettlementPending)?
        {
            if chunk.len() > gateway.limits.transport_chunk_bytes
                || bytes.len().checked_add(chunk.len()).is_none_or(|n| n > cap)
            {
                return Err(RelayError::SettlementPending);
            }
            bytes.extend_from_slice(&chunk);
        }
        let (output, observed, failure) = request.response(&bound, status, &bytes)?;
        let outcome = if failure.is_some() {
            Outcome::Failed
        } else {
            Outcome::Completed
        };
        let terminal_outcome = if failure.is_some() {
            relay::TerminalOutcome::Failed
        } else {
            relay::TerminalOutcome::Completed
        };
        let normalized = serde_json::to_vec(&output).map_err(|_| RelayError::SettlementPending)?;
        if std::str::from_utf8(&bytes).is_ok_and(|v| v.contains(&credential))
            || std::str::from_utf8(&normalized).is_ok_and(|v| v.contains(&credential))
        {
            return Err(RelayError::SettlementPending);
        }
        let complete = billing::complete_priced_usage(observed, &charge.price);
        gateway
            .bridge
            .persist_output(
                &relay::Reservation { id: id.clone() },
                &relay::OutputCheckpoint {
                    sequence: 1,
                    packet_sha256: Sha256::digest(&bytes).into(),
                    packet_bytes: bytes.len(),
                    usage: observed,
                    terminal_observed: complete.is_ok(),
                    terminal_outcome: complete.is_ok().then_some(terminal_outcome),
                },
            )
            .await?;
        let usage = complete?;
        let settled =
            billing::retry(|| gateway.bridge.charging.settle(&id, &worker, usage, outcome)).await;
        let confirmed = match settled {
            Ok(charge) => Some(charge),
            Err(charging::Error::CommitUnknown) => gateway
                .bridge
                .charging
                .resolve(&id)
                .await
                .map_err(billing::error)?,
            _ => None,
        };
        if !confirmed.as_ref().is_some_and(|resolved| {
            billing::confirmed_settlement(resolved, &id, &charge.price, usage, outcome)
        }) {
            return Err(RelayError::SettlementPending);
        }
        Ok((output, failure))
    })
    .await
    .unwrap_or(Err(RelayError::SettlementPending));
    match result {
        Ok((output, None)) => Ok(output),
        // Keep native error text and URLs private, even after zero settlement.
        Ok((_, Some(status))) => Err(RelayError::UpstreamStatus(status)),
        Err(error) => {
            let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
            Err(error)
        }
    }
}

fn endpoint(base: &str, request: &NativeRequest) -> Result<reqwest::Url, RelayError> {
    let mut url = relay::honeypot_network::validate_base_url(base, true)?;
    let path = url.path().trim_end_matches('/').to_owned();
    let suffix: Vec<&str> = match request {
        NativeRequest::Tool(request) => {
            if path.ends_with(&format!("/v1/tools/{}", request.kind.name())) {
                return Ok(url);
            }
            if path.ends_with("/v1/tools/search") || path.ends_with("/v1/tools/fetch") {
                return Err(RelayError::RouteUnavailable);
            }
            if path.ends_with("/v1/tools") {
                vec![request.kind.name()]
            } else if path.ends_with("/v1") {
                vec!["tools", request.kind.name()]
            } else {
                vec!["v1", "tools", request.kind.name()]
            }
        }
        NativeRequest::Embedding(_) => {
            if path
                .ends_with("/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding")
            {
                return Ok(url);
            }
            let mut suffix = if path.ends_with("/api/v1") {
                vec![]
            } else {
                vec!["api", "v1"]
            };
            suffix.extend([
                "services",
                "embeddings",
                "multimodal-embedding",
                "multimodal-embedding",
            ]);
            suffix
        }
    };
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| RelayError::RouteUnavailable)?;
    segments.pop_if_empty();
    for segment in suffix {
        segments.push(segment);
    }
    drop(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_operation_urls_preserve_prefix_and_do_not_duplicate_or_switch_operation() {
        let search = NativeRequest::Tool(
            deepseek::Request::parse(deepseek::Kind::Search, br#"{"query":"x"}"#).unwrap(),
        );
        for base in [
            "https://supplier.example/custom",
            "https://supplier.example/custom/v1",
            "https://supplier.example/custom/v1/tools",
            "https://supplier.example/custom/v1/tools/search",
        ] {
            assert_eq!(
                endpoint(base, &search).unwrap().as_str(),
                "https://supplier.example/custom/v1/tools/search"
            );
        }
        assert!(endpoint("https://supplier.example/v1/tools/fetch", &search).is_err());
        let embed = NativeRequest::Embedding(
            embedding::Request::parse(
                embedding::Client::OpenAi,
                br#"{"model":"alias","input":"text"}"#,
            )
            .unwrap(),
        );
        for base in [
            "https://supplier.example/custom",
            "https://supplier.example/custom/api/v1",
            "https://supplier.example/custom/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding",
        ] {
            assert_eq!(
                endpoint(base, &embed).unwrap().as_str(),
                "https://supplier.example/custom/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding"
            );
        }
    }
}
