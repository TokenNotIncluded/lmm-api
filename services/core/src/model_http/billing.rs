use super::Bridge;
use crate::{
    billing::charging::{self, Outcome, Usage},
    relay::{
        Billing, InputBlock, OutputCheckpoint, RelayError, RelayFuture, Report, Request,
        RequestContext, Reservation, Route,
    },
};
use std::{future::Future, time::Duration};

pub(super) fn error(error: charging::Error) -> RelayError {
    match error {
        charging::Error::Unauthorized => RelayError::Unauthorized,
        charging::Error::Forbidden => RelayError::Forbidden,
        charging::Error::NoFunds | charging::Error::BudgetExceeded => RelayError::NoFunds,
        charging::Error::Conflict | charging::Error::StaleWorker | charging::Error::WrongState => {
            RelayError::Conflict
        }
        _ => RelayError::BillingUnavailable,
    }
}

/// Retry the entire rolled-back operation with the same ID, payload and worker.
/// CommitUnknown is deliberately excluded; dispatch is never retried blindly.
pub(super) async fn retry<T, F, Fut>(mut operation: F) -> charging::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = charging::Result<T>>,
{
    for attempt in 0..3 {
        match operation().await {
            Err(charging::Error::Retry) if attempt < 2 => {
                tokio::time::sleep(Duration::from_millis(10 * (attempt + 1))).await
            }
            result => return result,
        }
    }
    unreachable!()
}

#[derive(Clone, Copy, Debug)]
struct RetryObservation {
    attempt: u8,
    duration: Duration,
    error: Option<charging::Error>,
    final_result: bool,
}

/// Observe the selected existing retry policy without broadening replayable errors.
async fn retry_observed<T, F, Fut, O>(
    deadline: Option<tokio::time::Instant>,
    mut operation: F,
    observe: O,
) -> charging::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = charging::Result<T>>,
    O: Fn(RetryObservation),
{
    let mut attempts = 0u8;
    let started = tokio::time::Instant::now();
    let observed = || {
        attempts += 1;
        let attempt = attempts;
        let operation = operation();
        let observe = &observe;
        async move {
            let started = tokio::time::Instant::now();
            let result = operation.await;
            observe(RetryObservation {
                attempt,
                duration: started.elapsed(),
                error: result.as_ref().err().copied(),
                final_result: false,
            });
            result
        }
    };
    let result = match deadline {
        Some(deadline) => retry_bounded(deadline, observed).await,
        None => retry(observed).await,
    };
    observe(RetryObservation {
        attempt: attempts,
        duration: started.elapsed(),
        error: result.as_ref().err().copied(),
        final_result: true,
    });
    result
}

#[derive(Clone, Copy)]
enum FinalizeStage {
    ProviderFinal,
    Settle,
}

// @mark model.settlement-retry role=implementation
async fn retry_finalize<T, F, Fut>(
    deadline: tokio::time::Instant,
    request_id: &str,
    stage: FinalizeStage,
    operation: F,
) -> charging::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = charging::Result<T>>,
{
    // Never interpolate unchecked identifiers or database error descriptions.
    let request_id =
        if request_id.len() == 64 && request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            request_id
        } else {
            "invalid_request_id"
        };
    let stage = match stage {
        FinalizeStage::ProviderFinal => "provider-final",
        FinalizeStage::Settle => "settle",
    };
    retry_observed(Some(deadline), operation, |observation| {
        if observation.error.is_none()
            && (!observation.final_result || observation.attempt == 1)
        {
            return;
        }
        // charging::Error is a payload-free enum, unlike an underlying SQL error.
        let category = observation.error.map_or_else(|| "Ok".to_owned(), |error| format!("{error:?}"));
        eprintln!(
            "model_finalize request_id={request_id} stage={stage} attempt={} duration_ms={} final={} category={category}",
            observation.attempt,
            observation.duration.as_millis(),
            observation.final_result,
        );
    })
    .await
}

/// Retry only an explicitly rolled-back reservation, preserving its ID/spec/worker.
/// The monotonic window limits when another attempt may START; it does not
/// interrupt an in-flight transaction or change the enclosing hook timeout.
// @mark model.reservation-retry role=implementation
async fn retry_reserve<T, F, Fut>(operation: F) -> charging::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = charging::Result<T>>,
{
    retry_bounded(
        tokio::time::Instant::now() + Duration::from_secs(2),
        operation,
    )
    .await
}

/// Shared rollback-only policy: at most eight starts in the supplied monotonic window.
async fn retry_bounded<T, F, Fut>(
    deadline: tokio::time::Instant,
    mut operation: F,
) -> charging::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = charging::Result<T>>,
{
    for attempt in 0..8 {
        if tokio::time::Instant::now() >= deadline {
            return Err(charging::Error::Retry);
        }
        match operation().await {
            Err(charging::Error::Retry) if attempt < 7 => {
                let next = tokio::time::Instant::now() + Duration::from_millis(10u64 << attempt);
                tokio::time::sleep_until(next.min(deadline)).await;
            }
            result => return result,
        }
    }
    unreachable!()
}

