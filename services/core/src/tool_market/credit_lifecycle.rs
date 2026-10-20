//! Publication lifecycle remains separate from administrator review approval.
use super::{credit_access::CreditStore, *};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRequest {
    pub operation_id: String,
    pub service_id: String,
    pub paused: bool,
    pub deleted: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleReceipt {
    #[serde(flatten)]
    pub request: LifecycleRequest,
    pub suspended: bool,
    pub revision: i64,
    pub updated_by: i64,
    pub updated_at: i64,
}
impl CreditStore {
    pub async fn lifecycle(&self, secret: &str, r: &LifecycleRequest) -> Result<LifecycleReceipt> {
        if !name(&r.operation_id) || !name(&r.service_id) {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "credit-access:{}:{}",
                actor.user_id, r.operation_id
            ))
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("credit-service:{}", r.service_id))
            .execute(&mut *tx)
            .await?;
        let published_owners:Vec<i64>=sqlx::query_scalar("SELECT DISTINCT seller_user_id FROM core_tool_market.credit_quotes WHERE snapshot->>'service_id'=$1 AND seller_user_id IS NOT NULL").bind(&r.service_id).fetch_all(&mut *tx).await?;
        let own_state:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_tool_market.credit_services WHERE owner_user_id=$1 AND service_id=$2)").bind(actor.user_id).bind(&r.service_id).fetch_one(&mut *tx).await?;
        if !own_state && published_owners.len() > 1 {
            return Err(Error::Conflict);
        }
        let target_owner = if own_state || published_owners.is_empty() {
            actor.user_id
        } else {
            published_owners[0]
        };
        if r.deleted && target_owner == actor.user_id {
            sqlx::query("INSERT INTO core_tool_market.credit_services(service_id,owner_user_id,updated_by,updated_at) VALUES($1,$2,$2,extract(epoch FROM clock_timestamp())::BIGINT) ON CONFLICT(owner_user_id,service_id) DO NOTHING").bind(&r.service_id).bind(actor.user_id).execute(&mut *tx).await?;
        }
        let old = Self::prior(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "service-lifecycle",
            r,
        )
        .await?;
        let row = sqlx::query(
            "SELECT * FROM core_tool_market.credit_services WHERE service_id=$1 AND owner_user_id=$2 FOR UPDATE",
        )
        .bind(&r.service_id)
        .bind(target_owner)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Missing)?;
        let owner: i64 = row.try_get("owner_user_id")?;
        let role: String =
            sqlx::query_scalar("SELECT platform_role FROM core_identity.users WHERE id=$1")
                .bind(actor.user_id)
                .fetch_one(&mut *tx)
                .await?;
        if owner != actor.user_id && !matches!(role.as_str(), "admin" | "superadmin") {
            return Err(Error::Forbidden);
        }
        if owner != actor.user_id
            && crate::identity::has_privileged_import_facts(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Storage)?
        {
            crate::identity::load_complete_policy(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Forbidden)?;
        }
        if let Some(old) = old {
            let Json(out): Json<LifecycleReceipt> = old
                .try_get::<Option<Json<LifecycleReceipt>>, _>("details")?
                .ok_or(Error::Conflict)?;
            if out.revision != row.try_get::<i64, _>("revision")? {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(out);
        }
        if row.try_get::<bool, _>("deleted")? && !r.deleted {
            return Err(Error::Conflict);
        }
        let suspended = if owner == actor.user_id {
            row.try_get("suspended")?
        } else {
            r.paused
        };
        if owner == actor.user_id && suspended && !r.paused && !r.deleted {
            return Err(Error::Forbidden);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        let out = LifecycleReceipt {
            request: r.clone(),
            suspended,
            revision: row
                .try_get::<i64, _>("revision")?
                .checked_add(1)
                .ok_or(Error::Invalid)?,
            updated_by: actor.user_id,
            updated_at: now,
        };
        sqlx::query("UPDATE core_tool_market.credit_services SET paused=$2,deleted=$3,suspended=$4,revision=$5,updated_by=$6,updated_at=$7 WHERE service_id=$1 AND owner_user_id=$8").bind(&r.service_id).bind(r.paused).bind(r.deleted).bind(suspended).bind(out.revision).bind(actor.user_id).bind(now).bind(owner).execute(&mut *tx).await?;
        Self::record(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "service-lifecycle",
            r,
            &r.service_id,
            now,
            Some(serde_json::to_value(&out).map_err(|_| Error::Invalid)?),
        )
        .await?;
        tx.commit().await?;
        Ok(out)
    }
    pub(super) async fn check_service(
        tx: &mut Transaction<'_, Postgres>,
        owner: i64,
        service: &str,
    ) -> Result<()> {
        let ok:bool=sqlx::query_scalar("SELECT NOT paused AND NOT deleted AND NOT suspended FROM core_tool_market.credit_services WHERE service_id=$1 AND owner_user_id=$2 FOR SHARE").bind(service).bind(owner).fetch_optional(&mut **tx).await?.ok_or(Error::Forbidden)?;
        if !ok {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
    pub(super) async fn check_version(
        tx: &mut Transaction<'_, Postgres>,
        owner: i64,
        service: &str,
        version: &str,
    ) -> Result<()> {
        let current:Option<String>=sqlx::query_scalar("SELECT active_version_id FROM core_tool_market.credit_services WHERE owner_user_id=$1 AND service_id=$2 AND NOT paused AND NOT deleted AND NOT suspended FOR SHARE").bind(owner).bind(service).fetch_optional(&mut **tx).await?;
        if current.as_deref() != Some(version) {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
}
