//! UDS service capability plus original user credential. Neither is inferred
//! from protobuf fields; the domain rechecks the Session inside its transaction.
use super::{AssistantModel, Binding, Error, Operation, State};
use crate::internal_rpc::pb;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tonic::{Request, Response, Status, metadata::MetadataMap};

#[derive(Clone)]
pub struct AssistantRpc {
    service: AssistantModel,
    token_digest: [u8; 32],
    enabled: bool,
    gifts: Option<crate::gift::GiftStore>,
    agent: Option<super::agent::store::AgentService>,
    agent_features: Vec<String>,
    facts: Option<crate::assistant_tools_core::facts::BusinessFactsStore>,
    confirmations: Option<crate::assistant_tools_core::confirmation::ConfirmationStore>,
}
fn single<'a>(metadata: &'a MetadataMap, name: &'static str) -> Result<&'a str, Status> {
    let mut values = metadata.get_all(name).iter();
    let value = values
        .next()
        .ok_or_else(|| Status::unauthenticated("missing assistant credential or protocol"))?;
    if values.next().is_some() {
        return Err(Status::unauthenticated(
            "duplicate assistant credential or protocol",
        ));
    }
    value
        .to_str()
        .map_err(|_| Status::unauthenticated("invalid assistant metadata"))
}
fn status(e: Error) -> Status {
    match e {
        Error::Invalid => Status::invalid_argument("invalid assistant request"),
        Error::Unauthorized => Status::unauthenticated("assistant session or grant expired"),
        Error::Forbidden => Status::permission_denied("assistant authority denied"),
        Error::Conflict => Status::aborted("assistant state changed; inspect original operation"),
        Error::NoFunds => Status::resource_exhausted("assistant funds or budget unavailable"),
        Error::Unavailable => {
            Status::unavailable("assistant unavailable; inspect original operation")
        }
        Error::NotFound => Status::not_found("assistant record not found"),
    }
}
impl AssistantRpc {
    pub fn new(service: AssistantModel, token: &[u8], enabled: bool) -> Result<Self, Error> {
        if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            service,
            token_digest: Sha256::digest(token).into(),
            enabled,
            gifts: None,
            agent: None,
            agent_features: Vec::new(),
            facts: None,
            confirmations: None,
        })
    }
    pub(crate) async fn with_agent(
        mut self,
        agent: super::agent::store::AgentService,
    ) -> Result<Self, Error> {
        agent.check_ready().await?;
        self.agent_features = agent.extension_features()?;
        self.agent = Some(agent);
        Ok(self)
    }
    pub(crate) fn with_gifts(mut self, gifts: crate::gift::GiftStore) -> Self {
        self.gifts = Some(gifts);
        self
    }
    pub(crate) fn with_business_facts(
        mut self,
        facts: crate::assistant_tools_core::facts::BusinessFactsStore,
    ) -> Self {
        self.facts = Some(facts);
        self
    }
    pub(crate) fn with_confirmations(
        mut self,
        confirmations: crate::assistant_tools_core::confirmation::ConfirmationStore,
    ) -> Self {
        self.confirmations = Some(confirmations);
        self
    }
    pub fn close_admission(&self) {
        self.service.close_admission();
    }
    pub async fn drain(&self, grace: std::time::Duration) -> bool {
        self.service.drain(grace).await
    }
    pub fn features(&self) -> Vec<String> {
        if !self.enabled {
            return vec![];
        }
        let mut features = vec![
            "assistant.personal_text.reserve.v1".into(),
            "assistant.personal_text.execute.v1".into(),
        ];
        if self.gifts.is_some() && self.service.gift_enabled() {
            features.push("assistant.new_user_gift.v1".into());
        }
        if self.facts.is_some() {
            features.push("assistant.business_facts.v1".into());
        }
        if self.confirmations.is_some() {
            features.push("assistant.tool_confirmation.v1".into());
        }
        if self.agent.is_some() {
            features.push("assistant.agent_turn.execute.v1".into());
            features.extend(self.agent_features.clone());
        }
        features
    }
    async fn verify_agent_session(&self, metadata: &MetadataMap) -> Result<(), Status> {
        let session = self.session(metadata)?;
        let mut tx = self.service.begin().await.map_err(status)?;
        super::actor(&mut tx, &session).await.map_err(status)?;
        tx.commit().await.map_err(|e| status(e.into()))?;
        Ok(())
    }
    fn session(&self, metadata: &MetadataMap) -> Result<String, Status> {
        let token = single(metadata, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("invalid assistant service credential"))?;
        if !(32..=256).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Status::unauthenticated(
                "invalid assistant service credential",
            ));
        }
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if !bool::from(digest.ct_eq(&self.token_digest)) {
            return Err(Status::unauthenticated(
                "invalid assistant service credential",
            ));
        }
        if single(metadata, "x-lmm-protocol")? != "1" {
            return Err(Status::failed_precondition("unsupported protocol major"));
        }
        if !self.enabled {
            return Err(Status::permission_denied(
                "assistant service permission denied",
            ));
        }
        let session = single(metadata, "x-lmm-user-credential")?;
        if !(32..=256).contains(&session.len())
            || !session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Status::unauthenticated("invalid assistant user credential"));
        }
        Ok(session.to_owned())
    }
}
fn binding(value: Option<pb::AssistantOperationBinding>) -> Result<Binding, Status> {
    let b = value.ok_or_else(|| Status::invalid_argument("missing assistant binding"))?;
    Ok(Binding {
        delegation_id: b.delegation_id,
        core_conversation_id: b.core_conversation_id,
        turn_id: b.turn_id,
        round: b.round,
        operation_key: b.operation_key,
        text_sha256: b.text_sha256,
    })
}
fn wire_binding(b: Binding) -> pb::AssistantOperationBinding {
    pb::AssistantOperationBinding {
        delegation_id: b.delegation_id,
        core_conversation_id: b.core_conversation_id,
        turn_id: b.turn_id,
        round: b.round,
        operation_key: b.operation_key,
        text_sha256: b.text_sha256,
    }
}
fn operation(o: Operation) -> pb::AssistantOperationReceipt {
    pb::AssistantOperationReceipt {
        binding: Some(wire_binding(o.binding)),
        request_id: o.request_id,
        state: match o.state {
            State::Reserved => pb::AssistantOperationState::Reserved,
            State::Streaming => pb::AssistantOperationState::Streaming,
            State::Settled => pb::AssistantOperationState::Settled,
            State::Released => pb::AssistantOperationState::Released,
            State::Unknown => pb::AssistantOperationState::Unknown,
        } as i32,
        reserved_credits: o.reserved_credits,
        settled_credits: o.settled_credits,
        answer_text: o.answer_text,
        gift_receipt: o.gift_receipt.map(wire_gift),
    }
}
fn grant(g: super::Grant) -> pb::AssistantGrantReceipt {
    pb::AssistantGrantReceipt {
        delegation_id: g.delegation_id,
        core_conversation_id: g.core_conversation_id,
        turn_id: g.turn_id,
        expires_at_unix: g.expires_at_unix,
        max_model_calls: g.max_model_calls,
        revoked: g.revoked,
    }
}
#[tonic::async_trait]
impl pb::core_assistant_model_server::CoreAssistantModel for AssistantRpc {
    async fn begin_conversation(
        &self,
        r: Request<pb::AssistantConversationKey>,
    ) -> Result<Response<pb::AssistantConversationReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let v = self
            .service
            .conversation(&session, &r.into_inner().creation_key, true)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AssistantConversationReceipt {
            creation_key: v.creation_key,
            core_conversation_id: v.core_conversation_id,
        }))
    }
    async fn lookup_conversation(
        &self,
        r: Request<pb::AssistantConversationKey>,
    ) -> Result<Response<pb::AssistantConversationReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let v = self
            .service
            .conversation(&session, &r.into_inner().creation_key, false)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AssistantConversationReceipt {
            creation_key: v.creation_key,
            core_conversation_id: v.core_conversation_id,
        }))
    }
    async fn issue_grant(
        &self,
        r: Request<pb::AssistantTurnKey>,
    ) -> Result<Response<pb::AssistantGrantReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let r = r.into_inner();
        Ok(Response::new(grant(
            self.service
                .turn(&session, &r.core_conversation_id, &r.turn_id, true)
                .await
                .map_err(status)?,
        )))
    }
    async fn lookup_grant(
        &self,
        r: Request<pb::AssistantTurnKey>,
    ) -> Result<Response<pb::AssistantGrantReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let r = r.into_inner();
        Ok(Response::new(grant(
            self.service
                .turn(&session, &r.core_conversation_id, &r.turn_id, false)
                .await
                .map_err(status)?,
        )))
    }
    async fn reserve_text(
        &self,
        r: Request<pb::AssistantTextReservation>,
    ) -> Result<Response<pb::AssistantOperationReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let r = r.into_inner();
        let b = binding(r.binding)?;
        Ok(Response::new(operation(
            self.service
                .reserve_text(&session, &b, &r.text)
                .await
                .map_err(status)?,
        )))
    }
    async fn get_operation(
        &self,
        r: Request<pb::AssistantOperationBinding>,
    ) -> Result<Response<pb::AssistantOperationReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let b = binding(Some(r.into_inner()))?;
        Ok(Response::new(operation(
            self.service.operation(&session, &b).await.map_err(status)?,
        )))
    }
    async fn revoke_grant(
        &self,
        r: Request<pb::AssistantRevokeGrant>,
    ) -> Result<Response<pb::AssistantRevokeReceipt>, Status> {
        let session = self.session(r.metadata())?;
        let r = r.into_inner();
        self.service
            .revoke(&session, &r.delegation_id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AssistantRevokeReceipt {
            delegation_id: r.delegation_id,
            revoked: true,
        }))
    }
    async fn start_agent_turn(
        &self,
        r: Request<pb::AssistantAgentStart>,
    ) -> Result<Response<pb::AssistantAgentTurn>, Status> {
        self.verify_agent_session(r.metadata()).await?;
        let session = self.session(r.metadata())?;
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| Status::unimplemented("agent execution unavailable"))?;
        let input = r.into_inner();
        let turn = agent
            .start(
                &session,
                &input.core_conversation_id,
                &input.client_turn_id,
                &input.text,
            )
            .await
            .map_err(status)?;
        Ok(Response::new(agent_turn(turn)?))
    }
    async fn get_agent_turn(
        &self,
        r: Request<pb::AssistantAgentLookup>,
    ) -> Result<Response<pb::AssistantAgentTurn>, Status> {
        self.verify_agent_session(r.metadata()).await?;
        let session = self.session(r.metadata())?;
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| Status::unimplemented("agent execution unavailable"))?;
        let input = r.into_inner();
        let turn = agent
            .get(&session, &input.core_conversation_id, &input.client_turn_id)
            .await
            .map_err(status)?;
        Ok(Response::new(agent_turn(turn)?))
    }
    async fn claim_agent_tool(
        &self,
        r: Request<pb::AssistantAgentClaim>,
    ) -> Result<Response<pb::AssistantAgentClaimReceipt>, Status> {
        self.verify_agent_session(r.metadata()).await?;
        let session = self.session(r.metadata())?;
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| Status::unimplemented("agent execution unavailable"))?;
        let input = r.into_inner();
        let claim = agent
            .claim(&session, &input.tool_operation_id, &input.claim_key)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AssistantAgentClaimReceipt {
            call: Some(agent_call(claim.call)?),
            claim_key: claim.claim_key,
            disposition: if claim.may_execute { 1 } else { 2 },
            accepted_result: claim.accepted.map(agent_result).transpose()?,
        }))
    }
    async fn complete_agent_tool(
        &self,
        r: Request<pb::AssistantAgentComplete>,
    ) -> Result<Response<pb::AssistantAgentResult>, Status> {
        self.verify_agent_session(r.metadata()).await?;
        let session = self.session(r.metadata())?;
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| Status::unimplemented("agent execution unavailable"))?;
        let input = r.into_inner();
        let state = match input.state {
            1 => "completed",
            2 => "unknown",
            _ => return Err(Status::invalid_argument("invalid agent result state")),
        };
        let result = agent
            .complete(
                &session,
                super::agent::store::Completion {
                    operation_id: &input.tool_operation_id,
                    claim_key: &input.claim_key,
                    state,
                    result_json: &input.result_json,
                    result_sha256: &input.result_sha256,
                    domain_receipt_id: &input.domain_receipt_id,
                },
            )
            .await
            .map_err(status)?;
        Ok(Response::new(agent_result(result)?))
    }
    async fn prepare_assistant_tool_confirmation(
        &self,
        r: Request<pb::PrepareAssistantToolConfirmationRequest>,
    ) -> Result<Response<pb::AssistantToolConfirmation>, Status> {
        let session = self.session(r.metadata())?;
        let store = self
            .confirmations
            .as_ref()
            .ok_or_else(|| Status::unimplemented("assistant confirmation unavailable"))?;
        let input = r.into_inner();
        if input.normalized_draft_json.len() > 16_384 {
            return Err(Status::invalid_argument("confirmation draft too large"));
        }
        let draft: serde_json::Value = serde_json::from_slice(&input.normalized_draft_json)
            .map_err(|_| Status::invalid_argument("invalid confirmation draft"))?;
        if !draft.is_object() {
            return Err(Status::invalid_argument("invalid confirmation draft"));
        }
        let result = store
            .prepare(&session, &input.tool_operation_id, &draft)
            .await
            .map_err(tools_status)?;
        Ok(Response::new(wire_confirmation(result)?))
    }
    async fn get_assistant_tool_confirmation_receipt(
        &self,
        r: Request<pb::GetAssistantToolConfirmationReceiptRequest>,
    ) -> Result<Response<pb::AssistantToolConfirmation>, Status> {
        let session = self.session(r.metadata())?;
        let store = self
            .confirmations
            .as_ref()
            .ok_or_else(|| Status::unimplemented("assistant confirmation unavailable"))?;
        let input = r.into_inner();
        if input.normalized_draft_sha256.len() != 32 {
            return Err(Status::invalid_argument("invalid confirmation digest"));
        }
        let result = store
            .receipt(
                &session,
                &input.tool_operation_id,
                &input.normalized_draft_sha256,
            )
            .await
            .map_err(tools_status)?;
        Ok(Response::new(wire_confirmation(result)?))
    }
    async fn get_assistant_business_facts(
        &self,
        r: Request<pb::GetAssistantBusinessFactsRequest>,
    ) -> Result<Response<pb::GetAssistantBusinessFactsResponse>, Status> {
        let session = self.session(r.metadata())?;
        let facts = self
            .facts
            .as_ref()
            .ok_or_else(|| Status::unimplemented("assistant business facts unavailable"))?;
        let value = match r.into_inner().kind {
            1 => serde_json::to_value(
                facts
                    .available_models(&session)
                    .await
                    .map_err(tools_status)?,
            )
            .map_err(|_| Status::unavailable("assistant facts unavailable"))?,
            2 => facts
                .support_eligibility(&session)
                .await
                .map_err(tools_status)?,
            _ => return Err(Status::invalid_argument("unsupported assistant fact kind")),
        };
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| Status::unavailable("assistant facts unavailable"))?;
        if bytes.len() > 60 * 1024 {
            return Err(Status::resource_exhausted(
                "assistant facts exceed response limit",
            ));
        }
        Ok(Response::new(pb::GetAssistantBusinessFactsResponse {
            facts_json: bytes,
        }))
    }
    async fn get_new_user_gift(
        &self,
        r: Request<pb::AssistantGiftRequest>,
    ) -> Result<Response<pb::AssistantGiftStatus>, Status> {
        let session = self.session(r.metadata())?;
        let gifts = self
            .gifts
            .as_ref()
            .ok_or_else(|| Status::unimplemented("gift model hook unavailable"))?;
        let gift = gifts.get(&session).await.map_err(gift_status)?;
        Ok(Response::new(pb::AssistantGiftStatus {
            gift: gift.map(wire_gift),
        }))
    }
    async fn claim_stored_new_user_gift(
        &self,
        r: Request<pb::AssistantGiftRequest>,
    ) -> Result<Response<pb::AssistantGiftClaim>, Status> {
        let session = self.session(r.metadata())?;
        let gifts = self
            .gifts
            .as_ref()
            .ok_or_else(|| Status::unimplemented("gift model hook unavailable"))?;
        let (gift, already_claimed) = gifts.claim_stored(&session).await.map_err(gift_status)?;
        Ok(Response::new(pb::AssistantGiftClaim {
            gift: Some(wire_gift(gift)),
            already_claimed,
        }))
    }
}

