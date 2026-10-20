//! An attempted supplier request with no authoritative final receipt retains
//! its entire hold. Only its original worker can persist this UNKNOWN marker.
use super::*;
impl Charging {
    pub(crate) async fn mark_unknown(&self, id: &str, worker: &[u8; 32]) -> Result<()> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        match stored.charge.state.as_str() {
            "streaming" => {
                sqlx::query("UPDATE core_charging.requests SET state='reconcile',revision=revision+1 WHERE id=$1 AND state='streaming'").bind(id).execute(&mut *tx).await?;
                event(
                    &mut tx,
                    id,
                    "unknown",
                    json!({"kind":"unknown_provider_usage","hold_retained":true}),
                )
                .await?;
            }
            "reconcile" | "settled" => (),
            _ => return Err(Error::WrongState),
        }
        commit(tx).await
    }
}
