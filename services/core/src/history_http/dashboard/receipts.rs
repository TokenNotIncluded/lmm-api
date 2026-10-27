//! Settled-only receipt validation adapted from usage.rs. No ledger writes.
use super::super::HistoryError as Error;
use crate::billing::{
    CREDITS_PER_USD,
    charging::{Change, LedgerCommand, Outcome, Price, Source, Usage},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgRow};
use std::collections::HashMap;
const JS_SAFE: i64 = 9_007_199_254_740_991;
const MAX_RECEIPTS: usize = 2048;

pub(super) fn safe(value: i64) -> Result<i64, Error> {
    if !(0..=JS_SAFE).contains(&value) {
        return Err(Error::Unavailable);
    }
    Ok(value)
}
pub(super) fn add(total: &mut i64, value: i64) -> Result<(), Error> {
    *total = safe(total.checked_add(value).ok_or(Error::Unavailable)?)?;
    Ok(())
}
fn usd(credits: i64) -> String {
    let millionths = i128::from(credits) * 1_000_000 / i128::from(CREDITS_PER_USD);
    format!("{}.{:06}", millionths / 1_000_000, millionths % 1_000_000)
}
#[derive(Default, Clone, Copy)]
pub(super) struct Amounts {
    gross: i64,
    refunded: i64,
}
impl Amounts {
    pub(super) fn accumulate(&mut self, other: Self) -> Result<(), Error> {
        add(&mut self.gross, other.gross)?;
        add(&mut self.refunded, other.refunded)
    }
    fn net(self) -> i64 {
        self.gross - self.refunded
    }
    pub(super) fn project(self, value: &mut Value) {
        value["gross_settled_credits"] = json!(self.gross);
        value["refunded_credits"] = json!(self.refunded);
        value["net_consumption_credits"] = json!(self.net());
        value["billing_basis"] = json!("net_consumption");
        value["billing_currency"] = json!("USD");
        // This API projects consumption, not ledger entries or wallet balances.
        value["wallet_debit_supported"] = json!(false);
        value["quota"] = json!(self.net());
        value["quota_usd"] = json!(usd(self.net()));
        value["quota_money_available"] = json!(true);
        value["quota_money_basis"] = json!("captured_credits_per_usd");
    }
}
// Matches the immutable Charging::command receipt identity, never a new command
// submission. Keeping the original request ID and tag prevents refund aliasing.
fn operation_id(request_id: &str, tag: &str) -> Result<String, Error> {
    let encoded = serde_json::to_vec(&("core-charging-v1", request_id, tag))
        .map_err(|_| Error::Unavailable)?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}
