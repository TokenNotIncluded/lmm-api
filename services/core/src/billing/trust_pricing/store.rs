use super::types::{Facts, TrustPolicy, UserPriceAuthority, evaluate};
use super::{Error, Result};
use serde_json::json;
use sqlx::{PgConnection, Row, types::Json};

/// authenticated_user_id must be the Core-authenticated key owner/OAuth subject,
/// never a caller-selected payer or an ID from Go. Caller owns and commits the
/// SERIALIZABLE admission transaction and retries the whole operation on 40001.
/// No pool, network, DDL or financial write occurs here.
pub(crate) async fn quote_in_transaction(
    c: &mut PgConnection,
    authenticated_user_id: i64,
) -> Result<UserPriceAuthority> {
    quote_inner(c, authenticated_user_id, false).await
}
/// Same authority for authenticated metadata; requires SERIALIZABLE READ ONLY.
/// No row locks or writes, and never converted into a new spend credential.
pub(crate) async fn quote_readonly_in_transaction(
    c: &mut PgConnection,
    authenticated_user_id: i64,
) -> Result<UserPriceAuthority> {
    let read_only: String = sqlx::query_scalar("SELECT current_setting('transaction_read_only')")
        .fetch_one(&mut *c)
        .await?;
    if read_only != "on" {
        return Err(Error::Invalid);
    }
    quote_inner(c, authenticated_user_id, true).await
}
async fn quote_inner(
    c: &mut PgConnection,
    authenticated_user_id: i64,
    read_only: bool,
) -> Result<UserPriceAuthority> {
    if authenticated_user_id <= 0 {
        return Err(Error::Denied);
    }
    let mode: String = sqlx::query_scalar("SELECT current_setting('transaction_isolation')")
        .fetch_one(&mut *c)
        .await?;
    if mode != "serializable" {
        return Err(Error::Invalid);
    }
    sqlx::query("SELECT pg_advisory_xact_lock_shared(741803219)")
        .execute(&mut *c)
        .await?;
    let at: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&mut *c)
            .await?;
    let user_sql = "SELECT u.platform_role,u.personal_account_id,floor(extract(epoch FROM a.created_at))::bigint AS created_at FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND u.active AND a.active";
    let user = sqlx::query(&if read_only {
        user_sql.to_owned()
    } else {
        format!("{user_sql} FOR SHARE OF u,a")
    })
    .bind(authenticated_user_id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Denied)?;
    let policy=sqlx::query("SELECT revision,spec,source_digest,source_cutoff,native_user_floor,floor(extract(epoch FROM native_since))::bigint AS native_since FROM core_trust_pricing.policies WHERE singleton")
        .fetch_optional(&mut *c).await?.ok_or(Error::Unavailable)?;
    let spec: Json<TrustPolicy> = policy.try_get("spec")?;
    spec.0.validate()?;
    let base=sqlx::query("SELECT original_created_at,console_activated_at,last_api_activity_at,override_level,projection_known,order_count,revision,source_digest FROM core_trust_pricing.user_bases WHERE user_id=$1")
        .bind(authenticated_user_id).fetch_optional(&mut *c).await?;
    let mut f = Facts {
        user_id: authenticated_user_id,
        role: user.try_get("platform_role")?,
        basis_revision: 0,
        created_at: user.try_get("created_at")?,
        activated_at: 0,
        last_api_at: 0,
        override_level: None,
        projection_known: true,
        net_paid_credits: 0,
        positive_orders: 0,
        last_paid_at: 0,
        evidence: json!({}),
    };
    let mut expected_orders = 0;
    let mut basis_digest = Vec::new();
    if let Some(row) = base {
        f.created_at = row.try_get("original_created_at")?;
        f.activated_at = row.try_get("console_activated_at")?;
        f.last_api_at = row.try_get("last_api_activity_at")?;
        f.override_level = row
            .try_get::<Option<i16>, _>("override_level")?
            .map(|v| u8::try_from(v).map_err(|_| Error::Invalid))
            .transpose()?;
        f.projection_known = row.try_get("projection_known")?;
        f.basis_revision = row.try_get("revision")?;
        expected_orders = row.try_get::<i64, _>("order_count")?;
        basis_digest = row.try_get::<Vec<u8>, _>("source_digest")?;
    } else if authenticated_user_id < policy.try_get::<i64, _>("native_user_floor")?
        || f.created_at < policy.try_get::<i64, _>("native_since")?
    {
        return Err(Error::Unavailable);
    }
    let legacy=sqlx::query("SELECT source_order_id,net_credits,paid_at,revision,source_digest FROM core_trust_pricing.legacy_paid_orders WHERE user_id=$1 ORDER BY source_order_id LIMIT 10001")
        .bind(authenticated_user_id).fetch_all(&mut *c).await?;
    if legacy.len() as i64 != expected_orders || legacy.len() > 10000 {
        return Err(Error::Unavailable);
    }
    let mut legacy_evidence = Vec::new();
    for r in legacy {
        let net: i64 = r.try_get("net_credits")?;
        let paid: i64 = r.try_get("paid_at")?;
        add_paid(&mut f, net, paid, at)?;
        legacy_evidence.push(json!([
            r.try_get::<i64, _>("source_order_id")?,
            net,
            paid,
            r.try_get::<i64, _>("revision")?,
            r.try_get::<Vec<u8>, _>("source_digest")?
        ]));
    }
    // Only actual Core credit journals grant paid entitlement. Pending/accepted
    // proofs, gifts, wallet transfers and precredit refund fences do not appear.
    // Refund journals, not a provider gross amount, reduce the present NET pool.
    let native=sqlx::query("SELECT i.id,i.credit_units,j.id AS journal_id,floor(extract(epoch FROM j.created_at))::bigint AS paid_at,COALESCE((SELECT sum(r.amount_units)::bigint FROM core_billing.ledger_journals r WHERE r.parent_journal_id=j.id AND r.kind='refund_topup'),0) AS refunded FROM core_payments.intents i JOIN core_billing.ledger_journals j ON j.id=i.credit_journal_id AND j.kind='credit' AND j.amount_units=i.credit_units JOIN core_billing.ledger_accounts w ON w.id=j.payee_id AND w.owner_account_id=i.account_id AND w.bucket='wallet' WHERE i.account_id=$1 ORDER BY i.id LIMIT 10001")
        .bind(user.try_get::<i64, _>("personal_account_id")?).fetch_all(&mut *c).await?;
    if native.len() > 10000 {
        return Err(Error::Unavailable);
    }
    let mut native_evidence = Vec::new();
    for r in native {
        let original: i64 = r.try_get("credit_units")?;
        let refunded: i64 = r.try_get("refunded")?;
        if refunded < 0 || refunded > original {
            return Err(Error::Unavailable);
        }
        let net = original - refunded;
        let paid: i64 = r.try_get("paid_at")?;
        add_paid(&mut f, net, paid, at)?;
        native_evidence.push(json!([
            r.try_get::<i64, _>("id")?,
            r.try_get::<i64, _>("journal_id")?,
            original,
            refunded,
            paid
        ]));
    }
    // Actual immutable final events, not HTTP status/login/preview/refund.
    // Zero measured usage is explicitly false, including genuine zero successes.
    let activity=sqlx::query("SELECT r.id,r.input_tokens,r.output_tokens,r.image_usage,r.outcome,e.detail,e.created_at,floor(extract(epoch FROM e.created_at))::bigint AS at FROM core_charging.requests r JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='final' WHERE r.actor_user_id=$1 AND r.state='settled' AND e.created_at>=to_timestamp($2) ORDER BY e.created_at DESC,r.id LIMIT 10001")
        .bind(authenticated_user_id).bind(f.last_api_at).fetch_all(&mut *c).await?;
    if activity.len() > 10000 {
        return Err(Error::Unavailable);
    }
    for r in activity {
        let when: i64 = r.try_get("at")?;
        if when < f.last_api_at {
            break;
        }
        let detail: Json<serde_json::Value> = r.try_get("detail")?;
        let usage: crate::billing::charging::Usage =
            serde_json::from_value(detail.0.get("usage").cloned().ok_or(Error::Unavailable)?)
                .map_err(|_| Error::Unavailable)?;
        if usage.input != r.try_get::<i64, _>("input_tokens")?
            || usage.output != r.try_get::<i64, _>("output_tokens")?
        {
            return Err(Error::Unavailable);
        }
        let measured = super::counted_api_activity(usage)?;
        // Older Core final events predate this bool but use the SAME measured
        // primitive. Their exact Usage is sufficient; no Go log/HTTP inference.
        let counted = match detail.0.get("counted_api_activity") {
            Some(serde_json::Value::Bool(v)) if *v == measured => *v,
            Some(_) => return Err(Error::Unavailable),
            None => measured,
        };
        if counted {
            f.last_api_at = when;
            break;
        }
    }
    f.evidence = json!({"policy_source":policy.try_get::<Vec<u8>,_>("source_digest")?,"source_cutoff":policy.try_get::<i64,_>("source_cutoff")?,"basis_source":basis_digest,"legacy":legacy_evidence,"native":native_evidence});
    evaluate(&spec.0, policy.try_get("revision")?, &f, at)
}
fn add_paid(f: &mut Facts, net: i64, paid: i64, at: i64) -> Result<()> {
    if net < 0 || paid <= 0 || paid > at {
        return Err(Error::Unavailable);
    }
    f.net_paid_credits = f
        .net_paid_credits
        .checked_add(net)
        .ok_or(Error::Unavailable)?;
    if net > 0 {
        f.positive_orders = f.positive_orders.checked_add(1).ok_or(Error::Unavailable)?;
        f.last_paid_at = f.last_paid_at.max(paid);
    }
    Ok(())
}
