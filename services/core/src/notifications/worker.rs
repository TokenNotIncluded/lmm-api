use super::{
    event_error, load_policy,
    policy::*,
    producer::{Evidence, Fact},
    storage,
};
use crate::{
    events::{self, CommandKey, Delivery, EventStore, InboxOutcome},
    humangrant::mail::{MailConfig, MailMessage, Mailer},
};
use serde::Deserialize;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::watch;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub mail: MailConfig,
}

pub struct NotificationRuntime {
    pool: PgPool,
    events: EventStore,
    mail: Mailer,
}
impl NotificationRuntime {
    /// Validate local configuration and existing durable subscription. This
    /// never provisions the consumer, changes policy, or sends a test message.
    pub async fn open(database_url: &str, config: &WorkerConfig) -> io::Result<Arc<Self>> {
        let mail = Mailer::open(&config.mail)?;
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_millis(500))
            .after_connect(|c, _| {
                Box::pin(async move {
                    for sql in [
                        "SET statement_timeout='1500ms'",
                        "SET lock_timeout='250ms'",
                        "SET idle_in_transaction_session_timeout='2s'",
                    ] {
                        sqlx::query(sql).execute(&mut *c).await?;
                    }
                    Ok(())
                })
            })
            .connect(database_url)
            .await
            .map_err(|_| invalid())?;
        let mut connection = pool.acquire().await.map_err(|_| invalid())?;
        if load_policy(&mut connection)
            .await
            .map_err(|_| invalid())?
            .is_none()
        {
            return Err(invalid());
        }
        drop(connection);
        let events = EventStore::from_pool(pool.clone())
            .with_lease_seconds(120)
            .map_err(|_| invalid())?;
        Ok(Arc::new(Self { pool, events, mail }))
    }

    async fn acknowledge_result(
        &self,
        delivery: &Delivery,
        fact: &Fact,
        status: &'static str,
    ) -> Result<(), sqlx::Error> {
        let key = command_key(
            fact.user_id,
            &delivery.event.key,
            "notifications.delivery.result",
        );
        let mut tx = self.pool.begin().await?;
        match events::begin_command(&mut tx, &key, &delivery.event.fingerprint)
            .await
            .map_err(event_error)?
        {
            InboxOutcome::New => {
                let result = serde_json::to_vec(&serde_json::json!({"status":status}))
                    .map_err(|_| storage())?;
                events::complete_command(&mut tx, &key, &result)
                    .await
                    .map_err(event_error)?;
            }
            InboxOutcome::Replay(_) => {}
        }
        events::acknowledge_transaction(
            &mut tx,
            SERVICE,
            CONSUMER,
            delivery.event.id,
            &delivery.lease_token,
        )
        .await
        .map_err(event_error)?;
        tx.commit().await?;
        Ok(())
    }

    async fn deliver(&self, delivery: &Delivery) -> Result<(), sqlx::Error> {
        let fact: Fact = serde_json::from_slice(&delivery.event.payload).map_err(|_| storage())?;
        let version = match &fact.evidence {
            Evidence::Balance { journal_id, .. } => *journal_id,
            Evidence::Routing { after_version, .. } | Evidence::Models { after_version, .. } => {
                i64::try_from(*after_version).map_err(|_| storage())?
            }
        };
        let fingerprint = events::NewEvent {
            key: &delivery.event.key,
            event_type: &delivery.event.event_type,
            schema_version: delivery.event.schema_version,
            resource_id: delivery.event.resource_id,
            resource_version: delivery.event.resource_version,
            payload: &delivery.event.payload,
        }
        .fingerprint();
        if !fact.valid()
            || delivery.event.id <= 0
            || fact.user_id != delivery.event.resource_id
            || version != delivery.event.resource_version
            || delivery.event.schema_version != 1
            || delivery.event.event_type != fact.kind.event_type()
            || fingerprint.as_slice() != delivery.event.fingerprint.as_slice()
        {
            return Err(storage());
        }
        let mut tx = self.pool.begin().await?;
        let attempt_key = command_key(
            fact.user_id,
            &delivery.event.key,
            "notifications.delivery.attempt",
        );
        // The immutable inbox receipt, not a lease-attempt counter, is the
        // no-resend boundary. Preflight DB failures remain safely retryable.
        match events::begin_command(&mut tx, &attempt_key, &delivery.event.fingerprint)
            .await
            .map_err(event_error)?
        {
            InboxOutcome::Replay(receipt) => {
                let status = match receipt.as_slice() {
                    b"attempt_started" => "unknown",
                    b"suppressed" => "suppressed",
                    _ => return Err(storage()),
                };
                tx.commit().await?;
                return self.acknowledge_result(delivery, &fact, status).await;
            }
            InboxOutcome::New => {}
        }
        let policy = load_policy(&mut tx).await?;
        let row = sqlx::query("SELECT u.platform_role,p.setting,p.email,p.email_login_allowed FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1 AND u.active AND a.active FOR SHARE OF u,a,p")
            .bind(fact.user_id).fetch_optional(&mut *tx).await?;
        let recipient = if let (Some(policy), Some(row)) = (policy, row) {
            let prefs =
                Preferences::read(&row.try_get::<String, _>("setting")?).map_err(|_| storage())?;
            let email: String = row.try_get("email")?;
            if policy.enabled(fact.kind)
                && prefs.enabled(fact.kind, &row.try_get::<String, _>("platform_role")?)
                && row.try_get::<bool, _>("email_login_allowed")?
                && prefs.email_matches(&email)
            {
                Some((email, prefs.language.unwrap_or_else(|| "en".into())))
            } else {
                None
            }
        } else {
            None
        };
        // A transaction that cannot COMMIT this receipt sends NOTHING. An
        // unknown COMMIT may leave a receipt but never authorizes a blind send.
        events::complete_command(
            &mut tx,
            &attempt_key,
            if recipient.is_some() {
                b"attempt_started"
            } else {
                b"suppressed"
            },
        )
        .await
        .map_err(event_error)?;
        tx.commit().await?;
        let Some((email, language)) = recipient else {
            return self.acknowledge_result(delivery, &fact, "suppressed").await;
        };
        let (subject, body) = render(&fact, &language);
        let id = format!("notification-{}", delivery.event.id);
        let accepted = self
            .mail
            .send_message(&MailMessage {
                id: &id,
                recipient: &email,
                subject,
                text: &body,
            })
            .await
            .is_ok();
        // Even a timeout or explicit provider rejection is terminal for this
        // attempt. No fallback, retry, or new idempotency key is manufactured.
        self.acknowledge_result(
            delivery,
            &fact,
            if accepted { "accepted" } else { "unknown" },
        )
        .await
    }
}

