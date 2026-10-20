//! Typed browser preview for assistant market connections. Building a preview
//! never creates consent; only the separate browser coordinator may redeem it.
use super::{credit_access::ConsentRequest, credits, *};
use crate::{
    assistant_model::agent::market_policy::{
        AssistantMarketPolicy, authorize_market_policy_context_in_transaction,
    },
    assistant_tools_core::context::VerifiedToolContext,
};
use sqlx::PgConnection;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarketConsentPreview {
    pub request: ConsentRequest,
    pub service_id: String,
    pub quote_id: String,
    pub resource_revision: i64,
    pub resource_schema_sha256: String,
    pub resource_price_sha256: String,
}
pub(crate) async fn build_preview(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    r: &ConsentRequest,
    policy: &AssistantMarketPolicy,
) -> Result<MarketConsentPreview> {
    if r.client_id != "builtin-assistant"
        || r.owner.user != context.user_id().to_string()
        || r.owner.account != context.user_id().to_string()
        || !name(&r.operation_id)
        || !name(&r.tool_id)
        || !name(&r.version_id)
        || !(1..=1_000_000).contains(&r.max_calls)
        || !(0..=credits::MAX_CREDITS).contains(&r.max_price_quota)
        || !(0..=credits::MAX_CREDITS).contains(&r.max_total_quota)
        || r.max_price_quota > r.max_total_quota
    {
        return Err(Error::Invalid);
    }
    let row=sqlx::query("SELECT q.* FROM core_tool_market.credit_quotes q LEFT JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id LEFT JOIN core_identity.users u ON u.id=q.seller_user_id LEFT JOIN core_identity.accounts a ON a.id=q.seller_account_id WHERE q.snapshot->>'tool_id'=$1 AND q.snapshot->>'version_id'=$2 AND q.approved AND NOT q.suspended AND ((p.active AND u.active AND a.active) OR EXISTS(SELECT FROM core_tool_market.credit_builtin_bindings b WHERE b.quote_id=q.id)) FOR SHARE OF q").bind(&r.tool_id).bind(&r.version_id).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let Json(q): Json<credits::Quote> = row.try_get("snapshot")?;
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::BIGINT")
            .fetch_one(&mut *c)
            .await?;
    if r.expires_at <= now
        || r.expires_at > q.expires
        || r.max_price_quota < q.price.ceiling()?
        || (row.try_get::<String, _>("visibility")? == "private"
            && row.try_get::<Option<i64>, _>("seller_user_id")? != Some(context.user_id()))
    {
        return Err(Error::Forbidden);
    }
    authorize_market_policy_context_in_transaction(&mut *c, context, &q.service_id, policy)
        .await
        .map_err(|e| match e {
            crate::assistant_model::Error::Unauthorized => Error::Unauthorized,
            crate::assistant_model::Error::Unavailable => Error::Storage,
            _ => Error::Forbidden,
        })?;
    let resource_revision = if let Some(seller) = row.try_get::<Option<i64>, _>("seller_user_id")? {
        sqlx::query_scalar("SELECT revision FROM core_tool_market.credit_services WHERE owner_user_id=$1 AND service_id=$2 AND active_version_id=$3 AND NOT paused AND NOT deleted AND NOT suspended FOR SHARE").bind(seller).bind(&q.service_id).bind(&q.version_id).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?
    } else {
        super::builtin_install::check_builtin_registry_in_transaction(&mut *c).await?;
        1
    };
    Ok(MarketConsentPreview {
        request: r.clone(),
        service_id: q.service_id,
        quote_id: q.id,
        resource_revision,
        resource_schema_sha256: q.remote_digest,
        resource_price_sha256: q.pricing_digest,
    })
}
