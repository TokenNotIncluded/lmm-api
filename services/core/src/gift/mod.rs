//! Core promotional gifts. No HTTP request accepts an amount or a recipient.
//! Qualification is independent of an editable profile email. Missing proof fails closed.
use crate::{
    billing::{
        Credits,
        ledger::{Action, Outcome, Request, Wallet},
    },
    identity::{self, CredentialKind},
};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, types::Json};

pub const SCHEMA: &str = include_str!("../../schema/gift.sql");
#[cfg(test)]
mod tests;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
}
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if e.as_database_error()
            .is_some_and(|e| matches!(e.code().as_deref(), Some("23505" | "40001" | "40P01")))
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
            _ => Self::Unavailable,
        }
    }
}
/// Constructed only from a strictly decoded tool call in Core's model transport.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GiftDecision {
    pub amount_credits: i64,
    pub reason: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub gift_id: String,
    pub owner_user_id: i64,
    pub credit_amount: i64,
    pub created_at_unix: i64,
    pub claimed_at_unix: Option<i64>,
    pub ledger_transaction_id: Option<i64>,
    pub max_credit_amount: i64,
    pub claim_available: bool,
    pub claim_blocked_code: String,
}
#[derive(Clone)]
pub struct GiftStore {
    pool: PgPool,
}
impl GiftStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn get(&self, session: &str) -> Result<Option<Receipt>, Error> {
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let receipt = read(&mut tx, a.user_id).await?;
        tx.commit().await?;
        Ok(receipt)
    }
    pub async fn claim_stored(&self, session: &str) -> Result<(Receipt, bool), Error> {
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        lock(&mut tx).await?;
        let old = read(&mut tx, a.user_id).await?.ok_or(Error::NotFound)?;
        if old.claimed_at_unix.is_some() {
            tx.commit().await?;
            return Ok((old, true));
        }
        if !old.claim_available {
            return Err(Error::Forbidden);
        }
        post(
            &mut tx,
            a.user_id,
            a.owner_account_id,
            &old.gift_id,
            old.credit_amount,
        )
        .await?;
        let r = read(&mut tx, a.user_id).await?.ok_or(Error::Unavailable)?;
        tx.commit().await?;
        Ok((r, false))
    }
}
async fn actor(c: &mut PgConnection, session: &str) -> Result<identity::Principal, Error> {
    let a = identity::authorize_in_transaction(c, session).await?;
    let personal: bool = sqlx::query_scalar(
        "SELECT personal_account_id=$2 FROM core_identity.users WHERE id=$1 FOR SHARE",
    )
    .bind(a.user_id)
    .bind(a.owner_account_id)
    .fetch_one(c)
    .await?;
    if a.credential_kind != CredentialKind::Session || !personal {
        return Err(Error::Forbidden);
    }
    Ok(a)
}
async fn lock(c: &mut PgConnection) -> Result<(), Error> {
    // Global gift admission serializes cap/identity/network races across processes.
    sqlx::query("SELECT pg_advisory_xact_lock(671124215333)")
        .execute(c)
        .await?;
    Ok(())
}
async fn eligibility(
    c: &mut PgConnection,
    user: i64,
    amount: i64,
    require_turns: bool,
) -> Result<(i64, String), Error> {
    sqlx::query("SELECT core_gift.lock_eligibility($1)")
        .bind(user)
        .execute(&mut *c)
        .await?;
    let p = sqlx::query("SELECT enabled,exclusive_cap FROM core_gift.policy WHERE singleton")
        .fetch_one(&mut *c)
        .await?;
    let cap: i64 = p.try_get("exclusive_cap")?;
    if !p.try_get::<bool, _>("enabled")? {
        return Ok((cap, "disabled".into()));
    }
    if amount <= 0 || amount >= cap {
        return Ok((cap, "cap_changed".into()));
    }
    let q=sqlx::query("SELECT q.* FROM core_gift.qualifications q JOIN core_identity.users u ON u.id=q.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE q.user_id=$1 AND u.active AND a.active AND u.platform_role='user' FOR SHARE OF u,a").bind(user).fetch_optional(&mut *c).await?;
    let Some(q) = q else {
        return Ok((cap, "qualification_missing".into()));
    };
    if q.try_get::<bool, _>("registration_hold")? {
        return Ok((cap, "registration_hold".into()));
    }
    if q.try_get::<bool, _>("reward_used")? {
        return Ok((cap, "reward_used".into()));
    }
    let observed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_gift.registration_profiles WHERE user_id=$1 AND observed_at BETWEEN extract(epoch FROM clock_timestamp())::bigint-900 AND extract(epoch FROM clock_timestamp())::bigint+60)").bind(user).fetch_one(&mut *c).await?;
    if require_turns && !observed {
        return Ok((cap, "qualification_missing".into()));
    }
    let own_messages:i64=sqlx::query_scalar("SELECT count(DISTINCT message_digest) FROM core_gift.fingerprints WHERE user_id=$1 AND created_at>extract(epoch FROM clock_timestamp()-interval '7 days')::bigint").bind(user).fetch_one(&mut *c).await?;
    if require_turns && own_messages < 2 {
        return Ok((cap, "qualification_missing".into()));
    }
    let peers=sqlx::query("WITH own AS (SELECT DISTINCT message_digest FROM core_gift.fingerprints WHERE user_id=$1 AND created_at>extract(epoch FROM clock_timestamp()-interval '7 days')::bigint), peers AS (SELECT f.user_id FROM core_gift.fingerprints f WHERE f.user_id<>$1 AND f.created_at>extract(epoch FROM clock_timestamp()-interval '7 days')::bigint AND f.token IN (SELECT token FROM core_gift.fingerprints WHERE user_id=$1 AND created_at>extract(epoch FROM clock_timestamp()-interval '7 days')::bigint) AND (SELECT count(*) FROM own)>=2 GROUP BY f.user_id HAVING count(DISTINCT f.message_digest)>=2 AND count(DISTINCT f.token)>=4) SELECT count(*) AS templates,count(*) FILTER(WHERE q.network_hash=$2) AS networks FROM peers p JOIN core_gift.qualifications q ON q.user_id=p.user_id JOIN core_gift.registration_profiles r ON r.user_id=p.user_id WHERE r.observed_at>extract(epoch FROM clock_timestamp()-interval '7 days')::bigint").bind(user).bind(q.try_get::<Vec<u8>,_>("network_hash")?).fetch_one(&mut *c).await?;
    if peers.try_get::<i64, _>("templates")? >= 4 && peers.try_get::<i64, _>("networks")? >= 3 {
        return Ok((cap, "registration_hold".into()));
    }
    if crate::abuse::is_marked_in_transaction(c, user)
        .await
        .map_err(|_| Error::Unavailable)?
    {
        return Ok((cap, "registration_hold".into()));
    }
    let email: Vec<u8> = q.try_get("email_hash")?;
    let current_email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM core_identity.user_profiles WHERE user_id=$1 FOR SHARE",
    )
    .bind(user)
    .fetch_optional(&mut *c)
    .await?;
    let Some(current_email) = current_email.and_then(|e| canonical_email(&e)) else {
        return Ok((cap, "qualification_missing".into()));
    };
    let key: Vec<u8> = sqlx::query_scalar("SELECT secret FROM core_gift.risk_key WHERE singleton")
        .fetch_one(&mut *c)
        .await?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Unavailable)?;
    mac.update(b"identity\0");
    mac.update(current_email.as_bytes());
    if mac.verify_slice(&email).is_err() {
        return Ok((cap, "qualification_missing".into()));
    }
    let net: Vec<u8> = q.try_get("network_hash")?;
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM core_gift.decisions WHERE email_hash=$1 AND user_id<>$2)",
    )
    .bind(email)
    .bind(user)
    .fetch_one(&mut *c)
    .await?;
    if used {
        return Ok((cap, "identity_used".into()));
    }
    let n:i64=sqlx::query_scalar("SELECT count(*) FROM core_gift.decisions WHERE network_hash=$1 AND user_id<>$2 AND created_at>extract(epoch FROM clock_timestamp()-interval '30 days')::bigint").bind(net).bind(user).fetch_one(&mut *c).await?;
    if n >= 3 {
        return Ok((cap, "network_limit".into()));
    }
    Ok((cap, String::new()))
}
pub(crate) async fn sponsor_eligible_in_transaction(
    c: &mut PgConnection,
    user: i64,
) -> Result<bool, Error> {
    lock(c).await?;
    let decided: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_gift.decisions WHERE user_id=$1)")
            .bind(user)
            .fetch_one(&mut *c)
            .await?;
    if decided {
        return Ok(false);
    }
    let (_, blocked) = eligibility(c, user, 1, false).await?;
    Ok(blocked.is_empty())
}
/// Authenticated Core conversation input only. The text comes from the already
/// persisted operation, never an extension-provided observation or tool result.
pub(crate) async fn record_user_turn_in_transaction(
    c: &mut PgConnection,
    user: i64,
    model_request_id: &str,
) -> Result<(), Error> {
    let text:Option<String>=sqlx::query_scalar("SELECT o.text FROM core_assistant.operations o JOIN core_assistant.grants g ON g.id=o.grant_id JOIN core_assistant.conversations cv ON cv.id=g.conversation_id WHERE o.request_id=$1 AND cv.owner_user_id=$2").bind(model_request_id).bind(user).fetch_optional(&mut *c).await?;
    let Some(text) = text else {
        return Err(Error::Forbidden);
    };
    let key: Vec<u8> = sqlx::query_scalar("SELECT secret FROM core_gift.risk_key WHERE singleton")
        .fetch_one(&mut *c)
        .await?;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&mut *c)
        .await?;
    sqlx::query("INSERT INTO core_gift.registration_profiles(user_id,observed_at) VALUES($1,$2) ON CONFLICT(user_id) DO UPDATE SET observed_at=EXCLUDED.observed_at").bind(user).bind(now).execute(&mut *c).await?;
    let (digest, tokens) = fingerprints(&key, &text)?;
    for token in tokens {
        sqlx::query("INSERT INTO core_gift.fingerprints(user_id,message_digest,token,created_at) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(user).bind(&digest).bind(token).bind(now).execute(&mut *c).await?;
    }
    sqlx::query("DELETE FROM core_gift.fingerprints WHERE user_id=$1 AND (id IN (SELECT id FROM core_gift.fingerprints WHERE user_id=$1 ORDER BY id DESC OFFSET 256) OR created_at<$2-604800)").bind(user).bind(now).execute(c).await?;
    Ok(())
}
fn fingerprints(key: &[u8], text: &str) -> Result<(Vec<u8>, Vec<Vec<u8>>), Error> {
    if key.is_empty() {
        return Err(Error::Invalid);
    }
    // Drop secret/contact-bearing tokens before normalization; private text itself
    // remains solely in Core's conversation store, never in this correlation index.
    let redacted = text
        .split_whitespace()
        .filter(|w| {
            !w.contains('@')
                && !w.contains("://")
                && !w.starts_with("sk-")
                && !w.starts_with("lmm_")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let chars = redacted
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || c.is_numeric())
        .take(2048)
        .collect::<Vec<_>>();
    if chars.len() < 40 {
        return Ok((Vec::new(), Vec::new()));
    }
    let hash = |namespace: &str, v: &str| -> Result<Vec<u8>, Error> {
        let mut h = Hmac::<Sha256>::new_from_slice(key).map_err(|_| Error::Unavailable)?;
        h.update(format!("registration-v1:{namespace}{v}").as_bytes());
        Ok(h.finalize().into_bytes().to_vec())
    };
    let digest = hash("message:", &chars.iter().collect::<String>())?;
    let mut tokens = std::collections::BTreeSet::new();
    for i in (0..=chars.len() - 20).step_by(4) {
        tokens.insert(hash(
            "shingle:",
            &chars[i..i + 20].iter().collect::<String>(),
        )?);
    }
    Ok((digest, tokens.into_iter().take(8).collect()))
}
/// Called only with Google's cryptographically verified private proof, during
/// successful fresh registration. Historical missing observations never qualify.
pub(crate) async fn observe_google_identity(
    c: &mut PgConnection,
    user: i64,
    proof: &crate::identity::VerifiedGoogleIdentity,
) -> Result<bool, Error> {
    let Some(email) = proof.verified_email() else {
        return Ok(false);
    };
    let Some(email) = canonical_email(email) else {
        return Ok(false);
    };
    sqlx::query("SELECT core_gift.lock_eligibility($1)")
        .bind(user)
        .execute(&mut *c)
        .await?;
    let r=sqlx::query("SELECT email_at_registration,host(source_ip) AS ip FROM core_abuse.registration_observations WHERE user_id=$1 AND source_policy_version=1 AND source_kind IN ('socket','trusted_x_forwarded_for') AND source_ip IS NOT NULL AND registered_at>clock_timestamp()-interval '60 seconds'").bind(user).fetch_optional(&mut *c).await?;
    let Some(r) = r else { return Ok(false) };
    if canonical_email(&r.try_get::<String, _>("email_at_registration")?).as_deref()
        != Some(email.as_str())
    {
        return Ok(false);
    }
    if crate::abuse::is_marked_in_transaction(c, user)
        .await
        .map_err(|_| Error::Unavailable)?
    {
        return Ok(false);
    }
    let ip: std::net::IpAddr = r
        .try_get::<String, _>("ip")?
        .parse()
        .map_err(|_| Error::Unavailable)?;
    let ip = match ip {
        std::net::IpAddr::V6(v) => v
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(v)),
        v => v,
    };
    let key: Vec<u8> = sqlx::query_scalar("SELECT secret FROM core_gift.risk_key WHERE singleton")
        .fetch_one(&mut *c)
        .await?;
    let hash = |domain: &str, value: &str| -> Result<Vec<u8>, Error> {
        let mut h = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Unavailable)?;
        h.update(domain.as_bytes());
        h.update(&[0]);
        h.update(value.as_bytes());
        Ok(h.finalize().into_bytes().to_vec())
    };
    let evidence = Sha256::digest(
        serde_json::to_vec(&(proof.issuer(), proof.subject(), &email))
            .map_err(|_| Error::Unavailable)?,
    )
    .to_vec();
    sqlx::query("INSERT INTO core_gift.qualifications(user_id,email_hash,network_hash,evidence_digest,evidence_kind,verified_at,registration_hold,reward_used) VALUES($1,$2,$3,$4,'google_verified_email_v1',clock_timestamp(),FALSE,FALSE) ON CONFLICT(user_id) DO NOTHING").bind(user).bind(hash("identity",&email)?).bind(hash("network",&ip.to_string())?).bind(evidence).execute(c).await?;
    Ok(true)
}
fn canonical_email(email: &str) -> Option<String> {
    let email = email.trim().to_ascii_lowercase();
    if email.len() > 320 || email.chars().any(char::is_control) {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || email.chars().any(char::is_whitespace)
    {
        return None;
    }
    let domain = if domain == "googlemail.com" {
        "gmail.com"
    } else {
        domain
    };
    // Same exact-domain promotional exclusion as fixed main email_policy.go.
    if [
        "10minutemail.com",
        "disposablemail.com",
        "emailondeck.com",
        "fakeinbox.com",
        "getnada.com",
        "guerrillamail.com",
        "maildrop.cc",
        "mailinator.com",
        "sharklasers.com",
        "tempmail.com",
        "temp-mail.org",
        "yopmail.com",
    ]
    .contains(&domain)
    {
        return None;
    }
    let local = match domain {
        "gmail.com" => local.split('+').next()?.replace('.', ""),
        "outlook.com" | "hotmail.com" | "live.com" => local.split('+').next()?.to_string(),
        _ => local.to_string(),
    };
    if local.is_empty() {
        None
    } else {
        Some(format!("{local}@{domain}"))
    }
}
async fn read(c: &mut PgConnection, user: i64) -> Result<Option<Receipt>, Error> {
    let row = sqlx::query("SELECT * FROM core_gift.decisions WHERE user_id=$1 FOR SHARE")
        .bind(user)
        .fetch_optional(&mut *c)
        .await?;
    let Some(r) = row else { return Ok(None) };
    let amount = r.try_get("amount_credits")?;
    let claimed: Option<i64> = r.try_get("claimed_at")?;
    let (cap, mut code) = eligibility(c, user, amount, true).await?;
    if claimed.is_some() {
        code = "already_claimed".into()
    }
    Ok(Some(Receipt {
        gift_id: r.try_get("id")?,
        owner_user_id: user,
        credit_amount: amount,
        created_at_unix: r.try_get("created_at")?,
        claimed_at_unix: claimed,
        ledger_transaction_id: r.try_get("ledger_journal_id")?,
        max_credit_amount: cap,
        claim_available: claimed.is_none() && code.is_empty(),
        claim_blocked_code: code,
    }))
}
pub(crate) async fn status_in_transaction(
    c: &mut PgConnection,
    user: i64,
) -> Result<Option<Receipt>, Error> {
    read(c, user).await
}
/// Only the Core model executor calls this, in the model settlement transaction.
/// A positive decision immediately posts; no extension-created offer is accepted.
pub(crate) async fn execute_in_transaction(
    c: &mut PgConnection,
    user: i64,
    conversation: &str,
    model_request_id: &str,
    decision: GiftDecision,
) -> Result<Option<Receipt>, Error> {
    if decision.amount_credits < 0
        || decision.reason.chars().count() > 240
        || decision.reason.chars().any(char::is_control)
    {
        return Err(Error::Invalid);
    }
    lock(c).await?;
    if let Some(r) = read(c, user).await? {
        return Ok(Some(r));
    }
    if decision.amount_credits == 0 {
        return Ok(None);
    }
    let belongs:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_assistant.conversations c JOIN core_assistant.grants g ON g.conversation_id=c.id JOIN core_assistant.operations o ON o.grant_id=g.id JOIN core_charging.requests q ON q.id=o.request_id WHERE c.id=$1 AND c.owner_user_id=$2 AND q.id=$3 AND q.assistant_grant_id=g.id AND q.actor_user_id=$2 AND (q.payer_account_id=c.owner_account_id OR q.payer_account_id=(g.policy->'sponsor'->>'treasury_account_id')::bigint) AND q.state='streaming')").bind(conversation).bind(user).bind(model_request_id).fetch_one(&mut *c).await?;
    if !belongs {
        return Err(Error::Forbidden);
    }
    let (_, blocked) = eligibility(c, user, decision.amount_credits, true).await?;
    if !blocked.is_empty() {
        return Err(Error::Forbidden);
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let id = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let reason = if decision.reason.trim().is_empty() {
        "Welcome gift"
    } else {
        decision.reason.trim()
    };
    sqlx::query("INSERT INTO core_gift.decisions(id,user_id,conversation_id,amount_credits,reason,email_hash,network_hash,created_at,model_request_id) SELECT $1,$2,$3,$4,$5,email_hash,network_hash,extract(epoch FROM clock_timestamp())::bigint,$6 FROM core_gift.qualifications WHERE user_id=$2").bind(&id).bind(user).bind(conversation).bind(decision.amount_credits).bind(reason).bind(model_request_id).execute(&mut *c).await?;
    let account: i64 = sqlx::query_scalar(
        "SELECT personal_account_id FROM core_identity.users WHERE id=$1 FOR SHARE",
    )
    .bind(user)
    .fetch_one(&mut *c)
    .await?;
    post(c, user, account, &id, decision.amount_credits).await?;
    read(c, user).await
}
async fn post(
    c: &mut PgConnection,
    user: i64,
    account: i64,
    id: &str,
    amount: i64,
) -> Result<(), Error> {
    let _: Json<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
        .bind(account)
        .fetch_one(&mut *c)
        .await?;
    let request = Request {
        scope: "core-assistant-gift".into(),
        operation_key: id.into(),
        actor_user_id: Some(user),
        reason: "Core assistant new-user gift".into(),
        action: Action::Gift {
            account_id: account,
            amount_units: Credits::try_from(amount).map_err(|_| Error::Invalid)?,
        },
    };
    request.validate().map_err(|_| Error::Invalid)?;
    let Json(out): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(request))
        .fetch_one(&mut *c)
        .await?;
    let Outcome::Posted { journal_id, .. } = out else {
        return Err(Error::Conflict);
    };
    sqlx::query("UPDATE core_gift.decisions SET claimed_at=extract(epoch FROM clock_timestamp())::bigint,ledger_journal_id=$2 WHERE id=$1 AND claimed_at IS NULL").bind(id).bind(journal_id).execute(c).await?;
    Ok(())
}
