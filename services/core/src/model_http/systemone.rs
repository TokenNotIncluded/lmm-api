//! Native JEV decisions. Structured questions never enter a Chat adapter.
#[path = "systemone_compression.rs"]
mod compression;
#[path = "systemone_contract.rs"]
mod contract;
#[path = "systemone_multipart.rs"]
mod multipart;
#[cfg(test)]
#[path = "systemone_tests.rs"]
mod tests;

use super::*;
use crate::{
    billing::charging::{self, Outcome},
    routing::{CreditRate, PayloadPolicy, Protocol, ReplayPermission, RouteRequest},
};
use axum::{body::to_bytes, extract::Request};
use contract::{DangerZone, Request as NativeRequest};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(super) struct Host {
    gate: Arc<Semaphore>,
    parallelism: usize,
    transport: crate::relay::provider_network::ProviderTransport,
}
impl Host {
    pub(super) fn new(limits: &Limits) -> Self {
        let parallelism = limits.max_in_flight.min(2);
        Self {
            gate: Arc::new(Semaphore::new(parallelism)),
            parallelism,
            transport: crate::relay::provider_network::ProviderTransport::new(parallelism),
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
        .route("/v1/systemone", post(handle))
        .route("/typesafe/v1/systemone", post(handle))
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
fn flags(headers: &HeaderMap) -> Result<DangerZone, RelayError> {
    let flag = |name| match one_header(headers, name)? {
        None => Ok(false),
        Some("true") => Ok(true),
        _ => Err(RelayError::Invalid(
            "System One danger-zone header must be true or absent",
        )),
    };
    Ok(DangerZone {
        images: flag("typesafe-danger-zone-images")?,
        long_context: flag("typesafe-danger-zone-long-context")?,
    })
}
pub(super) fn endpoint(base: &str) -> Result<reqwest::Url, RelayError> {
    let mut url = crate::relay::honeypot_network::validate_base_url(base, true)?;
    let has_version = url.path().trim_end_matches('/').ends_with("/v1");
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| RelayError::RouteUnavailable)?;
    segments.pop_if_empty();
    if !has_version {
        segments.push("v1");
    }
    segments.push("systemone");
    drop(segments);
    Ok(url)
}
async fn handle(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    // Existing OAuth/Pi scopes do not authorize this newly added native API.
    if oauth_header(&headers) {
        return public_error(RelayError::Forbidden);
    }
    if request.uri().query().is_some()
        || ["x-api-key", "x-goog-api-key", "x-auth-session"]
            .iter()
            .any(|h| headers.contains_key(*h))
    {
        return public_error(RelayError::Invalid(
            "unsupported System One credential or query",
        ));
    }
    let peer = peer.ok().map(|p| p.0.ip());
    if let Some(honeypot) = &gateway.honeypot {
        match honeypot.models(&headers, peer).await {
            Ok(Some(_)) => {
                return public_error(RelayError::Unsupported(
                    "System One honeypot policy is not configured",
                ));
            }
            Ok(None) => (),
            Err(error) => return public_error(error),
        }
    }
    let (principal, secret, groups) = match gateway.authenticate(&headers).await {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    let Some(key_id) = principal.key_id() else {
        return public_error(RelayError::Forbidden);
    };
    let flags = match flags(&headers) {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    let permit = match gateway.systemone.gate.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return public_error(RelayError::Busy),
    };
    let (parts, body) = request.into_parts();
    if let Some(length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
    {
        if length > compression::MAX_BODY {
            return public_error(RelayError::Limit("System One body"));
        }
    }
    let raw = match tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(body, compression::MAX_BODY),
    )
    .await
    {
        Ok(Ok(raw)) => raw,
        Ok(Err(_)) => return public_error(RelayError::Limit("System One body")),
        Err(_) => return public_error(RelayError::Timeout(relay::Phase::Read)),
    };
    // Keep the admission slot with this detached Core worker, including all
    // blocking parsing/decompression work after the bounded body was collected.
    match tokio::spawn(process(
        gateway, principal, secret, groups, peer, flags, key_id, headers, parts, raw, permit,
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
    flags: DangerZone,
    key_id: i64,
    headers: HeaderMap,
    mut parts: axum::http::request::Parts,
    raw: Bytes,
    permit: OwnedSemaphorePermit,
) -> Response {
    let decode_headers = headers.clone();
    let decoded =
        match tokio::task::spawn_blocking(move || compression::decode(&decode_headers, &raw)).await
        {
            Ok(Ok(v)) => v,
            Ok(Err(error)) => {
                return (
                    StatusCode::from_u16(error.status()).unwrap(),
                    Json(json!({"error":{"code":error.code()}})),
                )
                    .into_response();
            }
            Err(_) => return public_error(RelayError::BillingUnavailable),
        };
    parts.headers.remove(header::CONTENT_ENCODING);
    parts.headers.remove(header::CONTENT_LENGTH);
    let content_type = match one_header(&headers, header::CONTENT_TYPE.as_str()) {
        Ok(Some(v)) => v
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase(),
        _ => {
            return public_error(RelayError::Invalid(
                "missing or ambiguous System One content type",
            ));
        }
    };
    let request = if content_type == "application/json" {
        tokio::task::spawn_blocking(move || NativeRequest::parse_json(&decoded, flags))
            .await
            .unwrap_or(Err(RelayError::Invalid("System One parsing failed")))
    } else if content_type == "multipart/form-data" {
        multipart::parse(
            axum::http::Request::from_parts(parts, Body::from(decoded)),
            flags,
        )
        .await
    } else {
        Err(RelayError::Unsupported(
            "unsupported System One content type",
        ))
    };
    let request = match request {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    let id = match request_id(key_id, &headers) {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    let response_id = id.clone();
    // The original caller can disconnect; a dispatched worker must still settle
    // its own real receipt or persist UNKNOWN without sending a second request.
    let response = match execute(
        gateway, principal, secret, groups, peer, request, id, permit,
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
async fn execute(
    gateway: ModelGateway,
    principal: ModelIdentity,
    secret: String,
    groups: Vec<String>,
    peer: Option<IpAddr>,
    request: NativeRequest,
    id: String,
    _permit: OwnedSemaphorePermit,
) -> Result<serde_json::Value, RelayError> {
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
            protocol: Protocol::SystemOne,
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
    if !quote.price().is_input_only_scalar() {
        return Err(RelayError::Unsupported(
            "System One requires a frozen scalar input-only token tariff",
        ));
    }
    let price = routing::credit_price(quote.price(), plan.canonical_model(), &group)?;
    if price.upstream_cost_group_ppm.is_some()
        || price.image.is_some()
        || price.output_per_million != 0
        || price.cached_per_million != 0
    {
        return Err(RelayError::Unsupported(
            "System One requires a frozen input-only token tariff",
        ));
    }
    let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
    if attempt.protocol() != Protocol::SystemOne
        || attempt.payload_policy() != PayloadPolicy::Adapt
        || !attempt.is_available()
        || u128::from(attempt.timeout_ms()) < gateway.limits.total_timeout.as_millis()
    {
        return Err(RelayError::RouteUnavailable);
    }
    // Never reduce the established 65,536-token native context reservation.
    // Long-context requires a host-reviewed larger bound, not a client price.
    let input_token_ceiling = gateway.bridge.provider_policy.validate_systemone_mode(
        &attempt,
        gateway.bridge.policy.input_token_ceiling,
        request.flags().long_context,
    )?;
    let reference = attempt.credential_ref();
    let credential = gateway
        .bridge
        .credentials
        .get(&(reference.id, reference.revision))
        .ok_or(RelayError::RouteUnavailable)?
        .clone();
    let endpoint = endpoint(attempt.endpoint())?;
    let connection = tokio::time::timeout(
        gateway.limits.total_timeout,
        gateway
            .systemone
            .transport
            .prepare(&endpoint, &gateway.limits),
    )
    .await
    .map_err(|_| RelayError::RouteUnavailable)??;
    let mut upstream = request.wire().clone();
    upstream["model"] = json!(attempt.upstream_model());
    let mut builder = connection
        .client()
        .post(endpoint)
        .bearer_auth(&credential)
        .header(header::ACCEPT, "application/json");
    if request.flags().images {
        builder = builder.header("typesafe-danger-zone-images", "true");
    }
    if request.flags().long_context {
        builder = builder.header("typesafe-danger-zone-long-context", "true");
    }
    let upstream_request = if request.uploads().is_empty() {
        let encoded =
            serde_json::to_vec(&upstream).map_err(|_| RelayError::Invalid("System One request"))?;
        if encoded.len() > compression::MAX_BODY {
            return Err(RelayError::Limit("System One outbound body"));
        }
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(encoded)
            .build()
    } else {
        // Reparse with the trusted mapped model before constructing multipart.
        let native = NativeRequest::parse_multipart_json(
            &serde_json::to_vec(&upstream)
                .map_err(|_| RelayError::Invalid("System One request"))?,
            request.flags(),
            request.uploads().to_vec(),
        )?;
        builder.multipart(multipart::outbound(&native)?).build()
    }
    .map_err(|_| RelayError::Invalid("invalid System One provider request"))?;
    if upstream_request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > compression::MAX_BODY)
    {
        return Err(RelayError::Limit("System One outbound body"));
    }
    let reserve = i64::try_from(
        quote
            .reservation_ceiling(input_token_ceiling as u64, 0)
            .map_err(|_| RelayError::BillingUnavailable)?
            .0,
    )
    .map_err(|_| RelayError::BillingUnavailable)?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"lmm-system-one-v1\0");
    fingerprint.update(group.as_bytes());
    fingerprint.update(
        serde_json::to_vec(request.wire())
            .map_err(|_| RelayError::Invalid("System One request"))?,
    );
    fingerprint.update([
        u8::from(request.flags().images),
        u8::from(request.flags().long_context),
    ]);
    for upload in request.uploads() {
        fingerprint.update(upload.part.as_bytes());
        fingerprint.update([0]);
        fingerprint.update(&upload.bytes);
    }
    let fingerprint: [u8; 32] = fingerprint.finalize().into();
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    let route_id = format!(
        "r{}-t{}-u{}",
        attempt.route_id(),
        attempt.target_id(),
        attempt.upstream_id()
    );
    let selected = routing::Selected {
        attempt: attempt.clone(),
        price: price.clone(),
        quote,
        route_id,
        input_token_ceiling,
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
                output_token_ceiling: u32::MAX,
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
            .reserve_with_context(&secret, &spec, &worker, peer)
    })
    .await
    .map_err(billing::error)?;
    if charge.actor_user_id != principal.user_id() || charge.key_id != principal.key_id() {
        return Err(RelayError::Forbidden);
    }
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
    let observed = tokio::time::timeout(gateway.limits.total_timeout, async {
        let mut response = tokio::time::timeout(
            gateway.limits.header_timeout,
            connection.client().execute(upstream_request),
        )
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)?;
        if !connection.matches_peer(response.remote_addr())
            || !response.status().is_success()
            || response.headers().contains_key(header::CONTENT_ENCODING)
        {
            return Err(RelayError::SettlementPending);
        }
        if !response
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
        let mut observed = contract::Response::parse(&request, &bytes)?;
        let normalized =
            serde_json::to_vec(&observed.output).map_err(|_| RelayError::SettlementPending)?;
        if std::str::from_utf8(&bytes).is_ok_and(|v| v.contains(&credential))
            || std::str::from_utf8(&normalized).is_ok_and(|v| v.contains(&credential))
        {
            return Err(RelayError::SettlementPending);
        }
        gateway
            .bridge
            .provider_policy
            .validate_usage(&attempt, &observed.usage, false)?;
        if observed
            .usage
            .input_tokens
            .is_none_or(|n| n > input_token_ceiling as u64)
        {
            return Err(RelayError::SettlementPending);
        }
        // The reviewed official System One usage has no discounted cache field.
        observed.usage.cached_input_tokens = Some(0);
        let usage = billing::complete_usage(observed.usage, false)?;
        gateway
            .bridge
            .persist_output(
                &relay::Reservation { id: id.clone() },
                &relay::OutputCheckpoint {
                    sequence: 1,
                    packet_sha256: Sha256::digest(&bytes).into(),
                    packet_bytes: bytes.len(),
                    usage: observed.usage,
                    terminal_observed: true,
                },
            )
            .await?;
        let settled = billing::retry(|| {
            gateway
                .bridge
                .charging
                .settle(&id, &worker, usage, Outcome::Completed)
        })
        .await;
        match settled {
            Ok(charge) if charge.state == "settled" && charge.usage == usage => (),
            Err(charging::Error::CommitUnknown) => {
                let charge = gateway
                    .bridge
                    .charging
                    .resolve(&id)
                    .await
                    .map_err(billing::error)?
                    .ok_or(RelayError::SettlementPending)?;
                if charge.state != "settled" || charge.usage != usage {
                    return Err(RelayError::SettlementPending);
                }
            }
            _ => return Err(RelayError::SettlementPending),
        }
        Ok(observed.output)
    })
    .await
    .unwrap_or(Err(RelayError::SettlementPending));
    if observed.is_err() {
        let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
    }
    observed
}