impl Billing for Bridge {
    fn reserve<'a>(
        &'a self,
        context: &'a RequestContext,
        route: &'a Route,
        request: &'a Request,
    ) -> RelayFuture<'a, Reservation> {
        // @context-fold begin encode-context-max-reservation
        Box::pin(async move {
            let body = request.upstream_body(route)?;
            self.reserve_prepared(context, route, request, &body, None)
                .await
        })
        // @context-fold end encode-context-max-reservation
    }

    fn reserve_prepared<'a>(
        &'a self,
        context: &'a RequestContext,
        route: &'a Route,
        request: &'a Request,
        body: &'a [u8],
        observation: Option<&'a crate::relay::input_count::CountObservation>,
    ) -> RelayFuture<'a, Reservation> {
        Box::pin(async move {
            let (secret, worker, fingerprint, selected, output_ceiling, peer, oauth_issuer) = {
                let active = self
                    .active
                    .lock()
                    .map_err(|_| RelayError::BillingUnavailable)?;
                let op = active
                    .get(&context.request_id)
                    .ok_or(RelayError::Forbidden)?;
                (
                    op.secret.clone(),
                    op.worker,
                    op.fingerprint,
                    op.selected.clone().ok_or(RelayError::RouteUnavailable)?,
                    op.output_token_ceiling,
                    op.peer,
                    op.oauth_issuer.clone(),
                )
            };
            let protocol = match selected.attempt.protocol() {
                crate::routing::Protocol::OpenAiChat => crate::relay::UpstreamProtocol::Chat,
                crate::routing::Protocol::OpenAiResponses => {
                    crate::relay::UpstreamProtocol::Responses
                }
                crate::routing::Protocol::AnthropicMessages => {
                    crate::relay::UpstreamProtocol::Anthropic
                }
                crate::routing::Protocol::Gemini => crate::relay::UpstreamProtocol::Gemini,
                crate::routing::Protocol::SystemOne => return Err(RelayError::RouteUnavailable),
            };
            let reference = selected.attempt.credential_ref();
            let credential = self
                .credentials
                .get(&(reference.id, reference.revision))
                .ok_or(RelayError::RouteUnavailable)?;
            let expected_route = Route::new(
                selected.route_id.clone(),
                protocol,
                super::routing::operation_endpoint(
                    selected.attempt.endpoint(),
                    protocol,
                    selected.attempt.upstream_model(),
                    request.stream,
                )?
                .as_str(),
                selected.attempt.upstream_model().to_owned(),
                credential,
                selected.attempt.quote().version(),
            )?;
            let count_plan = self.provider_policy.count_plan(&selected.attempt)?;
            if selected.route_id != route.id
                || selected.attempt.quote().version() != route.price_version
                || route.protocol != expected_route.protocol
                || route.model != expected_route.model
                || route.endpoint != expected_route.endpoint
                || route.credential != expected_route.credential
                || route.payload_policy != selected.attempt.payload_policy()
                || route.cache_creation_mode
                    != selected
                        .price
                        .tiered
                        .as_ref()
                        .and_then(|t| t.cache_creation_mode)
                || route.count_plan != count_plan
                || request.max_output_tokens != output_ceiling
                || !selected.attempt.is_available()
            {
                return Err(RelayError::RouteUnavailable);
            }
            if body != request.upstream_body(route)?.as_slice() {
                return Err(RelayError::Conflict);
            }
            // Only the numeric-loopback synthetic contract budgets UTF-8 bytes
            // plus framing as tokens. External models use the frozen supplier
            // bound; byte length is not their tokenizer.
            let endpoint = crate::relay::honeypot_network::validate_base_url(
                selected.attempt.endpoint(),
                true,
            )?;
            if crate::relay::honeypot_network::literal_ip(&endpoint)
                .is_some_and(|ip| ip.is_loopback())
                && local_input_bound(request)? > selected.input_token_ceiling as u64
            {
                return Err(RelayError::Limit("local model input budget"));
            }
            // @context-fold begin gateway-prepared-contract
            use charging::contract::{
                self, CounterEvidence, PreparedReservation, ReservationBinding, ReservationBounds,
            };
            let wire_sha256 = contract::digest(body);
            let input_bound = match (&count_plan, observation) {
                (crate::relay::input_count::CountPlan::ContextMax, None) => {
                    selected.input_token_ceiling as u64
                }
                (
                    crate::relay::input_count::CountPlan::OpenAiResponsesExactV1(_),
                    Some(counted),
                ) if contract::hex_digest(&counted.wire_sha256) == wire_sha256
                    && counted.requested_output == request.max_output_tokens
                    && u64::from(counted.input_tokens) <= selected.input_token_ceiling as u64 =>
                {
                    u64::from(counted.input_tokens)
                }
                _ => return Err(RelayError::Conflict),
            };
            let (revision, semantics) = self
                .provider_policy
                .reservation_metadata(&selected.attempt)?;
            let binding = ReservationBinding {
                route_id: route.id.clone(),
                target_id: selected.attempt.target_id(),
                upstream_id: selected.attempt.upstream_id(),
                protocol: match route.protocol {
                    crate::relay::UpstreamProtocol::Chat => "chat",
                    crate::relay::UpstreamProtocol::Responses => "responses",
                    crate::relay::UpstreamProtocol::Anthropic => "anthropic",
                    crate::relay::UpstreamProtocol::Gemini => "gemini",
                }
                .into(),
                model_snapshot: route.model.clone(),
                generation_url: route.endpoint.as_str().into(),
                credential_ref: selected.attempt.credential_ref(),
                payload_policy_sha256: contract::typed_digest(&selected.attempt.payload_policy())
                    .map_err(error)?,
                provider_contract_revision: revision,
                projection_revision: observation
                    .map_or("context-max-v1", |o| o.projection_revision)
                    .into(),
                quote_version: route.price_version,
            };
            let bounds = ReservationBounds {
                model_input_max: selected.input_token_ceiling as u64,
                input: input_bound,
                requested_output: u64::from(request.max_output_tokens),
                output: u64::from(request.max_output_tokens),
            };
            let prepared = if let Some(counted) = observation {
                PreparedReservation::exact(
                    contract::hex_digest(&fingerprint),
                    binding,
                    wire_sha256,
                    CounterEvidence {
                        api: counted.api.into(),
                        input_tokens: u64::from(counted.input_tokens),
                        request_sha256: contract::hex_digest(&counted.request_sha256),
                        response_sha256: contract::hex_digest(&counted.response_sha256),
                    },
                    bounds,
                    semantics,
                )
            } else {
                PreparedReservation::context_max(
                    contract::hex_digest(&fingerprint),
                    binding,
                    wire_sha256,
                    bounds,
                    semantics,
                )
            }
            .map_err(error)?;
            // @context-fold end gateway-prepared-contract
            let reserve = if selected.price.upstream_cost_group_ppm.is_some() {
                let mut conservative = selected.price.clone();
                conservative.input_per_million = conservative
                    .input_per_million
                    .max(conservative.cached_per_million);
                conservative
                    .cost(Usage {
                        cache_creation: None,
                        image: None,
                        upstream_cost_atto_usd: Some(crate::upstream_cost::ATTO_USD),
                        input: input_bound as i64,
                        cached: 0,
                        output: i64::from(request.max_output_tokens),
                    })
                    .map_err(error)?
            } else {
                let reserve = selected
                    .quote
                    .reservation_ceiling(input_bound, u64::from(request.max_output_tokens))
                    .map_err(|_| RelayError::BillingUnavailable)?;
                i64::try_from(reserve.0).map_err(|_| RelayError::BillingUnavailable)?
            };
            let basis = if selected.price.upstream_cost_group_ppm.is_some() {
                charging::ReservationBasis::UpstreamCost {
                    atto_usd: crate::upstream_cost::ATTO_USD,
                }
            } else {
                charging::ReservationBasis::Tokens {
                    input: input_bound,
                    output: u64::from(request.max_output_tokens),
                }
            };
            let spec = charging::Request {
                id: context.request_id.clone(),
                fingerprint,
                price: selected.price,
                reserve,
                lease_seconds: self.policy.lease_seconds,
                maximum_seconds: self.policy.lease_seconds,
            };
            let charge = retry_reserve(|| async {
                self.charging
                    .reserve_prepared_model_with_context(
                        &secret,
                        &spec,
                        &worker,
                        peer,
                        oauth_issuer.as_ref(),
                        basis,
                        &prepared,
                    )
                    .await
            })
            .await;
            let charge = match charge {
                Err(charging::Error::CommitUnknown) => {
                    self.charging
                        .resolve_prepared_model(&spec, &worker, basis, &prepared)
                        .await
                }
                other => other,
            }
            .map_err(|cause| {
                let reason = match &cause {
                    charging::Error::NoFunds => Some("wallet"),
                    charging::Error::BudgetExceeded => Some("budget"),
                    _ => None,
                };
                if let Some(reason) = reason {
                    eprintln!(
                        "core_model_reserve_denied reason={reason} reserve_credits={reserve}"
                    );
                }
                error(cause)
            })?;
            if charge.actor_user_id != context.actor_user_id
                || charge.key_id != context.key_id
                || charge.oauth_source != context.oauth_source
            {
                return Err(RelayError::Forbidden);
            }
            self.freeze_selected_price(&spec.id, &charge)?;
            if !selected.attempt.is_available() {
                let _ = retry(|| self.charging.release(&spec.id, &worker)).await;
                return Err(RelayError::RouteUnavailable);
            }
            // A durable dispatch claim precedes any HTTP bytes. A false/unknown
            // result never dispatches; expiry recovery retains ambiguous holds.
            let started = retry(|| async {
                if let Some(issuer) = &oauth_issuer {
                    self.charging.start_oauth(issuer, &spec.id, &worker).await
                } else {
                    self.charging.start(&spec.id, &worker).await
                }
            })
            .await;
            let started = match started {
                Err(charging::Error::CommitUnknown) => {
                    let _ = self.charging.resolve(&spec.id).await;
                    return Err(RelayError::SettlementPending);
                }
                result => result.map_err(error)?,
            };
            if !started {
                return Err(RelayError::Conflict);
            }
            Ok(Reservation { id: spec.id })
        })
    }

    fn checkpoint<'a>(
        &'a self,
        reservation: &'a Reservation,
        checkpoint: &'a OutputCheckpoint,
    ) -> RelayFuture<'a, ()> {
        Box::pin(self.persist_output(reservation, checkpoint))
    }

    fn finalize<'a>(
        &'a self,
        reservation: &'a Reservation,
        report: &'a Report,
    ) -> RelayFuture<'a, ()> {
        Box::pin(async move {
            let (worker, output_ceiling, selected, secret, peer, oauth_issuer) = {
                let active = self
                    .active
                    .lock()
                    .map_err(|_| RelayError::BillingUnavailable)?;
                let op = active
                    .get(&reservation.id)
                    .ok_or(RelayError::BillingUnavailable)?;
                (
                    op.worker,
                    op.output_token_ceiling,
                    op.selected.clone().ok_or(RelayError::BillingUnavailable)?,
                    op.secret.clone(),
                    op.peer,
                    op.oauth_issuer.clone(),
                )
            };
            let mut stage = "report";
            let result = async {
                if reservation.id != report.request_id
                    || report.route_id.as_deref() != Some(selected.route_id.as_str())
                    || report.price_version != Some(selected.quote.price().version())
                {
                    return Err(RelayError::Conflict);
                }
                let frozen = self.charging.reservation_contract(&reservation.id).await;
                if report.upstream_attempted && report.provider_complete {
                    stage = "provider_observation";
                    // A broken proof cannot erase the supplier's raw terminal
                    // evidence. Without a readable proof only the original
                    // selected model ceiling is known; this audit record never
                    // authorizes a checkpoint, settlement or recovery.
                    let input_ceiling = frozen
                        .as_ref()
                        .map_or(selected.input_token_ceiling, |(_, contract)| {
                            contract.bounds().0 as i64
                        });
                    self.persist_provider_observation(
                        &reservation.id,
                        &worker,
                        &selected.route_id,
                        report.usage,
                        input_ceiling,
                        output_ceiling,
                    )
                    .await?;
                }
                let (frozen, contract) = frozen.map_err(|_| RelayError::SettlementPending)?;
                if frozen.price != selected.price {
                    return Err(RelayError::SettlementPending);
                }
                let price = frozen.price.clone();
                let upstream_cost = price.upstream_cost_group_ppm.is_some();
                stage = "usage";
                let (usage, outcome) = if !report.upstream_attempted {
                    (
                        Usage {
                            upstream_cost_atto_usd: upstream_cost.then_some(0),
                            ..Usage::default()
                        },
                        Outcome::Cancelled,
                    )
                } else {
                    // Partial/local output is not an authoritative final provider
                    // usage receipt. Never substitute missing usage with zero.
                    if !report.provider_complete {
                        return Err(RelayError::SettlementPending);
                    }
                    if upstream_cost {
                        stage = "provider_policy";
                        contract
                            .check_usage(
                                &frozen,
                                frozen_priced_usage(&contract, &frozen, report.usage)?,
                            )
                            .map_err(|_| RelayError::SettlementPending)?;
                        let usage = frozen_priced_usage(&contract, &frozen, report.usage)?;
                        if usage.upstream_cost_atto_usd.is_none() {
                            return Err(RelayError::SettlementPending);
                        }
                        stage = "usage_bound";
                        if usage.input > selected.input_token_ceiling
                            || usage.output > i64::from(output_ceiling)
                        {
                            return Err(RelayError::SettlementPending);
                        }
                        (usage, Outcome::Completed)
                    } else {
                        let usage = frozen_priced_usage(&contract, &frozen, report.usage)?;
                        // This input is either the original inclusive receipt
                        // or a checked reconstruction from actual Anthropic P
                        // and the original frozen absence-zero contract.
                        stage = "provider_policy";
                        contract
                            .check_usage(&frozen, usage)
                            .map_err(|_| RelayError::SettlementPending)?;
                        stage = "usage_bound";
                        if usage.input > selected.input_token_ceiling
                            || usage.output > i64::from(output_ceiling)
                        {
                            return Err(RelayError::SettlementPending);
                        }
                        // The legacy ledger must agree with the captured exact quote.
                        // Never reinterpret a price or a missing receipt at settlement.
                        stage = "quote";
                        let quoted = if let Some(tariff) = &selected.price.tiered {
                            selected.quote.charge_tiered_usage(
                                crate::routing::TieredUsage {
                                    input: usage.input as u64,
                                    cached: usage.cached as u64,
                                    output: usage.output as u64,
                                    created: usage.cache_creation.map_or(0, |v| v.total as u64),
                                    created_5m: usage
                                        .cache_creation
                                        .and_then(|v| v.five_minute.map(|v| v as u64)),
                                    created_1h: usage
                                        .cache_creation
                                        .and_then(|v| v.one_hour.map(|v| v as u64)),
                                    created_12h: usage
                                        .cache_creation
                                        .and_then(|v| v.twelve_hour.map(|v| v as u64)),
                                },
                                tariff.input_semantics,
                            )
                        } else {
                            let disjoint = crate::routing::Usage::from_totals(
                                usage.input as u64,
                                usage.output as u64,
                                usage.cached as u64,
                                0,
                            )
                            .map_err(|_| RelayError::SettlementPending)?;
                            selected.quote.charge(disjoint)
                        }
                        .map_err(|_| RelayError::SettlementPending)?;
                        let ledger = selected.price.cost(usage).map_err(error)?;
                        if i64::try_from(quoted.0).ok() != Some(ledger) {
                            return Err(RelayError::SettlementPending);
                        }
                        (usage, Outcome::Completed)
                    }
                };
                if upstream_cost {
                    let amount = price.cost(usage).map_err(error)?;
                    let charge = self
                        .charging
                        .resolve(&reservation.id)
                        .await
                        .map_err(error)?
                        .ok_or(RelayError::SettlementPending)?;
                    if amount > charge.reserved {
                        retry(|| async {
                            if let Some(issuer) = &oauth_issuer {
                                self.charging
                                    .increase_oauth_with_context(
                                        issuer,
                                        &secret,
                                        &reservation.id,
                                        &worker,
                                        amount,
                                        peer,
                                    )
                                    .await
                            } else {
                                self.charging
                                    .increase_with_context(
                                        &secret,
                                        &reservation.id,
                                        &worker,
                                        amount,
                                        peer,
                                    )
                                    .await
                            }
                        })
                        .await
                        .map_err(error)?;
                    }
                }
                // Non-streaming relay exposes the body only after this durable final
                // transaction. The checkpoint records the provider observation.
                // Both finalization stages share one start window; neither stage
                // cancels an in-flight transaction or replays an unknown commit.
                stage = "provider_checkpoint";
                let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                retry_finalize(
                    deadline,
                    &reservation.id,
                    FinalizeStage::ProviderFinal,
                    || {
                        self.charging
                            .checkpoint(&reservation.id, &worker, "provider-final", usage)
                    },
                )
                .await
                .map_err(error)?;
                stage = "settle";
                let settlement =
                    retry_finalize(deadline, &reservation.id, FinalizeStage::Settle, || {
                        self.charging
                            .settle(&reservation.id, &worker, usage, outcome)
                    })
                    .await;
                match settlement {
                    Ok(charge)
                        if confirmed_settlement(
                            &charge,
                            &reservation.id,
                            &selected.price,
                            usage,
                            outcome,
                        ) =>
                    {
                        Ok(())
                    }
                    Ok(_) => Err(RelayError::SettlementPending),
                    Err(charging::Error::CommitUnknown) => {
                        // Resolve the same operation after a lost COMMIT reply.
                        // Never create another request or dispatch another model call.
                        match self.charging.resolve(&reservation.id).await {
                            Ok(Some(charge))
                                if confirmed_settlement(
                                    &charge,
                                    &reservation.id,
                                    &selected.price,
                                    usage,
                                    outcome,
                                ) =>
                            {
                                Ok(())
                            }
                            _ => Err(RelayError::SettlementPending),
                        }
                    }
                    Err(e) => Err(error(e)),
                }
            }
            .await;
            if let Err(failure) = &result
                && report.upstream_attempted
            {
                eprintln!(
                    "core_model_finalize_failure code={} upstream_status={} provider_complete={} stage={} usage={:?} input_ceiling={} output_ceiling={}",
                    failure.code(),
                    report.upstream_status.unwrap_or(0),
                    report.provider_complete,
                    stage,
                    report.usage,
                    selected.input_token_ceiling,
                    output_ceiling,
                );
            }
            if result.is_err() && report.upstream_attempted {
                retry(|| self.charging.mark_unknown(&reservation.id, &worker))
                    .await
                    .map_err(error)?;
            }
            result
        })
    }
}

