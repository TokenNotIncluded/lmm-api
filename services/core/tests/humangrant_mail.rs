//! Real TCP SMTP exchange and PostgreSQL outbox transition checks.
use lmm_core::{
    core_schema,
    humangrant::{self, HumanGrantEngine, HumanGrantStore, mail::*, types::*},
};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

async fn smtp(accept: Option<bool>) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        stream
            .get_mut()
            .write_all(b"220 local.test ESMTP\r\n")
            .await
            .unwrap();
        let mut content = String::new();
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            let response = if line.starts_with("EHLO") {
                "250 local.test\r\n"
            } else if line.starts_with("MAIL FROM:") || line.starts_with("RCPT TO:") {
                "250 OK\r\n"
            } else if line.starts_with("DATA") {
                stream
                    .get_mut()
                    .write_all(b"354 Send data\r\n")
                    .await
                    .unwrap();
                loop {
                    let mut body = String::new();
                    if stream.read_line(&mut body).await.unwrap() == 0 {
                        return content;
                    }
                    if body == ".\r\n" {
                        break;
                    }
                    content.push_str(&body);
                }
                match accept {
                    Some(true) => "250 queued\r\n",
                    Some(false) => "451 rejected after DATA\r\n",
                    None => return content,
                }
            } else if line.starts_with("QUIT") {
                stream.get_mut().write_all(b"221 Bye\r\n").await.unwrap();
                break;
            } else {
                panic!("unexpected SMTP command");
            };
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        }
        content
    });
    (port, task)
}
fn config(port: u16) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        from: "sender@example.test".into(),
        tls: SmtpTls::Plaintext,
        username: None,
        password_file: None,
        allow_insecure_loopback: true,
    }
}
fn job(now: i64) -> MailJob {
    MailJob {
        id: "mail-id".into(),
        request_id: "request".into(),
        destination: "human@example.test".into(),
        url: "https://auth.example.test/human/secret-link".into(),
        state: "queued".into(),
        attempted_at: 0,
        retain_until: now + 600,
    }
}
#[tokio::test]
async fn smtp_sends_actual_message_and_requires_positive_acceptance() {
    for accept in [Some(true), Some(false), None] {
        let (port, capture) = smtp(accept).await;
        let mailer = SmtpMailer::open(&config(port)).unwrap();
        assert_eq!(
            mailer
                .send(&job(chrono::Utc::now().timestamp()))
                .await
                .is_ok(),
            accept == Some(true)
        );
        let captured = tokio::time::timeout(std::time::Duration::from_secs(3), capture)
            .await
            .unwrap()
            .unwrap();
        assert!(captured.contains("To: human@example.test"));
        assert!(captured.contains("Content-Type: text/plain"));
        assert!(captured.contains("https://auth.example.test/human/secret-link"));
        assert!(captured.contains("Message-ID: <mail-id@humangrant.invalid>"));
        assert!(!captured.contains("Authorization:"));
    }
}
#[test]
fn insecure_transport_and_header_injection_are_rejected() {
    let mut settings = config(25);
    settings.allow_insecure_loopback = false;
    assert!(SmtpMailer::open(&settings).is_err());
    settings.allow_insecure_loopback = true;
    settings.host = "localhost".into();
    assert!(SmtpMailer::open(&settings).is_err());
    settings.host = "192.0.2.1".into();
    assert!(SmtpMailer::open(&settings).is_err());
    for raw in [
        "a@example.test\r\nBcc: victim@example.test",
        "Name <a@example.test>",
        "a@example.test,b@example.test",
    ] {
        assert!(mailbox(raw).is_err());
    }
}
fn request(now: i64) -> Request {
    Request {
        id: "request".into(),
        link: "secret-link".into(),
        actor: "actor".into(),
        public_key: "key".into(),
        input: CreateRequest {
            client_id: CLIENT_ID.into(),
            target_service: SERVICE_ID.into(),
            purpose: "read profile".into(),
            requested_permissions: vec![Permission {
                action: PROFILE_READ.into(),
                resource: "1".into(),
            }],
            expected_human_id: None,
            session_ttl_seconds: 600,
        },
        epoch: 1,
        created: now,
        updated: now,
        expires: now + 600,
        phase: "waiting_human".into(),
        poll_after: 5,
        last_poll: 0,
        email_status: "not_requested".into(),
        last_email: 0,
        grant_id: None,
    }
}
#[sqlx::test(migrations = false)]
async fn outbox_dispatches_once_and_recovers_uncertain_attempt_without_resending(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    sqlx::raw_sql(humangrant::DDL).execute(&pool).await.unwrap();
    let (port, capture) = smtp(Some(true)).await;
    let mut engine = HumanGrantEngine::new(
        HumanGrantStore::new(pool.clone()),
        "https://auth.example.test".into(),
        false,
    );
    engine.mail = Some(Arc::new(SmtpMailer::open(&config(port)).unwrap().into()));
    let now = chrono::Utc::now().timestamp();
    let mut req = request(now);
    let input = EmailRequest {
        email: "human@example.test".into(),
        idempotency_key: "same-invitation-123".into(),
    };
    let mut tx = engine.store.begin().await.unwrap();
    engine
        .check_pending_request(&mut tx, &req, now)
        .await
        .unwrap();
    assert_eq!(
        enqueue(&engine, &mut tx, &mut req, input.clone(), now)
            .await
            .unwrap(),
        "queued"
    );
    assert_eq!(
        enqueue(&engine, &mut tx, &mut req, input.clone(), now)
            .await
            .unwrap(),
        "queued"
    );
    tx.commit().await.unwrap();
    assert!(dispatch_one(&engine, now).await.unwrap());
    assert!(capture.await.unwrap().contains("secret-link"));
    assert!(!dispatch_one(&engine, now).await.unwrap());
    let mut tx = engine.store.begin().await.unwrap();
    let jobs = tx.list::<MailJob>("mail", "", 10, now).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, "sent");
    assert!(jobs[0].destination.is_empty() && jobs[0].url.is_empty());
    let mut recovered = job(now);
    recovered.state = "sending".into();
    recovered.attempted_at = now - 121;
    tx.put(
        "mail",
        &recovered.id,
        "",
        recovered.retain_until,
        &recovered,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    // Listener has closed. Recovery must perform no network attempt.
    assert!(dispatch_one(&engine, now).await.unwrap());
    let mut tx = engine.store.begin().await.unwrap();
    let recovered: MailJob = tx.get("mail", "mail-id", now).await.unwrap().unwrap();
    assert_eq!(recovered.state, "unknown");
    assert!(recovered.destination.is_empty());
    let mut req: Request = tx.get("request", "request", now).await.unwrap().unwrap();
    assert_eq!(req.email_status, "unknown");
    let second = EmailRequest {
        email: "human@example.test".into(),
        idempotency_key: "different-key-123456".into(),
    };
    assert_eq!(
        enqueue(&engine, &mut tx, &mut req, second, now + 61)
            .await
            .unwrap_err()
            .error,
        "email_attempt_pending"
    );
    tx.rollback().await.unwrap();
    assert!(!dispatch_one(&engine, now).await.unwrap());
}
