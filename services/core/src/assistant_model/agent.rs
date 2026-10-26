//! Private proof of an already persisted provider tool proposal.
//! Construction never accepts a caller-supplied tool name, draft, actor or scope.
pub(crate) mod market_policy;
pub(crate) mod registry;
pub(crate) mod store;
mod worker;

use super::Error;
use crate::assistant_tools_core::context::VerifiedToolContext;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

/// Not deserializable. Confirmation coordinators receive this only after a live
/// original Session and its immutable Core operation have been checked together.
/// Market proof fields are read only by the authorizer below, which has no caller yet.
#[allow(dead_code)]
pub(crate) struct FrozenProposal {
    operation_id: String,
    turn_id: String,
    tool_name: String,
    tool_version: String,
    domain: String,
    arguments: Vec<u8>,
    arguments_sha256: [u8; 32],
    resource_kind: String,
    resource_id: String,
    resource_version: String,
    scope_grant_id: String,
    resource_schema_sha256: Option<[u8; 32]>,
    resource_price_sha256: Option<[u8; 32]>,
    host_policy: serde_json::Value,
    source_web_session_sid: Option<String>,
    expires_at_unix: i64,
}
impl FrozenProposal {
    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub(crate) fn turn_id(&self) -> &str {
        &self.turn_id
    }
    pub(crate) fn tool_name(&self) -> &str {
        &self.tool_name
    }
    pub(crate) fn tool_version(&self) -> &str {
        &self.tool_version
    }
    #[allow(dead_code)] // Market authorizer only; that path has no caller yet.
    pub(crate) fn domain(&self) -> &str {
        &self.domain
    }
    pub(crate) fn arguments_json(&self) -> &[u8] {
        &self.arguments
    }
    pub(crate) fn arguments_sha256(&self) -> &[u8; 32] {
        &self.arguments_sha256
    }
    pub(crate) fn resource_kind(&self) -> &str {
        &self.resource_kind
    }
    pub(crate) fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub(crate) fn resource_version(&self) -> &str {
        &self.resource_version
    }
    #[allow(dead_code)] // Market authorizer only; that path has no caller yet.
    pub(crate) fn scope_grant_id(&self) -> &str {
        &self.scope_grant_id
    }
    #[allow(dead_code)] // Market authorizer only; that path has no caller yet.
    pub(crate) fn resource_schema_sha256(&self) -> Option<&[u8; 32]> {
        self.resource_schema_sha256.as_ref()
    }
    #[allow(dead_code)] // Market authorizer only; that path has no caller yet.
    pub(crate) fn resource_price_sha256(&self) -> Option<&[u8; 32]> {
        self.resource_price_sha256.as_ref()
    }
    #[allow(dead_code)] // Market authorizer only; that path has no caller yet.
    pub(crate) fn host_policy(&self) -> &serde_json::Value {
        &self.host_policy
    }
    pub(crate) fn source_web_session_sid(&self) -> Option<&str> {
        self.source_web_session_sid.as_deref()
    }
    pub(crate) fn expires_at_unix(&self) -> i64 {
        self.expires_at_unix
    }
}
fn context_error(error: crate::assistant_tools_core::Error) -> Error {
    use crate::assistant_tools_core::Error as E;
    match error {
        E::Invalid => Error::Invalid,
        E::Unauthorized => Error::Unauthorized,
        E::Forbidden => Error::Forbidden,
        E::NotFound => Error::NotFound,
        E::Conflict => Error::Conflict,
        E::Unavailable => Error::Unavailable,
    }
}
/// This lookup becomes usable only with the new durable agent worker and schema.
/// The separate agent capability remains absent while those are not installed.
/// The caller owns the transaction so confirmation creation shares these locks.
pub(crate) async fn verify_proposal_in_transaction(
    c: &mut PgConnection,
    operation_id: &str,
    context: &VerifiedToolContext,
) -> Result<FrozenProposal, Error> {
    verify_bound_proposal(c, operation_id, context, 0).await
}
/// Browser confirmation may additionally project these fixed extension-owned
/// actions. The name is read from Core, never supplied as an authorization claim.
pub(crate) async fn verify_confirmation_proposal_in_transaction(
    c: &mut PgConnection,
    operation_id: &str,
    context: &VerifiedToolContext,
) -> Result<FrozenProposal, Error> {
    verify_bound_proposal(c, operation_id, context, 1).await
}
#[allow(dead_code)] // Market authorizer only; that path has no caller yet.
pub(crate) async fn verify_market_proposal_in_transaction(
    c: &mut PgConnection,
    operation_id: &str,
    context: &VerifiedToolContext,
) -> Result<FrozenProposal, Error> {
    verify_bound_proposal(c, operation_id, context, 2).await
}
async fn verify_bound_proposal(
    c: &mut PgConnection,
    operation_id: &str,
    context: &VerifiedToolContext,
    extension_mode: i32,
) -> Result<FrozenProposal, Error> {
    if operation_id.len() != 64
        || !operation_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::Invalid);
    }
    let facts = context.revalidate(c).await.map_err(context_error)?;
    let row = sqlx::query("SELECT o.id,o.turn_id,o.tool_name,o.tool_version,o.domain,o.arguments_json,o.arguments_sha256,o.resource_kind,o.resource_id,o.resource_version,o.scope_grant_id,o.resource_schema_sha256,o.resource_price_sha256,t.host_policy,t.source_web_session_sid,t.expires_at FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id JOIN core_assistant.agent_rounds r ON r.id=o.round_id AND r.turn_id=t.id JOIN core_charging.requests paid ON paid.id=r.model_request_id WHERE o.id=$1 AND t.owner_user_id=$2 AND t.owner_account_id=$3 AND t.session_id=$4 AND t.auth_version=$5 AND t.expires_at>extract(epoch FROM clock_timestamp())::bigint AND t.phase='tools_pending' AND o.state IN ('pending','claimed') AND (($6<>2 AND o.domain='registered_core') OR ($6=1 AND o.domain='extension' AND o.tool_name IN ('set_overview_greeting','create_site_issue','update_site_issue','request_human_support')) OR ($6=2 AND o.domain='extension' AND o.tool_name='call_market_tool' AND o.state='claimed')) AND r.state='settled' AND paid.state='settled' AND paid.actor_user_id=t.owner_user_id AND paid.owner_account_id=t.owner_account_id AND paid.assistant_grant_id IS NOT NULL AND octet_length(r.provider_response_sha256)=32 FOR UPDATE OF o,t,r,paid")
        .bind(operation_id).bind(context.user_id()).bind(context.account_id()).bind(context.credential_id()).bind(facts.auth_version).bind(extension_mode)
        .fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let family: Option<String> = row.try_get("source_web_session_sid")?;
    if family != facts.browser_session {
        return Err(Error::Forbidden);
    }
    let arguments: Vec<u8> = row.try_get("arguments_json")?;
    let digest: Vec<u8> = row.try_get("arguments_sha256")?;
    let arguments_sha256: [u8; 32] = digest.try_into().map_err(|_| Error::Unavailable)?;
    if arguments.len() > 65_536 || Sha256::digest(&arguments).as_slice() != arguments_sha256 {
        return Err(Error::Unavailable);
    }
    fn optional_hash(value: Option<Vec<u8>>) -> Result<Option<[u8; 32]>, Error> {
        value
            .map(|v| v.try_into().map_err(|_| Error::Unavailable))
            .transpose()
    }
    Ok(FrozenProposal {
        operation_id: row.try_get("id")?,
        turn_id: row.try_get("turn_id")?,
        tool_name: row.try_get("tool_name")?,
        tool_version: row.try_get("tool_version")?,
        domain: row.try_get("domain")?,
        arguments,
        arguments_sha256,
        resource_kind: row.try_get("resource_kind")?,
        resource_id: row.try_get("resource_id")?,
        resource_version: row.try_get("resource_version")?,
        scope_grant_id: row.try_get("scope_grant_id")?,
        resource_schema_sha256: optional_hash(row.try_get("resource_schema_sha256")?)?,
        resource_price_sha256: optional_hash(row.try_get("resource_price_sha256")?)?,
        host_policy: row.try_get("host_policy")?,
        source_web_session_sid: family,
        expires_at_unix: row.try_get("expires_at")?,
    })
}

// Installed only through the explicit agent component in a later batch. Startup
// never executes this DDL. Isolated PostgreSQL tests apply this exact asset.
#[cfg(test)]
const SCHEMA: &str = include_str!("../../schema/assistant_agent.sql");
#[cfg(test)]
mod tests;
