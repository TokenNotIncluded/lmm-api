//! Assistant market access comes from private Core host policy, the live user,
//! and a particular persisted provider proposal. It grants no payment source.
use super::super::Error;
use super::{FrozenProposal, verify_market_proposal_in_transaction};
use crate::{assistant_tools_core::context::VerifiedToolContext, identity::Principal};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, Row, types::Json};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssistantMarketPolicy {
    pub revision: String,
    pub minimum_level: i16,
    pub allowed_service_ids: BTreeSet<String>,
}
impl AssistantMarketPolicy {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !super::super::label(&self.revision, 128)
            || !(0..=6).contains(&self.minimum_level)
            || self.allowed_service_ids.is_empty()
            || self.allowed_service_ids.len() > 128
            || self
                .allowed_service_ids
                .iter()
                .any(|s| !super::super::label(s, 128))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
/// Eligibility only. A consent writer must additionally redeem a genuine Core
/// browser confirmation; an original Session bearer alone is not human consent.
pub(crate) struct MarketPolicyPermission {
    _private: (),
}
pub(crate) async fn authorize_market_policy_in_transaction(
    c: &mut PgConnection,
    actor: &Principal,
    service_id: &str,
    policy: &AssistantMarketPolicy,
) -> Result<MarketPolicyPermission, Error> {
    let context = VerifiedToolContext::verify(c, actor)
        .await
        .map_err(super::context_error)?;
    authorize_market_policy_context_in_transaction(c, &context, service_id, policy).await
}
/// The browser confirmation coordinator can reuse the exact same live policy
/// with its privately constructed context. This is not proof of human approval.
pub(crate) async fn authorize_market_policy_context_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    service_id: &str,
    policy: &AssistantMarketPolicy,
) -> Result<MarketPolicyPermission, Error> {
    policy.validate()?;
    if !policy.allowed_service_ids.contains(service_id) {
        return Err(Error::Forbidden);
    }
    let facts = context.revalidate(c).await.map_err(super::context_error)?;
    if matches!(facts.role.as_str(), "admin" | "superadmin")
        && crate::identity::has_privileged_import_facts(c, context.user_id())
            .await
            .map_err(|_| Error::Unavailable)?
    {
        crate::identity::load_complete_policy(c, context.user_id())
            .await
            .map_err(|error| match error {
                crate::identity::permissions::PolicyLoadError::Storage => Error::Unavailable,
                _ => Error::Forbidden,
            })?;
    }
    let level:i16=sqlx::query_scalar("SELECT CASE u.platform_role WHEN 'superadmin' THEN 6::smallint WHEN 'admin' THEN 5::smallint ELSE a.service_level END FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND a.id=$2 AND u.active AND a.active FOR SHARE OF u,a")
        .bind(context.user_id()).bind(context.account_id()).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    if level < policy.minimum_level {
        return Err(Error::Forbidden);
    }
    Ok(MarketPolicyPermission { _private: () })
}
pub(crate) struct MarketIntent<'a> {
    pub service_id: &'a str,
    pub tool_id: &'a str,
    pub version_id: &'a str,
    pub grant_id: &'a str,
    pub input_digest: &'a str,
    pub quote_id: &'a str,
}
pub(crate) struct MarketToolAuthority {
    operation_id: String,
    quote_id: String,
    consent_id: String,
}
impl MarketToolAuthority {
    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub(crate) fn quote_id(&self) -> &str {
        &self.quote_id
    }
    pub(crate) fn consent_id(&self) -> &str {
        &self.consent_id
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarketArguments {
    service_id: String,
    tool_id: String,
    version_id: String,
    grant_id: String,
    arguments: Value,
}
fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
fn check_proposal(proposal: &FrozenProposal, intent: &MarketIntent<'_>) -> Result<(), Error> {
    let args: MarketArguments =
        serde_json::from_slice(proposal.arguments_json()).map_err(|_| Error::Invalid)?;
    let input = crate::builtins::input_digest(&args.arguments).map_err(|_| Error::Invalid)?;
    if args.service_id != intent.service_id
        || args.tool_id != intent.tool_id
        || args.version_id != intent.version_id
        || args.grant_id != intent.grant_id
        || input != intent.input_digest
        || proposal.domain() != "extension"
        || proposal.tool_name() != "call_market_tool"
        || proposal.resource_kind() != "toolmarket"
        || proposal.resource_id() != intent.tool_id
        || proposal.resource_version() != intent.version_id
        || proposal.scope_grant_id() != intent.grant_id
    {
        return Err(Error::Forbidden);
    }
    Ok(())
}
pub(crate) async fn authorize_market_tool_operation_in_transaction(
    c: &mut PgConnection,
    actor: &Principal,
    operation_id: &str,
    intent: MarketIntent<'_>,
    policy: &AssistantMarketPolicy,
) -> Result<MarketToolAuthority, Error> {
    let _permission =
        authorize_market_policy_in_transaction(c, actor, intent.service_id, policy).await?;
    let context = VerifiedToolContext::verify(c, actor)
        .await
        .map_err(super::context_error)?;
    let proposal = verify_market_proposal_in_transaction(c, operation_id, &context).await?;
    let live = serde_json::to_value(policy).map_err(|_| Error::Unavailable)?;
    if proposal.host_policy().get("market") != Some(&live) {
        return Err(Error::Forbidden);
    }
    check_proposal(&proposal, &intent)?;
    // The inner schema binding is the quote's full remote-tool digest, not a
    // separately re-serialized InputSchema hash. Pricing uses the same quote.
    let quote=sqlx::query("SELECT snapshot,approved,suspended,visibility,seller_user_id FROM core_tool_market.credit_quotes WHERE id=$1 FOR SHARE")
        .bind(intent.quote_id).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let Json(q): Json<crate::tool_market::credits::Quote> = quote.try_get("snapshot")?;
    let consent=sqlx::query("SELECT actor_user_id,payer_account_id,quote_id,request,revoked_at FROM core_tool_market.credit_consents WHERE id=$1 FOR SHARE")
        .bind(intent.grant_id).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let Json(grant): Json<crate::tool_market::credit_access::ConsentRequest> =
        consent.try_get("request")?;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&mut *c)
        .await?;
    if q.id != intent.quote_id
        || q.service_id != intent.service_id
        || q.tool_id != intent.tool_id
        || q.version_id != intent.version_id
        || q.expires <= now
        || !quote.try_get::<bool, _>("approved")?
        || quote.try_get::<bool, _>("suspended")?
        || (quote.try_get::<String, _>("visibility")? == "private"
            && quote.try_get::<Option<i64>, _>("seller_user_id")? != Some(actor.user_id))
        || proposal.resource_schema_sha256().map(hex).as_deref() != Some(q.remote_digest.as_str())
        || proposal.resource_price_sha256().map(hex).as_deref() != Some(q.pricing_digest.as_str())
        || consent.try_get::<i64, _>("actor_user_id")? != actor.user_id
        || consent.try_get::<i64, _>("payer_account_id")? != actor.owner_account_id
        || consent.try_get::<String, _>("quote_id")? != intent.quote_id
        || consent.try_get::<i64, _>("revoked_at")? != 0
        || grant.client_id != "builtin-assistant"
        || grant.tool_id != intent.tool_id
        || grant.version_id != intent.version_id
        || grant.expires_at <= now
        || grant.owner.user != actor.user_id.to_string()
        || grant.owner.account != actor.user_id.to_string()
    {
        return Err(Error::Forbidden);
    }
    Ok(MarketToolAuthority {
        operation_id: operation_id.into(),
        quote_id: intent.quote_id.into(),
        consent_id: intent.grant_id.into(),
    })
}

pub(super) struct FrozenMarketScope {
    pub tool_id: String,
    pub version_id: String,
    pub grant_id: String,
    pub remote_sha256: [u8; 32],
    pub price_sha256: [u8; 32],
}
/// Freeze only a real, previously human-approved consent/quote. This creates
/// neither consent nor money; the actual reserve revalidates the same binding.
pub(super) async fn freeze_market_scope_in_transaction(
    c: &mut PgConnection,
    actor: &Principal,
    arguments: &[u8],
    policy: &AssistantMarketPolicy,
) -> Result<FrozenMarketScope, Error> {
    let args: MarketArguments = serde_json::from_slice(arguments).map_err(|_| Error::Invalid)?;
    if !args.arguments.is_object() {
        return Err(Error::Invalid);
    }
    let _permission =
        authorize_market_policy_in_transaction(c, actor, &args.service_id, policy).await?;
    let grant=sqlx::query("SELECT actor_user_id,payer_account_id,quote_id,request,revoked_at FROM core_tool_market.credit_consents WHERE id=$1 FOR SHARE")
        .bind(&args.grant_id).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let Json(consent): Json<crate::tool_market::credit_access::ConsentRequest> =
        grant.try_get("request")?;
    let quote=sqlx::query("SELECT snapshot,approved,suspended,visibility,seller_user_id FROM core_tool_market.credit_quotes WHERE id=$1")
        .bind(grant.try_get::<String,_>("quote_id")?).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let Json(q): Json<crate::tool_market::credits::Quote> = quote.try_get("snapshot")?;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&mut *c)
        .await?;
    if grant.try_get::<i64, _>("actor_user_id")? != actor.user_id
        || grant.try_get::<i64, _>("payer_account_id")? != actor.owner_account_id
        || grant.try_get::<i64, _>("revoked_at")? != 0
        || consent.client_id != "builtin-assistant"
        || consent.owner.user != actor.user_id.to_string()
        || consent.owner.account != actor.user_id.to_string()
        || consent.tool_id != args.tool_id
        || consent.version_id != args.version_id
        || consent.expires_at <= now
        || q.service_id != args.service_id
        || q.tool_id != args.tool_id
        || q.version_id != args.version_id
        || q.expires <= now
        || !quote.try_get::<bool, _>("approved")?
        || quote.try_get::<bool, _>("suspended")?
        || (quote.try_get::<String, _>("visibility")? == "private"
            && quote.try_get::<Option<i64>, _>("seller_user_id")? != Some(actor.user_id))
    {
        return Err(Error::Forbidden);
    }
    Ok(FrozenMarketScope {
        tool_id: q.tool_id,
        version_id: q.version_id,
        grant_id: args.grant_id,
        remote_sha256: super::registry::hex32(&q.remote_digest)?,
        price_sha256: super::registry::hex32(&q.pricing_digest)?,
    })
}
