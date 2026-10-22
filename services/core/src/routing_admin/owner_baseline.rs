//! Explicit offline owner bootstrap. Never called by HTTP or service startup.
use super::{AdminError, Result, plan};
use crate::routing::store::ReleaseDocument;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

#[derive(Serialize)]
pub struct BaselineReceipt {
    pub contract: &'static str,
    pub routing_version: String,
    pub pricing_version: String,
    pub snapshot_sha256: String,
    pub created: bool,
}

/// Register the already reviewed host document as the first durable release.
/// This requires the actual installation owner, not inherited DML or wrapper
/// privileges. All references, sealed prices and the active pointer commit once.
pub async fn install_owner_baseline(
    pool: &PgPool,
    document: ReleaseDocument,
) -> Result<BaselineReceipt> {
    document.encode().map_err(|_| AdminError::Invalid)?;
    let document = document
        .into_router()
        .map_err(|_| AdminError::Invalid)?
        .catalog_document()
        .map_err(|_| AdminError::Invalid)?;
    crate::core_schema::check(pool)
        .await
        .map_err(|_| AdminError::Unavailable)?;
    let mut transaction = pool.begin().await?;
    sqlx::raw_sql(
        "SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL synchronous_commit=on; \
         SET LOCAL lock_timeout='3s'; SET LOCAL statement_timeout='5s'; \
         SET LOCAL idle_in_transaction_session_timeout='10s'",
    )
    .execute(&mut *transaction)
    .await?;
    let owner: bool = sqlx::query_scalar(
        r#"SELECT current_user=session_user AND count(*)=13
          AND bool_and(pg_catalog.pg_get_userbyid(c.relowner)=current_user)
        FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE c.relkind='r' AND (
          (n.nspname='core_meta' AND c.relname IN ('schema_contract','component_contract','runtime_contract'))
          OR (n.nspname='public' AND c.relname IN ('routing_credential_refs','routing_price_versions',
            'routing_config_versions','routing_active_release','routing_price_rules','routing_groups',
            'routing_upstreams','routing_routes','routing_targets','routing_aliases')))
        "#,
    )
    .fetch_one(&mut *transaction)
    .await?;
    if !owner {
        return Err(AdminError::Forbidden);
    }
    // Coordinate with installation and lock the active singleton before reading
    // it, so a repeated owner command observes the first committed publication.
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(741803120)")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("SELECT singleton FROM public.routing_active_release WHERE singleton FOR UPDATE")
        .fetch_one(&mut *transaction)
        .await?;
    let active: Option<Value> = sqlx::query_scalar("SELECT core_routing_admin.read_active()")
        .fetch_one(&mut *transaction)
        .await?;
    let created = if let Some(active) = active {
        let active: ReleaseDocument =
            serde_json::from_value(active).map_err(|_| AdminError::Unavailable)?;
        if active != document {
            return Err(AdminError::Conflict);
        }
        false
    } else {
        for upstream in &document.routing.upstreams {
            sqlx::query("INSERT INTO public.routing_credential_refs(id,revision) VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(upstream.credential.id as i64)
                .bind(upstream.credential.revision as i64)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query("SELECT public.routing_stage_prices($1)")
            .bind(serde_json::to_value(&document.pricing).map_err(|_| AdminError::Invalid)?)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT public.routing_publish_config($1,$2,NULL,NULL)")
            .bind(serde_json::to_value(&document.routing).map_err(|_| AdminError::Invalid)?)
            .bind(document.pricing.version as i64)
            .execute(&mut *transaction)
            .await?;
        true
    };
    let expected = plan::Expected::from_release(&document)?;
    transaction.commit().await?;
    Ok(BaselineReceipt {
        contract: "core.routing-admin.baseline.v1",
        routing_version: expected.routing,
        pricing_version: expected.pricing,
        snapshot_sha256: expected.snapshot_sha256,
        created,
    })
}
