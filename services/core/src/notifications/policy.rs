//! Notification choices follow QuantumNous/new-api 6461f084ffcd. Financial
//! thresholds use exact credit_500k_usd integers, never display-currency floats.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_THRESHOLD: i64 = 9_007_199_254_740_991;
pub const POLICY_KEY: &str = "CoreNotificationsPolicyV1";
pub const CONSUMER: &str = "core-notifications-v1";
pub const SERVICE: &str = "core-notifications";
pub const BALANCE_EVENT: &str = "notifications.balance.v1";
pub const CHANNEL_EVENT: &str = "notifications.channel.v1";
pub const MODELS_EVENT: &str = "notifications.models.v1";
pub const EVENT_TYPES: [&str; 3] = [BALANCE_EVENT, CHANNEL_EVENT, MODELS_EVENT];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SitePolicy {
    pub revision: u32,
    pub quota_warning_enabled: bool,
    pub quota_warning_threshold: i64,
    pub channel_notify_enabled: bool,
    pub upstream_model_update_notify_enabled: bool,
}
impl SitePolicy {
    pub fn valid(&self) -> bool {
        self.revision == 1 && (1..=MAX_THRESHOLD).contains(&self.quota_warning_threshold)
    }
    pub fn enabled(&self, kind: Kind) -> bool {
        match kind {
            Kind::Balance => self.quota_warning_enabled,
            Kind::Channel => self.channel_notify_enabled,
            Kind::Models => self.upstream_model_update_notify_enabled,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Balance,
    Channel,
    Models,
}
impl Kind {
    pub fn event_type(self) -> &'static str {
        match self {
            Self::Balance => BALANCE_EVENT,
            Self::Channel => CHANNEL_EVENT,
            Self::Models => MODELS_EVENT,
        }
    }
}

// Existing profile JSON has unrelated keys. Read the notification subset while
// writes below merge only explicit fields into the locked original document.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Preferences {
    pub quota_warning_enabled: Option<bool>,
    pub channel_notify_enabled: Option<bool>,
    pub upstream_model_update_notify_enabled: Option<bool>,
    pub quota_warning_threshold: Option<i64>,
    pub notification_email: Option<String>,
    pub notify_type: Option<String>,
    pub language: Option<String>,
}
impl Preferences {
    pub fn read(raw: &str) -> Result<Self, ()> {
        let value: Value = serde_json::from_str(raw).map_err(|_| ())?;
        if !value.is_object() {
            return Err(());
        }
        let prefs: Self = serde_json::from_value(value).map_err(|_| ())?;
        if prefs
            .quota_warning_threshold
            .is_some_and(|n| !(0..=MAX_THRESHOLD).contains(&n))
        {
            return Err(());
        }
        Ok(prefs)
    }
    pub fn enabled(&self, kind: Kind, role: &str) -> bool {
        match kind {
            Kind::Balance => self.quota_warning_enabled.unwrap_or(true),
            Kind::Channel => {
                matches!(role, "admin" | "superadmin")
                    && self.channel_notify_enabled.unwrap_or(role == "superadmin")
            }
            Kind::Models => {
                matches!(role, "admin" | "superadmin")
                    && self.upstream_model_update_notify_enabled.unwrap_or(false)
            }
        }
    }
    pub fn threshold(&self, policy: &SitePolicy) -> i64 {
        self.quota_warning_threshold
            .filter(|n| *n > 0)
            .unwrap_or(policy.quota_warning_threshold)
    }
    pub fn email_matches(&self, verified_email: &str) -> bool {
        self.notify_type.as_deref().unwrap_or("email") == "email"
            && self
                .notification_email
                .as_deref()
                .is_none_or(|email| email.is_empty() || email.eq_ignore_ascii_case(verified_email))
            && crate::humangrant::mail::mailbox(verified_email).is_ok()
    }
}