fn command_key<'a>(user: i64, event_key: &'a str, method: &'static str) -> CommandKey<'a> {
    CommandKey {
        service_id: SERVICE,
        actor_id: user,
        method,
        idempotency_key: event_key,
    }
}

fn invalid() -> io::Error {
    io::Error::other(
        "notification worker requires valid mail configuration and an explicitly provisioned site policy",
    )
}

pub fn spawn_worker(
    runtime: Arc<NotificationRuntime>,
    mut stopped: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if *stopped.borrow() {
                return;
            }
            match runtime.events.pull(SERVICE, CONSUMER, 1).await {
                Ok(deliveries) if !deliveries.is_empty() => {
                    for delivery in deliveries {
                        if runtime.deliver(&delivery).await.is_err() {
                            // If IO already started the next delivery consults
                            // its durable inbox and records unknown, without IO.
                            let _ = runtime
                                .events
                                .retry(
                                    SERVICE,
                                    CONSUMER,
                                    delivery.event.id,
                                    &delivery.lease_token,
                                    3,
                                )
                                .await;
                        }
                    }
                }
                _ => {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {},
                        changed = stopped.changed() => { if changed.is_err() || *stopped.borrow() { return; } }
                    }
                }
            }
        }
    })
}

fn render(fact: &Fact, language: &str) -> (&'static str, String) {
    // Locale comes from the current recipient, never a triggering administrator.
    let names = match language {
        "zh" | "zh-CN" | "zhCN" => (
            "余额不足提醒",
            "渠道配置已更新",
            "渠道模型已更新",
            "积分余额",
            "提醒阈值",
            "配置版本",
            "变更渠道",
            "新增模型",
            "移除模型",
        ),
        "zh-TW" | "zhTW" => (
            "餘額不足提醒",
            "渠道設定已更新",
            "渠道模型已更新",
            "點數餘額",
            "提醒門檻",
            "設定版本",
            "變更渠道",
            "新增模型",
            "移除模型",
        ),
        "ja" => (
            "残高低下のお知らせ",
            "チャネル設定の更新",
            "チャネルモデルの更新",
            "クレジット残高",
            "通知しきい値",
            "設定バージョン",
            "変更チャネル",
            "追加モデル",
            "削除モデル",
        ),
        "fr" => (
            "Alerte de solde faible",
            "Configuration des canaux mise à jour",
            "Modèles des canaux mis à jour",
            "Solde en crédits",
            "Seuil d’alerte",
            "Version de configuration",
            "Canaux modifiés",
            "Modèles ajoutés",
            "Modèles supprimés",
        ),
        "ru" => (
            "Уведомление о низком балансе",
            "Настройки каналов обновлены",
            "Модели каналов обновлены",
            "Баланс кредитов",
            "Порог уведомления",
            "Версия настроек",
            "Изменённые каналы",
            "Добавленные модели",
            "Удалённые модели",
        ),
        "vi" => (
            "Cảnh báo số dư thấp",
            "Đã cập nhật cấu hình kênh",
            "Đã cập nhật mô hình của kênh",
            "Số dư tín dụng",
            "Ngưỡng cảnh báo",
            "Phiên bản cấu hình",
            "Kênh đã thay đổi",
            "Mô hình đã thêm",
            "Mô hình đã xóa",
        ),
        _ => (
            "Low balance notification",
            "Channel configuration updated",
            "Channel models updated",
            "Credit balance",
            "Warning threshold",
            "Configuration version",
            "Changed channels",
            "Added models",
            "Removed models",
        ),
    };
    match &fact.evidence {
        Evidence::Balance {
            after_credits,
            threshold_credits,
            ..
        } => (
            names.0,
            format!(
                "{}: {}\r\n{}: {}\r\n\r\n500,000 credits = USD 1.\r\n",
                names.3, after_credits, names.4, threshold_credits
            ),
        ),
        Evidence::Routing {
            after_version,
            changed_channels,
            ..
        } => (
            names.1,
            format!(
                "{}: {}\r\n{}: {}\r\n",
                names.5, after_version, names.6, changed_channels
            ),
        ),
        Evidence::Models {
            after_version,
            added,
            removed,
            ..
        } => (
            names.2,
            format!(
                "{}: {}\r\n{}: {}\r\n{}: {}\r\n",
                names.5, after_version, names.7, added, names.8, removed
            ),
        ),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod durable_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mail_uses_recipient_language_and_no_private_routing_document() {
        let fact = Fact {
            revision: 1,
            kind: Kind::Balance,
            user_id: 1,
            evidence: Evidence::Balance {
                account_id: 2,
                journal_id: 3,
                before_credits: 100,
                after_credits: -2,
                threshold_credits: 100,
            },
        };
        let (subject, body) = render(&fact, "ja");
        assert_eq!(subject, "残高低下のお知らせ");
        assert!(body.contains("-2"));
        assert!(body.contains("100"));
        assert!(!body.contains("journal"));
        assert_ne!(subject, render(&fact, "fr").0);
    }
}
