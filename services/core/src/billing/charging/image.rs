//! Session-owned drawing admission. The caller owns one SERIALIZABLE transaction
//! containing the confirmed Core source, ledger hold and dispatch claim.
use super::*;
use crate::tool_market::{
    credit_access::CreditStore,
    credit_execution::{BuiltinContext, BuiltinKind},
};

struct ImageSource {
    actor: i64,
    owner: i64,
    price: Price,
    fingerprint: Vec<u8>,
    at: i64,
}

fn image_id(id: &str) -> bool {
    id.strip_prefix("image:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

// Reuse the exact marketplace byte contract, including Go-compatible HTML
// and U+2028/U+2029 escapes and its input-size bound.
fn input_digest(input: &Value) -> Result<String> {
    crate::builtins::input_digest(input).map_err(builtin_error)
}
fn builtin_error(error: crate::tool_market::Error) -> Error {
    match error {
        crate::tool_market::Error::Invalid => Error::Invalid,
        crate::tool_market::Error::Unauthorized => Error::Unauthorized,
        crate::tool_market::Error::Forbidden => Error::Forbidden,
        crate::tool_market::Error::Conflict => Error::Conflict,
        crate::tool_market::Error::Missing => Error::NotFound,
        crate::tool_market::Error::Pending => Error::WrongState,
        crate::tool_market::Error::Retry => Error::Retry,
        crate::tool_market::Error::Storage => Error::Storage,
    }
}
fn reservation(price: &Price) -> Result<i64> {
    price.validate()?;
    let quote = price
        .image
        .as_ref()
        .ok_or(Error::Forbidden)?
        .clone()
        .restore()
        .map_err(|_| Error::Invalid)?;
    Ok(i64::from(quote.reservation_credits()))
}

/// Lock and independently authenticate the persisted source before spending or
/// dispatch. Neither operation JSON nor Go may choose a payer or credential.
async fn image_source(c: &mut PgConnection, session: &str, id: &str) -> Result<ImageSource> {
    if !image_id(id) {
        return Err(Error::Invalid);
    }
    let serializable: bool =
        sqlx::query_scalar("SELECT current_setting('transaction_isolation')='serializable'")
            .fetch_one(&mut *c)
            .await?;
    if !serializable {
        return Err(Error::Invalid);
    }
    let row = sqlx::query("SELECT session_id,actor_user_id,owner_account_id,auth_version,context,input,fingerprint,price,state,expires_at,confirmed_at,charge_id FROM core_image.operations WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut *c).await?.ok_or(Error::NotFound)?;
    let price: Price = serde_json::from_value(row.try_get("price")?).map_err(|_| Error::Storage)?;
    reservation(&price)?;
    let actor = crate::identity::authorize_assistant_model_in_transaction(c, session, &price.group)
        .await
        .map_err(authority::identity_error)?;
    if actor.credential_kind != crate::identity::CredentialKind::Session {
        return Err(Error::Forbidden);
    }
    let version: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE")
            .bind(actor.user_id)
            .fetch_one(&mut *c)
            .await?;
    let at = now(c).await?;
    let confirmed: Option<i64> = row.try_get("confirmed_at")?;
    let expires: i64 = row.try_get("expires_at")?;
    if row.try_get::<i64, _>("session_id")? != actor.credential_id
        || row.try_get::<i64, _>("actor_user_id")? != actor.user_id
        || row.try_get::<i64, _>("owner_account_id")? != actor.owner_account_id
        || row.try_get::<i64, _>("auth_version")? != version
    {
        return Err(Error::Forbidden);
    }
    if row.try_get::<String, _>("state")? != "confirmed"
        || row.try_get::<Option<String>, _>("charge_id")?.is_some()
        || expires <= at
        || confirmed
            .is_none_or(|confirmed| confirmed <= 0 || confirmed > at || confirmed >= expires)
    {
        return Err(Error::WrongState);
    }
    let context: BuiltinContext =
        serde_json::from_value(row.try_get("context")?).map_err(|_| Error::Storage)?;
    let stored_input: Value = row.try_get("input")?;
    let input = crate::model_http::image_execution::confirmed_drawing_arguments(&stored_input)
        .map_err(|_| Error::Forbidden)?;
    if !input.is_object() || input_digest(&input)? != context.input_digest {
        return Err(Error::Forbidden);
    }
    let builtin = CreditStore::validate_builtin_context(
        c,
        session,
        &context,
        BuiltinKind::code_owned("drawing", "generate").map_err(builtin_error)?,
    )
    .await
    .map_err(builtin_error)?;
    if builtin.user_id != actor.user_id || builtin.owner_account_id != actor.owner_account_id {
        return Err(Error::Forbidden);
    }
    crate::model_http::image_execution::authorize_drawing_reference_contract(
        &input,
        &builtin.version_id,
        &builtin.input_schema_digest,
    )
    .map_err(|_| Error::Forbidden)?;
    let fingerprint: Vec<u8> = row.try_get("fingerprint")?;
    if fingerprint.len() != 32 || fingerprint.iter().all(|byte| *byte == 0) {
        return Err(Error::Storage);
    }
    Ok(ImageSource {
        actor: actor.user_id,
        owner: actor.owner_account_id,
        price,
        fingerprint,
        at,
    })
}

impl Charging {
    pub(crate) async fn reserve_image(
        &self,
        c: &mut PgConnection,
        session: &str,
        operation_id: &str,
        request: &Request,
        worker: &[u8; 32],
    ) -> Result<Charge> {
        request.validate()?;
        if request.id != operation_id {
            return Err(Error::Invalid);
        }
        // Fail before creating a request even when every funding source is empty.
        let worker_digest = authority::worker_digest(worker)?;
        let source = image_source(c, session, operation_id).await?;
        if request.price != source.price
            || request.fingerprint.as_slice() != source.fingerprint.as_slice()
            || request.reserve != reservation(&source.price)?
        {
            return Err(Error::Forbidden);
        }
        let preference: String = sqlx::query_scalar("SELECT p.preference FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_billing.account_policies p ON p.account_id=a.id WHERE u.id=$1 AND a.id=$2 AND a.kind='personal' AND a.active FOR SHARE OF u,a,p")
            .bind(source.actor).bind(source.owner).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
        lock(c, operation_id).await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_charging.requests WHERE id=$1 OR image_operation_id=$1)")
            .bind(operation_id).fetch_one(&mut *c).await?;
        if exists {
            return Err(Error::Conflict);
        }
        let payer = authority::Payer {
            id: source.owner,
            preference,
        };
        let mut last = Error::NoFunds;
        for funding in limits::sources(c, &payer, &source.price, source.at).await? {
            sqlx::query("SAVEPOINT image_funding")
                .execute(&mut *c)
                .await?;
            let (sub, start, end) = match funding {
                Source::Wallet => (None, None, None),
                Source::Subscription { id, start, end } => (Some(id), Some(start), Some(end)),
            };
            sqlx::query("INSERT INTO core_charging.requests(id,actor_user_id,key_id,oauth_source,assistant_grant_id,assistant_sponsor_account_id,image_operation_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,source_subscription_id,source_start,source_end,authorized_at,lease_until,hard_deadline,state,reserved) VALUES($1,$2,NULL,NULL,NULL,NULL,$1,$3,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'reserved',$14)")
                .bind(operation_id).bind(source.actor).bind(source.owner).bind(request.fingerprint.to_vec()).bind(&worker_digest).bind(hash(request)?).bind(json(&request.price)?).bind(sub).bind(start).bind(end).bind(source.at).bind(source.at+request.lease_seconds).bind(source.at+request.maximum_seconds).bind(request.reserve).execute(&mut *c).await?;
            let charge = load(c, operation_id).await?.charge;
            match self.hold(c, &charge, source.at).await {
                Ok(()) => {
                    sqlx::query("RELEASE SAVEPOINT image_funding")
                        .execute(&mut *c)
                        .await?;
                    return Ok(charge);
                }
                Err(error @ (Error::NoFunds | Error::BudgetExceeded)) => {
                    last = error;
                    sqlx::query("ROLLBACK TO SAVEPOINT image_funding")
                        .execute(&mut *c)
                        .await?;
                    sqlx::query("RELEASE SAVEPOINT image_funding")
                        .execute(&mut *c)
                        .await?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    /// The source transition and ledger dispatch claim commit together. The
    /// caller may send network bytes only after that commit is known successful.
    pub(crate) async fn start_image(
        &self,
        c: &mut PgConnection,
        session: &str,
        operation_id: &str,
        worker: &[u8; 32],
    ) -> Result<()> {
        let source = image_source(c, session, operation_id).await?;
        lock(c, operation_id).await?;
        let stored = load(c, operation_id).await?;
        worker_matches(&stored, worker)?;
        live(&stored.charge, source.at)?;
        if stored.charge.state != "reserved" {
            return Err(Error::WrongState);
        }
        let row = sqlx::query("SELECT image_operation_id,key_id,oauth_source,assistant_grant_id,assistant_sponsor_account_id,fingerprint FROM core_charging.requests WHERE id=$1 FOR UPDATE")
            .bind(operation_id).fetch_one(&mut *c).await?;
        if stored.charge.id != operation_id
            || stored.charge.actor_user_id != source.actor
            || stored.charge.owner_account_id != source.owner
            || stored.charge.payer_account_id != source.owner
            || stored.charge.price != source.price
            || stored.charge.reserved != reservation(&source.price)?
            || row
                .try_get::<Option<String>, _>("image_operation_id")?
                .as_deref()
                != Some(operation_id)
            || row.try_get::<Option<i64>, _>("key_id")?.is_some()
            || row.try_get::<Option<Value>, _>("oauth_source")?.is_some()
            || row
                .try_get::<Option<String>, _>("assistant_grant_id")?
                .is_some()
            || row
                .try_get::<Option<i64>, _>("assistant_sponsor_account_id")?
                .is_some()
            || row.try_get::<Vec<u8>, _>("fingerprint")? != source.fingerprint
        {
            return Err(Error::Forbidden);
        }
        let claimed = sqlx::query("UPDATE core_image.operations SET state='dispatching',charge_id=$1 WHERE id=$1 AND state='confirmed' AND charge_id IS NULL AND expires_at>floor(extract(epoch FROM clock_timestamp()))::bigint")
            .bind(operation_id).execute(&mut *c).await?.rows_affected();
        if claimed != 1 {
            return Err(Error::Conflict);
        }
        sqlx::query(
            "UPDATE core_charging.requests SET state='streaming',revision=revision+1 WHERE id=$1",
        )
        .bind(operation_id)
        .execute(&mut *c)
        .await?;
        event(
            c,
            operation_id,
            "start",
            json!({"kind":"image_dispatch_claim"}),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_operation_identity_is_exact() {
        assert!(image_id(&format!("image:{}", "a".repeat(64))));
        for invalid in [
            "image:a".to_owned(),
            format!("image:{}", "A".repeat(64)),
            format!("text:{}", "a".repeat(64)),
            format!("image:{}", "g".repeat(64)),
        ] {
            assert!(!image_id(&invalid));
        }
    }
    #[test]
    fn image_confirmed_input_digest_is_recursive_and_exact() {
        let a: Value = serde_json::from_str(r#"{"z":[{"b":2,"a":1}],"a":"x"}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":"x","z":[{"a":1,"b":2}]}"#).unwrap();
        assert_eq!(input_digest(&a).unwrap(), input_digest(&b).unwrap());
        assert_ne!(
            input_digest(&a).unwrap(),
            input_digest(&json!({"a":"y","z":[{"a":1,"b":2}]})).unwrap()
        );
        assert_ne!(
            input_digest(&json!({"items":[1, 2]})).unwrap(),
            input_digest(&json!({"items":[2, 1]})).unwrap()
        );
    }
    #[test]
    fn image_confirmed_input_digest_matches_market_special_characters() {
        let input = json!({"prompt":"<scene>&light>\u{2028}line\u{2029}paragraph"});
        assert_eq!(
            input_digest(&input).unwrap(),
            crate::builtins::input_digest(&input).unwrap()
        );
        let generic = format!("{:x}", Sha256::digest(serde_json::to_vec(&input).unwrap()));
        assert_ne!(input_digest(&input).unwrap(), generic);
        assert_eq!(input_digest(&json!([1, 2])), Err(Error::Invalid));
        assert_eq!(
            input_digest(&json!({"prompt":"x".repeat(65536)})),
            Err(Error::Invalid)
        );
    }
}

impl Charging {
    /// Native API-key image source. Reuses the same current key policy, payer
    /// ordering, subscription entitlement and aggregate budgets as text traffic.
    pub(crate) async fn reserve_image_key(
        &self,
        c: &mut PgConnection,
        secret: &str,
        operation_id: &str,
        request: &Request,
        worker: &[u8; 32],
        peer: Option<std::net::IpAddr>,
    ) -> Result<Charge> {
        request.validate()?;
        if request.id != operation_id || !image_id(operation_id) {
            return Err(Error::Invalid);
        }
        let serializable: bool =
            sqlx::query_scalar("SELECT current_setting('transaction_isolation')='serializable'")
                .fetch_one(&mut *c)
                .await?;
        if !serializable {
            return Err(Error::Invalid);
        }
        let worker_digest = authority::worker_digest(worker)?;
        let actor = authority::authenticate(c, secret).await?;
        if actor.kind != "api_key" {
            return Err(Error::Forbidden);
        }
        authority::model_policy(c, secret, &request.price, peer).await?;
        authority::reject_new_model(c, actor.user).await?;
        let r = sqlx::query("SELECT * FROM core_image.operations WHERE id=$1 FOR UPDATE")
            .bind(operation_id)
            .fetch_optional(&mut *c)
            .await?
            .ok_or(Error::NotFound)?;
        let frozen: Price =
            serde_json::from_value(r.try_get("price")?).map_err(|_| Error::Storage)?;
        let at = now(c).await?;
        if r.try_get::<String, _>("source_kind")? != "api_key"
            || r.try_get::<Option<i64>, _>("key_id")? != Some(actor.key)
            || r.try_get::<Option<i64>, _>("session_id")?.is_some()
            || r.try_get::<i64, _>("actor_user_id")? != actor.user
            || r.try_get::<i64, _>("owner_account_id")? != actor.owner
            || r.try_get::<String, _>("state")? != "confirmed"
            || r.try_get::<i64, _>("expires_at")? <= at
            || r.try_get::<Option<String>, _>("charge_id")?.is_some()
            || frozen != request.price
            || request.reserve != reservation(&frozen)?
            || r.try_get::<Vec<u8>, _>("fingerprint")? != request.fingerprint
        {
            return Err(Error::Forbidden);
        }
        lock(c, operation_id).await?;
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_charging.requests WHERE id=$1 OR image_operation_id=$1)").bind(operation_id).fetch_one(&mut *c).await?;
        if exists {
            return Err(Error::Conflict);
        }
        let mut last = Error::NoFunds;
        for payer in authority::funding(c, &actor).await? {
            for funding in limits::sources(c, &payer, &frozen, at).await? {
                sqlx::query("SAVEPOINT image_key_funding")
                    .execute(&mut *c)
                    .await?;
                let (sub, start, end) = match funding {
                    Source::Wallet => (None, None, None),
                    Source::Subscription { id, start, end } => (Some(id), Some(start), Some(end)),
                };
                sqlx::query("INSERT INTO core_charging.requests(id,actor_user_id,key_id,image_operation_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,source_subscription_id,source_start,source_end,authorized_at,lease_until,hard_deadline,state,reserved) VALUES($1,$2,$3,$1,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'reserved',$16)")
                    .bind(operation_id).bind(actor.user).bind(actor.key).bind(actor.owner).bind(payer.id).bind(request.fingerprint.to_vec()).bind(&worker_digest).bind(hash(request)?).bind(json(&frozen)?).bind(sub).bind(start).bind(end).bind(at).bind(at+request.lease_seconds).bind(at+request.maximum_seconds).bind(request.reserve).execute(&mut *c).await?;
                let charge = load(c, operation_id).await?.charge;
                match self.hold(c, &charge, at).await {
                    Ok(()) => {
                        sqlx::query("RELEASE SAVEPOINT image_key_funding")
                            .execute(&mut *c)
                            .await?;
                        return Ok(charge);
                    }
                    Err(e @ (Error::NoFunds | Error::BudgetExceeded)) => {
                        last = e;
                        sqlx::query("ROLLBACK TO SAVEPOINT image_key_funding")
                            .execute(&mut *c)
                            .await?;
                        sqlx::query("RELEASE SAVEPOINT image_key_funding")
                            .execute(&mut *c)
                            .await?;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Err(last)
    }
    pub(crate) async fn start_image_key(
        &self,
        c: &mut PgConnection,
        secret: &str,
        id: &str,
        worker: &[u8; 32],
        peer: Option<std::net::IpAddr>,
    ) -> Result<()> {
        let actor = authority::authenticate(c, secret).await?;
        if actor.kind != "api_key" {
            return Err(Error::Forbidden);
        }
        lock(c, id).await?;
        let stored = load(c, id).await?;
        worker_matches(&stored, worker)?;
        let at = now(c).await?;
        live(&stored.charge, at)?;
        if stored.charge.key_id != Some(actor.key)
            || stored.charge.actor_user_id != actor.user
            || stored.charge.owner_account_id != actor.owner
            || stored.charge.oauth_source.is_some()
            || stored.charge.state != "reserved"
        {
            return Err(Error::Forbidden);
        }
        authority::model_policy(c, secret, &stored.charge.price, peer).await?;
        authority::reject_new_model(c, actor.user).await?;
        let updated=sqlx::query("UPDATE core_image.operations SET state='dispatching',charge_id=$1 WHERE id=$1 AND source_kind='api_key' AND key_id=$2 AND actor_user_id=$3 AND owner_account_id=$4 AND state='confirmed' AND charge_id IS NULL AND expires_at>$5 AND price=$6")
            .bind(id).bind(actor.key).bind(actor.user).bind(actor.owner).bind(at).bind(json(&stored.charge.price)?).execute(&mut *c).await?.rows_affected();
        if updated != 1 {
            return Err(Error::Conflict);
        }
        let claimed=sqlx::query("UPDATE core_charging.requests SET state='streaming',revision=revision+1 WHERE id=$1 AND image_operation_id=$1").bind(id).execute(&mut *c).await?.rows_affected();
        if claimed != 1 {
            return Err(Error::Conflict);
        }
        event(c, id, "start", json!({"kind":"image_key_dispatch_claim"})).await?;
        Ok(())
    }
}