fn local_input_bound(request: &Request) -> Result<u64, RelayError> {
    let mut bytes = 16u64;
    let mut add = |n: usize| -> Result<(), RelayError> {
        bytes = bytes
            .checked_add(n as u64)
            .ok_or(RelayError::Limit("input budget"))?;
        Ok(())
    };
    for message in &request.messages {
        add(32)?;
        for block in &message.content {
            add(16)?;
            match block {
                InputBlock::Text(value) | InputBlock::Refusal(value) => add(value.len())?,
                InputBlock::Thinking { text, signature } => {
                    add(text.len())?;
                    if let Some((_, value)) = signature {
                        add(value.len())?;
                    }
                }
                InputBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    signature,
                } => {
                    add(id.len())?;
                    add(name.len())?;
                    add(arguments.len())?;
                    if let Some((_, value)) = signature {
                        add(value.len())?;
                    }
                }
                InputBlock::ToolResult { id, content } => {
                    add(id.len())?;
                    add(content.len())?;
                }
            }
        }
    }
    for tool in &request.tools {
        add(64)?;
        add(tool.name.len())?;
        if let Some(value) = &tool.description {
            add(value.len())?;
        }
        add(serde_json::to_vec(&tool.parameters)
            .map_err(|_| RelayError::Invalid("tool schema"))?
            .len())?;
    }
    Ok(bytes)
}

/// The existing numeric-loopback Chat/Responses charging contract. Optional
/// cache reads retain its existing zero-default semantics; this is not a
/// normalization policy for external providers. Missing input/output totals
/// and cache creation unsupported by the ledger must retain the original hold.
/// All acceptance paths normalize using the persisted contract and price, never live configuration.
pub(super) fn frozen_priced_usage(
    contract: &charging::contract::ReservationContract,
    charge: &charging::Charge,
    mut observed: crate::relay::Usage,
) -> Result<Usage, RelayError> {
    // @context-fold begin frozen-priced-usage
    validate_frozen_observation(contract, charge, observed)?;
    if observed.cached_input_tokens.is_none() && !contract.semantics().missing_cached_tokens_is_zero
    {
        return Err(RelayError::SettlementPending);
    }
    if observed.cached_input_tokens.is_none() {
        observed.cached_input_tokens = Some(0);
    }
    if let Some(tariff) = &charge.price.tiered
        && tariff
            .tariff
            .compile()
            .map_err(|_| RelayError::SettlementPending)?
            .requires_cache_creation()
        && observed.cache_creation_input_tokens.is_none()
        && !contract.semantics().missing_cache_creation_tokens_is_zero
    {
        return Err(RelayError::SettlementPending);
    }
    let usage = if let Some(tariff) = &charge.price.tiered {
        complete_tiered_usage(observed, tariff)?
    } else {
        complete_usage(observed, charge.price.upstream_cost_group_ppm.is_some())?
    };
    contract
        .check_usage(charge, usage)
        .map_err(|_| RelayError::SettlementPending)?;
    Ok(usage)
    // @context-fold end frozen-priced-usage
}

