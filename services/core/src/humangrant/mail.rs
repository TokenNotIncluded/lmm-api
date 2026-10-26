//! Explicit SMTP configuration and durable, at-most-once dispatch attempts.
//! SMTP acceptance means the relay accepted the message, not inbox delivery.
use super::{HumanGrantEngine, HumanGrantTx, types::*};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use std::{io, net::IpAddr, path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub from: String,
    pub tls: SmtpTls,
    pub username: Option<String>,
    pub password_file: Option<PathBuf>,
    #[serde(default)]
    pub allow_insecure_loopback: bool,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTls {
    Starttls,
    Implicit,
    Plaintext,
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}
fn invalid() -> io::Error {
    io::Error::other("invalid HumanGrant SMTP configuration")
}

/// Accept a single bare mailbox only. Display names, header controls and lists
/// never enter generated headers or the queue.
pub fn mailbox(raw: &str) -> Result<Mailbox, ProtocolError> {
    if raw.len() > 254
        || raw.is_empty()
        || !raw.is_ascii()
        || raw
            .bytes()
            .any(|b| b <= 32 || b >= 127 || b"<>(),;:\\\"".contains(&b))
    {
        return Err(failure(400, "invalid_email"));
    }
    let address = raw
        .parse::<lettre::Address>()
        .map_err(|_| failure(400, "invalid_email"))?;
    Ok(Mailbox::new(None, address))
}
impl SmtpMailer {
    /// Constructs a transport without connecting or sending a test message.
    pub fn open(config: &SmtpConfig) -> io::Result<Self> {
        if config.port == 0
            || config.host.is_empty()
            || config.host.len() > 253
            || config
                .host
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && !b".-:".contains(&b))
        {
            return Err(invalid());
        }
        let from = mailbox(&config.from).map_err(|_| invalid())?;
        let mut builder = match config.tls {
            SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                .map_err(|_| invalid())?,
            SmtpTls::Implicit => {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host).map_err(|_| invalid())?
            }
            SmtpTls::Plaintext => {
                // A numeric IP avoids DNS rebinding of a purported local host.
                if !config.allow_insecure_loopback
                    || !config
                        .host
                        .parse::<IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
                    || config.username.is_some()
                    || config.password_file.is_some()
                {
                    return Err(invalid());
                }
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            }
        }
        .port(config.port)
        .timeout(Some(Duration::from_secs(15)));
        match (&config.username, &config.password_file) {
            (Some(username), Some(path)) if !username.is_empty() => {
                if !path.is_absolute()
                    || path
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir))
                {
                    return Err(invalid());
                }
                let mut prefix = PathBuf::new();
                for component in path.components() {
                    prefix.push(component);
                    if std::fs::symlink_metadata(&prefix)
                        .map_err(|_| invalid())?
                        .file_type()
                        .is_symlink()
                    {
                        return Err(invalid());
                    }
                }
                let meta = std::fs::symlink_metadata(path).map_err(|_| invalid())?;
                if !meta.is_file() || meta.len() > 4096 {
                    return Err(invalid());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if !matches!(meta.permissions().mode() & 0o777, 0o400 | 0o600) {
                        return Err(invalid());
                    }
                }
                let file = std::fs::File::open(path).map_err(|_| invalid())?;
                let opened = file.metadata().map_err(|_| invalid())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::{MetadataExt, PermissionsExt};
                    if meta.dev() != opened.dev()
                        || meta.ino() != opened.ino()
                        || !matches!(opened.permissions().mode() & 0o777, 0o400 | 0o600)
                    {
                        return Err(invalid());
                    }
                }
                use std::io::Read;
                let mut secret = zeroize::Zeroizing::new(String::new());
                file.take(4097)
                    .read_to_string(&mut secret)
                    .map_err(|_| invalid())?;
                if secret.len() > 4096 {
                    return Err(invalid());
                }
                let secret = secret.trim_end_matches(['\r', '\n']);
                if secret.is_empty() || secret.contains(['\r', '\n', '\0']) {
                    return Err(invalid());
                }
                builder =
                    builder.credentials(Credentials::new(username.clone(), secret.to_owned()));
            }
            (None, None) => {}
            _ => return Err(invalid()),
        }
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
    /// Attempts delivery without exposing recipient or SMTP failure details.
    ///
    /// # Errors
    /// Returns `email_delivery_unavailable` when construction or delivery fails.
    pub async fn send(&self, job: &MailJob) -> Result<(), ProtocolError> {
        let message = Message::builder().from(self.from.clone())
            .to(mailbox(&job.destination).map_err(|_| failure(503, "email_delivery_unavailable"))?)
            .message_id(Some(format!("<{}@humangrant.invalid>", job.id)))
            .subject("HumanGrant authorization request")
            .header(ContentType::TEXT_PLAIN)
            .body(format!("An AI requested your permission to read an account profile.\r\n\r\nOpen this link to inspect the request, sign in, and choose whether to approve:\r\n{}\r\n\r\nThis invitation expires with the request. Ignore it if you did not expect it. Opening it does not approve access.\r\n", job.url)).map_err(|_| failure(503, "email_delivery_unavailable"))?;
        tokio::time::timeout(Duration::from_secs(30), self.transport.send(message))
            .await
            .map_err(|_| failure(503, "email_delivery_unavailable"))?
            .map(|_| ())
            .map_err(|_| failure(503, "email_delivery_unavailable"))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MailJob {
    pub id: String,
    pub request_id: String,
    pub destination: String,
    pub url: String,
    pub state: String,
    pub attempted_at: i64,
    pub retain_until: i64,
}

/// The caller must authenticate the signed request owner before invoking this.
pub async fn enqueue(
    engine: &HumanGrantEngine,
    tx: &mut HumanGrantTx,
    request: &mut Request,
    input: EmailRequest,
    now: i64,
) -> Result<String, ProtocolError> {
    if engine.mail.is_none() {
        return Err(failure(503, "email_delivery_unavailable"));
    }
    mailbox(&input.email)?;
    if input.idempotency_key.len() < 16
        || input.idempotency_key.len() > 128
        || !input
            .idempotency_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(failure(400, "invalid_request"));
    }
    let id = super::signature::fingerprint(
        format!("{}:{}", request.id, input.idempotency_key).as_bytes(),
    );
    if let Some(job) = tx.get::<MailJob>("mail", &id, now).await? {
        return Ok(job.state);
    }
    // An uncertain previous SMTP attempt must be reconciled by an operator. A
    // new idempotency key is not permission to blindly duplicate that attempt.
    if request.email_status == "unknown"
        || request.email_status == "sending"
        || request.email_status == "queued"
    {
        return Err(failure(409, "email_attempt_pending"));
    }
    if request.last_email > now - 60
        || !engine
            .check_limits(
                tx,
                &[
                    (&format!("mail-actor:{}", request.actor), 3600, 5),
                    (
                        &format!(
                            "mail-destination:{}",
                            super::signature::fingerprint(
                                input.email.to_ascii_lowercase().as_bytes()
                            )
                        ),
                        3600,
                        3,
                    ),
                    ("mail-global", 3600, 100),
                ],
                now,
            )
            .await?
    {
        return Err(failure(429, "rate_limited"));
    }
    let job = MailJob {
        id: id.clone(),
        request_id: request.id.clone(),
        destination: input.email,
        url: format!("{}/human/{}", engine.origin, request.link),
        state: "queued".into(),
        attempted_at: 0,
        retain_until: request.created + RETENTION_SECONDS,
    };
    tx.put("mail", &id, "", job.retain_until, &job).await?;
    request.last_email = now;
    request.email_status = "queued".into();
    tx.put(
        "request",
        &request.id,
        &request.actor,
        request.created + RETENTION_SECONDS,
        request,
    )
    .await?;
    Ok("queued".into())
}