/// Real economic wallet position includes holds and signed debt. A reservation
/// moves cash between buckets; it does not spend it or rearm the notification.
pub fn balance_edge(after: i64, delta: i64, threshold: i64) -> Option<i64> {
    let before = after.checked_sub(delta)?;
    (delta < 0 && threshold > 0 && before >= threshold && after < threshold).then_some(before)
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsUpdate {
    pub quota_warning_enabled: Option<bool>,
    pub channel_notify_enabled: Option<bool>,
    pub upstream_model_update_notify_enabled: Option<bool>,
    pub quota_warning_threshold: Option<i64>,
    pub notification_email: Option<String>,
    pub notify_type: Option<String>,
}
impl SettingsUpdate {
    pub fn valid(&self) -> bool {
        self.quota_warning_threshold
            .is_none_or(|n| (0..=MAX_THRESHOLD).contains(&n))
            && self
                .notify_type
                .as_deref()
                .is_none_or(|kind| kind == "email")
            && self.notification_email.as_ref().is_none_or(|email| {
                email.is_empty() || crate::humangrant::mail::mailbox(email).is_ok()
            })
    }
    pub fn merge(&self, document: &mut Value) -> Result<(), ()> {
        let Some(object) = document.as_object_mut() else {
            return Err(());
        };
        for (key, value) in [
            ("quota_warning_enabled", self.quota_warning_enabled),
            ("channel_notify_enabled", self.channel_notify_enabled),
            (
                "upstream_model_update_notify_enabled",
                self.upstream_model_update_notify_enabled,
            ),
        ] {
            if let Some(value) = value {
                object.insert(key.into(), Value::Bool(value));
            }
        }
        if let Some(value) = self.quota_warning_threshold {
            object.insert("quota_warning_threshold".into(), value.into());
        }
        for (key, value) in [
            ("notification_email", &self.notification_email),
            ("notify_type", &self.notify_type),
        ] {
            if let Some(value) = value {
                object.insert(key.into(), value.clone().into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy() -> SitePolicy {
        SitePolicy {
            revision: 1,
            quota_warning_enabled: true,
            quota_warning_threshold: 100,
            channel_notify_enabled: true,
            upstream_model_update_notify_enabled: true,
        }
    }

    #[test]
    fn defaults_and_roles_are_independent_of_service_level() {
        let prefs = Preferences::read(r#"{"language":"ja","sidebar_modules":"{}"}"#).unwrap();
        assert!(prefs.enabled(Kind::Balance, "user"));
        assert!(prefs.enabled(Kind::Channel, "superadmin"));
        assert!(!prefs.enabled(Kind::Channel, "admin"));
        assert!(!prefs.enabled(Kind::Channel, "user"));
        assert!(!prefs.enabled(Kind::Models, "superadmin"));
        let prefs = Preferences::read(
            r#"{"channel_notify_enabled":true,"upstream_model_update_notify_enabled":true}"#,
        )
        .unwrap();
        assert!(prefs.enabled(Kind::Channel, "admin"));
        assert!(!prefs.enabled(Kind::Channel, "user"));
        assert!(!prefs.enabled(Kind::Models, "user"));
    }

    #[test]
    fn patch_preserves_unspecified_preferences_and_authority_fields() {
        let mut value = json!({"language":"fr", "quota_warning_enabled":false,
            "channel_notify_enabled":true, "allow_key_bypass_ip_policy":false});
        let patch: SettingsUpdate =
            serde_json::from_value(json!({"quota_warning_threshold":0})).unwrap();
        patch.merge(&mut value).unwrap();
        assert_eq!(value["language"], "fr");
        assert_eq!(value["quota_warning_enabled"], false);
        assert_eq!(value["channel_notify_enabled"], true);
        assert_eq!(value["allow_key_bypass_ip_policy"], false);
        assert_eq!(
            Preferences::read(&value.to_string())
                .unwrap()
                .threshold(&policy()),
            100
        );
        assert!(
            serde_json::from_value::<SettingsUpdate>(json!({"allow_key_bypass_ip_policy":true}))
                .is_err()
        );
    }

    #[test]
    fn only_real_downward_crossing_rearms_after_real_credit() {
        assert_eq!(balance_edge(90, -10, 100), Some(100));
        assert_eq!(balance_edge(100, -10, 100), None);
        assert_eq!(balance_edge(80, -10, 100), None);
        assert_eq!(balance_edge(90, 0, 100), None); // reserve/release
        assert_eq!(balance_edge(120, 40, 100), None); // topup
        assert_eq!(balance_edge(99, -21, 100), Some(120));
        assert_eq!(balance_edge(-3, -103, 100), Some(100));
        assert_eq!(balance_edge(i64::MAX, -1, 100), None);
    }

    #[test]
    fn obsolete_destinations_and_unverified_methods_never_fall_back() {
        let mut prefs = Preferences::default();
        assert!(prefs.email_matches("verified@example.invalid"));
        prefs.notification_email = Some("different@example.invalid".into());
        assert!(!prefs.email_matches("verified@example.invalid"));
        prefs.notification_email = None;
        prefs.notify_type = Some("webhook".into());
        assert!(!prefs.email_matches("verified@example.invalid"));
        assert!(
            !SettingsUpdate {
                notify_type: Some("webhook".into()),
                ..Default::default()
            }
            .valid()
        );
        assert!(
            !SettingsUpdate {
                quota_warning_threshold: Some(-1),
                ..Default::default()
            }
            .valid()
        );
        assert!(Preferences::read(r#"{"quota_warning_threshold":1.5}"#).is_err());
    }
}
