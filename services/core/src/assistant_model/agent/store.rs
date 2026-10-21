//! Durable turn journal. All identities and policy snapshots originate in Core.
use super::super::{AssistantModel, Error, actor, at, label, random};
use super::{
    market_policy::AssistantMarketPolicy,
    registry::{AgentRegistry, FREE_PRICE_VERSION, ToolDomain},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row, postgres::PgRow};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentPolicy {
    pub model: super::super::PersonalTextPolicy,
    pub max_rounds: u32,
    pub budget_credits: i64,
    pub max_tool_calls: u32,
    pub ttl_seconds: i64,
    pub allowed_tools: BTreeSet<String>,
    pub initial_tools: BTreeSet<String>,
    pub system_prompt: String,
    pub market: Option<AssistantMarketPolicy>,
}
impl AgentPolicy {
    fn validate(&self, registry: &AgentRegistry) -> Result<(), Error> {
        self.model.validate()?;
        if !(1..=32).contains(&self.max_rounds)
            || !(1..=256).contains(&self.max_tool_calls)
            || !(1..=900).contains(&self.ttl_seconds)
            || self.budget_credits < self.model.budget_credits
            || self.budget_credits > 9_007_199_254_740_991
            || self.allowed_tools.is_empty()
            || self.allowed_tools.len() > 72
            || self.initial_tools.is_empty()
            || self.initial_tools.len() > 8
            || !self.initial_tools.is_subset(&self.allowed_tools)
            || self.system_prompt.is_empty()
            || self.system_prompt.len() > 8192
        {
            return Err(Error::Invalid);
        }
        if let Some(market) = &self.market {
            market.validate()?;
        }
        for name in &self.allowed_tools {
            let tool = registry.get(name)?;
            if tool.domain == ToolDomain::RegisteredCore
                && !matches!(
                    name.as_str(),
                    "discover_tools"
                        | "end_conversation"
                        | "get_account_access"
                        | "get_user_overview"
                        | "list_my_api_keys"
                        | "get_new_user_gift_status"
                        | "get_registration_risk"
                        | "get_available_models"
                        | "get_model_pricing"
                )
            {
                return Err(Error::Unavailable);
            }
            if name == "call_market_tool" && self.market.is_none() {
                return Err(Error::Forbidden);
            }
        }
        Ok(())
    }
}
/// Reviewed host-only artifact and policy. Never part of an RPC message.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentHostConfig {
    pub registry_file: std::path::PathBuf,
    pub registry_sha256: String,
    pub policy: AgentPolicy,
}
#[derive(Clone)]
pub(crate) struct AgentService {
    pub(super) base: AssistantModel,
    pub(super) policy: Arc<AgentPolicy>,
    pub(super) registry: AgentRegistry,
}
#[derive(Clone, Debug)]
pub(crate) struct ToolScope {
    pub name: String,
    pub version: String,
    pub schema_sha256: Vec<u8>,
    pub price_version: String,
    pub price_sha256: Vec<u8>,
    pub resource_kind: String,
    pub resource_id: String,
    pub resource_version: String,
    pub grant_id: String,
    pub domain: String,
    pub resource_schema_sha256: Vec<u8>,
    pub resource_price_sha256: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct ToolCall {
    pub id: String,
    pub provider_call_id: String,
    pub model_operation_id: String,
    pub round: u32,
    pub ordinal: u32,
    pub scope: ToolScope,
    pub arguments_json: Vec<u8>,
    pub arguments_sha256: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct TurnSnapshot {
    pub conversation_id: String,
    pub client_turn_id: String,
    pub owner_user_id: i64,
    pub owner_account_id: i64,
    pub text_sha256: Vec<u8>,
    pub phase: String,
    pub revision: u64,
    pub round: u32,
    pub max_rounds: u32,
    pub budget_credits: i64,
    pub held_credits: i64,
    pub settled_credits: i64,
    pub pending_calls: Vec<ToolCall>,
    pub final_text: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct AcceptedResult {
    pub operation_id: String,
    pub claim_key: String,
    pub state: String,
    pub result_sha256: Vec<u8>,
    pub domain_receipt_id: String,
}
pub(crate) struct ClaimReceipt {
    pub call: ToolCall,
    pub claim_key: String,
    pub may_execute: bool,
    pub accepted: Option<AcceptedResult>,
}
pub(crate) struct Completion<'a> {
    pub operation_id: &'a str,
    pub claim_key: &'a str,
    pub state: &'a str,
    pub result_json: &'a [u8],
    pub result_sha256: &'a [u8],
    pub domain_receipt_id: &'a str,
}
fn id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(super) fn scope(row: &PgRow) -> Result<ToolScope, Error> {
    Ok(ToolScope {
        name: row.try_get("tool_name")?,
        version: row.try_get("tool_version")?,
        schema_sha256: row.try_get("schema_sha256")?,
        price_version: row.try_get("price_version")?,
        price_sha256: row.try_get("price_sha256")?,
        resource_kind: row.try_get("resource_kind")?,
        resource_id: row.try_get("resource_id")?,
        resource_version: row.try_get("resource_version")?,
        grant_id: row.try_get("scope_grant_id")?,
        domain: row.try_get("domain")?,
        resource_schema_sha256: row
            .try_get::<Option<Vec<u8>>, _>("resource_schema_sha256")?
            .unwrap_or_default(),
        resource_price_sha256: row
            .try_get::<Option<Vec<u8>>, _>("resource_price_sha256")?
            .unwrap_or_default(),
    })
}
pub(super) fn tool_call(row: &PgRow) -> Result<ToolCall, Error> {
    let arguments_json: Vec<u8> = row.try_get("arguments_json")?;
    let arguments_sha256: Vec<u8> = row.try_get("arguments_sha256")?;
    if Sha256::digest(&arguments_json).as_slice() != arguments_sha256 {
        return Err(Error::Unavailable);
    }
    Ok(ToolCall {
        id: row.try_get("id")?,
        provider_call_id: row.try_get("provider_call_id")?,
        model_operation_id: row.try_get("model_request_id")?,
        round: u32::try_from(row.try_get::<i32, _>("round")?).map_err(|_| Error::Unavailable)?,
        ordinal: u32::try_from(row.try_get::<i32, _>("ordinal")?)
            .map_err(|_| Error::Unavailable)?,
        scope: scope(row)?,
        arguments_json,
        arguments_sha256,
    })
}
fn accepted(row: &PgRow) -> Result<AcceptedResult, Error> {
    Ok(AcceptedResult {
        operation_id: row.try_get("id")?,
        claim_key: row
            .try_get::<Option<String>, _>("claim_key")?
            .ok_or(Error::Unavailable)?,
        state: row.try_get("state")?,
        result_sha256: row
            .try_get::<Option<Vec<u8>>, _>("result_sha256")?
            .ok_or(Error::Unavailable)?,
        domain_receipt_id: row
            .try_get::<Option<String>, _>("domain_receipt_id")?
            .unwrap_or_default(),
    })
}
impl AgentService {
    pub(crate) fn from_host(base: AssistantModel, config: AgentHostConfig) -> Result<Self, Error> {
        let artifact = crate::config::private_file(&config.registry_file, 2_097_152)
            .map_err(|_| Error::Unavailable)?;
        let registry = AgentRegistry::from_reviewed(&artifact, &config.registry_sha256)?;
        Self::new(base, config.policy, registry)
    }
    pub(crate) fn new(
        base: AssistantModel,
        policy: AgentPolicy,
        registry: AgentRegistry,
    ) -> Result<Self, Error> {
        policy.validate(&registry)?;
        if policy.model != base.policy {
            return Err(Error::Invalid);
        }
        Ok(Self {
            base,
            policy: Arc::new(policy),
            registry,
        })
    }
    pub(crate) async fn check_ready(&self) -> Result<(), Error> {
        let mut tx = self.base.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        for (table, mutable) in [
            (
                "agent_turns",
                vec!["phase", "revision", "final_text", "loaded_tools"],
            ),
            (
                "agent_rounds",
                vec![
                    "state",
                    "provider_response_json",
                    "provider_response_sha256",
                ],
            ),
            (
                "agent_tool_operations",
                vec![
                    "state",
                    "claim_key",
                    "result_json",
                    "result_sha256",
                    "domain_receipt_id",
                ],
            ),
            ("agent_tool_observations", vec![]),
        ] {
            let relation = format!("core_assistant.{table}");
            let valid:bool=sqlx::query_scalar("SELECT has_table_privilege(current_user,$1,'SELECT') AND has_table_privilege(current_user,$1,'INSERT') AND NOT has_table_privilege(current_user,$1,'DELETE') AND NOT has_table_privilege(current_user,$1,'TRUNCATE') AND NOT EXISTS(SELECT FROM pg_attribute WHERE attrelid=$1::regclass AND attnum>0 AND NOT attisdropped AND has_column_privilege(current_user,$1,attname,'UPDATE') IS DISTINCT FROM (attname=ANY($2::text[])))")
                .bind(&relation).bind(mutable).fetch_one(&mut *tx).await?;
            if !valid {
                return Err(Error::Unavailable);
            }
        }
        // Exact columns are queried even with an empty component: a partial
        // deployment cannot advertise a worker based only on table existence.
        sqlx::query("SELECT t.history_position,t.price,t.loaded_tools,t.source_web_session_sid,r.context_json,r.context_sha256,r.provider_response_json,r.provider_response_sha256,o.arguments_json,o.arguments_sha256,o.resource_schema_sha256,o.resource_price_sha256,b.claim_key,b.result_sha256 FROM core_assistant.agent_turns t LEFT JOIN core_assistant.agent_rounds r ON r.turn_id=t.id LEFT JOIN core_assistant.agent_tool_operations o ON o.round_id=r.id LEFT JOIN core_assistant.agent_tool_observations b ON b.operation_id=o.id WHERE FALSE").execute(&mut *tx).await?;
        let guards:i64=sqlx::query_scalar("SELECT count(*) FROM pg_trigger WHERE NOT tgisinternal AND tgenabled='O' AND tgname=ANY($1::text[]) AND tgrelid=ANY(ARRAY['core_assistant.agent_turns'::regclass,'core_assistant.agent_rounds'::regclass,'core_assistant.agent_tool_operations'::regclass,'core_assistant.agent_tool_observations'::regclass,'core_charging.requests'::regclass])")
          .bind(vec!["agent_turn_immutable","agent_round_immutable","agent_tool_immutable","agent_observation_immutable","agent_turn_binding","agent_round_binding","agent_tool_binding","agent_observation_binding","agent_charge_binding","agent_legacy_exclusion"]).fetch_one(&mut *tx).await?;
        if guards != 10 {
            return Err(Error::Unavailable);
        }
        tx.commit().await?;
        Ok(())
    }
    pub(super) async fn check_live_turn(
        &self,
        c: &mut PgConnection,
        session: &str,
        t: &PgRow,
    ) -> Result<(), Error> {
        let principal = actor(c, session).await?;
        let current: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(principal.user_id)
        .fetch_one(&mut *c)
        .await?;
        if t.try_get::<i64, _>("owner_user_id")? != principal.user_id
            || t.try_get::<i64, _>("owner_account_id")? != principal.owner_account_id
            || t.try_get::<i64, _>("session_id")? != principal.credential_id
            || t.try_get::<i64, _>("auth_version")? != current
            || t.try_get::<i64, _>("expires_at")? <= at(c).await?
            || t.try_get::<Vec<u8>, _>("registry_sha256")? != self.registry.digest()
            || t.try_get::<Value, _>("host_policy")?
                != serde_json::to_value(&*self.policy).map_err(|_| Error::Unavailable)?
        {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
    async fn snapshot(&self, c: &mut PgConnection, turn_id: &str) -> Result<TurnSnapshot, Error> {
        let t=sqlx::query("SELECT t.*,coalesce((SELECT max(round) FROM core_assistant.agent_rounds WHERE turn_id=t.id),0) AS current_round,coalesce((SELECT sum(CASE WHEN q.state IN ('reserved','streaming','reconcile') THEN q.reserved ELSE 0 END) FROM core_assistant.agent_rounds r JOIN core_charging.requests q ON q.id=r.model_request_id WHERE r.turn_id=t.id),0)::bigint AS model_held,coalesce((SELECT sum(q.settled) FROM core_assistant.agent_rounds r JOIN core_charging.requests q ON q.id=r.model_request_id WHERE r.turn_id=t.id AND q.state='settled'),0)::bigint AS model_settled FROM core_assistant.agent_turns t WHERE t.id=$1").bind(turn_id).fetch_optional(&mut *c).await?.ok_or(Error::NotFound)?;
        let calls=sqlx::query("SELECT o.*,r.model_request_id,r.round FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_rounds r ON r.id=o.round_id WHERE o.turn_id=$1 AND o.state IN ('pending','claimed','unknown') AND o.domain='extension' AND NOT EXISTS(SELECT FROM core_assistant.agent_tool_operations earlier WHERE earlier.round_id=o.round_id AND earlier.ordinal<o.ordinal AND earlier.domain='registered_core' AND earlier.state IN ('pending','claimed','unknown')) ORDER BY r.round,o.ordinal").bind(turn_id).fetch_all(&mut *c).await?;
        Ok(TurnSnapshot {
            conversation_id: t.try_get("conversation_id")?,
            client_turn_id: t.try_get("client_turn_id")?,
            owner_user_id: t.try_get("owner_user_id")?,
            owner_account_id: t.try_get("owner_account_id")?,
            text_sha256: t.try_get("text_sha256")?,
            phase: t.try_get("phase")?,
            revision: u64::try_from(t.try_get::<i64, _>("revision")?)
                .map_err(|_| Error::Unavailable)?,
            round: u32::try_from(t.try_get::<i32, _>("current_round")?)
                .map_err(|_| Error::Unavailable)?,
            max_rounds: u32::try_from(t.try_get::<i32, _>("max_rounds")?)
                .map_err(|_| Error::Unavailable)?,
            budget_credits: t.try_get("budget_credits")?,
            held_credits: t.try_get("model_held")?,
            settled_credits: t.try_get("model_settled")?,
            pending_calls: calls.iter().map(tool_call).collect::<Result<_, _>>()?,
            final_text: t.try_get("final_text")?,
        })
    }
    pub(crate) async fn start(
        &self,
        session: &str,
        conversation_id: &str,
        client_turn_id: &str,
        text: &str,
    ) -> Result<TurnSnapshot, Error> {
        if !label(client_turn_id, 128)
            || text.is_empty()
            || text.len() > 8192
            || self.base.permits.is_closed()
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let principal = actor(&mut tx, session).await?;
        self.base
            .lock_conversation(&mut tx, &principal, conversation_id)
            .await?;
        let prior=sqlx::query("SELECT id,text_sha256 FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id=$2 FOR UPDATE").bind(conversation_id).bind(client_turn_id).fetch_optional(&mut *tx).await?;
        let text_sha256 = Sha256::digest(text.as_bytes()).to_vec();
        let turn_id = if let Some(prior) = prior {
            if prior.try_get::<Vec<u8>, _>("text_sha256")? != text_sha256 {
                return Err(Error::Conflict);
            }
            prior.try_get::<String, _>("id")?
        } else {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_assistant.agent_turns WHERE conversation_id=$1 AND phase IN ('new','model_running','tools_pending','unknown')) OR EXISTS(SELECT FROM core_assistant.grants g JOIN core_charging.requests q ON q.assistant_grant_id=g.id WHERE g.conversation_id=$1 AND q.state NOT IN ('settled','released'))").bind(conversation_id).fetch_one(&mut *tx).await?;
            if active {
                return Err(Error::Conflict);
            }
            let context = crate::assistant_tools_core::context::VerifiedToolContext::verify(
                &mut tx, &principal,
            )
            .await
            .map_err(super::context_error)?;
            let facts = context
                .revalidate(&mut tx)
                .await
                .map_err(super::context_error)?;
            let turn_id = random("");
            let now = at(&mut tx).await?;
            let price = self.base.quote_in_transaction(&mut tx, &principal).await?;
            sqlx::query("INSERT INTO core_assistant.agent_turns(id,conversation_id,client_turn_id,session_id,source_web_session_sid,auth_version,owner_user_id,owner_account_id,original_text,text_sha256,registry_sha256,host_policy,max_rounds,budget_credits,expires_at,phase,price,loaded_tools) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'new',$16,$17)")
                .bind(&turn_id).bind(conversation_id).bind(client_turn_id).bind(principal.credential_id).bind(facts.browser_session).bind(facts.auth_version).bind(principal.user_id).bind(principal.owner_account_id).bind(text).bind(&text_sha256).bind(self.registry.digest().to_vec()).bind(serde_json::to_value(&*self.policy).map_err(|_|Error::Unavailable)?).bind(self.policy.max_rounds as i32).bind(self.policy.budget_credits).bind(now+self.policy.ttl_seconds).bind(serde_json::to_value(price).map_err(|_|Error::Unavailable)?).bind(serde_json::to_value(&self.policy.initial_tools).map_err(|_|Error::Unavailable)?).execute(&mut *tx).await?;
            turn_id
        };
        let result = self.snapshot(&mut tx, &turn_id).await?;
        tx.commit().await?;
        self.kick(session, &turn_id);
        Ok(result)
    }
    pub(crate) async fn get(
        &self,
        session: &str,
        conversation_id: &str,
        client_turn_id: &str,
    ) -> Result<TurnSnapshot, Error> {
        for attempt in 0..3 {
            match self
                .get_once(session, conversation_id, client_turn_id)
                .await
            {
                Err(Error::Conflict) if attempt < 2 => tokio::task::yield_now().await,
                result => return result,
            }
        }
        Err(Error::Conflict)
    }
    async fn get_once(
        &self,
        session: &str,
        conversation_id: &str,
        client_turn_id: &str,
    ) -> Result<TurnSnapshot, Error> {
        let mut tx = self.base.begin().await?;
        let principal = actor(&mut tx, session).await?;
        self.base
            .lock_conversation(&mut tx, &principal, conversation_id)
            .await?;
        let turn_id:String=sqlx::query_scalar("SELECT id FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id=$2").bind(conversation_id).bind(client_turn_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        // Charging recovery may have marked the original attempt UNKNOWN after
        // process death. Reflect that durable evidence without any redispatch.
        let unknown=sqlx::query("UPDATE core_assistant.agent_rounds r SET state='unknown' FROM core_charging.requests q WHERE r.turn_id=$1 AND q.id=r.model_request_id AND q.state='reconcile' AND r.state='streaming'").bind(&turn_id).execute(&mut *tx).await?.rows_affected();
        if unknown > 0 {
            sqlx::query("UPDATE core_assistant.agent_turns SET phase='unknown',revision=revision+1 WHERE id=$1 AND phase='model_running'").bind(&turn_id).execute(&mut *tx).await?;
        }
        let result = self.snapshot(&mut tx, &turn_id).await?;
        tx.commit().await?;
        // Wake only still-unclaimed work. A durable dispatched/unknown model
        // operation is never sent again by this wakeup.
        if result.phase == "new" || result.phase == "tools_pending" {
            self.kick(session, &turn_id);
        }
        Ok(result)
    }
    pub(crate) async fn claim(
        &self,
        session: &str,
        operation_id: &str,
        claim_key: &str,
    ) -> Result<ClaimReceipt, Error> {
        if !id(operation_id) || !label(claim_key, 128) {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let principal = actor(&mut tx, session).await?;
        let conversation: String = sqlx::query_scalar("SELECT t.conversation_id FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE o.id=$1 AND t.owner_user_id=$2")
            .bind(operation_id).bind(principal.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        super::super::lock_conversation_id(&mut tx, &conversation).await?;
        let row=sqlx::query("SELECT o.*,r.model_request_id,r.round,t.owner_user_id,t.conversation_id FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_rounds r ON r.id=o.round_id JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE o.id=$1 FOR UPDATE OF o,t").bind(operation_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if row.try_get::<i64, _>("owner_user_id")? != principal.user_id
            || row.try_get::<String, _>("domain")? != "extension"
        {
            return Err(Error::Forbidden);
        }
        let prior: Option<String> = row.try_get("claim_key")?;
        let state: String = row.try_get("state")?;
        let may_execute = if let Some(prior) = prior {
            if prior != claim_key {
                return Err(Error::Conflict);
            }
            false
        } else {
            if state != "pending" {
                return Err(Error::Conflict);
            }
            let turn =
                sqlx::query("SELECT * FROM core_assistant.agent_turns WHERE id=$1 FOR UPDATE")
                    .bind(row.try_get::<String, _>("turn_id")?)
                    .fetch_one(&mut *tx)
                    .await?;
            self.check_live_turn(&mut tx, session, &turn).await?;
            let earlier:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_assistant.agent_tool_operations previous JOIN core_assistant.agent_rounds r ON r.id=previous.round_id WHERE previous.turn_id=$1 AND previous.state IN ('pending','claimed','unknown') AND (r.round<$2 OR (r.round=$2 AND previous.ordinal<$3)))")
                .bind(row.try_get::<String,_>("turn_id")?).bind(row.try_get::<i32,_>("round")?).bind(row.try_get::<i32,_>("ordinal")?).fetch_one(&mut *tx).await?;
            if earlier {
                return Err(Error::Conflict);
            }
            sqlx::query("UPDATE core_assistant.agent_tool_operations SET state='claimed',claim_key=$2 WHERE id=$1").bind(operation_id).bind(claim_key).execute(&mut *tx).await?;
            true
        };
        let call = tool_call(&row)?;
        let accepted = if matches!(state.as_str(), "completed" | "unknown") {
            Some(accepted(&row)?)
        } else {
            None
        };
        tx.commit().await?;
        Ok(ClaimReceipt {
            call,
            claim_key: claim_key.into(),
            may_execute,
            accepted,
        })
    }
    pub(crate) async fn complete(
        &self,
        session: &str,
        input: Completion<'_>,
    ) -> Result<AcceptedResult, Error> {
        if !id(input.operation_id)
            || !label(input.claim_key, 128)
            || !matches!(input.state, "completed" | "unknown")
            || input.result_json.len() > 65_536
            || input.result_sha256.len() != 32
            || Sha256::digest(input.result_json).as_slice() != input.result_sha256
            || (input.state == "completed" && input.domain_receipt_id.is_empty())
            || input.domain_receipt_id.len() > 256
        {
            return Err(Error::Invalid);
        }
        let value: Value = serde_json::from_slice(input.result_json).map_err(|_| Error::Invalid)?;
        if !value.is_object() {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let principal = actor(&mut tx, session).await?;
        let conversation: String = sqlx::query_scalar("SELECT t.conversation_id FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE o.id=$1 AND t.owner_user_id=$2")
            .bind(input.operation_id).bind(principal.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        super::super::lock_conversation_id(&mut tx, &conversation).await?;
        let row=sqlx::query("SELECT o.*,t.owner_user_id,t.conversation_id FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE o.id=$1 FOR UPDATE OF o,t").bind(input.operation_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if row.try_get::<i64, _>("owner_user_id")? != principal.user_id
            || row.try_get::<String, _>("domain")? != "extension"
            || row.try_get::<Option<String>, _>("claim_key")?.as_deref() != Some(input.claim_key)
        {
            return Err(Error::Forbidden);
        }
        let old: String = row.try_get("state")?;
        if old == "completed" || (old == "unknown" && input.state == "unknown") {
            let result = accepted(&row)?;
            if result.state != input.state
                || result.result_sha256 != input.result_sha256
                || result.domain_receipt_id != input.domain_receipt_id
            {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(result);
        }
        if old != "claimed" && !(old == "unknown" && input.state == "completed") {
            return Err(Error::Conflict);
        }
        // Results are data, not a grant. A financial adapter must reference its
        // actual Core domain receipt; it cannot settle funds through this API.
        self.validate_domain_receipt(&mut tx, &principal, &row, &input)
            .await?;
        sqlx::query("INSERT INTO core_assistant.agent_tool_observations(operation_id,claim_key,state,result_json,result_sha256,domain_receipt_id) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(operation_id,state,result_sha256) DO NOTHING")
            .bind(input.operation_id).bind(input.claim_key).bind(input.state).bind(input.result_json).bind(input.result_sha256).bind((!input.domain_receipt_id.is_empty()).then_some(input.domain_receipt_id)).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_assistant.agent_tool_operations SET state=$2,result_json=$3,result_sha256=$4,domain_receipt_id=$5 WHERE id=$1").bind(input.operation_id).bind(input.state).bind(input.result_json).bind(input.result_sha256).bind((!input.domain_receipt_id.is_empty()).then_some(input.domain_receipt_id)).execute(&mut *tx).await?;
        let turn_id: String = row.try_get("turn_id")?;
        sqlx::query("UPDATE core_assistant.agent_turns SET revision=revision+1 WHERE id=$1")
            .bind(&turn_id)
            .execute(&mut *tx)
            .await?;
        let result = AcceptedResult {
            operation_id: input.operation_id.into(),
            claim_key: input.claim_key.into(),
            state: input.state.into(),
            result_sha256: input.result_sha256.into(),
            domain_receipt_id: input.domain_receipt_id.into(),
        };
        tx.commit().await?;
        if input.state == "completed" {
            self.kick(session, &turn_id);
        }
        Ok(result)
    }
    async fn validate_domain_receipt(
        &self,
        c: &mut PgConnection,
        actor: &crate::identity::Principal,
        row: &PgRow,
        input: &Completion<'_>,
    ) -> Result<(), Error> {
        if input.state == "unknown" {
            return Ok(());
        }
        if row.try_get::<String, _>("tool_name")? == "call_market_tool" {
            let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_tool_market.credit_calls WHERE id=$1 AND actor_user_id=$2 AND payer_account_id=$3 AND state IN ('settled','released') AND (snapshot#>>'{request,assistant_tool_operation_id}')=$4)")
                .bind(input.domain_receipt_id).bind(actor.user_id).bind(actor.owner_account_id).bind(input.operation_id).fetch_one(&mut *c).await?;
            if !valid {
                return Err(Error::Forbidden);
            }
        }
        Ok(())
    }
    pub(crate) fn extension_features(&self) -> Result<Vec<String>, Error> {
        let mut tuples = Vec::new();
        let mut features = Vec::new();
        for name in &self.policy.allowed_tools {
            let tool = self.registry.get(name)?;
            if tool.domain != ToolDomain::Extension {
                continue;
            }
            let hex = |h: &[u8]| h.iter().map(|b| format!("{b:02x}")).collect::<String>();
            let tuple = serde_json::json!([
                tool.name,
                tool.version,
                hex(&tool.schema_sha256),
                FREE_PRICE_VERSION,
                hex(&tool.price_sha256)
            ]);
            let bytes = serde_json::to_vec(&tuple).map_err(|_| Error::Unavailable)?;
            features.push(format!(
                "assistant.agent_extension.v1:{}:{}",
                tool.name,
                hex(&Sha256::digest(bytes))
            ));
            tuples.push(tuple);
        }
        let bytes = serde_json::to_vec(&tuples).map_err(|_| Error::Unavailable)?;
        features.push(format!(
            "assistant.agent_extensions.sha256:{:x}",
            Sha256::digest(bytes)
        ));
        Ok(features)
    }
}
