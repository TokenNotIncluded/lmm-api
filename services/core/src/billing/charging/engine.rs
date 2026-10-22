#[path = "assistant.rs"]
mod assistant;
#[path = "cache_evidence.rs"]
mod cache_evidence;
#[path = "image.rs"]
mod image;
#[path = "model_unknown.rs"]
mod model_unknown;
use super::{authority, limits, types::*};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use std::{
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct Charging {
    pool: PgPool,
    ledger: Arc<dyn Ledger>,
    trust_pricing: Arc<AtomicBool>,
}
struct Stored {
    charge: Charge,
    worker: Vec<u8>,
    spec: Vec<u8>,
}
fn json<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|_| Error::Invalid)
}
fn hash<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(Sha256::digest(serde_json::to_vec(value).map_err(|_| Error::Invalid)?).to_vec())
}
fn admission_spec(
    request: &Request,
    basis: Option<ReservationBasis>,
    trusted: bool,
) -> Result<Vec<u8>> {
    if trusted {
        hash(&(
            "core-charging-trust-admission-v1",
            request,
            basis.ok_or(Error::Conflict)?,
        ))
    } else {
        hash(request)
    }
}
fn trust_error(error: crate::billing::trust_pricing::Error) -> Error {
    use crate::billing::trust_pricing::Error as Trust;
    match error {
        Trust::Retry => Error::Retry,
        Trust::Denied => Error::Forbidden,
        Trust::Invalid | Trust::Unavailable | Trust::Storage => Error::Storage,
    }
}
#[cfg(test)]
mod trust_admission_tests {
    use super::*;
    fn request() -> Request {
        Request {
            id: "trust-request".into(),
            fingerprint: [1; 32],
            price: Price {
                user_trust: None,
                image: None,
                upstream_cost_group_ppm: None,
                tiered: None,
                version: "v1".into(),
                model: "model".into(),
                group: "default".into(),
                input_per_million: 10_000,
                cached_per_million: 0,
                output_per_million: 20_000,
            },
            reserve: 2,
            lease_seconds: 60,
            maximum_seconds: 60,
        }
    }
    #[test]
    fn trusted_spec_binds_original_bounds_and_cannot_downgrade_to_legacy() {
        let request = request();
        let basis = Some(ReservationBasis::Tokens {
            input: 101,
            output: 0,
        });
        let legacy = hash(&request).unwrap();
        assert_eq!(admission_spec(&request, basis, false).unwrap(), legacy);
        let trusted = admission_spec(&request, basis, true).unwrap();
        assert_ne!(trusted, legacy);
        assert_ne!(
            trusted,
            admission_spec(
                &request,
                Some(ReservationBasis::Tokens {
                    input: 102,
                    output: 0
                }),
                true
            )
            .unwrap()
        );
        assert_eq!(admission_spec(&request, None, true), Err(Error::Conflict));
        assert_eq!(admission_spec(&request, basis, true).unwrap(), trusted);
    }
    #[test]
    fn trust_serialization_retry_retains_the_existing_whole_transaction_policy() {
        assert_eq!(
            trust_error(crate::billing::trust_pricing::Error::Retry),
            Error::Retry
        );
        assert_eq!(
            trust_error(crate::billing::trust_pricing::Error::Denied),
            Error::Forbidden
        );
        assert_eq!(
            trust_error(crate::billing::trust_pricing::Error::Unavailable),
            Error::Storage
        );
    }
}
// The host derives activity from measured usage, independently of price or
// response status. Request/client/Go input cannot supply this decision.
fn counted_api_activity(usage: Usage) -> bool {
    usage.valid()
        && (usage.input > 0
            || usage.cached > 0
            || usage.output > 0
            || usage.cache_creation.is_some_and(|v| v.total > 0)
            || usage.image.is_some_and(|v| v.output_images > 0)
            || usage.upstream_cost_atto_usd.is_some_and(|v| v > 0))
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn activity_uses_measured_work_including_free_and_partial_usage() {
        // Neither a zero price nor Completed/Failed/Cancelled changes usage.
        for usage in [
            Usage {
                input: 1,
                ..Usage::default()
            },
            Usage {
                output: 1,
                ..Usage::default()
            },
            Usage {
                input: 1,
                cached: 1,
                ..Usage::default()
            },
            Usage {
                input: 1,
                cache_creation: Some(CacheCreationUsage {
                    twelve_hour: None,
                    total: 1,
                    five_minute: None,
                    one_hour: None,
                    cache_creation_mode: None,
                    reported_total: None,
                }),
                ..Usage::default()
            },
            Usage {
                upstream_cost_atto_usd: Some(1),
                ..Usage::default()
            },
            Usage {
                image: Some(crate::model_http::image_price::ImageUsage {
                    output_images: 1,
                    provider_receipt_sha256: [1; 32],
                }),
                ..Usage::default()
            },
        ] {
            assert!(counted_api_activity(usage));
        }
    }

    #[test]
    fn zero_or_invalid_usage_does_not_fabricate_activity() {
        for usage in [
            Usage::default(),
            Usage {
                upstream_cost_atto_usd: Some(0),
                ..Usage::default()
            },
            Usage {
                input: -1,
                output: 1,
                ..Usage::default()
            },
        ] {
            assert!(!counted_api_activity(usage));
        }
    }
}
fn decode(row: PgRow) -> Result<Stored> {
    let source = match row.try_get::<Option<i64>, _>("source_subscription_id")? {
        None => Source::Wallet,
        Some(id) => Source::Subscription {
            id,
            start: row.try_get("source_start")?,
            end: row.try_get("source_end")?,
        },
    };
    let stored = Stored {
        worker: row.try_get("worker_digest")?,
        spec: row.try_get("spec_digest")?,
        charge: Charge {
            id: row.try_get("id")?,
            actor_user_id: row.try_get("actor_user_id")?,
            key_id: row.try_get("key_id")?,
            oauth_source: row
                .try_get::<Option<Value>, _>("oauth_source")?
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Storage)?,
            owner_account_id: row.try_get("owner_account_id")?,
            payer_account_id: row.try_get("payer_account_id")?,
            price: serde_json::from_value(row.try_get("price")?).map_err(|_| Error::Storage)?,
            source,
            authorized_at: row.try_get("authorized_at")?,
            lease_until: row.try_get("lease_until")?,
            hard_deadline: row.try_get("hard_deadline")?,
            state: row.try_get("state")?,
            reserved: row.try_get("reserved")?,
            usage: cache_evidence::restore(
                Usage {
                    image: row
                        .try_get::<Option<Value>, _>("image_usage")?
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|_| Error::Storage)?,
                    upstream_cost_atto_usd: row
                        .try_get::<Option<String>, _>("upstream_cost_atto_usd")?
                        .map(|v| v.parse::<u128>().map_err(|_| Error::Storage))
                        .transpose()?,
                    cache_creation: None,
                    input: row.try_get("input_tokens")?,
                    cached: row.try_get("cached_tokens")?,
                    output: row.try_get("output_tokens")?,
                },
                row.try_get("usage_evidence")?,
            )?,
            settled: row.try_get("settled")?,
            refunded: row.try_get("refunded")?,
            revision: row.try_get("revision")?,
            outcome: row.try_get("outcome")?,
        },
    };
    stored
        .charge
        .price
        .validate_actor(stored.charge.actor_user_id)
        .map_err(|_| Error::Storage)?;
    let cache_tariff = stored
        .charge
        .price
        .tiered
        .as_ref()
        .map(|t| t.tariff.compile().map(|t| t.requires_cache_creation()))
        .transpose()
        .map_err(|_| Error::Storage)?
        .unwrap_or(false);
    if cache_tariff
        && (stored.charge.usage.input != 0
            || stored.charge.usage.cached != 0
            || stored.charge.usage.output != 0
            || (stored.charge.state == "settled"
                && stored.charge.outcome.as_deref() == Some("completed")))
        && stored.charge.usage.cache_creation.is_none()
    {
        return Err(Error::Storage);
    }
    if let Some(tariff) = &stored.charge.price.tiered {
        if stored
            .charge
            .usage
            .cache_creation
            .is_some_and(|usage| usage.cache_creation_mode != tariff.cache_creation_mode)
        {
            return Err(Error::Storage);
        }
        if tariff.cache_creation_mode.is_some() || tariff.tariff.revision == 4 {
            stored
                .charge
                .price
                .cost(stored.charge.usage)
                .map_err(|_| Error::Storage)?;
        }
    }
    match (&stored.charge.key_id, &stored.charge.oauth_source) {
        (Some(key), None) if *key > 0 => (),
        (None, Some(source))
            if source.user_id == stored.charge.actor_user_id
                && source.personal_account_id == stored.charge.owner_account_id
                && source.personal_account_id == stored.charge.payer_account_id =>
        {
            source.validate_stored().map_err(|_| Error::Storage)?
        }
        (None, None)
            if row
                .try_get::<Option<String>, _>("assistant_grant_id")?
                .is_some()
                && (stored.charge.owner_account_id == stored.charge.payer_account_id
                    || row.try_get::<Option<i64>, _>("assistant_sponsor_account_id")?
                        == Some(stored.charge.payer_account_id)) =>
        {
            ()
        }
        (None, None)
            if row
                .try_get::<Option<String>, _>("image_operation_id")?
                .is_some()
                && stored.charge.price.image.is_some()
                && stored.charge.owner_account_id == stored.charge.payer_account_id =>
        {
            ()
        }
        _ => return Err(Error::Storage),
    }
    Ok(stored)
}
async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}
async fn lock(c: &mut PgConnection, id: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(Error::Invalid);
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core_charging:' || $1,0))")
        .bind(id)
        .execute(c)
        .await?;
    Ok(())
}
async fn select_stored(c: &mut PgConnection, id: &str, for_update: bool) -> Result<Option<Stored>> {
    let mut statement = String::from(
        "SELECT r.*, \
         (SELECT max((e.detail #>> '{usage,upstream_cost_atto_usd}')::numeric)::text \
          FROM core_charging.usage_events e WHERE e.request_id=r.id) AS upstream_cost_atto_usd, \
         (SELECT jsonb_agg(jsonb_build_object('event_id',e.event_id,'usage',e.detail->'usage')) \
          FROM core_charging.usage_events e WHERE e.request_id=r.id \
          AND (e.event_id='final' OR e.event_id LIKE 'usage:%')) AS usage_evidence \
         FROM core_charging.requests r WHERE r.id=$1",
    );
    if for_update {
        statement.push_str(" FOR UPDATE");
    }
    sqlx::query(&statement)
        .bind(id)
        .fetch_optional(c)
        .await?
        .map(decode)
        .transpose()
}
async fn load(c: &mut PgConnection, id: &str) -> Result<Stored> {
    select_stored(c, id, true).await?.ok_or(Error::NotFound)
}
fn worker_matches(stored: &Stored, worker: &[u8; 32]) -> Result<()> {
    if stored
        .worker
        .ct_eq(&authority::worker_digest(worker)?)
        .unwrap_u8()
        == 1
    {
        Ok(())
    } else {
        Err(Error::StaleWorker)
    }
}
fn live(charge: &Charge, at: i64) -> Result<()> {
    if at >= charge.lease_until || at >= charge.hard_deadline {
        Err(Error::StaleWorker)
    } else {
        Ok(())
    }
}
async fn commit(tx: Transaction<'_, Postgres>) -> Result<()> {
    tx.commit().await.map_err(|error| {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01") => Error::Retry,
            _ => Error::CommitUnknown,
        }
    })
}
fn command(charge: &Charge, tag: &str, change: Change) -> Result<LedgerCommand> {
    let operation_id = hash(&("core-charging-v1", &charge.id, tag))?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Ok(LedgerCommand {
        operation_id,
        request_id: charge.id.clone(),
        actor_user_id: charge.actor_user_id,
        key_id: charge.key_id,
        oauth_source: charge.oauth_source.clone(),
        owner_account_id: charge.owner_account_id,
        payer_account_id: charge.payer_account_id,
        price_version: charge.price.version.clone(),
        source: charge.source,
        change,
    })
}
async fn event(c: &mut PgConnection, id: &str, event_id: &str, detail: Value) -> Result<()> {
    sqlx::query(
        "INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES ($1,$2,$3)",
    )
    .bind(id)
    .bind(event_id)
    .bind(detail)
    .execute(c)
    .await?;
    Ok(())
}