async fn finish(
    tx: &mut HumanGrantTx,
    job: &mut MailJob,
    state: &str,
    now: i64,
) -> Result<(), ProtocolError> {
    job.state = state.into();
    job.destination.clear();
    job.url.clear();
    tx.put("mail", &job.id, "", job.retain_until, job).await?;
    if let Some(mut request) = tx.get::<Request>("request", &job.request_id, now).await? {
        request.email_status = state.into();
        tx.put(
            "request",
            &request.id,
            &request.actor,
            request.created + RETENTION_SECONDS,
            &request,
        )
        .await?;
    }
    Ok(())
}

/// One bounded dispatch. The global HumanGrant gate serializes claims across
/// processes; no database lock is held during network IO. A lost SMTP ACK or
/// crashed sender becomes unknown and is never retried automatically.
pub async fn dispatch_one(engine: &HumanGrantEngine, now: i64) -> Result<bool, ProtocolError> {
    let Some(mailer) = &engine.mail else {
        return Ok(false);
    };
    let mut tx = engine.store.begin().await?;
    let raw: Option<String> = sqlx::query_scalar("SELECT payload FROM core_humangrant.human_grant_records WHERE kind='mail' AND (expires_at=0 OR expires_at>$1) AND (payload::jsonb->>'state'='queued' OR (payload::jsonb->>'state'='sending' AND (payload::jsonb->>'attempted_at')::bigint <= $1 - 120)) ORDER BY id LIMIT 1")
        .bind(now).fetch_optional(&mut *tx.tx).await.map_err(|_| failure(500,"internal_error"))?;
    let Some(raw) = raw else {
        tx.rollback().await?;
        return Ok(false);
    };
    let mut job: MailJob =
        serde_json::from_str(&raw).map_err(|_| failure(500, "internal_error"))?;
    if job.state == "sending" {
        finish(&mut tx, &mut job, "unknown", now).await?;
        tx.commit().await?;
        return Ok(true);
    }
    let request = tx.get::<Request>("request", &job.request_id, now).await?;
    if let Some(mut request) = request {
        if let Err(error) = engine.check_pending_request(&mut tx, &request, now).await {
            if error.status >= 500 {
                return Err(error);
            }
            finish(&mut tx, &mut job, "cancelled", now).await?;
            tx.commit().await?;
            return Ok(true);
        }
        job.state = "sending".into();
        job.attempted_at = now;
        request.email_status = "sending".into();
        tx.put(
            "request",
            &request.id,
            &request.actor,
            request.created + RETENTION_SECONDS,
            &request,
        )
        .await?;
    } else {
        finish(&mut tx, &mut job, "cancelled", now).await?;
        tx.commit().await?;
        return Ok(true);
    }
    tx.put("mail", &job.id, "", job.retain_until, &job).await?;
    tx.commit().await?;
    let state = if mailer.send(&job).await.is_ok() {
        "sent"
    } else {
        "unknown"
    };
    let mut tx = engine.store.begin().await?;
    finish(&mut tx, &mut job, state, chrono::Utc::now().timestamp()).await?;
    tx.commit().await?;
    Ok(true)
}

/// Host owns shutdown and joins this task; dropping HTTP does not stop it.
pub fn spawn_worker(
    engine: Arc<HumanGrantEngine>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if *shutdown.borrow() {
                break;
            }
            // Errors intentionally omit destination, link, credentials and SMTP
            // provider text. A failed final write leaves a recoverable sending.
            let active = dispatch_one(&engine, chrono::Utc::now().timestamp()).await;
            let wait = if matches!(active, Ok(true)) {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(2)
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {},
                changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break; } },
            }
        }
    })
}