fn gift_status(error: crate::gift::Error) -> Status {
    match error {
        crate::gift::Error::Invalid => Status::invalid_argument("invalid stored gift request"),
        crate::gift::Error::Unauthorized => Status::unauthenticated("gift session expired"),
        crate::gift::Error::Forbidden => Status::permission_denied("gift eligibility denied"),
        crate::gift::Error::NotFound => Status::not_found("gift decision not found"),
        crate::gift::Error::Conflict => Status::aborted("gift state changed; inspect same gift"),
        crate::gift::Error::Unavailable => {
            Status::unavailable("gift unavailable; inspect same gift")
        }
    }
}
fn wire_gift(g: crate::gift::Receipt) -> pb::AssistantGiftReceipt {
    pb::AssistantGiftReceipt {
        gift_id: g.gift_id,
        owner_user_id: g.owner_user_id,
        state: if g.claimed_at_unix.is_some() {
            pb::AssistantGiftState::Claimed
        } else {
            pb::AssistantGiftState::Offered
        } as i32,
        credit_amount: g.credit_amount,
        created_at_unix: g.created_at_unix,
        claimed_at_unix: g.claimed_at_unix.unwrap_or(0),
        ledger_transaction_id: g
            .ledger_transaction_id
            .map(|n| n.to_string())
            .unwrap_or_default(),
        max_credit_amount: g.max_credit_amount,
        claim_available: g.claim_available,
        claim_blocked_code: g.claim_blocked_code,
    }
}