#[derive(Deserialize)]
struct StoredCommand {
    operation_id: String,
    command: Value,
}
#[derive(Deserialize)]
struct Refund {
    refund_id: String,
    amount: i64,
}
#[derive(Deserialize)]
struct FinalEvent {
    kind: String,
    usage: Usage,
    outcome: Outcome,
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|_| Error::Unavailable)
}
pub(super) struct Receipt {
    pub(super) user: i64,
    pub(super) key: Option<i64>,
    pub(super) oauth_source: Option<crate::identity::grants::OAuthSource>,
    pub(super) credential_source: &'static str,
    pub(super) sponsored: bool,
    pub(super) at: i64,
    pub(super) price: Price,
    pub(super) token_name: String,
    pub(super) payer_kind: String,
    source: Source,
    pub(super) usage: Usage,
    pub(super) amounts: Amounts,
}
impl Receipt {
    pub(super) fn from_row(row: PgRow) -> Result<Self, Error> {
        let id: String = row.try_get("id")?;
        let price: Price = decode(row.try_get("price")?)?;
        price.validate().map_err(|_| Error::Unavailable)?;
        let user = safe(row.try_get("actor_user_id")?)?;
        let key = row
            .try_get::<Option<i64>, _>("key_id")?
            .map(safe)
            .transpose()?;
        let oauth_source: Option<crate::identity::grants::OAuthSource> = row
            .try_get::<Option<Value>, _>("oauth_source")?
            .map(decode)
            .transpose()?;
        let owner: i64 = row.try_get("owner_account_id")?;
        let payer: i64 = row.try_get("payer_account_id")?;
        let image_operation: Option<String> = row.try_get("image_operation_id")?;
        let assistant_grant: Option<String> = row.try_get("assistant_grant_id")?;
        let assistant_sponsor: Option<i64> = row.try_get("assistant_sponsor_account_id")?;
        if price.image.is_some() != image_operation.is_some()
            || image_operation.as_deref().is_some_and(|op| op != id)
        {
            return Err(Error::Unavailable);
        }
        let credential_source = match (&key, &oauth_source) {
            (Some(key), None)
                if *key > 0 && assistant_grant.is_none() && assistant_sponsor.is_none() =>
            {
                "api_key"
            }
            (None, Some(source))
                if image_operation.is_none()
                    && assistant_grant.is_none()
                    && assistant_sponsor.is_none() =>
            {
                source.validate_stored().map_err(|_| Error::Unavailable)?;
                "oauth_grant"
            }
            (None, None)
                if image_operation.is_some()
                    && assistant_grant.is_none()
                    && assistant_sponsor.is_none()
                    && owner == payer =>
            {
                "drawing_session"
            }
            (None, None)
                if image_operation.is_none()
                    && assistant_grant.as_ref().is_some_and(|g| !g.is_empty())
                    && (assistant_sponsor == Some(payer)
                        || (assistant_sponsor.is_none() && owner == payer)) =>
            {
                "assistant_session"
            }
            _ => return Err(Error::Unavailable),
        };
        if let Some(source) = &oauth_source
            && (source.user_id != user
                || source.personal_account_id != owner
                || source.personal_account_id != payer)
        {
            return Err(Error::Unavailable);
        }
        let source = match row.try_get::<Option<i64>, _>("source_subscription_id")? {
            None => Source::Wallet,
            Some(id) => Source::Subscription {
                id,
                start: row.try_get("source_start")?,
                end: row.try_get("source_end")?,
            },
        };
        let state: String = row.try_get("state")?;
        let outcome: Option<String> = row.try_get("outcome")?;
        let reserved = safe(row.try_get("reserved")?)?;
        let amounts = Amounts {
            gross: safe(row.try_get("settled")?)?,
            refunded: safe(row.try_get("refunded")?)?,
        };
        let mut usage = Usage {
            image: row
                .try_get::<Option<Value>, _>("image_usage")?
                .map(decode)
                .transpose()?,
            input: safe(row.try_get("input_tokens")?)?,
            cached: safe(row.try_get("cached_tokens")?)?,
            output: safe(row.try_get("output_tokens")?)?,
            ..Default::default()
        };
        safe(
            usage
                .input
                .checked_add(usage.output)
                .ok_or(Error::Unavailable)?,
        )?;
        if !usage.valid()
            || (usage.image.is_some() && price.image.is_none())
            || (state == "settled" && price.image.is_some() && usage.image.is_none())
            || amounts.refunded > amounts.gross
            || amounts.gross > reserved
        {
            return Err(Error::Unavailable);
        }
        let commands: Vec<StoredCommand> = decode(row.try_get("commands")?)?;
        let refunds: Vec<Refund> = decode(row.try_get("refunds")?)?;
        if commands.len() > MAX_RECEIPTS || refunds.len() > MAX_RECEIPTS {
            return Err(Error::TooDense);
        }
        let mut receipts = HashMap::new();
        for stored in commands {
            let command: LedgerCommand = decode(stored.command.clone())?;
            // LedgerCommand/Source deserialization alone ignores unknown fields.
            // Require the complete canonical receipt, including source shape.
            if serde_json::to_value(&command).map_err(|_| Error::Unavailable)? != stored.command
                || stored.operation_id != command.operation_id
                || command.request_id != id
                || command.actor_user_id != user
                || command.key_id != key
                || command.oauth_source != oauth_source
                || command.owner_account_id != owner
                || command.payer_account_id != payer
                || command.price_version != price.version
                || command.source != source
                || receipts
                    .insert(stored.operation_id, command.change)
                    .is_some()
            {
                return Err(Error::Unavailable);
            }
        }
        if state == "settled" {
            let event: FinalEvent = decode(
                row.try_get::<Option<Value>, _>("final_event")?
                    .ok_or(Error::Unavailable)?,
            )?;
            let final_outcome = match event.outcome {
                Outcome::Completed => "completed",
                Outcome::Cancelled => "cancelled",
                Outcome::Failed => "failed",
            };
            if event.kind != "final"
                || Some(final_outcome) != outcome.as_deref()
                || event.usage.input != usage.input
                || event.usage.cached != usage.cached
                || event.usage.output != usage.output
                || event.usage.image != usage.image
                || price.cost(event.usage).map_err(|_| Error::Unavailable)? != amounts.gross
            {
                return Err(Error::Unavailable);
            }
            usage = event.usage;
            if receipts.remove(&operation_id(&id, "settle")?)
                != Some(Change::Settle {
                    amount: amounts.gross,
                })
            {
                return Err(Error::Unavailable);
            }
            let mut refund_total = 0;
            for refund in refunds {
                if refund.amount <= 0
                    || receipts.remove(&operation_id(&id, &format!("refund:{}", refund.refund_id))?)
                        != Some(Change::Refund {
                            amount: refund.amount,
                        })
                {
                    return Err(Error::Unavailable);
                }
                add(&mut refund_total, refund.amount)?;
            }
            if refund_total != amounts.refunded {
                return Err(Error::Unavailable);
            }
        } else {
            return Err(Error::Unavailable);
        }
        // A refund/settle command with no matching row/terminal record is corrupt,
        // even if it uses another operation ID or claims a different amount.
        if receipts
            .values()
            .any(|c| matches!(c, Change::Refund { .. } | Change::Settle { .. }))
        {
            return Err(Error::Unavailable);
        }
        Ok(Self {
            user,
            key,
            oauth_source,
            credential_source,
            sponsored: assistant_sponsor.is_some(),
            at: safe(row.try_get("authorized_at")?)?,
            price,
            token_name: row.try_get("token_name")?,
            payer_kind: row.try_get("payer_kind")?,
            source,
            usage,
            amounts,
        })
    }
    pub(super) fn subscription(&self) -> bool {
        matches!(self.source, Source::Subscription { .. })
    }
}
