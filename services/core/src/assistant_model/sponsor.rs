//! Host-owned purpose-limited model sponsorship. This is not an extension DTO.
//! The policy is serialized into the immutable Core grant before any hold.
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SponsorPolicy {
    pub policy_id: String,
    pub treasury_account_id: i64,
    pub max_credits_per_user: i64,
    pub global_budget_credits: i64,
    pub max_calls_per_user: u32,
    pub expires_at_unix: i64,
}
impl SponsorPolicy {
    pub(crate) fn validate(&self) -> Result<(), super::Error> {
        if self.policy_id.is_empty()
            || self.policy_id.len() > 128
            || !self
                .policy_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || self.treasury_account_id <= 0
            || !(1..=9_007_199_254_740_991).contains(&self.max_credits_per_user)
            || !(self.max_credits_per_user..=9_007_199_254_740_991)
                .contains(&self.global_budget_credits)
            || !(1..=8).contains(&self.max_calls_per_user)
            || self.expires_at_unix <= 0
        {
            return Err(super::Error::Invalid);
        }
        Ok(())
    }
    /// The original user and request cost come from Core Session/routing. No wire
    /// field chooses this policy, payer, group, model or spending ceiling.
    pub(crate) async fn admit(
        &self,
        c: &mut PgConnection,
        user: i64,
        owner: i64,
        reserve: i64,
    ) -> Result<(), super::Error> {
        self.validate()?;
        let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
            .fetch_one(&mut *c)
            .await?;
        if now >= self.expires_at_unix
            || reserve < 0
            || reserve > self.max_credits_per_user
            || self.treasury_account_id == owner
        {
            return Err(super::Error::Forbidden);
        }
        let active: Option<bool> =
            sqlx::query_scalar("SELECT active FROM core_identity.accounts WHERE id=$1 FOR SHARE")
                .bind(self.treasury_account_id)
                .fetch_optional(&mut *c)
                .await?;
        if active != Some(true) {
            return Err(super::Error::Forbidden);
        }
        if !crate::gift::sponsor_eligible_in_transaction(c, user)
            .await
            .map_err(|_| super::Error::Unavailable)?
        {
            return Err(super::Error::Forbidden);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,9912))")
            .bind(&self.policy_id)
            .execute(&mut *c)
            .await?;
        let totals=sqlx::query("SELECT count(*) FILTER(WHERE r.actor_user_id=$2)::bigint AS user_calls,coalesce(sum(core_charging.committed_credits(r.state,r.reserved,r.settled,r.refunded)) FILTER(WHERE r.actor_user_id=$2),0)::bigint AS user_credits,coalesce(sum(core_charging.committed_credits(r.state,r.reserved,r.settled,r.refunded)),0)::bigint AS all_credits FROM core_charging.requests r JOIN core_assistant.grants g ON g.id=r.assistant_grant_id WHERE g.policy#>>'{sponsor,policy_id}'=$1 AND r.state<>'released'")
   .bind(&self.policy_id).bind(user).fetch_one(&mut *c).await?;
        let calls: i64 = totals.try_get("user_calls")?;
        let used: i64 = totals.try_get("user_credits")?;
        let total: i64 = totals.try_get("all_credits")?;
        if calls >= i64::from(self.max_calls_per_user)
            || used
                .checked_add(reserve)
                .is_none_or(|v| v > self.max_credits_per_user)
            || total
                .checked_add(reserve)
                .is_none_or(|v| v > self.global_budget_credits)
        {
            return Err(super::Error::NoFunds);
        }
        Ok(())
    }
}