pub(super) fn validate_frozen_observation(
    contract: &charging::contract::ReservationContract,
    charge: &charging::Charge,
    observed: crate::relay::Usage,
) -> Result<(), RelayError> {
    // @context-fold begin frozen-observation-bounds
    contract
        .validate(charge)
        .map_err(|_| RelayError::SettlementPending)?;
    observed
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    let (input, output) = contract.bounds();
    // TTL buckets are disjoint subsets; totals and cached tokens are not added
    // twice. Anthropic's uncached counter is disjoint from both cached classes.
    let add = |a: u64, b: u64| a.checked_add(b).ok_or(RelayError::SettlementPending);
    let cache_creation = observed
        .cache_creation_input_tokens
        .unwrap_or(0)
        .max(
            observed
                .cache_creation_details
                .map(|details| {
                    add(
                        details.ephemeral_5m_input_tokens.unwrap_or(0),
                        details.ephemeral_1h_input_tokens.unwrap_or(0),
                    )
                })
                .transpose()?
                .unwrap_or(0),
        )
        .max(
            observed
                .openai_cache_write_details
                .map(|details| {
                    add(
                        details.ttl_30m_tokens.unwrap_or(0),
                        details.ttl_12h_tokens.unwrap_or(0),
                    )
                })
                .transpose()?
                .unwrap_or(0),
        );
    let disjoint = add(observed.cached_input_tokens.unwrap_or(0), cache_creation)?;
    if disjoint > input
        || observed.input_tokens.is_some_and(|total| disjoint > total)
        || add(observed.uncached_input_tokens.unwrap_or(0), disjoint)? > input
    {
        return Err(RelayError::SettlementPending);
    }
    let known_input = observed.input_tokens.or(observed.uncached_input_tokens);
    if known_input.is_some_and(|v| v > input)
        || observed.cached_input_tokens.is_some_and(|v| v > input)
        || observed
            .cache_creation_input_tokens
            .is_some_and(|v| v > input)
        || observed.cache_creation_details.is_some_and(|v| {
            v.ephemeral_5m_input_tokens.is_some_and(|v| v > input)
                || v.ephemeral_1h_input_tokens.is_some_and(|v| v > input)
        })
        || observed.openai_cache_write_details.is_some_and(|v| {
            v.ttl_30m_tokens.is_some_and(|v| v > input)
                || v.ttl_12h_tokens.is_some_and(|v| v > input)
        })
        || observed.output_tokens.is_some_and(|v| v > output)
        || observed.reasoning_output_tokens.is_some_and(|v| v > output)
    {
        return Err(RelayError::SettlementPending);
    }
    Ok(())
    // @context-fold end frozen-observation-bounds
}

pub(super) fn complete_usage(
    observed: crate::relay::Usage,
    upstream_cost: bool,
) -> Result<Usage, RelayError> {
    observed
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    if observed.cache_creation_mode.is_some() || observed.uncached_input_tokens.is_some() {
        return Err(RelayError::SettlementPending);
    }
    if !upstream_cost
        && (observed
            .cache_creation_input_tokens
            .is_some_and(|value| value != 0)
            || observed.cache_creation_details.is_some_and(|details| {
                details.ephemeral_5m_input_tokens.is_some_and(|v| v != 0)
                    || details.ephemeral_1h_input_tokens.is_some_and(|v| v != 0)
            })
            || observed.openai_cache_write_details.is_some_and(|details| {
                details.ttl_30m_tokens.is_some_and(|v| v != 0)
                    || details.ttl_12h_tokens.is_some_and(|v| v != 0)
            }))
    {
        return Err(RelayError::SettlementPending);
    }
    let input = observed
        .input_tokens
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(RelayError::SettlementPending)?;
    let output = observed
        .output_tokens
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(RelayError::SettlementPending)?;
    let cached = i64::try_from(observed.cached_input_tokens.unwrap_or(0))
        .map_err(|_| RelayError::SettlementPending)?;
    let upstream_cost_atto_usd = if upstream_cost {
        observed.upstream_cost_atto_usd
    } else {
        None
    };
    Ok(Usage {
        image: None,
        upstream_cost_atto_usd,
        cache_creation: None,
        input,
        cached,
        output,
    })
}

/// Interpret provider evidence only using the original frozen tariff. Old
/// strict revisions and the generic flat contract share this single dispatch.
pub(super) fn complete_tiered_usage(
    observed: crate::relay::Usage,
    tariff: &crate::routing::PersistedTieredTariff,
) -> Result<Usage, RelayError> {
    tariff
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    if tariff.tariff.revision == 4 {
        return complete_openai_cache_usage(observed, tariff);
    }
    // Historical frozen tariffs never priced twelve-hour writes. Keep the
    // original hold instead of discarding a new, unsupported charge dimension.
    if observed
        .openai_cache_write_details
        .is_some_and(|details| details.ttl_12h_tokens.is_some_and(|value| value != 0))
    {
        return Err(RelayError::SettlementPending);
    }
    let compiled = tariff
        .tariff
        .compile()
        .map_err(|_| RelayError::SettlementPending)?;
    match compiled.cache_creation_mode() {
        Some(crate::routing::CacheCreationMode::GenericRemainderV1) => {
            complete_generic_cache_usage(observed, tariff)
        }
        None if compiled.requires_cache_creation() => complete_strict_cache_usage(
            observed,
            tariff.input_semantics,
            tariff.missing_cache_creation_is_zero,
        ),
        None => complete_usage(observed, false),
    }
}

/// The OpenAI write total includes both lifetimes. A positive aggregate without
/// a measured twelve-hour subset is insufficient for this frozen price.
fn complete_openai_cache_usage(
    observed: crate::relay::Usage,
    tariff: &crate::routing::PersistedTieredTariff,
) -> Result<Usage, RelayError> {
    observed
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    if observed.cache_creation_mode.is_some()
        || observed.uncached_input_tokens.is_some()
        || tariff.input_semantics != crate::routing::InputSemantics::Inclusive
        || observed.cache_creation_details.is_some_and(|details| {
            details
                .ephemeral_5m_input_tokens
                .is_some_and(|value| value != 0)
                || details
                    .ephemeral_1h_input_tokens
                    .is_some_and(|value| value != 0)
        })
    {
        return Err(RelayError::SettlementPending);
    }
    let integer = |value: u64| i64::try_from(value).map_err(|_| RelayError::SettlementPending);
    let total = match observed.cache_creation_input_tokens {
        Some(value) => value,
        None if tariff.missing_cache_creation_is_zero
            && observed.openai_cache_write_details.is_none()
            && observed.cache_creation_details.is_none() =>
        {
            0
        }
        None => return Err(RelayError::SettlementPending),
    };
    let twelve_hour = if total == 0 {
        0
    } else {
        observed
            .openai_cache_write_details
            .and_then(|details| details.ttl_12h_tokens)
            .ok_or(RelayError::SettlementPending)?
    };
    let usage = Usage {
        cache_creation: Some(charging::CacheCreationUsage {
            total: integer(total)?,
            five_minute: None,
            one_hour: None,
            twelve_hour: Some(integer(twelve_hour)?),
            cache_creation_mode: None,
            reported_total: None,
        }),
        input: integer(observed.input_tokens.ok_or(RelayError::SettlementPending)?)?,
        cached: integer(observed.cached_input_tokens.unwrap_or(0))?,
        output: integer(
            observed
                .output_tokens
                .ok_or(RelayError::SettlementPending)?,
        )?,
        ..Usage::default()
    };
    if !usage.valid() {
        return Err(RelayError::SettlementPending);
    }
    Ok(usage)
}

#[cfg(test)]
mod openai_cache_billing_tests {
    use super::*;
    use crate::relay::{OpenAiCacheWriteDetails, UpstreamProtocol};
    use crate::routing::{InputSemantics, PersistedTieredTariff};
    use serde_json::json;

    fn price() -> PersistedTieredTariff {
        serde_json::from_value(json!({
            "tariff": {
                "revision": 4, "source_digest": "ab".repeat(32),
                "uses_cache_read": true, "uses_cache_write": true,
                "uses_cache_write_12h": true,
                "tiers": [{"up_to_input_tokens": null, "rates": {
                    "input": {"numerator":1,"denominator":1},
                    "output": {"numerator":3,"denominator":1},
                    "cache_read": {"numerator":1,"denominator":10},
                    "cache_write": {"numerator":5,"denominator":4},
                    "cache_write_12h": {"numerator":5,"denominator":2}
                }}]
            },
            "input_semantics": "inclusive", "group_multiplier_ppm": 1000000,
            "speed_multiplier_ppm": 1000000, "credits_per_usd": 500000
        }))
        .unwrap()
    }