impl Charging {
    /// No default ledger. Supplying a test double never enables production.
    pub fn new(pool: PgPool, ledger: Arc<dyn Ledger>) -> Self {
        Self {
            pool,
            ledger,
            trust_pricing: Arc::new(AtomicBool::new(false)),
        }
    }
    /// The production Rust host enables this once after checking component 25.
    /// There is deliberately no runtime/client switch to disable entitlements.
    pub(crate) fn enable_trust_pricing(&self) {
        self.trust_pricing.store(true, Ordering::Release);
    }
    pub(crate) fn trust_pricing_enabled(&self) -> bool {
        self.trust_pricing.load(Ordering::Acquire)
    }
    pub(crate) async fn freeze_trust_request(
        &self,
        c: &mut PgConnection,
        actor: i64,
        request: &Request,
        basis: ReservationBasis,
    ) -> Result<Request> {
        request.validate()?;
        if request.price.user_trust.is_some()
            || request.reserve != request.price.reservation_ceiling(basis)?
        {
            return Err(Error::Invalid);
        }
        let mut frozen = request.clone();
        if self.trust_pricing_enabled() {
            let authority = crate::billing::trust_pricing::quote_in_transaction(c, actor)
                .await
                .map_err(trust_error)?;
            frozen.price = frozen.price.with_user_authority(&authority)?;
            frozen.price.validate_actor(actor)?;
            frozen.reserve = frozen.price.reservation_ceiling(basis)?;
        }
        Ok(frozen)
    }
    async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await?;
        for statement in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL synchronous_commit=on",
            "SET LOCAL lock_timeout='2s'",
            "SET LOCAL statement_timeout='5s'",
            "SET LOCAL idle_in_transaction_session_timeout='10s'",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    /// Stable ID + full request fingerprint + immutable price + worker token.
    /// The worker token must be 32 random bytes created by the Rust gateway.
    /// Candidate funding is completely authenticated BEFORE attempting a hold.
    pub async fn reserve(
        &self,
        secret: &str,
        request: &Request,
        worker: &[u8; 32],
    ) -> Result<Charge> {
        self.reserve_with_context(secret, request, worker, None)
            .await
    }
    /// The host supplies the trusted network peer. Missing peer information
    /// fails closed for keys with IP restrictions.
    pub async fn reserve_with_context(
        &self,
        secret: &str,
        request: &Request,
        worker: &[u8; 32],
        peer: Option<IpAddr>,
    ) -> Result<Charge> {
        self.reserve_authorized(secret, request, worker, peer, None, None)
            .await
    }
    /// A real scoped OAuth access token, not a submitted DTO or a native key.
    pub async fn reserve_oauth_with_context(
        &self,
        issuer: &crate::identity::grants::LmmIssuer,
        secret: &str,
        request: &Request,
        worker: &[u8; 32],
        peer: Option<IpAddr>,
    ) -> Result<Charge> {
        self.reserve_authorized(secret, request, worker, peer, Some(issuer), None)
            .await
    }
    pub(crate) async fn reserve_model_with_context(
        &self,
        secret: &str,
        request: &Request,
        worker: &[u8; 32],
        peer: Option<IpAddr>,
        basis: ReservationBasis,
    ) -> Result<Charge> {
        self.reserve_authorized(secret, request, worker, peer, None, Some(basis))
            .await
    }
    pub(crate) async fn reserve_model_oauth_with_context(
        &self,
        issuer: &crate::identity::grants::LmmIssuer,
        secret: &str,
        request: &Request,
        worker: &[u8; 32],
        peer: Option<IpAddr>,
        basis: ReservationBasis,
    ) -> Result<Charge> {
        self.reserve_authorized(secret, request, worker, peer, Some(issuer), Some(basis))
            .await
    }
    async fn reserve_authorized(
        &self,
        secret: &str,
        request: &Request,
        worker: &[u8; 32],
        peer: Option<IpAddr>,
        issuer: Option<&crate::identity::grants::LmmIssuer>,
        basis: Option<ReservationBasis>,
    ) -> Result<Charge> {
        request.validate()?;
        // A deserialized persisted snapshot is not fresh admission authority.
        if request.price.user_trust.is_some() {
            return Err(Error::Invalid);
        }
        let worker_digest = authority::worker_digest(worker)?;
        let mut tx = self.begin().await?;
        lock(&mut tx, &request.id).await?;
        let (user, key, owner, oauth, native) = if let Some(issuer) = issuer {
            let source = issuer
                .authorize_model_in_transaction(
                    &mut tx,
                    secret,
                    Some(&request.price.model),
                    Some(&request.price.group),
                )
                .await
                .map_err(authority::identity_error)?;
            (
                source.user_id,
                None,
                source.personal_account_id,
                Some(source),
                None,
            )
        } else {
            let actor = authority::authenticate(&mut tx, secret).await?;
            if actor.kind != "api_key" {
                return Err(Error::Forbidden);
            }
            authority::model_policy(&mut tx, secret, &request.price, peer).await?;
            (actor.user, Some(actor.key), actor.owner, None, Some(actor))
        };
        if let Some(stored) = select_stored(&mut tx, &request.id, true).await? {
            worker_matches(&stored, worker)?;
            let spec = admission_spec(request, basis, stored.charge.price.user_trust.is_some())?;
            if stored.charge.actor_user_id != user
                || stored.charge.key_id != key
                || stored.charge.owner_account_id != owner
                || stored.charge.oauth_source != oauth
                || stored.spec != spec
            {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        if self.trust_pricing_enabled() && basis.is_none() {
            return Err(Error::Forbidden);
        }
        let spec = admission_spec(request, basis, self.trust_pricing_enabled())?;
        let effective_request = if self.trust_pricing_enabled() {
            self.freeze_trust_request(&mut tx, user, request, basis.ok_or(Error::Forbidden)?)
                .await?
        } else {
            request.clone()
        };
        let request = &effective_request;
        // Existing exact operation receipts above remain safely replayable.
        // Only a NEW reservation is excluded by the current abuse mark.
        authority::reject_new_model(&mut tx, user).await?;
        let payers = match (&native, &oauth) {
            (Some(actor), None) => authority::funding(&mut tx, actor).await?,
            (None, Some(source)) => vec![authority::oauth_payer(&mut tx, source).await?],
            _ => return Err(Error::Storage),
        };
        let at = now(&mut tx).await?;
        let mut last_error = Error::NoFunds;
        for payer in payers {
            for source in limits::sources(&mut tx, &payer, &request.price, at).await? {
                sqlx::query("SAVEPOINT funding_candidate")
                    .execute(&mut *tx)
                    .await?;
                let (sub, start, end) = match source {
                    Source::Wallet => (None, None, None),
                    Source::Subscription { id, start, end } => (Some(id), Some(start), Some(end)),
                };
                sqlx::query(
                    "INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id, \
                     oauth_source,fingerprint,worker_digest,spec_digest,price,source_subscription_id,source_start,source_end, \
                     authorized_at,lease_until,hard_deadline,state,reserved) \
                     VALUES ($1,$2,$3,$4,$5,$17,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'reserved',$16)",
                )
                .bind(&request.id).bind(user).bind(key).bind(owner).bind(payer.id)
                .bind(request.fingerprint.to_vec()).bind(&worker_digest).bind(&spec).bind(json(&request.price)?)
                .bind(sub).bind(start).bind(end).bind(at).bind(at + request.lease_seconds)
                .bind(at + request.maximum_seconds).bind(request.reserve).bind(oauth.as_ref().map(json).transpose()?).execute(&mut *tx).await
                .map_err(|error| {
                    // Waiting for the request advisory lock does not refresh a
                    // SERIALIZABLE snapshot. A concurrent identical ID may have
                    // committed after our snapshot, so this one prechecked PK
                    // collision needs a fresh whole transaction. The replay
                    // path then checks the complete specification and worker.
                    let database = error.as_database_error();
                    if database.is_some_and(|e| e.code().as_deref() == Some("23505")
                        && e.constraint() == Some("requests_pkey")) {
                        Error::Retry
                    } else {
                        Error::from(error)
                    }
                })?;
                let charge = load(&mut tx, &request.id).await?.charge;
                let attempt = self.hold(&mut tx, &charge, at).await;
                match attempt {
                    Ok(()) => {
                        sqlx::query("RELEASE SAVEPOINT funding_candidate")
                            .execute(&mut *tx)
                            .await?;
                        commit(tx).await?;
                        return Ok(charge);
                    }
                    Err(error @ (Error::NoFunds | Error::BudgetExceeded)) => {
                        last_error = error;
                        sqlx::query("ROLLBACK TO SAVEPOINT funding_candidate")
                            .execute(&mut *tx)
                            .await?;
                        sqlx::query("RELEASE SAVEPOINT funding_candidate")
                            .execute(&mut *tx)
                            .await?;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Err(last_error)
    }
    async fn hold(&self, c: &mut PgConnection, charge: &Charge, at: i64) -> Result<()> {
        limits::budgets(c, charge, charge.reserved).await?;
        limits::entitlement(c, charge, charge.reserved, at).await?;
        self.ledger
            .apply(
                c,
                command(
                    charge,
                    &format!("reserve:{}", charge.reserved),
                    Change::Reserve {
                        total: charge.reserved,
                    },
                )?,
            )
            .await?;
        Ok(())
    }
    /// Returns true ONLY for the first durable reserved -> streaming transition.
    /// On false or CommitUnknown, do not dispatch upstream again. A provider
    /// request ID / status check is required to resolve an ambiguous dispatch.
    pub async fn start(&self, id: &str, worker: &[u8; 32]) -> Result<bool> {
        self.start_authorized(id, worker, None).await
    }
    pub async fn start_oauth(
        &self,
        issuer: &crate::identity::grants::LmmIssuer,
        id: &str,
        worker: &[u8; 32],
    ) -> Result<bool> {
        self.start_authorized(id, worker, Some(issuer)).await
    }
    async fn start_authorized(
        &self,
        id: &str,
        worker: &[u8; 32],
        issuer: Option<&crate::identity::grants::LmmIssuer>,
    ) -> Result<bool> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        live(&stored.charge, now(&mut tx).await?)?;
        if stored.charge.state == "streaming" {
            commit(tx).await?;
            return Ok(false);
        }
        if stored.charge.state != "reserved" {
            return Err(Error::WrongState);
        }
        match (&stored.charge.oauth_source, issuer) {
            (Some(source), Some(issuer)) => {
                issuer
                    .recheck_model_source(
                        &mut tx,
                        source,
                        &stored.charge.price.model,
                        &stored.charge.price.group,
                    )
                    .await
                    .map_err(authority::identity_error)?;
                authority::reject_new_model(&mut tx, source.user_id).await?;
            }
            (None, None) if stored.charge.key_id.is_some() => (),
            _ => return Err(Error::Forbidden),
        }
        sqlx::query(
            "UPDATE core_charging.requests SET state='streaming',revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        event(
            &mut tx,
            id,
            "start",
            json!({"kind":"upstream_dispatch_claim"}),
        )
        .await?;
        commit(tx).await?;
        Ok(true)
    }
    /// Extension keeps the original payer, price, subscription cycle and budget
    /// attribution time. It rechecks current key/team authority. No source switch.
    pub async fn increase(
        &self,
        secret: &str,
        id: &str,
        worker: &[u8; 32],
        total: i64,
    ) -> Result<Charge> {
        self.increase_with_context(secret, id, worker, total, None)
            .await
    }
    pub async fn increase_with_context(
        &self,
        secret: &str,
        id: &str,
        worker: &[u8; 32],
        total: i64,
        peer: Option<IpAddr>,
    ) -> Result<Charge> {
        self.increase_authorized(secret, id, worker, total, peer, None)
            .await
    }
    pub async fn increase_oauth_with_context(
        &self,
        issuer: &crate::identity::grants::LmmIssuer,
        secret: &str,
        id: &str,
        worker: &[u8; 32],
        total: i64,
        peer: Option<IpAddr>,
    ) -> Result<Charge> {
        self.increase_authorized(secret, id, worker, total, peer, Some(issuer))
            .await
    }
    async fn increase_authorized(
        &self,
        secret: &str,
        id: &str,
        worker: &[u8; 32],
        total: i64,
        peer: Option<IpAddr>,
        issuer: Option<&crate::identity::grants::LmmIssuer>,
    ) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let at = now(&mut tx).await?;
        live(&stored.charge, at)?;
        let old = stored.charge;
        if !matches!(old.state.as_str(), "reserved" | "streaming") {
            return Err(Error::WrongState);
        }
        match (&old.oauth_source, issuer) {
            (Some(original), Some(issuer)) => {
                let current = issuer
                    .authorize_model_in_transaction(
                        &mut tx,
                        secret,
                        Some(&old.price.model),
                        Some(&old.price.group),
                    )
                    .await
                    .map_err(authority::identity_error)?;
                if old.key_id.is_some()
                    || current.grant_id != original.grant_id
                    || current.client_id != original.client_id
                    || current.issuer != original.issuer
                    || current.resource != original.resource
                    || current.user_id != old.actor_user_id
                    || current.personal_account_id != old.payer_account_id
                    || current.personal_account_id != old.owner_account_id
                    || current.user_version != original.user_version
                    || current.consent_version != original.consent_version
                    || current.budget_credits != original.budget_credits
                {
                    return Err(Error::Forbidden);
                }
                let _ = authority::oauth_payer(&mut tx, &current).await?;
                if total > old.reserved {
                    authority::reject_new_model(&mut tx, current.user_id).await?;
                }
            }
            (None, None) => {
                let actor = authority::authenticate(&mut tx, secret).await?;
                if Some(actor.key) != old.key_id
                    || actor.user != old.actor_user_id
                    || actor.owner != old.owner_account_id
                {
                    return Err(Error::Forbidden);
                }
                authority::model_policy(&mut tx, secret, &old.price, peer).await?;
                if !authority::funding(&mut tx, &actor)
                    .await?
                    .iter()
                    .any(|p| p.id == old.payer_account_id)
                {
                    return Err(Error::Forbidden);
                }
            }
            _ => return Err(Error::Forbidden),
        }
        if total < old.reserved {
            return Err(Error::Invalid);
        }
        if total == old.reserved {
            commit(tx).await?;
            return Ok(old);
        }
        sqlx::query(
            "UPDATE core_charging.requests SET reserved=$2,revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .bind(total)
        .execute(&mut *tx)
        .await?;
        let charge = load(&mut tx, id).await?.charge;
        self.hold(&mut tx, &charge, at).await?;
        commit(tx).await?;
        Ok(charge)
    }
    /// Persist cumulative usage BEFORE exposing corresponding output. Never use
    /// per-chunk rounded charges. Duplicate event IDs must have identical usage.
    pub async fn checkpoint(
        &self,
        id: &str,
        worker: &[u8; 32],
        event_id: &str,
        usage: Usage,
    ) -> Result<Charge> {
        if !valid_id(event_id) || event_id.len() > 120 {
            return Err(Error::Invalid);
        }
        let event_id = format!("usage:{event_id}");
        let detail = json!({"kind":"usage","usage":usage});
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(&event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        live(&stored.charge, now(&mut tx).await?)?;
        if stored.charge.state != "streaming" {
            return Err(Error::WrongState);
        }
        Self::check_usage(&stored.charge, usage)?;
        if let Some(image) = usage.image {
            sqlx::query("UPDATE core_charging.requests SET input_tokens=$2,cached_tokens=$3,output_tokens=$4,image_usage=$5,revision=revision+1 WHERE id=$1")
                .bind(id).bind(usage.input).bind(usage.cached).bind(usage.output).bind(json(&image)?).execute(&mut *tx).await?;
        } else {
            sqlx::query("UPDATE core_charging.requests SET input_tokens=$2,cached_tokens=$3,output_tokens=$4,revision=revision+1 WHERE id=$1")
                .bind(id).bind(usage.input).bind(usage.cached).bind(usage.output).execute(&mut *tx).await?;
        }
        event(&mut tx, id, &event_id, detail).await?;
        let charge = load(&mut tx, id).await?.charge;
        commit(tx).await?;
        Ok(charge)
    }
    fn check_usage(charge: &Charge, usage: Usage) -> Result<i64> {
        if !usage.follows(charge.usage) {
            return Err(Error::Invalid);
        }
        let amount = charge.price.cost(usage)?;
        if amount > charge.reserved {
            return Err(Error::NeedReservation);
        }
        Ok(amount)
    }
    pub async fn heartbeat(&self, id: &str, worker: &[u8; 32], seconds: i64) -> Result<Charge> {
        if !(1..=300).contains(&seconds) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let at = now(&mut tx).await?;
        live(&stored.charge, at)?;
        if !matches!(stored.charge.state.as_str(), "reserved" | "streaming") {
            return Err(Error::WrongState);
        }
        let lease = stored
            .charge
            .lease_until
            .max(at + seconds)
            .min(stored.charge.hard_deadline);
        sqlx::query(
            "UPDATE core_charging.requests SET lease_until=$2,revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .bind(lease)
        .execute(&mut *tx)
        .await?;
        let charge = load(&mut tx, id).await?.charge;
        commit(tx).await?;
        Ok(charge)
    }
    /// Cancellation or failure AFTER dispatch must settle authoritative partial
    /// usage; it is not a zero-cost release. Old membership need not remain active.
    pub async fn settle(
        &self,
        id: &str,
        worker: &[u8; 32],
        usage: Usage,
        outcome: Outcome,
    ) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        if stored.charge.state == "settled" {
            if stored.charge.usage != usage
                || stored.charge.outcome.as_deref() != Some(outcome.text())
            {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        live(&stored.charge, now(&mut tx).await?)?;
        if stored.charge.state != "streaming" {
            return Err(Error::WrongState);
        }
        let charge = self
            .finish(&mut tx, stored.charge, usage, outcome, None)
            .await?;
        commit(tx).await?;
        Ok(charge)
    }
    async fn finish(
        &self,
        c: &mut PgConnection,
        charge: Charge,
        usage: Usage,
        outcome: Outcome,
        evidence: Option<&str>,
    ) -> Result<Charge> {
        let amount = Self::check_usage(&charge, usage)?;
        self.ledger
            .apply(c, command(&charge, "settle", Change::Settle { amount })?)
            .await?;
        if let Some(image) = usage.image {
            sqlx::query("UPDATE core_charging.requests SET state='settled',input_tokens=$2,cached_tokens=$3,output_tokens=$4,settled=$5,outcome=$6,image_usage=$7,revision=revision+1 WHERE id=$1")
                .bind(&charge.id).bind(usage.input).bind(usage.cached).bind(usage.output).bind(amount)
                .bind(outcome.text()).bind(json(&image)?).execute(&mut *c).await?;
        } else {
            sqlx::query("UPDATE core_charging.requests SET state='settled',input_tokens=$2,cached_tokens=$3,output_tokens=$4,settled=$5,outcome=$6,revision=revision+1 WHERE id=$1")
                .bind(&charge.id).bind(usage.input).bind(usage.cached).bind(usage.output).bind(amount)
                .bind(outcome.text()).execute(&mut *c).await?;
        }
        event(
            c,
            &charge.id,
            "final",
            json!({"kind":"final","usage":usage,"outcome":outcome,"evidence":evidence,
                "counted_api_activity":counted_api_activity(usage)}),
        )
        .await?;
        Ok(load(c, &charge.id).await?.charge)
    }
    /// Safe only before start(). After upstream dispatch, use settle/reconcile.
    pub async fn release(&self, id: &str, worker: &[u8; 32]) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        if stored.charge.state == "released" {
            commit(tx).await?;
            return Ok(stored.charge);
        }
        if stored.charge.state != "reserved" {
            return Err(Error::WrongState);
        }
        self.release_unstarted(&mut tx, &stored.charge).await?;
        let charge = load(&mut tx, id).await?.charge;
        commit(tx).await?;
        Ok(charge)
    }
    async fn release_unstarted(&self, c: &mut PgConnection, charge: &Charge) -> Result<()> {
        self.ledger
            .apply(c, command(charge, "release", Change::Release)?)
            .await?;
        sqlx::query(
            "UPDATE core_charging.requests SET state='released',revision=revision+1 WHERE id=$1",
        )
        .bind(&charge.id)
        .execute(&mut *c)
        .await?;
        event(
            c,
            &charge.id,
            "release",
            json!({"kind":"release_before_dispatch"}),
        )
        .await
    }
    /// Trusted Rust recovery service only; do not expose as a Go/client action.
    /// Evidence must identify an authoritative provider result, not an estimate.
    pub async fn reconcile(
        &self,
        id: &str,
        expected_revision: i64,
        usage: Usage,
        outcome: Outcome,
        evidence: &str,
    ) -> Result<Charge> {
        Self::evidence(evidence)?;
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        if stored.charge.state == "settled" {
            let previous: Value = sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'")
                .bind(id).fetch_one(&mut *tx).await?;
            if stored.charge.usage != usage
                || stored.charge.outcome.as_deref() != Some(outcome.text())
                || previous.get("evidence").and_then(Value::as_str) != Some(evidence)
            {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        if stored.charge.state != "reconcile" || stored.charge.revision != expected_revision {
            return Err(Error::WrongState);
        }
        let charge = self
            .finish(&mut tx, stored.charge, usage, outcome, Some(evidence))
            .await?;
        commit(tx).await?;
        Ok(charge)
    }
    /// Bounded recovery. Expired streams retain the entire hold, even with zero
    /// locally observed output. A process crash is not proof of no provider use.
    // @mark model.recovery role=authority
    pub async fn recover_expired(&self, limit: i64) -> Result<Vec<Charge>> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid);
        }
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM core_charging.requests WHERE state IN ('reserved','streaming') \
             AND lease_until<=floor(extract(epoch FROM clock_timestamp()))::bigint ORDER BY lease_until,id LIMIT $1",
        ).bind(limit).fetch_all(&self.pool).await?;
        let mut recovered = Vec::new();
        for id in ids {
            let mut tx = self.begin().await?;
            lock(&mut tx, &id).await?;
            let stored = load(&mut tx, &id).await?;
            if stored.charge.lease_until > now(&mut tx).await? {
                continue;
            }
            match stored.charge.state.as_str() {
                "reserved" => self.release_unstarted(&mut tx, &stored.charge).await?,
                "streaming" => {
                    sqlx::query("UPDATE core_charging.requests SET state='reconcile',revision=revision+1 WHERE id=$1")
                        .bind(&id).execute(&mut *tx).await?;
                    event(
                        &mut tx,
                        &id,
                        "recovery",
                        json!({"kind":"unknown_provider_usage","hold_retained":true}),
                    )
                    .await?;
                }
                _ => continue,
            }
            let charge = load(&mut tx, &id).await?.charge;
            commit(tx).await?;
            recovered.push(charge);
        }
        Ok(recovered)
    }
    /// Internal recovery read: READ COMMITTED intentionally takes a fresh
    /// snapshot AFTER the advisory lock. It waits for any unknown commit to
    /// resolve before reporting absence. Not an unauthenticated status endpoint.
    pub async fn resolve(&self, id: &str) -> Result<Option<Charge>> {
        let mut tx = self.pool.begin().await?;
        for statement in [
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            "SET LOCAL lock_timeout='2s'",
            "SET LOCAL statement_timeout='5s'",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        lock(&mut tx, id).await?;
        let result = select_stored(&mut tx, id, false).await?.map(|s| s.charge);
        commit(tx).await?;
        Ok(result)
    }
    /// Trusted refund service only. The original payer, subscription window and
    /// budget attribution stay fixed, even after key revocation/team removal.
    pub async fn refund(
        &self,
        id: &str,
        refund_id: &str,
        amount: i64,
        evidence: &str,
    ) -> Result<Charge> {
        Self::evidence(evidence)?;
        if !valid_id(refund_id) || amount <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        if let Some(row) = sqlx::query("SELECT amount,evidence FROM core_charging.refunds WHERE request_id=$1 AND refund_id=$2")
            .bind(id).bind(refund_id).fetch_optional(&mut *tx).await?
        {
            if row.try_get::<i64, _>("amount")? != amount || row.try_get::<String, _>("evidence")? != evidence {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        let charge = stored.charge;
        if charge.state != "settled" || amount > charge.settled - charge.refunded {
            return Err(Error::Invalid);
        }
        self.ledger
            .apply(
                &mut tx,
                command(
                    &charge,
                    &format!("refund:{refund_id}"),
                    Change::Refund { amount },
                )?,
            )
            .await?;
        sqlx::query("INSERT INTO core_charging.refunds(request_id,refund_id,amount,evidence) VALUES ($1,$2,$3,$4)")
            .bind(id).bind(refund_id).bind(amount).bind(evidence).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_charging.requests SET refunded=refunded+$2,revision=revision+1 WHERE id=$1")
            .bind(id).bind(amount).execute(&mut *tx).await?;
        let charge = load(&mut tx, id).await?.charge;
        commit(tx).await?;
        Ok(charge)
    }
    fn evidence(evidence: &str) -> Result<()> {
        if evidence.trim().is_empty() || evidence.len() > 500 {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
    /// Budget changes keep stable scope/issuer/period identity. Changing a cap
    /// never clears history. A second issuer adds a constraint, not an override.
    pub async fn set_budget(&self, session: &str, budget: &Budget) -> Result<i64> {
        budget.validate()?;
        let mut tx = self.begin().await?;
        let actor = authority::authenticate(&mut tx, session).await?;
        authority::manage_budget(&mut tx, &actor, budget).await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO core_charging.budgets(scope,account_id,user_id,key_id,issuer_user_id,period,anchor,seconds,limit_credits) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
             ON CONFLICT (scope,account_id,user_id,key_id,issuer_user_id,period,anchor,seconds) \
             DO UPDATE SET limit_credits=EXCLUDED.limit_credits,unlimited=FALSE,display_limit_credits=NULL,revision=core_charging.budgets.revision+1 RETURNING id",
        )
        .bind(&budget.scope).bind(budget.account_id).bind(budget.user_id).bind(budget.key_id)
        .bind(actor.user).bind(&budget.period).bind(budget.anchor).bind(budget.seconds).bind(budget.limit)
        .fetch_one(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO core_charging.policy_events(actor_user_id,budget_id,detail) VALUES ($1,$2,$3)",
        )
        .bind(actor.user)
        .bind(id)
        .bind(json(budget)?)
        .execute(&mut *tx)
        .await?;
        commit(tx).await?;
        Ok(id)
    }
}
