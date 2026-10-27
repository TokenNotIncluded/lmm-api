//! An attempted supplier request with no authoritative final receipt retains
//! its entire hold. Only its original worker can persist this UNKNOWN marker.
use super::*;
impl Charging {
    /// Audit only: never feeds accepted usage, provider cost or recovery.
    pub(crate) async fn record_provider_observation(
        &self,
        id: &str,
        worker: &[u8; 32],
        route_id: &str,
        raw_usage: &Value,
        input_token_ceiling: i64,
        output_token_ceiling: u32,
    ) -> Result<()> {
        // @context-fold begin charging-provider-observation
        if !raw_usage.is_object() {
            return Err(Error::Invalid);
        }
        let detail = json!({
            "kind": "provider_observation",
            "provider_complete": true,
            "settlement_eligible": false,
            "route_id": route_id,
            "raw_usage": raw_usage,
            "input_token_ceiling": input_token_ceiling,
            "output_token_ceiling": output_token_ceiling,
        });
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let event_id = "provider-observation:terminal";
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            return commit(tx).await;
        }
        if !matches!(stored.charge.state.as_str(), "streaming" | "reconcile") {
            return Err(Error::WrongState);
        }
        event(&mut tx, id, event_id, detail).await?;
        commit(tx).await
        // @context-fold end charging-provider-observation
    }

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
