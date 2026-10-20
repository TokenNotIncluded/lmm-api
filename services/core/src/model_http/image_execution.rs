//! Core-owned drawing confirmation, dispatch and receipt recovery. Go never
//! supplies a payer, tariff, provider URL, usage or a successful-payment flag.
#[path = "image_api_execution.rs"]
mod api_execution;
#[path = "image_drawing_input.rs"]
mod drawing_input;
use super::{
    image_price::{ImageEndpoint, ImagePriceConfig, ImageQuote, ImageQuoteRequest, ImageUsage},
    image_provider::ImageProvider,
};
use crate::{
    billing::charging::{Charge, Charging, Outcome, Price, Request, Usage},
    identity::{self, BrowserAuth, CredentialKind},
    relay::Limits,
    tool_market::{
        credit_access::CreditStore,
        credit_execution::{BuiltinContext, BuiltinKind},
    },
};
pub(crate) use drawing_input::MAX_PREVIEW_BODY;
pub use drawing_input::{DrawingPreviewInput, DrawingUpload, ReferenceImage};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use std::{collections::BTreeMap, sync::Arc};
use subtle::ConstantTimeEq;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Missing,
    Conflict,
    Pending,
    NoFunds,
    Retry,
    Storage,
}
type Result<T> = std::result::Result<T, Error>;
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match e.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01" | "55P03") => Self::Retry,
            Some("23505") => Self::Conflict,
            _ => Self::Storage,
        }
    }
}
impl From<crate::billing::charging::Error> for Error {
    fn from(e: crate::billing::charging::Error) -> Self {
        use crate::billing::charging::Error as E;
        match e {
            E::Invalid => Self::Invalid,
            E::Unauthorized => Self::Unauthorized,
            E::Forbidden => Self::Forbidden,
            E::NotFound => Self::Missing,
            E::NoFunds | E::BudgetExceeded => Self::NoFunds,
            E::Retry => Self::Retry,
            E::Conflict => Self::Conflict,
            E::WrongState | E::StaleWorker | E::CommitUnknown => Self::Pending,
            _ => Self::Storage,
        }
    }
}
fn identity_error(e: identity::IdentityError) -> Error {
    match e {
        identity::IdentityError::Unauthorized => Error::Unauthorized,
        identity::IdentityError::Forbidden => Error::Forbidden,
        identity::IdentityError::Invalid => Error::Invalid,
        identity::IdentityError::Conflict => Error::Retry,
        identity::IdentityError::Storage => Error::Storage,
    }
}
fn market_error(e: crate::tool_market::Error) -> Error {
    use crate::tool_market::Error as E;
    match e {
        E::Invalid => Error::Invalid,
        E::Unauthorized => Error::Unauthorized,
        E::Forbidden => Error::Forbidden,
        E::Conflict => Error::Conflict,
        E::Missing => Error::Missing,
        E::Pending => Error::Pending,
        E::Retry => Error::Retry,
        E::Storage => Error::Storage,
    }
}

/// Installation-owned material; deliberately not Debug or serde.
pub(crate) struct ProviderSpec {
    pub upstream_id: u64,
    pub vendor: String,
    pub upstream_model: String,
    pub base_url: String,
    pub secret: String,
    pub local_http: bool,
}
#[derive(Clone)]
pub struct ImageService {
    pool: PgPool,
    charging: Charging,
    prices: Arc<ImagePriceConfig>,
    providers: Arc<BTreeMap<u64, ImageProvider>>,
    gate: Arc<tokio::sync::Semaphore>,
    parallelism: usize,
    credential_digests: Arc<Vec<[u8; 32]>>,
}
impl ImageService {
    /// Host readiness only: no user admission, provider dispatch or funds write.
    pub(crate) async fn ready(&self) -> Result<()> {
        if self.providers.is_empty() || self.prices.rules.is_empty() || self.gate.is_closed() {
            return Err(Error::Pending);
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            crate::runtime_database::check(
                &self.pool,
                crate::runtime_database::RequiredCapabilities {
                    models: true,
                    images: true,
                    ..Default::default()
                },
            ),
        )
        .await
        .map_err(|_| Error::Pending)?
        .map_err(|_| Error::Pending)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DrawingInput {
    pub prompt: String,
    pub model: String,
    pub group: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default = "one")]
    pub n: u32,
}
fn one() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub operation_id: String,
    pub confirmation_path: String,
    pub preview_digest: String,
    pub input: DrawingPreviewInput,
    pub reservation_credits: i64,
    pub expires_at: i64,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub struct Capability {
    pub endpoint: ImageEndpoint,
    pub max_reference_images: u32,
    pub max_reference_bytes: u64,
    pub max_total_reference_bytes: u64,
    pub model: String,
    pub group: String,
    pub size: String,
    pub quality: String,
    pub max_n: u32,
    pub credits_per_image: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Receipt {
    pub operation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub state: String,
    pub preview_digest: String,
    pub reservation_credits: i64,
    pub charged_credits: Option<i64>,
    /// Present only after the same Core charge is durably settled.
    pub output: Option<Value>,
}
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        v => v.clone(),
    }
}
fn digest<T: Serialize>(v: &T) -> Result<[u8; 32]> {
    let v = serde_json::to_value(v).map_err(|_| Error::Invalid)?;
    Ok(Sha256::digest(serde_json::to_vec(&canonical(&v)).map_err(|_| Error::Invalid)?).into())
}
fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}
fn random() -> Result<[u8; 32]> {
    let mut b = [0; 32];
    OsRng.try_fill_bytes(&mut b).map_err(|_| Error::Storage)?;
    Ok(b)
}
fn valid_id(id: &str) -> bool {
    id.strip_prefix("image:").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}
