//! Explicit host assembly. Startup validates existing storage and never runs DDL.
use super::{HumanGrantEngine, HumanGrantStore, signature};
use axum::Router;
use sqlx::PgPool;
use std::{io, sync::Arc};

/// Build only when explicitly configured by the host, using its exact Web origin.
/// HTTP is accepted solely for explicitly enabled loopback development.
pub async fn open(pool: PgPool, origin: &str, local_http: bool) -> io::Result<Router> {
    open_with_mail(pool, origin, local_http, None)
        .await
        .map(|(router, _)| router)
}

pub async fn open_with_mail(
    pool: PgPool,
    origin: &str,
    local_http: bool,
    smtp: Option<&super::mail::SmtpConfig>,
) -> io::Result<(Router, Arc<HumanGrantEngine>)> {
    let canonical = signature::origin(origin, local_http)
        .map_err(|_| io::Error::other("invalid HumanGrant Web origin"))?;
    let store = HumanGrantStore::new(pool);
    check_ready(&store).await?;
    let mut engine = HumanGrantEngine::new(store, canonical, local_http);
    engine.mail = smtp
        .map(super::mail::SmtpMailer::open)
        .transpose()?
        .map(Arc::new);
    let engine = Arc::new(engine);
    Ok((super::router(engine.clone()), engine))
}

/// Check the seed and required privileges with no schema or data changes.
/// Core's assembled schema fingerprint check must run before this function.
pub async fn check_ready(store: &HumanGrantStore) -> io::Result<()> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege(current_user, 'core_humangrant', 'USAGE') \
         AND has_table_privilege(current_user, 'core_humangrant.human_grant_records', 'SELECT') \
         AND has_table_privilege(current_user, 'core_humangrant.human_grant_records', 'INSERT') \
         AND has_table_privilege(current_user, 'core_humangrant.human_grant_records', 'UPDATE') \
         AND has_table_privilege(current_user, 'core_humangrant.human_grant_records', 'DELETE') \
         AND has_table_privilege(current_user, 'core_humangrant.human_grant_gate', 'SELECT') \
         AND has_column_privilege(current_user, 'core_humangrant.human_grant_gate', 'revision', 'UPDATE')",
    )
    .fetch_one(store.pool())
    .await
    .map_err(|_| io::Error::other("HumanGrant storage is unavailable"))?;
    if !allowed {
        return Err(io::Error::other(
            "HumanGrant runtime privileges are incomplete",
        ));
    }
    let tx = store
        .begin()
        .await
        .map_err(|_| io::Error::other("HumanGrant gate is unavailable"))?;
    tx.rollback()
        .await
        .map_err(|_| io::Error::other("HumanGrant readiness rollback failed"))
}
