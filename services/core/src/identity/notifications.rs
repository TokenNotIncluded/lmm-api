//! Session-scoped notification preferences. No price, key policy or financial
//! permission is accepted by this endpoint.
use super::*;
use crate::notifications::{Kind, Preferences, SettingsUpdate};

impl IdentityStore {
    pub async fn notification_settings(&self, secret: &str) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        let actor = session(&mut tx, secret).await?;
        let value = view(&mut tx, actor.user_id).await?;
        tx.commit().await?;
        Ok(value)
    }

    pub async fn update_notification_settings(
        &self,
        secret: &str,
        input: &SettingsUpdate,
    ) -> Result<Value> {
        if !input.valid() {
            return Err(IdentityError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let actor = session(&mut tx, secret).await?;
        let row = sqlx::query("SELECT u.platform_role,p.setting,p.email,p.email_login_allowed FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1 FOR SHARE OF u FOR UPDATE OF p")
            .bind(actor.user_id).fetch_one(&mut *tx).await?;
        let role: String = row.try_get("platform_role")?;
        if !matches!(role.as_str(), "admin" | "superadmin")
            && (input.channel_notify_enabled == Some(true)
                || input.upstream_model_update_notify_enabled == Some(true))
        {
            return Err(IdentityError::Forbidden);
        }
        let email: String = row.try_get("email")?;
        if input.notification_email.as_ref().is_some_and(|value| {
            !value.is_empty()
                && (!row.get::<bool, _>("email_login_allowed")
                    || !value.eq_ignore_ascii_case(&email))
        }) {
            return Err(IdentityError::Invalid);
        }
        let raw: String = row.try_get("setting")?;
        let mut value: Value = serde_json::from_str(&raw).map_err(|_| IdentityError::Storage)?;
        input
            .merge(&mut value)
            .map_err(|_| IdentityError::Storage)?;
        sqlx::query("UPDATE core_identity.user_profiles SET setting=$2 WHERE user_id=$1")
            .bind(actor.user_id)
            .bind(value.to_string())
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            &actor,
            "notifications.preferences.update",
            json!({"user_id":actor.user_id}),
        )
        .await?;
        let result = view(&mut tx, actor.user_id).await?;
        tx.commit().await?;
        Ok(result)
    }
}

async fn view(c: &mut PgConnection, user: i64) -> Result<Value> {
    let row = sqlx::query("SELECT u.platform_role,p.setting,p.email,p.email_login_allowed FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1")
        .bind(user).fetch_one(&mut *c).await?;
    let prefs = Preferences::read(&row.try_get::<String, _>("setting")?)
        .map_err(|_| IdentityError::Storage)?;
    let role: String = row.try_get("platform_role")?;
    let email: String = row.try_get("email")?;
    let verified = row.try_get::<bool, _>("email_login_allowed")?
        && crate::humangrant::mail::mailbox(&email).is_ok();
    let policy = crate::notifications::load_policy(c).await?;
    Ok(json!({
        "user_id":user,"notify_type":prefs.notify_type.as_deref().unwrap_or("email"),
        "quota_warning_enabled":prefs.enabled(Kind::Balance, &role),
        "channel_notify_enabled":prefs.enabled(Kind::Channel, &role),
        "upstream_model_update_notify_enabled":prefs.enabled(Kind::Models, &role),
        "quota_warning_threshold":prefs.quota_warning_threshold.unwrap_or(0),
        "notification_email":if verified {email.as_str()} else {""},
        "verified_email_available":verified,
        "admin_notifications_allowed":matches!(role.as_str(), "admin" | "superadmin"),
        "site":policy
    }))
}
