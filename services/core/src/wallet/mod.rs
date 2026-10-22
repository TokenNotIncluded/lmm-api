// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
//! Personal-wallet transfer authority. Go keeps bearer material; Core stores only
//! its digest and the immutable payer/amount. Browser confirmation owns the hold.
use crate::{
    billing::{
        Credits,
        ledger::{Action, Balance, Ledger, Outcome, Request, Wallet},
    },
    identity::{self, BrowserAuth, CredentialKind},
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, types::Json};
pub mod http;
pub const SCHEMA: &str = include_str!("schema.sql");
#[derive(Debug, Clone, Copy)]
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
            .is_some_and(|d| matches!(d.code().as_deref(), Some("23505" | "40001" | "40P01")))
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
#[derive(Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub id: i64,
    pub request_key: String,
    pub amount_credits: i64,
    pub state: String,
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
    pub finished_at: Option<i64>,
}
const VIEW: &str =
    "id,request_key,amount AS amount_credits,state,created_at,confirmed_at,finished_at";
#[derive(Serialize)]
pub struct WalletBalance {
    pub owner_user_id: i64,
    pub credits_per_usd: i64,
    #[serde(flatten)]
    pub balance: Balance,
}
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}
impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Read-only startup authority check. Schema installation is an offline owner action.
    pub async fn check_ready(&self) -> Result<(), Error> {
        sqlx::query("SELECT id,owner_user,payer_account,request_key,amount,token_digest,state,hold_journal,final_journal,recipient_user,created_at,confirmed_at,finished_at,confirmation_digest,confirmation_sid,confirmation_kind,confirmation_expires FROM core_wallet.transfers LIMIT 0").execute(&self.pool).await?;
        let allowed: bool=sqlx::query_scalar("SELECT has_table_privilege(current_user,'core_wallet.transfers','SELECT') AND has_table_privilege(current_user,'core_wallet.transfers','INSERT') AND has_sequence_privilege(current_user,'core_wallet.transfers_id_seq','USAGE') AND has_sequence_privilege(current_user,'core_wallet.transfers_id_seq','SELECT') AND NOT has_table_privilege(current_user,'core_wallet.transfers','DELETE,TRUNCATE') AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['owner_user','payer_account','request_key','amount','token_digest']) c WHERE has_column_privilege(current_user,'core_wallet.transfers',c,'UPDATE')) AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['state','hold_journal','final_journal','recipient_user','confirmed_at','finished_at','confirmation_digest','confirmation_sid','confirmation_kind','confirmation_expires']) c WHERE NOT has_column_privilege(current_user,'core_wallet.transfers',c,'UPDATE'))").fetch_one(&self.pool).await?;
        if !allowed {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
    pub async fn balance(&self, session: &str) -> Result<WalletBalance, Error> {
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let _: Json<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
            .bind(a.owner_account_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        let balance = Ledger::from_pool(self.pool.clone())
            .balance(a.owner_account_id)
            .await
            .map_err(|_| Error::Unavailable)?;
        Ok(WalletBalance {
            owner_user_id: a.user_id,
            credits_per_usd: 500000,
            balance,
        })
    }
    pub async fn prepare(
        &self,
        session: &str,
        amount: i64,
        key: &str,
        token: &str,
    ) -> Result<Transfer, Error> {
        if amount <= 0
            || !(16..=64).contains(&key.len())
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !valid_token(token)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(a.owner_account_id)
            .execute(&mut *tx)
            .await?;
        let existing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_wallet.transfers WHERE owner_user=$1 AND request_key=$2)").bind(a.user_id).bind(key).fetch_one(&mut *tx).await?;
        if !existing {
            identity::reject_marked_new_action(
                &mut tx,
                a.user_id,
                identity::NewResourceAction::CommercePayment,
            )
            .await?;
        }
        sqlx::query("INSERT INTO core_wallet.transfers(owner_user,payer_account,request_key,amount,token_digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT(owner_user,request_key) DO NOTHING").bind(a.user_id).bind(a.owner_account_id).bind(key).bind(amount).bind(hash(token)).execute(&mut *tx).await?;
        let row=sqlx::query("SELECT amount,token_digest,payer_account FROM core_wallet.transfers WHERE owner_user=$1 AND request_key=$2 FOR UPDATE").bind(a.user_id).bind(key).fetch_one(&mut *tx).await?;
        if row.try_get::<i64, _>("amount")? != amount
            || row.try_get::<Vec<u8>, _>("token_digest")? != hash(token)
            || row.try_get::<i64, _>("payer_account")? != a.owner_account_id
        {
            return Err(Error::Conflict);
        }
        let out = sqlx::query_as::<_, Transfer>(&format!(
            "SELECT {VIEW} FROM core_wallet.transfers WHERE owner_user=$1 AND request_key=$2"
        ))
        .bind(a.user_id)
        .bind(key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(out)
    }
    /// ACK-lost recovery: authenticate the original Session and read only this
    /// sender's immutable key. Absence never creates or reserves anything.
    pub async fn query_by_request_key(&self, session: &str, key: &str) -> Result<Transfer, Error> {
        if !(16..=64).contains(&key.len())
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let out = sqlx::query_as::<_, Transfer>(&format!(
            "SELECT {VIEW} FROM core_wallet.transfers WHERE owner_user=$1 AND request_key=$2"
        ))
        .bind(a.user_id)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn list(&self, session: &str, before: i64) -> Result<Vec<Transfer>, Error> {
        if before < 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let rows=sqlx::query_as::<_,Transfer>(&format!("SELECT {VIEW} FROM core_wallet.transfers WHERE owner_user=$1 AND ($2=0 OR id<$2) ORDER BY id DESC LIMIT 50")).bind(a.user_id).bind(before).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(rows)
    }
    pub async fn inspect(&self, session: &str, token: &str) -> Result<Receipt, Error> {
        if !valid_token(token) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let r=sqlx::query("SELECT id,amount,state,created_at,finished_at,owner_user,recipient_user FROM core_wallet.transfers WHERE token_digest=$1 AND state<>'awaiting_confirmation'").bind(hash(token)).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let out = receipt(&r, a.user_id)?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn claim(&self, session: &str, token: &str) -> Result<Receipt, Error> {
        if !valid_token(token) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let r = sqlx::query("SELECT * FROM core_wallet.transfers WHERE token_digest=$1 FOR UPDATE")
            .bind(hash(token))
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let state: String = r.try_get("state")?;
        let owner: i64 = r.try_get("owner_user")?;
        if owner == a.user_id {
            return Err(Error::Forbidden);
        }
        if state == "claimed" && r.try_get::<Option<i64>, _>("recipient_user")? == Some(a.user_id) {
            let out = receipt(&r, a.user_id)?;
            tx.commit().await?;
            return Ok(out);
        }
        if state != "pending" {
            return Err(Error::Conflict);
        }
        // Recipient admission is new. The sender's original confirmed hold may
        // settle after a later mark, as required by the abuse-control boundary.
        identity::reject_marked_new_action(
            &mut tx,
            a.user_id,
            identity::NewResourceAction::CommercePayment,
        )
        .await?;
        let id: i64 = r.try_get("id")?;
        let final_journal = post(
            &mut tx,
            a.user_id,
            format!("claim-{id}"),
            Action::CommerceCapture {
                reservation_journal_id: r.try_get("hold_journal")?,
                destination_account_id: a.owner_account_id,
                amount_units: credits(r.try_get("amount")?)?,
            },
        )
        .await?;
        sqlx::query("UPDATE core_wallet.transfers SET state='claimed',recipient_user=$2,final_journal=$3,finished_at=extract(epoch FROM clock_timestamp())::bigint WHERE id=$1").bind(id).bind(a.user_id).bind(final_journal).execute(&mut *tx).await?;
        let r = sqlx::query("SELECT * FROM core_wallet.transfers WHERE id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let out = receipt(&r, a.user_id)?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn request_cancel(&self, session: &str, id: i64) -> Result<Transfer, Error> {
        if id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, session).await?;
        let out=sqlx::query_as::<_,Transfer>(&format!("SELECT {VIEW} FROM core_wallet.transfers WHERE id=$1 AND owner_user=$2 AND state IN ('pending','cancelled')")).bind(id).bind(a.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::Conflict)?;
        tx.commit().await?;
        Ok(out)
    }
    /// Browser-only cookie proof, shared with native OAuth consent; no Session bearer can approve.
    pub async fn confirmation_page(
        &self,
        cookie: &str,
        id: i64,
        kind: &str,
    ) -> Result<(Transfer, String), Error> {
        if kind != "create" && kind != "cancel" {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie).await?;
        let out=sqlx::query_as::<_,Transfer>(&format!("SELECT {VIEW} FROM core_wallet.transfers WHERE id=$1 AND owner_user=$2 AND state=$3 FOR UPDATE")).bind(id).bind(a.user).bind(if kind=="create"{"awaiting_confirmation"}else{"pending"}).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let csrf = random()?;
        sqlx::query("UPDATE core_wallet.transfers SET confirmation_digest=$2,confirmation_sid=$3,confirmation_kind=$4,confirmation_expires=extract(epoch FROM clock_timestamp())::bigint+600 WHERE id=$1").bind(id).bind(hash(&csrf)).bind(a.sid).bind(kind).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok((out, csrf))
    }
    pub async fn confirm(
        &self,
        cookie: &str,
        id: i64,
        csrf: &str,
        kind: &str,
    ) -> Result<Transfer, Error> {
        if !valid_token(csrf) {
            return Err(Error::Invalid);
        }
        if kind != "create" && kind != "cancel" {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let a = BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie).await?;
        let r=sqlx::query("SELECT *,confirmation_expires>=extract(epoch FROM clock_timestamp())::bigint AS fresh FROM core_wallet.transfers WHERE id=$1 AND owner_user=$2 FOR UPDATE").bind(id).bind(a.user).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if r.try_get::<Option<String>, _>("confirmation_kind")? != Some(kind.into())
            || r.try_get::<Option<Vec<u8>>, _>("confirmation_digest")? != Some(hash(csrf))
            || r.try_get::<Option<String>, _>("confirmation_sid")? != Some(a.sid)
            || !r.try_get::<Option<bool>, _>("fresh")?.unwrap_or(false)
        {
            return Err(Error::Forbidden);
        }
        let state: String = r.try_get("state")?;
        if kind == "cancel" {
            if state == "pending" {
                let final_journal = post(
                    &mut tx,
                    a.user,
                    format!("cancel-{id}"),
                    Action::Release {
                        reservation_journal_id: r.try_get("hold_journal")?,
                    },
                )
                .await?;
                sqlx::query("UPDATE core_wallet.transfers SET state='cancelled',final_journal=$2,finished_at=extract(epoch FROM clock_timestamp())::bigint WHERE id=$1").bind(id).bind(final_journal).execute(&mut *tx).await?;
            } else if state != "cancelled" {
                return Err(Error::Conflict);
            }
        } else if state == "awaiting_confirmation" {
            identity::reject_marked_new_action(
                &mut tx,
                a.user,
                identity::NewResourceAction::CommercePayment,
            )
            .await?;
            let journal = post(
                &mut tx,
                a.user,
                format!("hold-{id}"),
                Action::Reserve {
                    account_id: r.try_get("payer_account")?,
                    amount_units: credits(r.try_get("amount")?)?,
                },
            )
            .await?;
            sqlx::query("UPDATE core_wallet.transfers SET state='pending',hold_journal=$2,confirmed_at=extract(epoch FROM clock_timestamp())::bigint WHERE id=$1").bind(id).bind(journal).execute(&mut *tx).await?;
        } else if state != "pending" {
            return Err(Error::Conflict);
        }
        let out = sqlx::query_as::<_, Transfer>(&format!(
            "SELECT {VIEW} FROM core_wallet.transfers WHERE id=$1"
        ))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(out)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: i64,
    pub amount_credits: i64,
    pub state: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub is_sender: bool,
    pub claimed_by_me: bool,
}
fn receipt(r: &sqlx::postgres::PgRow, user: i64) -> Result<Receipt, Error> {
    Ok(Receipt {
        id: r.try_get("id")?,
        amount_credits: r.try_get("amount")?,
        state: r.try_get("state")?,
        created_at: r.try_get("created_at")?,
        finished_at: r.try_get("finished_at")?,
        is_sender: r.try_get::<i64, _>("owner_user")? == user,
        claimed_by_me: r.try_get::<Option<i64>, _>("recipient_user")? == Some(user),
    })
}
fn credits(a: i64) -> Result<Credits, Error> {
    Credits::try_from(a).map_err(|_| Error::Invalid)
}
fn hash(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}
fn valid_token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn random() -> Result<String, Error> {
    let mut b = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut b)
        .map_err(|_| Error::Unavailable)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
async fn actor(c: &mut PgConnection, s: &str) -> Result<identity::Principal, Error> {
    let a = identity::authorize_in_transaction(c, s).await?;
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
async fn post(c: &mut PgConnection, user: i64, key: String, action: Action) -> Result<i64, Error> {
    let account = match &action {
        Action::Reserve { account_id, .. } => Some(*account_id),
        Action::CommerceCapture {
            destination_account_id,
            ..
        } => Some(*destination_account_id),
        _ => None,
    };
    if let Some(id) = account {
        let _: Json<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
            .bind(id)
            .fetch_one(&mut *c)
            .await?;
    }
    let req = Request {
        scope: "core-wallet-transfer".into(),
        operation_key: key,
        actor_user_id: Some(user),
        reason: "Confirmed personal wallet transfer".into(),
        action,
    };
    req.validate().map_err(|_| Error::Invalid)?;
    let out = crate::billing::ledger::post_in_transaction(c, &req).await?;
    match out {
        Outcome::Posted { journal_id, .. } => Ok(journal_id),
        _ => Err(Error::Conflict),
    }
}

#[cfg(test)]
mod tests;
