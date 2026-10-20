use super::{Error, context::VerifiedToolContext};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
/// Eligibility is external recharge evidence, never a wallet/gift balance.
pub async fn eligibility_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    let r=sqlx::query("SELECT count(*)::bigint AS payments,COALESCE(sum(amount_minor),0)::text AS amount_minor FROM core_payments.intents i WHERE i.account_id=$1 AND i.created_by_user_id=$2 AND i.provider='stripe' AND i.environment='live' AND i.amount_minor>0 AND i.credit_journal_id IS NOT NULL AND i.provider_transaction IS NOT NULL AND i.refunded_minor=0 AND NOT i.refund_fenced AND EXISTS(SELECT 1 FROM core_payments.accepted_proofs p WHERE p.intent_id=i.id) AND NOT EXISTS(SELECT 1 FROM core_payments.refund_holds h WHERE h.intent_id=i.id AND h.state IN ('held','committed')) AND NOT EXISTS(SELECT 1 FROM core_payments.blocked_refunds b WHERE b.intent_id=i.id)")
        .bind(actor.account).bind(actor.user).fetch_one(c).await?;
    let payments: i64 = r.try_get("payments")?;
    Ok(
        json!({"ok":true,"scope":"self","eligible":payments>0,"successful_external_payments":payments,"unrefunded_amount_minor":r.try_get::<String,_>("amount_minor")?,"currency":"USD","evidence":"core verified external payment receipts","wallet_balance_is_not_qualification":true}),
    )
}