fn tools_status(error: crate::assistant_tools_core::Error) -> Status {
    use crate::assistant_tools_core::Error as E;
    match error {
        E::Invalid => Status::invalid_argument("invalid assistant tool request"),
        E::Unauthorized => Status::unauthenticated("assistant tool session expired"),
        E::Forbidden => Status::permission_denied("assistant tool authority denied"),
        E::NotFound => Status::not_found("assistant tool record not found"),
        E::Conflict => Status::aborted("assistant tool state changed; inspect original operation"),
        E::Unavailable => Status::unavailable("assistant tool unavailable"),
    }
}
fn wire_confirmation(
    value: crate::assistant_tools_core::confirmation::Challenge,
) -> Result<pb::AssistantToolConfirmation, Status> {
    fn digest(value: &str) -> Result<Vec<u8>, Status> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Status::unavailable("invalid stored confirmation digest"));
        }
        (0..32)
            .map(|n| {
                u8::from_str_radix(&value[n * 2..n * 2 + 2], 16)
                    .map_err(|_| Status::unavailable("invalid stored confirmation digest"))
            })
            .collect()
    }
    let preview = serde_json::to_vec(&value.preview)
        .map_err(|_| Status::unavailable("confirmation preview unavailable"))?;
    if preview.len() > 16_384 || value.executed {
        return Err(Status::unavailable("invalid stored confirmation preview"));
    }
    Ok(pb::AssistantToolConfirmation {
        challenge_id: value.challenge_id,
        tool_operation_id: value.operation_id,
        tool_name: value.tool_name,
        tool_version: value.tool_version,
        source_arguments_sha256: digest(&value.arguments_sha256)?,
        normalized_draft_sha256: digest(&value.draft_sha256)?,
        preview_json: preview,
        resource_kind: value.resource_kind,
        resource_id: value.resource_id,
        resource_version: value.resource_version,
        expires_at_unix: value.expires_at_unix,
        state: value.state,
        browser_path: value.browser_path,
        executed: false,
    })
}

