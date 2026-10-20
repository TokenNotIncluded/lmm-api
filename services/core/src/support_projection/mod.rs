//! Read-only seller customer view. Go owns support relationships and notes;
//! Core owns profiles and integer-credit economics. No spending authority.
pub mod http;
use crate::identity::{self, CredentialKind};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct SupportProjection {
    pool: PgPool,
}
#[derive(Debug)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Unavailable,
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<identity::IdentityError> for Error {
    fn from(v: identity::IdentityError) -> Self {
        match v {
            identity::IdentityError::Unauthorized => Self::Unauthorized,
            identity::IdentityError::Forbidden => Self::Forbidden,
            _ => Self::Unavailable,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub buyer_id: i64,
    pub conversation_id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub seller_id: i64,
    #[serde(default)]
    pub buyer_id: Option<i64>,
    pub relationships: Vec<Relationship>,
    pub query: String,
    pub offset: i64,
    pub limit: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Customer {
    pub buyer_id: i64,
    pub display_name: String,
    pub order_count: i64,
    pub paid_quota: i64,
    pub last_order_at: i64,
    pub last_order_id: String,
    pub conversation_id: String,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        if self.seller_id <= 0
            || self
                .buyer_id
                .is_some_and(|id| id <= 0 || id == self.seller_id)
            || self.relationships.len() > 5000
            || self.query.chars().count() > 100
            || self.offset < 0
            || self.offset > 100000
            || !(1..=100).contains(&self.limit)
        {
            return Err(Error::Invalid);
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in &self.relationships {
            if r.buyer_id <= 0
                || r.buyer_id == self.seller_id
                || r.conversation_id.is_empty()
                || r.conversation_id.len() > 128
                || r.conversation_id.chars().any(char::is_control)
                || !seen.insert(r.buyer_id)
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
impl SupportProjection {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Verify the configured read port without requiring commerce write grants.
    /// Startup never creates a schema or attempts a probe mutation.
    pub async fn check_readonly(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT o.order_id,o.buyer_account_id,o.seller_account_id,o.credits,o.authorized_at,o.reserve_journal_id,u.id,u.personal_account_id,u.active,p.username,p.display_name FROM core_commerce.orders o JOIN core_identity.users u ON u.personal_account_id=o.buyer_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE FALSE")
            .fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn customers(&self, secret: &str, request: &Request) -> Result<Vec<Customer>, Error> {
        request.validate()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let actor = identity::authenticate_readonly_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session
            || actor.user_id != request.seller_id
            || actor.owner.kind != crate::accounts::AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(Error::Forbidden);
        }
        let ids: Vec<i64> = request.relationships.iter().map(|r| r.buyer_id).collect();
        // ids are trusted extension relationships, never browser-supplied. The
        // private transport grants only this public-name projection, not users/emails.
        let exact = request.buyer_id;
        let needle = request.query.trim().to_lowercase();
        let rows=sqlx::query("WITH orders AS (SELECT u.id buyer_id,count(*)::bigint order_count,COALESCE(sum(o.credits) FILTER(WHERE o.reserve_journal_id IS NOT NULL),0)::bigint paid_quota,max(o.authorized_at) last_order_at,(array_agg(o.order_id ORDER BY o.authorized_at DESC,o.order_id DESC))[1] last_order_id FROM core_commerce.orders o JOIN core_identity.users u ON u.personal_account_id=o.buyer_account_id WHERE o.seller_account_id=$1 GROUP BY u.id), candidates AS (SELECT buyer_id FROM orders UNION SELECT unnest($2::bigint[])) SELECT u.id buyer_id,COALESCE(NULLIF(p.display_name,''),p.username) display_name,COALESCE(o.order_count,0)::bigint order_count,COALESCE(o.paid_quota,0)::bigint paid_quota,COALESCE(o.last_order_at,0)::bigint last_order_at,COALESCE(o.last_order_id,'') last_order_id FROM candidates c JOIN core_identity.users u ON u.id=c.buyer_id JOIN core_identity.user_profiles p ON p.user_id=u.id LEFT JOIN orders o ON o.buyer_id=u.id WHERE u.active AND u.id<>$3 AND ($4::bigint IS NULL OR u.id=$4) AND ($4::bigint IS NOT NULL OR $5='' OR strpos(lower(p.username),$5)>0 OR strpos(lower(p.display_name),$5)>0) ORDER BY last_order_at DESC,u.id DESC OFFSET $6 LIMIT $7")
   .bind(actor.owner_account_id).bind(&ids).bind(actor.user_id).bind(exact).bind(needle).bind(request.offset).bind(request.limit).fetch_all(&mut *tx).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let buyer_id = row.try_get("buyer_id")?;
            out.push(Customer {
                buyer_id,
                display_name: row.try_get("display_name")?,
                order_count: row.try_get("order_count")?,
                paid_quota: row.try_get("paid_quota")?,
                last_order_at: row.try_get("last_order_at")?,
                last_order_id: row.try_get("last_order_id")?,
                conversation_id: request
                    .relationships
                    .iter()
                    .find(|r| r.buyer_id == buyer_id)
                    .map(|r| r.conversation_id.clone())
                    .unwrap_or_default(),
            });
        }
        tx.commit().await?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_bounds_and_duplicate_relationships() {
        let mut r = Request {
            seller_id: 1,
            buyer_id: None,
            relationships: vec![Relationship {
                buyer_id: 2,
                conversation_id: "conversation".into(),
            }],
            query: String::new(),
            offset: 0,
            limit: 100,
        };
        assert!(r.validate().is_ok());
        r.relationships.push(r.relationships[0].clone());
        assert!(matches!(r.validate(), Err(Error::Invalid)));
        r.relationships.pop();
        r.limit = 101;
        assert!(matches!(r.validate(), Err(Error::Invalid)));
    }
}