    #[test]
    fn openai_observation_survives_normalization_and_frozen_ledger_price() {
        for (protocol, input, output, details) in [
            (
                UpstreamProtocol::Chat,
                "prompt_tokens",
                "completion_tokens",
                "prompt_tokens_details",
            ),
            (
                UpstreamProtocol::Responses,
                "input_tokens",
                "output_tokens",
                "input_tokens_details",
            ),
        ] {
            let observation = crate::relay::provider_usage(
                protocol,
                &json!({
                    input: 12000, output: 0,
                    details: {"cached_tokens":4000,"cache_write_tokens":7000,
                        "cache_write_tokens_details":{"ttl_30m_tokens":3000,"ttl_12h_tokens":4000}}
                }),
            )
            .unwrap();
            let tariff = price();
            let usage = complete_tiered_usage(observation, &tariff).unwrap();
            assert_eq!(usage.cache_creation.unwrap().twelve_hour, Some(4000));
            let bytes = serde_json::to_vec(&usage).unwrap();
            let restored: Usage = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(restored, usage);
            let ledger = charging::Price {
                version: "synthetic-cache-12h".into(),
                model: "synthetic".into(),
                group: "allowed".into(),
                input_per_million: 0,
                cached_per_million: 0,
                output_per_million: 0,
                tiered: Some(tariff),
                upstream_cost_group_ppm: None,
                image: None,
                user_trust: None,
            };
            // Official input example is exactly USD 0.01515 = 7,575 credits.
            assert_eq!(ledger.cost(restored), Ok(7575));
            assert!(
                ledger
                    .reservation_ceiling(charging::ReservationBasis::Tokens {
                        input: 12000,
                        output: 0,
                    })
                    .unwrap()
                    >= 7575
            );
        }
    }

    #[test]
    fn cache_12h_missing_evidence_stays_pending_but_explicit_zero_settles() {
        let mut observed = crate::relay::Usage {
            input_tokens: Some(100),
            output_tokens: Some(1),
            cached_input_tokens: Some(20),
            cache_creation_input_tokens: Some(30),
            ..Default::default()
        };
        assert_eq!(
            complete_tiered_usage(observed, &price()),
            Err(RelayError::SettlementPending)
        );
        observed.openai_cache_write_details = Some(OpenAiCacheWriteDetails {
            ttl_30m_tokens: Some(30),
            ttl_12h_tokens: None,
        });
        assert_eq!(
            complete_tiered_usage(observed, &price()),
            Err(RelayError::SettlementPending)
        );
        observed
            .openai_cache_write_details
            .as_mut()
            .unwrap()
            .ttl_12h_tokens = Some(0);
        assert_eq!(
            complete_tiered_usage(observed, &price())
                .unwrap()
                .cache_creation
                .unwrap()
                .twelve_hour,
            Some(0)
        );
        observed.cache_creation_input_tokens = None;
        let mut tariff = price();
        tariff.missing_cache_creation_is_zero = true;
        assert_eq!(
            complete_tiered_usage(observed, &tariff),
            Err(RelayError::SettlementPending)
        );
    }

    #[test]
    fn historical_absence_zero_cannot_erase_new_openai_write_evidence() {
        for revision in [2, 3] {
            let mut tariff = price();
            tariff.tariff.revision = revision;
            tariff.tariff.uses_cache_write_12h = false;
            tariff.tariff.tiers[0].rates.cache_write_12h = None;
            tariff.missing_cache_creation_is_zero = true;
            if revision == 3 {
                tariff.tariff.uses_cache_write_1h = true;
                tariff.tariff.tiers[0].rates.cache_write_1h = Some(crate::routing::DecimalRate {
                    numerator: 2,
                    denominator: 1,
                });
                tariff.cache_creation_mode =
                    Some(crate::routing::CacheCreationMode::GenericRemainderV1);
            }
            let observed = crate::relay::Usage {
                input_tokens: Some(10),
                output_tokens: Some(1),
                cached_input_tokens: Some(0),
                cache_creation_mode: tariff.cache_creation_mode,
                openai_cache_write_details: Some(OpenAiCacheWriteDetails {
                    ttl_30m_tokens: Some(3),
                    ttl_12h_tokens: Some(0),
                }),
                ..Default::default()
            };
            assert!(tariff.validate().is_ok());
            assert!(observed.validate().is_ok());
            assert_eq!(
                complete_tiered_usage(observed, &tariff),
                Err(RelayError::SettlementPending)
            );
        }
        let unpriced = crate::relay::Usage {
            input_tokens: Some(10),
            output_tokens: Some(1),
            openai_cache_write_details: Some(OpenAiCacheWriteDetails {
                ttl_30m_tokens: None,
                ttl_12h_tokens: Some(3),
            }),
            ..Default::default()
        };
        assert_eq!(
            complete_usage(unpriced, false),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            complete_strict_cache_usage(unpriced, InputSemantics::Inclusive, true),
            Err(RelayError::SettlementPending)
        );
    }
}

fn complete_generic_cache_usage(
    observed: crate::relay::Usage,
    tariff: &crate::routing::PersistedTieredTariff,
) -> Result<Usage, RelayError> {
    use crate::routing::{CacheCreationMode, CacheCreationPartition, InputSemantics};
    observed
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    if observed.cache_creation_mode != Some(CacheCreationMode::GenericRemainderV1)
        || observed.cache_creation_mode != tariff.cache_creation_mode
    {
        return Err(RelayError::SettlementPending);
    }
    let integer = |value: u64| i64::try_from(value).map_err(|_| RelayError::SettlementPending);
    let details = observed.cache_creation_details;
    let five = details.and_then(|value| value.ephemeral_5m_input_tokens);
    let hour = details.and_then(|value| value.ephemeral_1h_input_tokens);
    let total = match observed.cache_creation_input_tokens {
        Some(reported) => CacheCreationPartition::from_reported(reported, five, hour)
            .map_err(|_| RelayError::SettlementPending)?
            .total(),
        None if tariff.missing_cache_creation_is_zero
            && details.is_none()
            && observed.openai_cache_write_details.is_none() =>
        {
            0
        }
        // Even two known TTL buckets cannot prove the third, generic amount is
        // zero. Only an actual aggregate or the frozen absence contract can.
        None => return Err(RelayError::SettlementPending),
    };
    let cached = observed.cached_input_tokens.unwrap_or(0);
    let input = match tariff.input_semantics {
        InputSemantics::Inclusive => {
            if observed.uncached_input_tokens.is_some() {
                return Err(RelayError::SettlementPending);
            }
            observed.input_tokens.ok_or(RelayError::SettlementPending)?
        }
        InputSemantics::AnthropicText => {
            let uncached = observed
                .uncached_input_tokens
                .ok_or(RelayError::SettlementPending)?;
            let inclusive = uncached
                .checked_add(cached)
                .and_then(|value| value.checked_add(total))
                .ok_or(RelayError::SettlementPending)?;
            if observed
                .input_tokens
                .is_some_and(|value| value != inclusive)
            {
                return Err(RelayError::SettlementPending);
            }
            inclusive
        }
    };
    let usage = Usage {
        image: None,
        upstream_cost_atto_usd: None,
        cache_creation: Some(charging::CacheCreationUsage {
            twelve_hour: None,
            total: integer(total)?,
            five_minute: five.map(integer).transpose()?,
            one_hour: hour.map(integer).transpose()?,
            cache_creation_mode: tariff.cache_creation_mode,
            reported_total: observed
                .cache_creation_input_tokens
                .map(integer)
                .transpose()?,
        }),
        input: integer(input)?,
        cached: integer(cached)?,
        output: integer(
            observed
                .output_tokens
                .ok_or(RelayError::SettlementPending)?,
        )?,
    };
    if !usage.valid() {
        return Err(RelayError::SettlementPending);
    }
    Ok(usage)
}

