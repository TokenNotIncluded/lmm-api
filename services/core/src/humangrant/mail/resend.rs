//! Resend delivery follows the upstream provider contract, adapted to Core's
//! durable at-most-once mail queue rather than the old Go option map.
//! Reference: QuantumNous/new-api 32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14.
//! API: https://resend.com/docs/api-reference/emails/send-email
use super::{MailMessage, mailbox};
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, time::Duration};
use zeroize::Zeroizing;

const ENDPOINT: &str = "https://api.resend.com/emails";
const MAX_RESPONSE: usize = 4096;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResendConfig {
    pub from: String,
    #[serde(default)]
    pub from_name: String,
    pub api_key_file: PathBuf,
}

// Deliberately no Debug/Serialize: authorization is never a status field.
pub struct ResendMailer {
    client: Client,
    endpoint: Url,
    authorization: HeaderValue,
    from: String,
}

fn invalid() -> io::Error {
    io::Error::other("invalid Resend mail configuration")
}

impl ResendMailer {
    /// Local validation only. The key comes from a private regular file; there
    /// is no caller-controlled host, insecure TLS mode, or activation test send.
    pub fn open(config: &ResendConfig) -> io::Result<Self> {
        let from = sender(config)?;
        let bytes = Zeroizing::new(crate::config::private_file(&config.api_key_file, 4096)?);
        let key = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        let authorization = authorization(key)?;
        let client = client(Duration::from_secs(20), true)?;
        Ok(Self {
            client,
            endpoint: Url::parse(ENDPOINT).map_err(|_| invalid())?,
            authorization,
            from,
        })
    }

    pub(crate) async fn send_message(&self, message: &MailMessage<'_>) -> Result<(), ()> {
        message.validate()?;
        let payload = Payload {
            from: &self.from,
            to: [message.recipient],
            subject: message.subject,
            text: message.text,
        };
        // A stable key supplies an additional provider deduplication guard.
        // Its 24-hour retention is NOT permission to retry an unknown send.
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Idempotency-Key", format!("lmm-mail/{}", message.id))
            .json(&payload)
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|size| size > MAX_RESPONSE)
            {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        accepted(&bytes)
    }
}

#[derive(Serialize)]
struct Payload<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    text: &'a str,
}

fn sender(config: &ResendConfig) -> io::Result<String> {
    let from = mailbox(&config.from).map_err(|_| invalid())?;
    let local = config.from.split('@').next().unwrap_or_default();
    if local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || config.from_name.len() > 256
        || config.from_name.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    Ok(lettre::message::Mailbox::new(
        (!config.from_name.is_empty()).then(|| config.from_name.clone()),
        from.email,
    )
    .to_string())
}

fn authorization(raw: &str) -> io::Result<HeaderValue> {
    let key = raw.trim_end_matches(['\r', '\n']);
    if key.is_empty() || key.len() > 4096 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid());
    }
    let bearer = Zeroizing::new(format!("Bearer {key}"));
    let mut value = HeaderValue::from_str(&bearer).map_err(|_| invalid())?;
    value.set_sensitive(true);
    Ok(value)
}

fn client(timeout: Duration, https_only: bool) -> io::Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .no_proxy()
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|_| invalid())
}