async fn begin(pool: &PgPool) -> Result<Transaction<'_, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='250ms'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='1500ms'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
fn price(quote: &ImageQuote) -> Price {
    Price {
        image: Some(quote.persisted()),
        upstream_cost_group_ppm: None,
        tiered: None,
        version: format!("image-{}", quote.revision()),
        model: quote.binding().semantic_model.clone(),
        group: quote.binding().billing_group.clone(),
        input_per_million: 0,
        cached_per_million: 0,
        output_per_million: 0,
    }
}
impl ImageService {
    pub(crate) fn new(
        pool: PgPool,
        charging: Charging,
        prices: ImagePriceConfig,
        specs: Vec<ProviderSpec>,
        limits: &Limits,
    ) -> Result<Self> {
        prices.validate().map_err(|_| Error::Invalid)?;
        if prices.rules.is_empty() || prices.rules.len() > 1024 || limits.max_in_flight == 0 {
            return Err(Error::Invalid);
        }
        let mut credential_digests = Vec::new();
        let mut providers = BTreeMap::new();
        for s in specs {
            credential_digests.push(Sha256::digest(s.secret.as_bytes()).into());
            if prices.rules.iter().any(|rule| {
                rule.upstream_id == s.upstream_id
                    && (rule.vendor != s.vendor || rule.upstream_model != s.upstream_model)
            }) {
                return Err(Error::Invalid);
            }
            let provider = ImageProvider::new_with_local_http(
                s.upstream_id,
                &s.vendor,
                &s.upstream_model,
                &s.base_url,
                &s.secret,
                limits,
                s.local_http,
            )
            .map_err(|_| Error::Invalid)?;
            if providers.insert(s.upstream_id, provider).is_some() {
                return Err(Error::Invalid);
            }
        }
        for rule in &prices.rules {
            if !providers.contains_key(&rule.upstream_id) {
                return Err(Error::Invalid);
            }
        }
        Ok(Self {
            pool,
            charging,
            prices: Arc::new(prices),
            providers: Arc::new(providers),
            gate: Arc::new(tokio::sync::Semaphore::new(limits.max_in_flight.min(2))),
            parallelism: limits.max_in_flight.min(2),
            credential_digests: Arc::new(credential_digests),
        })
    }
    pub(crate) async fn drain(&self, grace: std::time::Duration) -> usize {
        self.gate.close();
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let remaining = self
                .parallelism
                .saturating_sub(self.gate.available_permits());
            if remaining == 0 || tokio::time::Instant::now() >= deadline {
                return remaining;
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + std::time::Duration::from_millis(20)),
            )
            .await;
        }
    }
    pub(crate) fn catalog(&self, group: &str) -> Vec<String> {
        self.prices
            .rules
            .iter()
            .filter(|r| r.billing_group == group)
            .map(|r| r.semantic_model.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    /// Safe immutable tariff metadata only; never provider identity or credentials.
    pub(crate) fn pricing_projection(
        &self,
    ) -> std::result::Result<
        (u64, Vec<(String, String, super::pricing::ImageVariant)>),
        super::pricing::PricingError,
    > {
        let mut counts = BTreeMap::new();
        for rule in &self.prices.rules {
            *counts
                .entry((
                    &rule.billing_group,
                    &rule.semantic_model,
                    rule.endpoint,
                    &rule.size,
                    &rule.quality,
                ))
                .or_insert(0_usize) += 1;
        }
        let mut result = Vec::new();
        for rule in &self.prices.rules {
            // Configured dimensions are normalized by ImageService::new. More
            // than one supplier binding is the same ambiguity quote_for rejects.
            if counts[&(
                &rule.billing_group,
                &rule.semantic_model,
                rule.endpoint,
                &rule.size,
                &rule.quality,
            )] != 1
            {
                continue;
            }
            let config = ImagePriceConfig {
                revision: self.prices.revision,
                rules: vec![rule.clone()],
            };
            let quote = config
                .quote(&ImageQuoteRequest {
                    upstream_id: rule.upstream_id,
                    vendor: rule.vendor.clone(),
                    billing_group: rule.billing_group.clone(),
                    upstream_model: rule.upstream_model.clone(),
                    semantic_model: rule.semantic_model.clone(),
                    endpoint: rule.endpoint,
                    size: rule.size.clone(),
                    quality: rule.quality.clone(),
                    n: 1,
                })
                .map_err(|_| super::pricing::PricingError::Unavailable)?;
            result.push((
                rule.billing_group.clone(),
                rule.semantic_model.clone(),
                super::pricing::image_variant(rule, &quote)?,
            ));
        }
        Ok((self.prices.revision, result))
    }
    pub(crate) fn contains_credential(&self, secret: &str) -> bool {
        let digest: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
        let mut matched = subtle::Choice::from(0);
        for existing in self.credential_digests.iter() {
            matched |= existing.ct_eq(&digest)
        }
        bool::from(matched)
    }
    fn drawing_quote(&self, input: &DrawingPreviewInput) -> Result<ImageQuote> {
        if input.prompt.chars().count() > 2000 || !(1..=4).contains(&input.n) {
            return Err(Error::Invalid);
        }
        self.quote_for(
            &input.settings,
            if input.reference_images.is_empty() {
                ImageEndpoint::Generations
            } else {
                ImageEndpoint::Edits
            },
        )
    }
    fn quote_for(&self, input: &DrawingInput, endpoint: ImageEndpoint) -> Result<ImageQuote> {
        if input.prompt.is_empty()
            || input.prompt.len() > 64 * 1024
            || input.prompt.contains('\0')
            || !(1..=128).contains(&input.n)
        {
            return Err(Error::Invalid);
        }
        let mut matches = Vec::new();
        for r in &self.prices.rules {
            if r.semantic_model != input.model
                || r.billing_group != input.group
                || r.endpoint != endpoint
            {
                continue;
            }
            let request = ImageQuoteRequest {
                upstream_id: r.upstream_id,
                vendor: r.vendor.clone(),
                billing_group: input.group.clone(),
                upstream_model: r.upstream_model.clone(),
                semantic_model: input.model.clone(),
                endpoint,
                size: input.size.clone(),
                quality: input.quality.clone(),
                n: input.n,
            };
            if let Ok(q) = self.prices.quote(&request) {
                if !matches.contains(&q) {
                    matches.push(q)
                }
            }
        }
        if matches.len() != 1 {
            return Err(Error::Invalid);
        }
        Ok(matches.remove(0))
    }
    pub async fn capabilities(
        &self,
        session: &str,
        context: &BuiltinContext,
    ) -> Result<Vec<Capability>> {
        let mut tx = begin(&self.pool).await?;
        let a = identity::authorize_in_transaction(&mut tx, session)
            .await
            .map_err(identity_error)?;
        if a.credential_kind != CredentialKind::Session {
            return Err(Error::Forbidden);
        }
        CreditStore::validate_builtin_context(
            &mut tx,
            session,
            context,
            BuiltinKind::code_owned("drawing", "list_capabilities").map_err(market_error)?,
        )
        .await
        .map_err(market_error)?;
        let groups:Vec<String>=sqlx::query_scalar("SELECT group_name FROM core_identity.user_routing_groups WHERE user_id=$1 AND enabled ORDER BY group_name").bind(a.user_id).fetch_all(&mut *tx).await?;
        let mut out = Vec::new();
        for rule in &self.prices.rules {
            if !groups.contains(&rule.billing_group) {
                continue;
            }
            let input = DrawingInput {
                prompt: "capability quote".into(),
                model: rule.semantic_model.clone(),
                group: rule.billing_group.clone(),
                size: rule.size.clone(),
                quality: rule.quality.clone(),
                n: 1,
            };
            if let Ok(quote) = self.quote_for(&input, rule.endpoint) {
                let edits = rule.endpoint == ImageEndpoint::Edits;
                let item = Capability {
                    endpoint: rule.endpoint,
                    max_reference_images: if edits {
                        drawing_input::MAX_REFERENCES as u32
                    } else {
                        0
                    },
                    max_reference_bytes: if edits {
                        drawing_input::MAX_REFERENCE_BYTES as u64
                    } else {
                        0
                    },
                    max_total_reference_bytes: if edits {
                        drawing_input::MAX_TOTAL_REFERENCE_BYTES as u64
                    } else {
                        0
                    },
                    model: input.model,
                    group: input.group,
                    size: input.size,
                    quality: input.quality,
                    max_n: rule.max_n.min(4),
                    credits_per_image: quote.reservation_credits().into(),
                };
                if !out.contains(&item) {
                    out.push(item)
                }
            }
        }
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok(out)
    }
    /// One deterministic operation per original Core market dispatch. Raw input,
    /// including omitted defaults, is hashed exactly as the original tool grant.
    pub async fn preview(
        &self,
        session: &str,
        context: &BuiltinContext,
        input: Value,
    ) -> Result<Preview> {
        self.preview_with_uploads(session, context, input, vec![])
            .await
    }
    pub async fn preview_with_uploads(
        &self,
        session: &str,
        context: &BuiltinContext,
        input: Value,
        uploads: Vec<DrawingUpload>,
    ) -> Result<Preview> {
        let _permit = self
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Pending)?;
        let parsed = drawing_input::parse_arguments(&input)?;
        // Bytes are checked before a quote can be confirmed. The immutable input
        // digest remains the exact original market arguments, not this sidecar.
        if crate::builtins::input_digest(&input).map_err(|_| Error::Invalid)?
            != context.input_digest
        {
            return Err(Error::Forbidden);
        }
        let quote = self.drawing_quote(&parsed)?;
        let frozen = price(&quote);
        let mut tx = begin(&self.pool).await?;
        let actor =
            identity::authorize_assistant_model_in_transaction(&mut tx, session, &parsed.group)
                .await
                .map_err(identity_error)?;
        let builtin = CreditStore::validate_builtin_context(
            &mut tx,
            session,
            context,
            BuiltinKind::code_owned("drawing", "generate").map_err(market_error)?,
        )
        .await
        .map_err(market_error)?;
        if builtin.user_id != actor.user_id || builtin.owner_account_id != actor.owner_account_id {
            return Err(Error::Forbidden);
        }
        drawing_input::authorize_references(
            &input,
            &builtin.version_id,
            &builtin.input_schema_digest,
        )?;
        let stored_input = if uploads.is_empty() {
            drawing_input::freeze(input.clone(), uploads)?
        } else {
            let original = input.clone();
            tokio::task::spawn_blocking(move || drawing_input::freeze(original, uploads))
                .await
                .map_err(|_| Error::Pending)??
        };
        let id = format!(
            "image:{}",
            hex(&digest(&json!([
                actor.credential_id,
                context.call_id,
                context.dispatch_id
            ]))?)
        );
        let source = api_execution::SourceKey::Drawing {
            credential_id: actor.credential_id,
        };
        // Metadata includes Core-verified content SHA/length/MIME. Hashing the
        // original small arguments binds the bytes without copying base64 into
        // several canonicalization buffers under the process memory limit.
        let fingerprint = digest(&json!([source, context, input, frozen]))?;
        let at = now(&mut tx).await?;
        let expires_at:i64=sqlx::query_scalar("SELECT LEAST(c.resolve_by,(g.request->>'expires_at')::bigint,$2) FROM core_tool_market.credit_calls c JOIN core_tool_market.credit_consents g ON g.id=c.consent_id WHERE c.id=$1")
            .bind(&builtin.call_id).bind(at+600).fetch_one(&mut *tx).await?;
        if expires_at <= at {
            return Err(Error::Forbidden);
        }
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO core_image.operations(id,session_id,actor_user_id,owner_account_id,auth_version,context,input,fingerprint,price,state,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'previewed',$10,$11) ON CONFLICT(id) DO NOTHING")
            .bind(&id).bind(actor.credential_id).bind(actor.user_id).bind(actor.owner_account_id).bind(version).bind(json!(context)).bind(&stored_input).bind(fingerprint.as_slice()).bind(json!(frozen)).bind(at).bind(expires_at).execute(&mut *tx).await?;
        let row = sqlx::query(
            "SELECT fingerprint,expires_at,state FROM core_image.operations WHERE id=$1 FOR SHARE",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        if row.try_get::<Vec<u8>, _>("fingerprint")? != fingerprint {
            return Err(Error::Conflict);
        }
        let expires_at = row.try_get("expires_at")?;
        let state = row.try_get("state")?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        let binding = quote.binding();
        Ok(Preview {
            confirmation_path: format!("/core/v1/images/confirm/{id}"),
            operation_id: id,
            preview_digest: hex(&fingerprint),
            input: DrawingPreviewInput {
                settings: DrawingInput {
                    size: binding.size.clone(),
                    quality: binding.quality.clone(),
                    n: binding.n,
                    ..parsed.settings
                },
                reference_images: parsed.reference_images,
            },
            reservation_credits: quote.reservation_credits().into(),
            expires_at,
            state,
        })
    }
    /// Cookie identity, never Session bearer, authorizes the explicit browser page.
    pub(crate) async fn confirmation_page(
        &self,
        cookie: &str,
        id: &str,
    ) -> Result<(Preview, String)> {
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let mut tx = begin(&self.pool).await?;
        let a = BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie)
            .await
            .map_err(identity_error)?;
        let r = sqlx::query(
            "SELECT id,session_id,actor_user_id,owner_account_id,auth_version,price,fingerprint,state,created_at,expires_at,confirmation_digest,confirmation_sid,settled_credits,output,CASE WHEN input->>'kind'='drawing_reference_v1' THEN input->'arguments' ELSE input END AS input FROM core_image.operations WHERE id=$1 AND actor_user_id=$2 AND source_kind='drawing' FOR UPDATE",
        )
        .bind(id)
        .bind(a.user)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Missing)?;
        let at = now(&mut tx).await?;
        if r.try_get::<i64, _>("auth_version")? != a.version
            || r.try_get::<i64, _>("owner_account_id")? != a.account
            || r.try_get::<i64, _>("expires_at")? <= at
            || r.try_get::<String, _>("state")? != "previewed"
        {
            return Err(Error::Conflict);
        }
        let p = preview_row(&r)?;
        let csrf = hex(&random()?);
        sqlx::query("UPDATE core_image.operations SET confirmation_digest=$2,confirmation_sid=$3 WHERE id=$1").bind(id).bind(Sha256::digest(csrf.as_bytes()).as_slice()).bind(a.sid).execute(&mut *tx).await?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok((p, csrf))
    }
    pub(crate) async fn confirm(&self, cookie: &str, id: &str, csrf: &str) -> Result<Receipt> {
        if !valid_id(id) || csrf.len() != 64 {
            return Err(Error::Invalid);
        }
        let mut tx = begin(&self.pool).await?;
        let a = BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie)
            .await
            .map_err(identity_error)?;
        let r = sqlx::query(
            "SELECT id,session_id,actor_user_id,owner_account_id,auth_version,price,fingerprint,state,created_at,expires_at,confirmation_digest,confirmation_sid,settled_credits,output,CASE WHEN input->>'kind'='drawing_reference_v1' THEN input->'arguments' ELSE input END AS input FROM core_image.operations WHERE id=$1 AND actor_user_id=$2 AND source_kind='drawing' FOR UPDATE",
        )
        .bind(id)
        .bind(a.user)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Missing)?;
        let at = now(&mut tx).await?;
        let expected: Option<Vec<u8>> = r.try_get("confirmation_digest")?;
        if r.try_get::<i64, _>("auth_version")? != a.version
            || r.try_get::<i64, _>("owner_account_id")? != a.account
            || r.try_get::<i64, _>("expires_at")? <= at
            || r.try_get::<Option<String>, _>("confirmation_sid")? != Some(a.sid)
            || !expected.is_some_and(|d| d.ct_eq(Sha256::digest(csrf.as_bytes()).as_slice()).into())
        {
            return Err(Error::Forbidden);
        }
        let state: String = r.try_get("state")?;
        if state == "previewed" {
            sqlx::query(
                "UPDATE core_image.operations SET state='confirmed',confirmed_at=$2 WHERE id=$1",
            )
            .bind(id)
            .bind(at)
            .execute(&mut *tx)
            .await?;
        } else if state != "confirmed" {
            return Err(Error::Conflict);
        }
        let mut out = receipt_row(&r)?;
        out.state = "confirmed".into();
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok(out)
    }
    /// The opaque operation identifies persisted browser confirmation. No Go bool
    /// or supplied receipt is accepted as authority. Replay queries the same id.
    pub async fn execute(&self, session: &str, id: &str) -> Result<Receipt> {
        let service = self.clone();
        let session = session.to_owned();
        let id = id.to_owned();
        // Detach from browser/Go connection cancellation after admission: the
        // original worker must finish recording and paying for an attempted POST.
        tokio::spawn(async move { service.execute_owned(&session, &id).await })
            .await
            .map_err(|_| Error::Pending)?
    }
    async fn execute_owned(&self, session: &str, id: &str) -> Result<Receipt> {
        let _permit = self
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Pending)?;
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let (quote, prompt, fingerprint, edits) = self.prepare_execution(session, id).await?;
        let provider = self
            .providers
            .get(&quote.binding().upstream_id)
            .ok_or(Error::Invalid)?;
        let prepared = if let Some(edits) = edits {
            provider.prepare_edits(&quote, &prompt, edits).await
        } else {
            provider.prepare(&quote, &prompt).await
        }
        .map_err(|_| Error::Pending)?;
        let worker = random()?;
        let spec = Request {
            id: id.into(),
            fingerprint,
            price: price(&quote),
            reserve: quote.reservation_credits().into(),
            lease_seconds: 300,
            maximum_seconds: 300,
        };
        let mut tx = begin(&self.pool).await?;
        self.charging
            .reserve_image(&mut tx, session, id, &spec, &worker)
            .await?;
        self.charging
            .start_image(&mut tx, session, id, &worker)
            .await?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        let result = async {
        // From this point, no path may release or dispatch again after ambiguity.
        let observation = prepared.dispatch().await.map_err(|_| Error::Pending)?;
        let image = ImageUsage {
            output_images: observation.output_images,
            provider_receipt_sha256: observation.provider_receipt_sha256,
        };
        quote
            .settlement(image.output_images)
            .map_err(|_| Error::Pending)?;
        let output_digest = digest(&observation.data)?;
        let mut tx = begin(&self.pool).await?;
        let changed=sqlx::query("UPDATE core_image.operations SET state='billing_pending',output=$2,output_digest=$3,image_usage=$4 WHERE id=$1 AND state='dispatching' AND charge_id=$1 AND output IS NULL")
            .bind(id).bind(observation.data).bind(output_digest.as_slice()).bind(json!(image)).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Pending);
        }
        tx.commit().await.map_err(|_| Error::Pending)?;
        let charge = self
            .charging
            .settle(
                id,
                &worker,
                Usage {
                    image: Some(image),
                    ..Usage::default()
                },
                Outcome::Completed,
            )
            .await?;
        self.complete(&charge).await?;
        self.query(session, id).await
        }.await;
        if result.is_err() {
            self.charging.mark_unknown(id, &worker).await?;
        }
        result
    }
    async fn prepare_execution(
        &self,
        session: &str,
        id: &str,
    ) -> Result<(
        ImageQuote,
        String,
        [u8; 32],
        Option<super::image_provider::ImageMultipartInput>,
    )> {
        let mut tx = begin(&self.pool).await?;
        let r = sqlx::query(
            "SELECT * FROM core_image.operations WHERE id=$1 AND source_kind='drawing' FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Missing)?;
        let frozen: Price =
            serde_json::from_value(r.try_get("price")?).map_err(|_| Error::Storage)?;
        let a = identity::authorize_assistant_model_in_transaction(&mut tx, session, &frozen.group)
            .await
            .map_err(identity_error)?;
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(a.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if r.try_get::<i64, _>("session_id")? != a.credential_id
            || r.try_get::<i64, _>("actor_user_id")? != a.user_id
            || r.try_get::<i64, _>("owner_account_id")? != a.owner_account_id
            || r.try_get::<i64, _>("auth_version")? != version
        {
            return Err(Error::Forbidden);
        }
        if r.try_get::<String, _>("state")? != "confirmed"
            || r.try_get::<i64, _>("expires_at")? <= now(&mut tx).await?
        {
            return Err(Error::Pending);
        }
        let context: BuiltinContext =
            serde_json::from_value(r.try_get("context")?).map_err(|_| Error::Storage)?;
        CreditStore::validate_builtin_context(
            &mut tx,
            session,
            &context,
            BuiltinKind::code_owned("drawing", "generate").map_err(market_error)?,
        )
        .await
        .map_err(market_error)?;
        let stored: Value = r.try_get("input")?;
        let raw = drawing_input::arguments(&stored)?;
        if crate::builtins::input_digest(&raw).map_err(|_| Error::Storage)? != context.input_digest
        {
            return Err(Error::Storage);
        }
        let input = drawing_input::parse_arguments(&raw).map_err(|_| Error::Storage)?;
        let quote = frozen
            .image
            .ok_or(Error::Storage)?
            .restore()
            .map_err(|_| Error::Storage)?;
        // Current server pricing must still be the exact confirmed revision.
        if self.drawing_quote(&input)? != quote {
            return Err(Error::Conflict);
        }
        let fp: Vec<u8> = r.try_get("fingerprint")?;
        let fp = fp.try_into().map_err(|_| Error::Storage)?;
        let edits = tokio::task::spawn_blocking(move || drawing_input::restore(&stored))
            .await
            .map_err(|_| Error::Pending)??;
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok((quote, input.settings.prompt, fp, edits))
    }
    /// ACK-loss recovery is read only: derive the original id from the current
    /// real Session and Core call, without creating a preview or another send.
    pub async fn query_by_context(
        &self,
        session: &str,
        context: &BuiltinContext,
    ) -> Result<Receipt> {
        let mut tx = begin(&self.pool).await?;
        let actor = identity::authorize_in_transaction(&mut tx, session)
            .await
            .map_err(identity_error)?;
        if actor.credential_kind != CredentialKind::Session {
            return Err(Error::Forbidden);
        }
        let id = format!(
            "image:{}",
            hex(&digest(&json!([
                actor.credential_id,
                context.call_id,
                context.dispatch_id
            ]))?)
        );
        tx.commit().await.map_err(|_| Error::Pending)?;
        self.query_for_context(session, &id, context).await
    }
    pub async fn query_for_context(
        &self,
        session: &str,
        id: &str,
        context: &BuiltinContext,
    ) -> Result<Receipt> {
        let receipt = self.query(session, id).await?;
        let stored: Value = sqlx::query_scalar(
            "SELECT context FROM core_image.operations WHERE id=$1 AND source_kind='drawing'",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let stored: BuiltinContext = serde_json::from_value(stored).map_err(|_| Error::Storage)?;
        if stored != *context {
            return Err(Error::Forbidden);
        }
        Ok(receipt)
    }
    pub async fn execute_for_context(
        &self,
        session: &str,
        id: &str,
        context: &BuiltinContext,
    ) -> Result<Receipt> {
        let receipt = self.query_for_context(session, id, context).await?;
        if receipt.state != "confirmed" {
            return Ok(receipt);
        }
        self.execute(session, id).await
    }
    pub async fn query(&self, session: &str, id: &str) -> Result<Receipt> {
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let mut tx = begin(&self.pool).await?;
        let a = identity::authorize_in_transaction(&mut tx, session)
            .await
            .map_err(identity_error)?;
        if a.credential_kind != CredentialKind::Session {
            return Err(Error::Forbidden);
        }
        let r=sqlx::query("SELECT id,session_id,actor_user_id,owner_account_id,auth_version,price,fingerprint,state,created_at,expires_at,confirmation_digest,confirmation_sid,settled_credits,output,CASE WHEN input->>'kind'='drawing_reference_v1' THEN input->'arguments' ELSE input END AS input FROM core_image.operations WHERE id=$1 AND actor_user_id=$2 AND owner_account_id=$3 AND session_id=$4 AND source_kind='drawing' FOR SHARE").bind(id).bind(a.user_id).bind(a.owner_account_id).bind(a.credential_id).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        let out = receipt_row(&r)?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok(out)
    }
    async fn complete(&self, charge: &Charge) -> Result<()> {
        if charge.state != "settled" || charge.outcome.as_deref() != Some("completed") {
            return Err(Error::Pending);
        }
        let image = charge.usage.image.ok_or(Error::Storage)?;
        let mut tx = begin(&self.pool).await?;
        let r=sqlx::query("SELECT price,image_usage,output,output_digest,state FROM core_image.operations WHERE id=$1 AND charge_id=$1 FOR UPDATE").bind(&charge.id).fetch_one(&mut *tx).await?;
        let stored: Price =
            serde_json::from_value(r.try_get("price")?).map_err(|_| Error::Storage)?;
        let usage: ImageUsage =
            serde_json::from_value(r.try_get("image_usage")?).map_err(|_| Error::Storage)?;
        let output: Value = r.try_get("output")?;
        if stored != charge.price
            || usage != image
            || r.try_get::<Vec<u8>, _>("output_digest")? != digest(&output)?
            || stored.cost(charge.usage)? != charge.settled
        {
            return Err(Error::Storage);
        }
        sqlx::query("UPDATE core_image.operations SET state='completed',settled_credits=$2 WHERE id=$1 AND state IN ('billing_pending','completed')").bind(&charge.id).bind(charge.settled).execute(&mut *tx).await?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok(())
    }
    /// Trusted host recovery only. A stored provider receipt can finish payment;
    /// a missing receipt always retains UNKNOWN and can never trigger a new POST.
    pub(crate) async fn recover_receipts(&self, limit: i64) -> Result<usize> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid);
        }
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_image.operations WHERE state='billing_pending' ORDER BY created_at,id LIMIT $1").bind(limit).fetch_all(&self.pool).await?;
        let mut finished = 0;
        for id in ids {
            let Some(mut charge) = self.charging.resolve(&id).await? else {
                return Err(Error::Storage);
            };
            if charge.state == "reconcile" {
                let r=sqlx::query("SELECT image_usage,output,output_digest FROM core_image.operations WHERE id=$1 AND state='billing_pending'").bind(&id).fetch_one(&self.pool).await?;
                let image: ImageUsage = serde_json::from_value(r.try_get("image_usage")?)
                    .map_err(|_| Error::Storage)?;
                let output: Value = r.try_get("output")?;
                if r.try_get::<Vec<u8>, _>("output_digest")? != digest(&output)? {
                    return Err(Error::Storage);
                }
                charge = self
                    .charging
                    .reconcile(
                        &id,
                        charge.revision,
                        Usage {
                            image: Some(image),
                            ..Usage::default()
                        },
                        Outcome::Completed,
                        &format!(
                            "image-provider-sha256:{}",
                            hex(&image.provider_receipt_sha256)
                        ),
                    )
                    .await?;
            }
            if charge.state == "settled" {
                self.complete(&charge).await?;
                finished += 1;
            }
        }
        Ok(finished)
    }
}
/// Private storage projection used by the same-transaction image charge guard.
/// It never accepts a caller envelope: the argument is the immutable Core row.
pub(crate) fn authorize_drawing_reference_contract(
    input: &Value,
    version: &str,
    schema: &str,
) -> Result<()> {
    drawing_input::authorize_references(input, version, schema)
}
pub(crate) fn confirmed_drawing_arguments(stored: &Value) -> Result<Value> {
    let arguments = drawing_input::arguments(stored)?;
    drawing_input::parse_arguments(&arguments).map_err(|_| Error::Storage)?;
    Ok(arguments)
}
fn preview_row(r: &sqlx::postgres::PgRow) -> Result<Preview> {
    let frozen: Price = serde_json::from_value(r.try_get("price")?).map_err(|_| Error::Storage)?;
    let quote = frozen
        .image
        .ok_or(Error::Storage)?
        .restore()
        .map_err(|_| Error::Storage)?;
    let stored: Value = r.try_get("input")?;
    let mut input = drawing_input::parse_arguments(&drawing_input::arguments(&stored)?)
        .map_err(|_| Error::Storage)?;
    input.settings.size = quote.binding().size.clone();
    input.settings.quality = quote.binding().quality.clone();
    input.settings.n = quote.binding().n;
    Ok(Preview {
        confirmation_path: format!("/core/v1/images/confirm/{}", r.try_get::<String, _>("id")?),
        operation_id: r.try_get("id")?,
        preview_digest: hex(&r.try_get::<Vec<u8>, _>("fingerprint")?),
        input,
        reservation_credits: quote.reservation_credits().into(),
        expires_at: r.try_get("expires_at")?,
        state: r.try_get("state")?,
    })
}
fn receipt_row(r: &sqlx::postgres::PgRow) -> Result<Receipt> {
    let p = preview_row(r)?;
    let completed = p.state == "completed";
    Ok(Receipt {
        operation_id: p.operation_id,
        expires_at: Some(p.expires_at),
        state: p.state,
        preview_digest: p.preview_digest,
        reservation_credits: p.reservation_credits,
        charged_credits: if completed {
            r.try_get("settled_credits")?
        } else {
            None
        },
        output: if completed {
            r.try_get("output")?
        } else {
            None
        },
    })
}

#[cfg(test)]
#[path = "image_execution_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "image_drawing_tests.rs"]
mod drawing_tests;