fn agent_call(call: super::agent::store::ToolCall) -> Result<pb::AssistantAgentToolCall, Status> {
    let scope = call.scope;
    let domain = match scope.domain.as_str() {
        "registered_core" => 1,
        "extension" => 2,
        _ => return Err(Status::internal("invalid stored agent domain")),
    };
    Ok(pb::AssistantAgentToolCall {
        tool_operation_id: call.id,
        provider_call_id: call.provider_call_id,
        model_operation_id: call.model_operation_id,
        round: call.round,
        ordinal: call.ordinal,
        scope: Some(pb::AssistantAgentScope {
            tool_name: scope.name,
            tool_version: scope.version,
            schema_sha256: scope.schema_sha256,
            price_version: scope.price_version,
            price_sha256: scope.price_sha256,
            resource_kind: scope.resource_kind,
            resource_id: scope.resource_id,
            resource_version: scope.resource_version,
            grant_id: scope.grant_id,
            domain,
            resource_schema_sha256: scope.resource_schema_sha256,
            resource_price_sha256: scope.resource_price_sha256,
        }),
        arguments_json: call.arguments_json,
        arguments_sha256: call.arguments_sha256,
    })
}
fn agent_result(
    result: super::agent::store::AcceptedResult,
) -> Result<pb::AssistantAgentResult, Status> {
    let state = match result.state.as_str() {
        "completed" => 1,
        "unknown" => 2,
        _ => return Err(Status::internal("invalid stored agent result")),
    };
    Ok(pb::AssistantAgentResult {
        tool_operation_id: result.operation_id,
        claim_key: result.claim_key,
        state,
        result_sha256: result.result_sha256,
        domain_receipt_id: result.domain_receipt_id,
    })
}
fn agent_turn(turn: super::agent::store::TurnSnapshot) -> Result<pb::AssistantAgentTurn, Status> {
    let phase = match turn.phase.as_str() {
        "new" => 1,
        "model_running" => 2,
        "tools_pending" => 3,
        "completed" => 4,
        "unknown" => 5,
        "rejected" => 6,
        "expired" => 7,
        _ => return Err(Status::internal("invalid stored agent phase")),
    };
    Ok(pb::AssistantAgentTurn {
        core_conversation_id: turn.conversation_id,
        client_turn_id: turn.client_turn_id,
        owner_user_id: turn.owner_user_id,
        owner_account_id: turn.owner_account_id,
        original_text_sha256: turn.text_sha256,
        phase,
        revision: turn.revision,
        round: turn.round,
        max_rounds: turn.max_rounds,
        model_budget_credits: turn.budget_credits,
        model_held_credits: turn.held_credits,
        model_settled_credits: turn.settled_credits,
        pending_calls: turn
            .pending_calls
            .into_iter()
            .map(agent_call)
            .collect::<Result<_, _>>()?,
        final_text: turn.final_text,
    })
}