/// Revision 2 preserves its original complete two-TTL partition. Unknown
/// measurements remain pending unless the frozen supplier contract says zero.
fn complete_strict_cache_usage(
    observed: crate::relay::Usage,
    semantics: crate::routing::InputSemantics,
    missing_creation_is_zero: bool,
) -> Result<Usage, RelayError> {
    observed
        .validate()
        .map_err(|_| RelayError::SettlementPending)?;
    if observed.cache_creation_mode.is_some() || observed.uncached_input_tokens.is_some() {
        return Err(RelayError::SettlementPending);
    }
    let integer = |v: u64| i64::try_from(v).map_err(|_| RelayError::SettlementPending);
    let total = match observed.cache_creation_input_tokens {
        Some(value) => integer(value)?,
        None if missing_creation_is_zero
            && observed.cache_creation_details.is_none()
            && observed.openai_cache_write_details.is_none() =>
        {
            0
        }
        None => return Err(RelayError::SettlementPending),
    };
    let details = observed.cache_creation_details;
    let mut five_minute = details
        .and_then(|v| v.ephemeral_5m_input_tokens)
        .map(integer)
        .transpose()?;
    let mut one_hour = details
        .and_then(|v| v.ephemeral_1h_input_tokens)
        .map(integer)
        .transpose()?;
    if total == 0 {
        // An authoritative aggregate zero proves both disjoint subsets zero.
        five_minute = Some(0);
        one_hour = Some(0);
    } else if let Some(five) = five_minute
        && one_hour.is_none()
    {
        one_hour = Some(
            total
                .checked_sub(five)
                .ok_or(RelayError::SettlementPending)?,
        );
    } else if let Some(hour) = one_hour
        && five_minute.is_none()
    {
        five_minute = Some(
            total
                .checked_sub(hour)
                .ok_or(RelayError::SettlementPending)?,
        );
    }
    if semantics == crate::routing::InputSemantics::AnthropicText
        && (five_minute.is_none() || one_hour.is_none())
    {
        return Err(RelayError::SettlementPending);
    }
    let usage = Usage {
        image: None,
        upstream_cost_atto_usd: None,
        cache_creation: Some(charging::CacheCreationUsage {
            twelve_hour: None,
            total,
            five_minute,
            one_hour,
            cache_creation_mode: None,
            reported_total: None,
        }),
        input: integer(observed.input_tokens.ok_or(RelayError::SettlementPending)?)?,
        cached: integer(observed.cached_input_tokens.unwrap_or(0))?,
        output: integer(
            observed
                .output_tokens
                .ok_or(RelayError::SettlementPending)?,
        )?,
    };
    if !usage.valid() {
        return Err(RelayError::SettlementPending);
    }
    Ok(usage)
}

/// Only the original operation and its immutable price can confirm a lost ACK.
/// This verifies a read receipt; it never retries dispatch or changes funds.
pub(super) fn confirmed_settlement(
    charge: &charging::Charge,
    id: &str,
    price: &charging::Price,
    usage: Usage,
    outcome: Outcome,
) -> bool {
    let expected = match outcome {
        Outcome::Completed => "completed",
        Outcome::Cancelled => "cancelled",
        Outcome::Failed => "failed",
    };
    charge.id == id
        && charge.state == "settled"
        && &charge.price == price
        && charge.usage == usage
        && charge.outcome.as_deref() == Some(expected)
        && price.cost(usage).ok() == Some(charge.settled)
}

#[cfg(test)]
mod reserve_retry_tests {
    use super::*;
    use std::{cell::Cell, future, task::Poll};

    #[tokio::test]
    async fn explicit_rollbacks_beyond_three_attempts_preserve_success() {
        let calls = Cell::new(0);
        let result = retry_reserve(|| {
            calls.set(calls.get() + 1);
            future::ready(if calls.get() <= 4 {
                Err(charging::Error::Retry)
            } else {
                Ok("original-reservation-handle")
            })
        })
        .await;
        assert_eq!(result, Ok("original-reservation-handle"));
        assert_eq!(calls.get(), 5);
    }

    #[tokio::test]
    async fn explicit_rollbacks_beyond_three_attempts_preserve_business_rejection() {
        let calls = Cell::new(0);
        let result = retry_reserve(|| {
            calls.set(calls.get() + 1);
            future::ready(Err::<(), _>(if calls.get() <= 4 {
                charging::Error::Retry
            } else {
                charging::Error::BudgetExceeded
            }))
        })
        .await;
        assert_eq!(result, Err(charging::Error::BudgetExceeded));
        assert_eq!(calls.get(), 5);
    }

    #[tokio::test]
    async fn every_non_retry_error_returns_without_replay() {
        use charging::Error::*;
        for original in [
            Invalid,
            Unauthorized,
            Forbidden,
            Conflict,
            BudgetExceeded,
            NoFunds,
            NeedReservation,
            StaleWorker,
            WrongState,
            NotFound,
            Storage,
            Ledger,
            CommitUnknown,
        ] {
            let calls = Cell::new(0);
            let result = retry_reserve(|| {
                calls.set(calls.get() + 1);
                future::ready(Err::<(), _>(original))
            })
            .await;
            assert_eq!(result, Err(original));
            assert_eq!(calls.get(), 1, "unexpected replay of {original:?}");
        }
    }

    #[tokio::test]
    // @mark model.reservation-retry role=test
    async fn permanent_retry_has_a_finite_attempt_limit() {
        let calls = Cell::new(0);
        let result = retry_reserve(|| {
            calls.set(calls.get() + 1);
            future::ready(Err::<(), _>(charging::Error::Retry))
        })
        .await;
        assert_eq!(result, Err(charging::Error::Retry));
        assert!(calls.get() > 0 && calls.get() <= 8);
    }

    #[tokio::test]
    async fn elapsed_transaction_time_prevents_a_new_attempt_after_window() {
        let calls = Cell::new(0);
        let result = retry_reserve(|| {
            calls.set(calls.get() + 1);
            async {
                tokio::time::sleep(Duration::from_millis(2050)).await;
                Err::<(), _>(charging::Error::Retry)
            }
        })
        .await;
        assert_eq!(result, Err(charging::Error::Retry));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn an_in_flight_transaction_result_is_not_interrupted_by_window() {
        let calls = Cell::new(0);
        let result = retry_reserve(|| {
            calls.set(calls.get() + 1);
            async {
                tokio::time::sleep(Duration::from_millis(2050)).await;
                Ok("committed-after-start-window")
            }
        })
        .await;
        assert_eq!(result, Ok("committed-after-start-window"));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn dropping_during_backoff_never_starts_another_operation() {
        let calls = Cell::new(0);
        let mut pending = Box::pin(retry_reserve(|| {
            calls.set(calls.get() + 1);
            future::ready(Err::<(), _>(charging::Error::Retry))
        }));
        // One deterministic poll completes the first operation and reaches the
        // pending backoff. Dropping models an actual hook-timeout future drop,
        // not a client disconnect (which deliberately does not cancel reserve).
        let first = future::poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx))).await;
        assert!(first.is_pending());
        assert_eq!(calls.get(), 1);
        drop(pending);
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn existing_non_reservation_retry_keeps_three_attempts() {
        let calls = Cell::new(0);
        let result = retry(|| {
            calls.set(calls.get() + 1);
            future::ready(Err::<(), _>(charging::Error::Retry))
        })
        .await;
        assert_eq!(result, Err(charging::Error::Retry));
        assert_eq!(calls.get(), 3);
    }
}

#[cfg(test)]
mod finalize_diagnostic_tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        future,
    };

    async fn observed_case(original: charging::Result<&'static str>, expected_attempts: u8) {
        let calls = Cell::new(0u8);
        let observations = RefCell::new(Vec::new());
        let result = retry_observed(
            None,
            || {
                calls.set(calls.get() + 1);
                future::ready(original)
            },
            |observation| observations.borrow_mut().push(observation),
        )
        .await;
        assert_eq!(result, original);
        assert_eq!(calls.get(), expected_attempts);
        let observations = observations.into_inner();
        assert_eq!(observations.len(), usize::from(expected_attempts) + 1);
        for (index, observation) in observations[..usize::from(expected_attempts)]
            .iter()
            .enumerate()
        {
            assert_eq!(observation.attempt, index as u8 + 1);
            assert!(!observation.final_result);
            assert_eq!(observation.error, original.err());
        }
        let final_result = observations.last().unwrap();
        assert!(final_result.final_result);
        assert_eq!(final_result.attempt, expected_attempts);
        assert_eq!(final_result.error, original.err());
    }

    #[tokio::test]
    async fn retry_exhaustion_is_observed_after_exactly_three_attempts() {
        observed_case(Err(charging::Error::Retry), 3).await;
    }

    #[tokio::test]
    async fn commit_unknown_is_observed_without_replaying() {
        observed_case(Err(charging::Error::CommitUnknown), 1).await;
    }

    #[tokio::test]
    async fn non_retry_errors_are_observed_without_replaying() {
        use charging::Error::*;
        for error in [
            Invalid,
            Unauthorized,
            Forbidden,
            Conflict,
            BudgetExceeded,
            NoFunds,
            NeedReservation,
            StaleWorker,
            WrongState,
            NotFound,
            Storage,
            Ledger,
        ] {
            observed_case(Err(error), 1).await;
        }
    }

    #[tokio::test]
    async fn success_preserves_the_original_return_value() {
        observed_case(Ok("original-settlement"), 1).await;
    }
}

