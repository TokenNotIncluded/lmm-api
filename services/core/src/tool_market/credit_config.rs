//! Runtime market policy changes require a current personal superadmin Session.
//! Existing calls keep their original recipients/rate; new calls read this row.
use super::{credit_access::CreditStore, *};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigRequest {
    pub operation_id: String,
    pub enabled: bool,
    pub fee_bps: i64,
    pub recipient_id: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub fee_bps: i64,
    pub recipient_id: i64,
    pub updated_by: i64,
    pub updated_at: i64,
    pub version: i64,
}
#[derive(Serialize)]
pub struct ConfigReceipt {
    pub operation_id: String,
    #[serde(flatten)]
    pub config: Config,
}
impl CreditStore {
    pub async fn config(&self) -> Result<Config> {
        let row = sqlx::query("SELECT * FROM core_tool_market.credit_policy WHERE singleton")
            .fetch_one(&self.base.pool)
            .await?;
        Ok(Config {
            enabled: row.try_get("enabled")?,
            fee_bps: row.try_get("fee_bps")?,
            recipient_id: row
                .try_get::<Option<i64>, _>("platform_user_id")?
                .unwrap_or(0),
            updated_by: row.try_get::<Option<i64>, _>("updated_by")?.unwrap_or(0),
            updated_at: row.try_get("updated_at")?,
            version: row.try_get("version")?,
        })
    }
    pub async fn update_config(&self, secret: &str, r: &ConfigRequest) -> Result<ConfigReceipt> {
        if !name(&r.operation_id) || !(0..=10000).contains(&r.fee_bps) || r.recipient_id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        let role: String = sqlx::query_scalar(
            "SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if role != "superadmin" {
            return Err(Error::Forbidden);
        }
        if crate::identity::has_privileged_import_facts(&mut tx, actor.user_id)
            .await
            .map_err(|_| Error::Storage)?
        {
            crate::identity::load_complete_policy(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Forbidden)?;
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "credit-access:{}:{}",
                actor.user_id, r.operation_id
            ))
            .execute(&mut *tx)
            .await?;
        let prior =
            Self::prior(&mut tx, actor.user_id, &r.operation_id, "config-update", r).await?;
        let wallet:i64=sqlx::query_scalar("SELECT u.personal_account_id FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND u.active AND a.active AND u.platform_role='superadmin' FOR SHARE OF u,a").bind(r.recipient_id).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let row =
            sqlx::query("SELECT * FROM core_tool_market.credit_policy WHERE singleton FOR UPDATE")
                .fetch_one(&mut *tx)
                .await?;
        if prior.is_some() {
            if row.try_get::<bool, _>("enabled")? != r.enabled
                || row.try_get::<i64, _>("fee_bps")? != r.fee_bps
                || row.try_get::<Option<i64>, _>("platform_user_id")? != Some(r.recipient_id)
            {
                return Err(Error::Conflict);
            }
            let config = Config {
                enabled: r.enabled,
                fee_bps: r.fee_bps,
                recipient_id: r.recipient_id,
                updated_by: row.try_get::<i64, _>("updated_by")?,
                updated_at: row.try_get("updated_at")?,
                version: row.try_get("version")?,
            };
            tx.commit().await?;
            return Ok(ConfigReceipt {
                operation_id: r.operation_id.clone(),
                config,
            });
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        let version:i64=sqlx::query_scalar("UPDATE core_tool_market.credit_policy SET version=version+1,enabled=$1,platform_user_id=$2,platform_account_id=$3,fee_bps=$4,updated_by=$5,updated_at=$6 WHERE singleton RETURNING version").bind(r.enabled).bind(r.recipient_id).bind(wallet).bind(r.fee_bps).bind(actor.user_id).bind(now).fetch_one(&mut *tx).await?;
        let config = Config {
            enabled: r.enabled,
            fee_bps: r.fee_bps,
            recipient_id: r.recipient_id,
            updated_by: actor.user_id,
            updated_at: now,
            version,
        };
        Self::record(
            &mut tx,
            actor.user_id,
            &r.operation_id,
            "config-update",
            r,
            "config",
            now,
            Some(serde_json::to_value(&config).map_err(|_| Error::Invalid)?),
        )
        .await?;
        tx.commit().await?;
        Ok(ConfigReceipt {
            operation_id: r.operation_id.clone(),
            config,
        })
    }
}