fn accepted(bytes: &[u8]) -> Result<(), ()> {
    #[derive(Deserialize)]
    struct Receipt {
        id: String,
    }
    if bytes.len() > MAX_RESPONSE {
        return Err(());
    }
    let receipt: Receipt = serde_json::from_slice(bytes).map_err(|_| ())?;
    if receipt.id.is_empty()
        || receipt.id.len() > 256
        || !receipt
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::humangrant::mail::MailConfig;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn config() -> ResendConfig {
        ResendConfig {
            from: "sender@example.invalid".into(),
            from_name: "Example notice".into(),
            api_key_file: "/not-opened-by-shape-tests".into(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn activation_reads_a_private_key_without_sending_and_rejects_symlink_or_public_key() {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
        let directory =
            std::env::temp_dir().join(format!("lmm-resend-local-{}", rand::random::<u64>()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let path = directory.join("key");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(b"synthetic-key-never-sent\n").unwrap();
        drop(file);
        let settings = ResendConfig {
            api_key_file: path.clone(),
            ..config()
        };
        let mailer = ResendMailer::open(&settings).unwrap();
        assert_eq!(mailer.endpoint.as_str(), ENDPOINT);
        assert!(mailer.authorization.is_sensitive());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(ResendMailer::open(&settings).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.join("link");
        symlink(&path, &link).unwrap();
        assert!(
            ResendMailer::open(&ResendConfig {
                api_key_file: link,
                ..config()
            })
            .is_err()
        );
    }

    #[test]
    fn selector_sender_key_and_receipt_fail_without_echoing_secrets() {
        assert!(
            serde_json::from_value::<MailConfig>(json!({
                "provider":"resend", "from":"sender@example.invalid", "from_name":"Example",
                "api_key_file":"/private/resend.key"
            }))
            .is_ok()
        );
        for value in [
            json!({"provider":"unknown"}),
            json!({"provider":"resend","from":"sender@example.invalid","api_key_file":"/private/key","host":"evil.example"}),
            json!({"provider":"resend","from":"sender@example.invalid","api_key_file":"/private/key","smtp":{"host":"other"}}),
        ] {
            assert!(serde_json::from_value::<MailConfig>(value).is_err());
        }
        assert!(
            sender(&config())
                .unwrap()
                .contains("sender@example.invalid")
        );
        for name in ["x\r\nBcc: other@example.invalid", "x\0y"] {
            assert!(
                sender(&ResendConfig {
                    from_name: name.into(),
                    ..config()
                })
                .is_err()
            );
        }
        for address in [
            "a@example.invalid,b@example.invalid",
            "Display <a@example.invalid>",
            "a..b@example.invalid",
        ] {
            assert!(
                sender(&ResendConfig {
                    from: address.into(),
                    ..config()
                })
                .is_err()
            );
        }
        for key in [
            "",
            " ",
            "fixture key",
            "fixture\r\nInjected: yes",
            "fixture\0key",
        ] {
            assert_eq!(
                authorization(key).unwrap_err().to_string(),
                "invalid Resend mail configuration"
            );
        }
        assert!(authorization("fixture-key\n").unwrap().is_sensitive());
        for body in [
            b"{}".as_slice(),
            br#"{"id":""}"#,
            br#"{"id":null}"#,
            br#"{"id":"ok","id":"again"}"#,
            br#"{"id":"ok"} trailing"#,
        ] {
            assert!(accepted(body).is_err());
        }
        assert!(accepted(br#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#).is_ok());
        assert!(accepted(&vec![b' '; MAX_RESPONSE + 1]).is_err());
    }

    async fn server(
        status: &str,
        body: &str,
        extra: &str,
    ) -> (Url, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint =
            Url::parse(&format!("http://{}/emails", listener.local_addr().unwrap())).unwrap();
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut part = [0u8; 2048];
                let count = stream.read(&mut part).await.unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&part[..count]);
                assert!(bytes.len() < 32768);
                if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..at]).unwrap();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= at + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(reply.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(25), listener.accept())
                    .await
                    .is_err()
            );
            bytes
        });
        (endpoint, task)
    }

    #[tokio::test]
    async fn actual_local_http_contract_never_redirects_retries_or_accepts_bad_receipts() {
        for (status, response, extra, success) in [
            ("200 OK", r#"{"id":"synthetic-accepted-id"}"#, "", true),
            ("200 OK", "{}", "", false),
            ("200 OK", r#"{"id":""}"#, "", false),
            (
                "429 Too Many Requests",
                r#"{"id":"not-accepted"}"#,
                "Retry-After: 0\r\n",
                false,
            ),
            ("302 Found", "{}", "Location: /redirect-target\r\n", false),
        ] {
            let (endpoint, capture) = server(status, response, extra).await;
            let mailer = ResendMailer {
                client: client(Duration::from_secs(1), false).unwrap(),
                endpoint,
                authorization: authorization("synthetic-local-key").unwrap(),
                from: sender(&config()).unwrap(),
            };
            let message = MailMessage {
                id: "local-mail-1",
                recipient: "recipient@example.invalid",
                subject: "Synthetic local notice",
                text: "Plain text \"content\"\nwith a second line.",
            };
            assert_eq!(mailer.send_message(&message).await.is_ok(), success);
            let bytes = tokio::time::timeout(Duration::from_secs(2), capture)
                .await
                .unwrap()
                .unwrap();
            let split = bytes
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap();
            let header = std::str::from_utf8(&bytes[..split])
                .unwrap()
                .to_ascii_lowercase();
            assert!(header.starts_with("post /emails http/1.1\r\n"));
            assert!(header.contains("authorization: bearer synthetic-local-key\r\n"));
            assert!(header.contains("idempotency-key: lmm-mail/local-mail-1\r\n"));
            let payload: serde_json::Value = serde_json::from_slice(&bytes[split + 4..]).unwrap();
            assert_eq!(payload["to"], json!([message.recipient]));
            assert_eq!(payload["subject"], message.subject);
            assert_eq!(payload["text"], message.text);
            assert!(payload.get("html").is_none());
        }
    }
}