#[cfg(test)]
mod settlement_retry_tests {
    use super::*;
    use std::{cell::Cell, future};
    const REQUEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[tokio::test]
    async fn three_explicit_rollbacks_then_success_preserves_the_original_value() {
        let calls = Cell::new(0);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let result = retry_finalize(deadline, REQUEST, FinalizeStage::Settle, || {
            calls.set(calls.get() + 1);
            future::ready(if calls.get() <= 3 {
                Err(charging::Error::Retry)
            } else {
                Ok("original-settlement")
            })
        })
        .await;
        assert_eq!(result, Ok("original-settlement"));
        assert_eq!(calls.get(), 4);
    }

    #[tokio::test]
    async fn every_non_retry_error_including_unknown_commit_never_replays() {
        use charging::Error::*;
        for error in [
            Invalid,
            Unauthorized,
            Forbidden,
            Conflict,
            BudgetExceeded,
            NoFunds,
            NeedReservation,
            StaleWorker,
            WrongState,
            NotFound,
            Storage,
            Ledger,
            CommitUnknown,
        ] {
            let calls = Cell::new(0);
            let result = retry_finalize(
                tokio::time::Instant::now() + Duration::from_secs(2),
                REQUEST,
                FinalizeStage::Settle,
                || {
                    calls.set(calls.get() + 1);
                    future::ready(Err::<(), _>(error))
                },
            )
            .await;
            assert_eq!(result, Err(error));
            assert_eq!(calls.get(), 1);
        }
    }

    #[tokio::test]
    async fn permanent_rollbacks_have_exactly_eight_attempts() {
        let calls = Cell::new(0);
        let result = retry_finalize(
            tokio::time::Instant::now() + Duration::from_secs(2),
            REQUEST,
            FinalizeStage::Settle,
            || {
                calls.set(calls.get() + 1);
                future::ready(Err::<(), _>(charging::Error::Retry))
            },
        )
        .await;
        assert_eq!(result, Err(charging::Error::Retry));
        assert_eq!(calls.get(), 8);
    }

    #[tokio::test]
    async fn expired_shared_window_does_not_start_settle_after_checkpoint() {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
        let checkpoint =
            retry_finalize(deadline, REQUEST, FinalizeStage::ProviderFinal, || async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok("checkpoint-committed")
            })
            .await;
        assert_eq!(checkpoint, Ok("checkpoint-committed"));
        let calls = Cell::new(0);
        let settlement = retry_finalize(deadline, REQUEST, FinalizeStage::Settle, || {
            calls.set(calls.get() + 1);
            future::ready(Ok(()))
        })
        .await;
        assert_eq!(settlement, Err(charging::Error::Retry));
        assert_eq!(calls.get(), 0);
    }

    #[tokio::test]
    async fn an_inflight_settlement_is_not_cancelled_by_the_start_window() {
        let result = retry_finalize(
            tokio::time::Instant::now() + Duration::from_millis(20),
            REQUEST,
            FinalizeStage::Settle,
            || async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok("settlement-committed-after-start-window")
            },
        )
        .await;
        assert_eq!(result, Ok("settlement-committed-after-start-window"));
    }

    #[tokio::test]
    async fn rollback_after_the_window_cannot_start_a_second_settlement() {
        let calls = Cell::new(0);
        let result = retry_finalize(
            tokio::time::Instant::now() + Duration::from_millis(20),
            REQUEST,
            FinalizeStage::Settle,
            || {
                calls.set(calls.get() + 1);
                async {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    Err::<(), _>(charging::Error::Retry)
                }
            },
        )
        .await;
        assert_eq!(result, Err(charging::Error::Retry));
        assert_eq!(calls.get(), 1);
    }
}

#[cfg(test)]
mod a07_billing_tests {
    use super::*;
    use crate::relay::Usage as Observed;

    fn receipt() -> charging::Charge {
        charging::Charge {
            id: "synthetic-model-operation".into(),
            actor_user_id: 1,
            key_id: Some(2),
            oauth_source: None,
            owner_account_id: 3,
            payer_account_id: 3,
            price: charging::Price {
                user_trust: None,
                image: None,
                upstream_cost_group_ppm: None,
                tiered: None,
                version: "routing-1-rule-1".into(),
                model: "synthetic-model".into(),
                group: "default".into(),
                input_per_million: 1_000_000,
                cached_per_million: 250_000,
                output_per_million: 2_000_000,
            },
            source: charging::Source::Wallet,
            authorized_at: 1,
            lease_until: 2,
            hard_deadline: 3,
            state: "settled".into(),
            reserved: 100,
            usage: Usage {
                cache_creation: None,
                image: None,
                upstream_cost_atto_usd: None,
                input: 10,
                cached: 2,
                output: 20,
            },
            settled: 49,
            refunded: 0,
            revision: 4,
            outcome: Some("completed".into()),
        }
    }

    #[test]
    fn final_receipt_rejects_unknown_or_inconsistent_usage() {
        for observed in [
            Observed::default(),
            Observed {
                input_tokens: Some(1),
                ..Observed::default()
            },
            Observed {
                output_tokens: Some(1),
                ..Observed::default()
            },
            Observed {
                input_tokens: Some(2),
                output_tokens: Some(3),
                cached_input_tokens: Some(3),
                ..Observed::default()
            },
            Observed {
                input_tokens: Some(2),
                output_tokens: Some(3),
                reasoning_output_tokens: Some(4),
                ..Observed::default()
            },
            Observed {
                input_tokens: Some(2),
                output_tokens: Some(3),
                cache_creation_input_tokens: Some(1),
                ..Observed::default()
            },
            Observed {
                input_tokens: Some(i64::MAX as u64 + 1),
                output_tokens: Some(0),
                ..Observed::default()
            },
        ] {
            assert_eq!(
                complete_usage(observed, false),
                Err(RelayError::SettlementPending)
            );
        }
    }

    #[test]
    fn final_receipt_keeps_cache_as_a_subset_not_an_extra_charge() {
        assert_eq!(
            complete_usage(
                Observed {
                    cache_creation_details: None,
                    upstream_cost_atto_usd: None,
                    input_tokens: Some(10),
                    output_tokens: Some(20),
                    cached_input_tokens: Some(2),
                    cache_creation_input_tokens: Some(0),
                    reasoning_output_tokens: Some(5),
                    cache_creation_mode: None,
                    uncached_input_tokens: None,
                    openai_cache_write_details: None,
                },
                false,
            ),
            Ok(Usage {
                cache_creation: None,
                image: None,
                upstream_cost_atto_usd: None,
                input: 10,
                cached: 2,
                output: 20
            })
        );
    }

    #[test]
    fn complete_usage_rejects_cache_creation_for_token_pricing_even_with_reported_cost() {
        let observed = Observed {
            cache_creation_details: None,
            upstream_cost_atto_usd: Some(123_456_789_000_000_000),
            input_tokens: Some(10),
            output_tokens: Some(20),
            cached_input_tokens: Some(2),
            cache_creation_input_tokens: Some(1),
            reasoning_output_tokens: Some(5),
            cache_creation_mode: None,
            uncached_input_tokens: None,
            openai_cache_write_details: None,
        };
        assert_eq!(
            complete_usage(observed, false),
            Err(RelayError::SettlementPending)
        );
    }

    #[test]
    fn complete_usage_strips_reported_cost_for_token_pricing() {
        let observed = Observed {
            cache_creation_details: None,
            upstream_cost_atto_usd: Some(123_456_789_000_000_000),
            input_tokens: Some(10),
            output_tokens: Some(20),
            cached_input_tokens: Some(2),
            cache_creation_input_tokens: Some(0),
            reasoning_output_tokens: Some(5),
            cache_creation_mode: None,
            uncached_input_tokens: None,
            openai_cache_write_details: None,
        };
        assert_eq!(
            complete_usage(observed, false),
            Ok(Usage {
                cache_creation: None,
                image: None,
                upstream_cost_atto_usd: None,
                input: 10,
                cached: 2,
                output: 20,
            })
        );
    }

    #[test]
    fn complete_usage_accepts_cache_creation_when_upstream_cost_is_present() {
        assert_eq!(
            complete_usage(
                Observed {
                    cache_creation_details: None,
                    upstream_cost_atto_usd: Some(123_456_789_000_000_000),
                    input_tokens: Some(10),
                    output_tokens: Some(20),
                    cached_input_tokens: Some(2),
                    cache_creation_input_tokens: Some(1),
                    reasoning_output_tokens: Some(5),
                    cache_creation_mode: None,
                    uncached_input_tokens: None,
                    openai_cache_write_details: None,
                },
                true,
            ),
            Ok(Usage {
                cache_creation: None,
                image: None,
                upstream_cost_atto_usd: Some(123_456_789_000_000_000),
                input: 10,
                cached: 2,
                output: 20
            })
        );
    }

