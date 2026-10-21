//! Durable personal assistant admission and execution, entirely owned by Core.
//! Go supplies identifiers and current user text; never payer, price or history.
pub(crate) mod agent;
mod gift_execution;
#[cfg(unix)]
pub mod rpc;
pub mod sponsor;
pub use sponsor::SponsorPolicy;

use crate::{
    billing::{
        charging::{self, Charging},
        transactional_ledger::TransactionalLedger,
    },
    identity::{self, CredentialKind},
    model_http::ModelGateway,
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersonalTextPolicy {
    pub model: String,
    pub group: String,
    pub budget_credits: i64,
    pub max_output_tokens: u32,
    pub ttl_seconds: i64,
    #[serde(default)]
    pub gift_tools: bool,
    #[serde(default)]
    pub sponsor: Option<SponsorPolicy>,
}
impl PersonalTextPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(sponsor) = &self.sponsor {
            sponsor.validate()?;
            if !self.gift_tools || self.budget_credits > sponsor.max_credits_per_user {
                return Err(Error::Invalid);
            }
        }
        if self.model.is_empty()
            || self.model.len() > 256
            || self
                .model
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || self.group.is_empty()
            || self.group.len() > 64
            || self
                .group
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || self.budget_credits <= 0
            || self.budget_credits > 9_007_199_254_740_991
            || !(1..=4096).contains(&self.max_output_tokens)
            || !(1..=900).contains(&self.ttl_seconds)
        {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Conflict,
    NoFunds,
    Unavailable,
    NotFound,
}
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if e.as_database_error()
            .is_some_and(|d| matches!(d.code().as_deref(), Some("40001" | "40P01" | "23505")))
        {
            Self::Conflict
        } else {
            Self::Unavailable
        }
    }
}
impl From<identity::IdentityError> for Error {
    fn from(e: identity::IdentityError) -> Self {
        match e {
            identity::IdentityError::Unauthorized => Self::Unauthorized,
            identity::IdentityError::Forbidden => Self::Forbidden,
            identity::IdentityError::Invalid => Self::Invalid,
            identity::IdentityError::Conflict => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}
impl From<charging::Error> for Error {
    fn from(e: charging::Error) -> Self {
        match e {
            charging::Error::Unauthorized => Self::Unauthorized,
            charging::Error::Forbidden => Self::Forbidden,
            charging::Error::Invalid => Self::Invalid,
            charging::Error::Conflict | charging::Error::Retry => Self::Conflict,
            charging::Error::NoFunds | charging::Error::BudgetExceeded => Self::NoFunds,
            _ => Self::Unavailable,
        }
    }
}
impl From<crate::billing::trust_pricing::Error> for Error {
    fn from(error: crate::billing::trust_pricing::Error) -> Self {
        match error {
            crate::billing::trust_pricing::Error::Denied => Self::Forbidden,
            crate::billing::trust_pricing::Error::Retry => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Conversation {
    pub creation_key: String,
    pub core_conversation_id: String,
}
#[derive(Clone, Debug)]
pub struct Grant {
    pub delegation_id: String,
    pub core_conversation_id: String,
    pub turn_id: String,
    pub expires_at_unix: i64,
    pub max_model_calls: u32,
    pub revoked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub delegation_id: String,
    pub core_conversation_id: String,
    pub turn_id: String,
    pub round: u32,
    pub operation_key: String,
    pub text_sha256: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Reserved,
    Streaming,
    Settled,
    Released,
    Unknown,
}
#[derive(Clone, Debug)]
pub struct Operation {
    pub binding: Binding,
    pub request_id: String,
    pub state: State,
    pub reserved_credits: i64,
    pub settled_credits: Option<i64>,
    pub answer_text: Option<String>,
    pub gift_receipt: Option<crate::gift::Receipt>,
}
#[derive(Clone)]
pub struct AssistantModel {
    pool: PgPool,
    gateway: ModelGateway,
    charging: Charging,
    policy: PersonalTextPolicy,
    permits: Arc<tokio::sync::Semaphore>,
    capacity: usize,
    transport: crate::relay::provider_network::ProviderTransport,
    changed: Arc<tokio::sync::Notify>,
}
struct ActiveGuard {
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    changed: Arc<tokio::sync::Notify>,
}
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        drop(self.permit.take());
        self.changed.notify_waiters();
    }
}
fn label(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}
fn random(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "{prefix}{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
async fn actor(c: &mut PgConnection, session: &str) -> Result<identity::Principal, Error> {
    let a = identity::authorize_in_transaction(c, session).await?;
    if a.credential_kind != CredentialKind::Session {
        return Err(Error::Forbidden);
    };
    let personal: bool = sqlx::query_scalar(
        "SELECT personal_account_id=$2 FROM core_identity.users WHERE id=$1 FOR SHARE",
    )
    .bind(a.user_id)
    .bind(a.owner_account_id)
    .fetch_one(c)
    .await?;
    if !personal {
        return Err(Error::Forbidden);
    }
    Ok(a)
}
pub(crate) async fn lock_conversation_id(c: &mut PgConnection, id: &str) -> Result<(), Error> {
    // Conversations are immutable: runtime has no UPDATE privilege. A shared
    // advisory namespace serializes their grants and workers without granting it.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,908534117))")
        .bind(id)
        .execute(c)
        .await?;
    Ok(())
}
async fn at(c: &mut PgConnection) -> Result<i64, Error> {
    Ok(
        sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
            .fetch_one(c)
            .await?,
    )
}
fn grant(row: &PgRow) -> Result<Grant, Error> {
    Ok(Grant {
        delegation_id: row.try_get("id")?,
        core_conversation_id: row.try_get("conversation_id")?,
        turn_id: row.try_get("turn_id")?,
        expires_at_unix: row.try_get("expires_at")?,
        max_model_calls: 1,
        revoked: row.try_get("revoked")?,
    })
}
impl AssistantModel {
    pub fn new(
        pool: PgPool,
        gateway: ModelGateway,
        policy: PersonalTextPolicy,
    ) -> Result<Self, Error> {
        policy.validate()?;
        gateway
            .validate_assistant_policy(&policy.model, &policy.group, policy.max_output_tokens)
            .map_err(|_| Error::Unavailable)?;
        gateway
            .assistant_quote(&policy.model, &policy.group)
            .map_err(|_| Error::Unavailable)?;
        let capacity = gateway.assistant_parallelism();
        let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
        if gateway.trust_pricing_enabled() {
            charging.enable_trust_pricing();
        }
        Ok(Self {
            charging,
            pool,
            gateway,
            policy,
            permits: Arc::new(tokio::sync::Semaphore::new(capacity)),
            capacity,
            transport: crate::relay::provider_network::ProviderTransport::new(capacity),
            changed: Arc::new(tokio::sync::Notify::new()),
        })
    }
    pub(crate) fn gift_enabled(&self) -> bool {
        self.policy.gift_tools
    }
    async fn finalize(
        &self,
        session: &str,
        request_id: &str,
        worker: &[u8; 32],
        observation: crate::model_http::assistant_execution::AssistantObservation,
    ) -> Result<(), Error> {
        let mut tx = self.begin().await?;
        let conversation: String = sqlx::query_scalar("SELECT g.conversation_id FROM core_charging.requests r JOIN core_assistant.grants g ON g.id=r.assistant_grant_id WHERE r.id=$1")
            .bind(request_id).fetch_one(&mut *tx).await?;
        lock_conversation_id(&mut tx, &conversation).await?;
        let answer = match observation.answer {
            crate::model_http::assistant_execution::AssistantAnswer::Text(answer) => answer,
            crate::model_http::assistant_execution::AssistantAnswer::Gift(call) => {
                let result = gift_execution::execute(&mut tx, session, request_id, &call).await?;
                let receipt = result
                    .receipt
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|_| Error::Unavailable)?;
                sqlx::query("UPDATE core_assistant.operations SET gift_receipt=$2,gift_tool_id=$3,gift_response_sha256=$4 WHERE request_id=$1 AND answer_text IS NULL").bind(request_id).bind(receipt).bind(result.tool_id).bind(result.response_sha256.to_vec()).execute(&mut *tx).await?;
                result.answer
            }
        };
        self.charging
            .settle_assistant(&mut tx, request_id, worker, observation.usage, &answer)
            .await?;
        tx.commit().await.map_err(|_| Error::Unavailable)?;
        Ok(())
    }
    pub fn close_admission(&self) {
        self.permits.close();
    }
    pub async fn drain(&self, grace: std::time::Duration) -> bool {
        self.close_admission();
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let changed = self.changed.notified();
            if self.permits.available_permits() == self.capacity {
                return true;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return false;
            }
        }
    }
    async fn begin(&self) -> Result<Transaction<'_, Postgres>, Error> {
        let mut t = self.pool.begin().await?;
        for s in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL synchronous_commit=on",
            "SET LOCAL lock_timeout='2s'",
            "SET LOCAL statement_timeout='5s'",
        ] {
            sqlx::query(s).execute(&mut *t).await?;
        }
        Ok(t)
    }
    async fn quote_in_transaction(
        &self,
        c: &mut PgConnection,
        actor: &identity::Principal,
    ) -> Result<charging::Price, Error> {
        let price = self
            .gateway
            .assistant_quote(&self.policy.model, &self.policy.group)
            .map_err(|_| Error::Unavailable)?;
        if !self.charging.trust_pricing_enabled() && !self.gateway.trust_pricing_enabled() {
            return Ok(price);
        }
        self.charging.enable_trust_pricing();
        let authority = crate::billing::trust_pricing::quote_in_transaction(c, actor.user_id)
            .await
            .map_err(Error::from)?;
        price.with_user_authority(&authority).map_err(Error::from)
    }
    pub async fn conversation(
        &self,
        session: &str,
        key: &str,
        create: bool,
    ) -> Result<Conversation, Error> {
        if !label(key, 128) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, session).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,9911))")
            .bind(format!("{}:{key}", a.user_id))
            .execute(&mut *tx)
            .await?;
        let existing:Option<String>=sqlx::query_scalar("SELECT id FROM core_assistant.conversations WHERE owner_user_id=$1 AND owner_account_id=$2 AND creation_key=$3").bind(a.user_id).bind(a.owner_account_id).bind(key).fetch_optional(&mut *tx).await?;
        let id = match existing {
            Some(id) => id,
            None if create => {
                identity::reject_marked_new_action(
                    &mut tx,
                    a.user_id,
                    identity::NewResourceAction::RemoteExecution,
                )
                .await?;
                let id = random("lmm_cv_");
                sqlx::query("INSERT INTO core_assistant.conversations(id,owner_user_id,owner_account_id,creation_key,created_at) VALUES($1,$2,$3,$4,$5)").bind(&id).bind(a.user_id).bind(a.owner_account_id).bind(key).bind(at(&mut tx).await?).execute(&mut *tx).await?;
                id
            }
            None => return Err(Error::NotFound),
        };
        tx.commit().await?;
        Ok(Conversation {
            creation_key: key.into(),
            core_conversation_id: id,
        })
    }
    pub async fn turn(
        &self,
        session: &str,
        conversation_id: &str,
        turn_id: &str,
        create: bool,
    ) -> Result<Grant, Error> {
        if !label(turn_id, 124) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, session).await?;
        self.lock_conversation(&mut tx, &a, conversation_id).await?;
        let old=sqlx::query("SELECT * FROM core_assistant.grants WHERE conversation_id=$1 AND turn_id=$2 FOR UPDATE").bind(conversation_id).bind(turn_id).fetch_optional(&mut *tx).await?;
        if let Some(old) = old {
            let g = grant(&old)?;
            tx.commit().await?;
            return Ok(g);
        }
        if !create {
            return Err(Error::NotFound);
        }
        identity::reject_marked_new_action(
            &mut tx,
            a.user_id,
            identity::NewResourceAction::RemoteExecution,
        )
        .await?;
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_assistant.grants g LEFT JOIN core_assistant.operations o ON o.grant_id=g.id LEFT JOIN core_charging.requests r ON r.id=o.request_id WHERE g.conversation_id=$1 AND ((o.request_id IS NOT NULL AND r.state NOT IN ('settled','released')) OR (o.request_id IS NULL AND NOT g.revoked AND g.expires_at>extract(epoch FROM clock_timestamp())::bigint)))").bind(conversation_id).fetch_one(&mut *tx).await?;
        if active {
            return Err(Error::Conflict);
        }
        if self.policy.sponsor.is_none() {
            identity::authorize_assistant_model_in_transaction(
                &mut tx,
                session,
                &self.policy.group,
            )
            .await?;
        }
        if let Some(sponsor) = &self.policy.sponsor {
            sponsor
                .admit(&mut tx, a.user_id, a.owner_account_id, 0)
                .await?;
        }
        let price = self.quote_in_transaction(&mut tx, &a).await?;
        let now = at(&mut tx).await?;
        let id = random("lmm_ad_");
        let version: i64 =
            sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=$1")
                .bind(a.user_id)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("INSERT INTO core_assistant.grants(id,conversation_id,turn_id,session_id,auth_version,policy,price,issued_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(&id).bind(conversation_id).bind(turn_id).bind(a.credential_id).bind(version).bind(serde_json::to_value(&self.policy).map_err(|_|Error::Invalid)?).bind(serde_json::to_value(price).map_err(|_|Error::Invalid)?).bind(now).bind(now+self.policy.ttl_seconds).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Grant {
            delegation_id: id,
            core_conversation_id: conversation_id.into(),
            turn_id: turn_id.into(),
            expires_at_unix: now + self.policy.ttl_seconds,
            max_model_calls: 1,
            revoked: false,
        })
    }
    async fn lock_conversation(
        &self,
        c: &mut PgConnection,
        a: &identity::Principal,
        id: &str,
    ) -> Result<(), Error> {
        lock_conversation_id(c, id).await?;
        let owned:Option<String>=sqlx::query_scalar("SELECT id FROM core_assistant.conversations WHERE id=$1 AND owner_user_id=$2 AND owner_account_id=$3").bind(id).bind(a.user_id).bind(a.owner_account_id).fetch_optional(c).await?;
        owned.ok_or(Error::Forbidden)?;
        Ok(())
    }
    async fn lookup_locked(
        &self,
        c: &mut PgConnection,
        b: &Binding,
    ) -> Result<Option<Operation>, Error> {
        if b.round != 0
            || b.operation_key != format!("{}:0", b.turn_id)
            || b.text_sha256.len() != 32
        {
            return Err(Error::Invalid);
        }
        let row=sqlx::query("SELECT o.*,g.conversation_id,g.turn_id,r.state,r.reserved,r.settled FROM core_assistant.operations o JOIN core_assistant.grants g ON g.id=o.grant_id JOIN core_charging.requests r ON r.id=o.request_id WHERE o.grant_id=$1 AND r.assistant_grant_id=g.id FOR UPDATE OF o,r").bind(&b.delegation_id).fetch_optional(c).await?;
        let Some(r) = row else { return Ok(None) };
        if r.try_get::<String, _>("conversation_id")? != b.core_conversation_id
            || r.try_get::<String, _>("turn_id")? != b.turn_id
            || r.try_get::<String, _>("operation_key")? != b.operation_key
            || r.try_get::<Vec<u8>, _>("text_sha256")? != b.text_sha256
        {
            return Err(Error::Conflict);
        }
        let answer: Option<String> = r.try_get("answer_text")?;
        let state = match r.try_get::<String, _>("state")?.as_str() {
            "reserved" => State::Reserved,
            "streaming" => State::Streaming,
            "settled" if answer.is_some() => State::Settled,
            "released" => State::Released,
            _ => State::Unknown,
        };
        Ok(Some(Operation {
            binding: b.clone(),
            request_id: r.try_get("request_id")?,
            state,
            reserved_credits: r.try_get("reserved")?,
            settled_credits: (state == State::Settled)
                .then(|| r.try_get("settled"))
                .transpose()?,
            gift_receipt: if state == State::Settled {
                r.try_get::<Option<Value>, _>("gift_receipt")?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| Error::Unavailable)?
            } else {
                None
            },
            answer_text: if state == State::Settled {
                answer
            } else {
                None
            },
        }))
    }
    pub async fn operation(&self, session: &str, b: &Binding) -> Result<Operation, Error> {
        // A status poll can overlap the detached worker's serializable commit.
        // Reopen only this read transaction; never repeat admission or dispatch.
        for attempt in 0..3 {
            match self.operation_once(session, b).await {
                Err(Error::Conflict) if attempt < 2 => tokio::task::yield_now().await,
                result => return result,
            }
        }
        Err(Error::Conflict)
    }
    async fn operation_once(&self, session: &str, b: &Binding) -> Result<Operation, Error> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, session).await?;
        self.lock_conversation(&mut tx, &a, &b.core_conversation_id)
            .await?;
        let op = self
            .lookup_locked(&mut tx, b)
            .await?
            .ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok(op)
    }
    pub async fn revoke(&self, session: &str, id: &str) -> Result<(), Error> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, session).await?;
        let conversation: Option<String> =
            sqlx::query_scalar("SELECT conversation_id FROM core_assistant.grants WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        self.lock_conversation(&mut tx, &a, &conversation.ok_or(Error::NotFound)?)
            .await?;
        sqlx::query("UPDATE core_assistant.grants SET revoked=TRUE WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn reserve_text(
        &self,
        session: &str,
        b: &Binding,
        text: &str,
    ) -> Result<Operation, Error> {
        if text.is_empty()
            || text.len() > 8192
            || Sha256::digest(text.as_bytes()).as_slice() != b.text_sha256
        {
            return Err(Error::Invalid);
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Unavailable)?;
        let permit = ActiveGuard {
            permit: Some(permit),
            changed: self.changed.clone(),
        };
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, session).await?;
        self.lock_conversation(&mut tx, &a, &b.core_conversation_id)
            .await?;
        if let Some(op) = self.lookup_locked(&mut tx, b).await? {
            tx.commit().await?;
            return Ok(op);
        }
        let row=sqlx::query("SELECT * FROM core_assistant.grants WHERE id=$1 AND conversation_id=$2 AND turn_id=$3 FOR UPDATE").bind(&b.delegation_id).bind(&b.core_conversation_id).bind(&b.turn_id).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let policy: PersonalTextPolicy =
            serde_json::from_value(row.try_get("policy")?).map_err(|_| Error::Unavailable)?;
        // Host policy changes invalidate old, unspent capabilities. Status reads remain available.
        if policy != self.policy {
            return Err(Error::Forbidden);
        }
        let history=sqlx::query("SELECT o.text,o.answer_text FROM core_assistant.operations o JOIN core_assistant.grants g ON g.id=o.grant_id JOIN core_charging.requests r ON r.id=o.request_id WHERE g.conversation_id=$1 AND r.state='settled' AND o.answer_text IS NOT NULL ORDER BY o.position LIMIT 65").bind(&b.core_conversation_id).fetch_all(&mut *tx).await?;
        if history.len() > 64 {
            return Err(Error::Invalid);
        }
        let mut messages = Vec::new();
        for h in history {
            messages.push(json!({"role":"user","content":h.try_get::<String,_>("text")?}));
            messages
                .push(json!({"role":"assistant","content":h.try_get::<String,_>("answer_text")?}));
        }
        messages.push(json!({"role":"user","content":text}));
        let gift_cap = if policy.gift_tools {
            let cap: Option<i64> = sqlx::query_scalar(
                "SELECT exclusive_cap FROM core_gift.policy WHERE singleton AND enabled",
            )
            .fetch_optional(&mut *tx)
            .await?;
            Some(cap.filter(|v| *v > 1).ok_or(Error::Forbidden)?)
        } else {
            None
        };
        let stored_price: charging::Price =
            serde_json::from_value(row.try_get("price")?).map_err(|_| Error::Unavailable)?;
        let prepared = self
            .gateway
            .prepare_assistant(
                &policy.model,
                &policy.group,
                policy.max_output_tokens,
                Value::Array(messages),
                &self.transport,
                gift_cap,
                &stored_price,
                a.user_id,
            )
            .await
            .map_err(|_| Error::Unavailable)?;
        let request_id = random("");
        let mut worker = [0u8; 32];
        OsRng.fill_bytes(&mut worker);
        let spec = charging::Request {
            id: request_id.clone(),
            fingerprint: prepared.fingerprint,
            price: prepared.price.clone(),
            reserve: prepared.reserve,
            lease_seconds: 300,
            maximum_seconds: 300,
        };
        self.charging
            .reserve_assistant(&mut tx, session, &b.delegation_id, &spec, &worker)
            .await?;
        sqlx::query("INSERT INTO core_assistant.operations(grant_id,request_id,text_sha256,operation_key,text) VALUES($1,$2,$3,$4,$5)").bind(&b.delegation_id).bind(&request_id).bind(&b.text_sha256).bind(&b.operation_key).bind(text).execute(&mut *tx).await?;
        if policy.gift_tools {
            crate::gift::record_user_turn_in_transaction(&mut tx, a.user_id, &request_id)
                .await
                .map_err(|_| Error::Unavailable)?;
        }
        self.charging
            .start_assistant(&mut tx, &request_id, &worker)
            .await?;
        // A lost COMMIT acknowledgment never enters the network branch. Recovery only
        // inspects the original operation, retaining its hold.
        tx.commit().await.map_err(|_| Error::Unavailable)?;
        let service = self.clone();
        let binding = b.clone();
        let original = session.to_owned();
        // The Core task owns execution independently of an RPC disconnect. It has no
        // retry loop; process death leaves a durable unknown operation.
        let task = tokio::spawn(async move {
            let _permit = permit;
            let mut settled = false;
            if let Ok(observation) = service.gateway.execute_assistant(prepared).await {
                settled = service
                    .finalize(&original, &request_id, &worker, observation)
                    .await
                    .is_ok();
            }
            if !settled {
                if let Ok(mut tx) = service.begin().await {
                    if service
                        .charging
                        .unknown_assistant(&mut tx, &request_id, &worker)
                        .await
                        .is_ok()
                    {
                        let _ = tx.commit().await;
                    }
                }
            }
            service.operation(&original, &binding).await
        });
        task.await.map_err(|_| Error::Unavailable)?
    }
}

#[cfg(test)]
mod trust_error_tests {
    use super::Error;
    use crate::billing::trust_pricing;

    #[test]
    fn trust_retry_preserves_whole_transaction_conflict_semantics() {
        assert_eq!(Error::from(trust_pricing::Error::Retry), Error::Conflict);
        assert_eq!(Error::from(trust_pricing::Error::Denied), Error::Forbidden);
        for error in [
            trust_pricing::Error::Invalid,
            trust_pricing::Error::Unavailable,
            trust_pricing::Error::Storage,
        ] {
            assert_eq!(Error::from(error), Error::Unavailable);
        }
    }
}