    #[test]
    fn settlement_confirmation_requires_original_price_and_exact_amount() {
        let expected = receipt();
        let confirms = |value: &charging::Charge| {
            confirmed_settlement(
                value,
                &expected.id,
                &expected.price,
                expected.usage,
                Outcome::Completed,
            )
        };
        assert!(confirms(&expected));
        for field in 0..8 {
            let mut changed = expected.clone();
            match field {
                0 => changed.id.push_str("-other"),
                1 => changed.state = "reconcile".into(),
                2 => changed.price.version.push_str("-other"),
                3 => changed.price.input_per_million += 1,
                4 => changed.usage.output += 1,
                5 => changed.settled -= 1,
                6 => changed.settled += 1,
                _ => changed.outcome = Some("cancelled".into()),
            }
            assert!(!confirms(&changed));
        }
    }

    #[tokio::test]
    async fn uncertain_commit_is_read_not_retried() {
        let mut calls = 0;
        let result = retry(|| {
            calls += 1;
            std::future::ready(Err::<(), _>(charging::Error::CommitUnknown))
        })
        .await;
        assert_eq!(result, Err(charging::Error::CommitUnknown));
        assert_eq!(calls, 1);
    }

    fn generic_frozen_tariff() -> crate::routing::PersistedTieredTariff {
        use crate::routing::*;
        let rate = |numerator, denominator| DecimalRate {
            numerator,
            denominator,
        };
        PersistedTieredTariff {
            tariff: TieredTariff {
                uses_cache_write_12h: false,
                revision: 3,
                source_digest: "ab".repeat(32),
                uses_cache_read: true,
                uses_cache_write: true,
                uses_cache_write_1h: true,
                tiers: vec![TokenTier {
                    up_to_input_tokens: None,
                    rates: TieredRates {
                        cache_write_12h: None,
                        input: rate(5, 1),
                        output: rate(25, 1),
                        cache_read: rate(1, 2),
                        cache_write: rate(25, 4),
                        cache_write_1h: Some(rate(10, 1)),
                    },
                }],
            },
            input_semantics: InputSemantics::AnthropicText,
            group_multiplier_ppm: 1_000_000,
            speed_multiplier_ppm: 1_000_000,
            credits_per_usd: 500_000,
            missing_cache_creation_is_zero: false,
            cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
        }
    }

    fn generic_anthropic(value: serde_json::Value) -> crate::relay::Usage {
        crate::relay::provider_usage_with_cache_creation_mode(
            crate::relay::UpstreamProtocol::Anthropic,
            &value,
            Some(crate::routing::CacheCreationMode::GenericRemainderV1),
        )
        .unwrap()
    }

    #[test]
    fn generic_normalization_preserves_raw_aggregate_and_each_unknown_ttl() {
        let tariff = generic_frozen_tariff();
        let observed = generic_anthropic(serde_json::json!({
            "input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,
            "cache_creation_input_tokens":2,
            "cache_creation":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4}
        }));
        let usage = complete_tiered_usage(observed, &tariff).unwrap();
        assert_eq!(usage.input, 37);
        assert_eq!(
            usage.cache_creation.unwrap(),
            charging::CacheCreationUsage {
                twelve_hour: None,
                total: 7,
                five_minute: Some(3),
                one_hour: Some(4),
                cache_creation_mode: tariff.cache_creation_mode,
                reported_total: Some(2),
            }
        );
        let aggregate_only = generic_anthropic(serde_json::json!({
            "input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,
            "cache_creation_input_tokens":50
        }));
        let usage = complete_tiered_usage(aggregate_only, &tariff).unwrap();
        assert_eq!(usage.input, 80);
        assert_eq!(usage.cache_creation.unwrap().five_minute, None);
        assert_eq!(usage.cache_creation.unwrap().one_hour, None);
        assert_eq!(usage.cache_creation.unwrap().reported_total, Some(50));
    }

    #[test]
    fn generic_missing_aggregate_requires_the_original_complete_absence_zero_contract() {
        let mut tariff = generic_frozen_tariff();
        let absent = generic_anthropic(serde_json::json!({
            "input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20
        }));
        assert_eq!(absent.input_tokens, None);
        assert_eq!(absent.uncached_input_tokens, Some(10));
        assert_eq!(
            complete_tiered_usage(absent, &tariff),
            Err(RelayError::SettlementPending)
        );
        tariff.missing_cache_creation_is_zero = true;
        let usage = complete_tiered_usage(absent, &tariff).unwrap();
        assert_eq!(usage.input, 30);
        assert_eq!(
            usage.cache_creation.unwrap(),
            charging::CacheCreationUsage {
                twelve_hour: None,
                total: 0,
                five_minute: None,
                one_hour: None,
                cache_creation_mode: tariff.cache_creation_mode,
                reported_total: None,
            }
        );
        for details in [
            serde_json::json!({}),
            serde_json::json!({"ephemeral_5m_input_tokens":0}),
            serde_json::json!({"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4}),
        ] {
            let observed = generic_anthropic(serde_json::json!({
                "input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,"cache_creation":details
            }));
            assert_eq!(
                complete_tiered_usage(observed, &tariff),
                Err(RelayError::SettlementPending)
            );
        }
    }

    #[test]
    fn generic_mode_cannot_be_inferred_from_receipt_fields_or_relabel_old_tariffs() {
        let receipt = serde_json::json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,"cache_creation_input_tokens":0});
        let strict =
            crate::relay::provider_usage(crate::relay::UpstreamProtocol::Anthropic, &receipt)
                .unwrap();
        let mut tariff = generic_frozen_tariff();
        assert_eq!(
            complete_tiered_usage(strict, &tariff),
            Err(RelayError::SettlementPending)
        );
        let generic = generic_anthropic(receipt);
        assert!(complete_tiered_usage(generic, &tariff).is_ok());
        tariff.tariff.revision = 2;
        tariff.cache_creation_mode = None;
        assert_eq!(
            complete_tiered_usage(generic, &tariff),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            complete_usage(generic, false),
            Err(RelayError::SettlementPending)
        );
    }

    #[test]
    fn cache_v2_requires_real_ttls_and_keeps_unknown_creation_pending() {
        use crate::relay::{CacheCreationDetails, Usage as Observed};
        use crate::routing::InputSemantics;
        let observed = Observed {
            input_tokens: Some(100),
            output_tokens: Some(5),
            cached_input_tokens: Some(20),
            cache_creation_input_tokens: Some(70),
            cache_creation_details: Some(CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(30),
                ephemeral_1h_input_tokens: Some(40),
            }),
            ..Observed::default()
        };
        let usage =
            complete_strict_cache_usage(observed, InputSemantics::AnthropicText, false).unwrap();
        assert_eq!(
            usage.cache_creation,
            Some(charging::CacheCreationUsage {
                twelve_hour: None,
                total: 70,
                five_minute: Some(30),
                one_hour: Some(40),
                cache_creation_mode: None,
                reported_total: None,
            })
        );
        for details in [
            CacheCreationDetails {
                ephemeral_5m_input_tokens: None,
                ephemeral_1h_input_tokens: Some(40),
            },
            CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(30),
                ephemeral_1h_input_tokens: None,
            },
        ] {
            assert_eq!(
                complete_strict_cache_usage(
                    Observed {
                        cache_creation_details: Some(details),
                        ..observed
                    },
                    InputSemantics::AnthropicText,
                    false
                )
                .unwrap(),
                usage
            );
        }
        assert_eq!(
            complete_usage(observed, false),
            Err(RelayError::SettlementPending)
        );
        let aggregate_only = Observed {
            cache_creation_details: None,
            ..observed
        };
        assert_eq!(
            complete_strict_cache_usage(aggregate_only, InputSemantics::AnthropicText, false),
            Err(RelayError::SettlementPending)
        );
        assert!(
            complete_strict_cache_usage(aggregate_only, InputSemantics::Inclusive, false).is_ok()
        );
        let unknown = Observed {
            cache_creation_input_tokens: None,
            cache_creation_details: None,
            ..observed
        };
        assert_eq!(
            complete_strict_cache_usage(unknown, InputSemantics::Inclusive, false),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            complete_strict_cache_usage(unknown, InputSemantics::Inclusive, true)
                .unwrap()
                .cache_creation
                .unwrap()
                .total,
            0
        );
        let zero = Observed {
            cache_creation_input_tokens: Some(0),
            cache_creation_details: None,
            ..observed
        };
        assert_eq!(
            complete_strict_cache_usage(zero, InputSemantics::AnthropicText, false)
                .unwrap()
                .cache_creation
                .unwrap()
                .one_hour,
            Some(0)
        );
    }

    #[tokio::test]
    async fn rolled_back_operations_keep_a_bounded_retry_count() {
        let mut calls = 0;
        let result = retry(|| {
            calls += 1;
            std::future::ready(Err::<(), _>(charging::Error::Retry))
        })
        .await;
        assert_eq!(result, Err(charging::Error::Retry));
        assert_eq!(calls, 3);
    }
}
