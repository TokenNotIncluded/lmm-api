//! Isolated PostgreSQL/loopback tests. No running service, external mailbox or
//! production configuration participates. These require the normal SQLx test DB.
use super::*;
use crate::billing::{
    Credits,
    ledger::{self, Action, Ledger, Outcome, Request},
};
use crate::identity::{IdentityError, IdentityStore};
use crate::notifications::{PolicyInstallation, SettingsUpdate, SitePolicy, install_policy};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

struct Fixture {
    ledger: Ledger,
    identity: IdentityStore,
    session: String,
    account: i64,
}
fn policy() -> SitePolicy {
    SitePolicy {
        revision: 1,
        quota_warning_enabled: true,
        quota_warning_threshold: 100,
        channel_notify_enabled: true,
        upstream_model_update_notify_enabled: true,
    }
}
async fn fresh(pool: &PgPool) -> Fixture {
    let identity = IdentityStore::from_pool(pool.clone());
    identity.init_database().await.unwrap();
    sqlx::raw_sql(ledger::SCHEMA).execute(pool).await.unwrap();
    sqlx::raw_sql(crate::events::SCHEMA)
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../schema/console.sql"))
        .execute(pool)
        .await
        .unwrap();
    let session = identity.bootstrap_user(1, 4).await.unwrap().secret;
    let account = identity.authorize(&session).await.unwrap().owner_account_id;
    sqlx::query("UPDATE core_identity.user_profiles SET email='verified@example.invalid',email_login_allowed=TRUE,setting=$1 WHERE user_id=1")
        .bind(json!({"language":"ja","allow_key_bypass_ip_policy":false}).to_string()).execute(pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    install_policy(
        pool,
        &PolicyInstallation {
            expected_revision: None,
            policy: policy(),
        },
    )
    .await
    .unwrap();
    Fixture {
        ledger,
        identity,
        session,
        account,
    }
}
fn request(key: &str, action: Action) -> Request {
    Request {
        scope: "notification-tests".into(),
        operation_key: key.into(),
        actor_user_id: Some(1),
        reason: "isolated notification integration".into(),
        action,
    }
}
fn units(amount: i64) -> Credits {
    Credits::try_from(amount).unwrap()
}
fn posted(outcome: Outcome) -> i64 {
    match outcome {
        Outcome::Posted { journal_id, .. } => journal_id,
        other => panic!("expected posted: {other:?}"),
    }
}
async fn credit(f: &Fixture, key: &str, amount: i64) {
    posted(
        f.ledger
            .execute(&request(
                key,
                Action::Credit {
                    account_id: f.account,
                    amount_units: units(amount),
                    payment_reference: format!("synthetic:{key}"),
                },
            ))
            .await
            .unwrap(),
    );
}
async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM core_events.outbox WHERE event_type='notifications.balance.v1'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn holds_do_not_trigger_and_true_topup_rearms_one_crossing(pool: PgPool) {
    let f = fresh(&pool).await;
    credit(&f, "fund", 200).await;
    let reserve = posted(
        f.ledger
            .execute(&request(
                "reserve",
                Action::Reserve {
                    account_id: f.account,
                    amount_units: units(150),
                },
            ))
            .await
            .unwrap(),
    );
    assert_eq!(count(&pool).await, 0);
    let capture = request(
        "capture",
        Action::Capture {
            reservation_journal_id: reserve,
            amount_units: units(150),
        },
    );
    let original = f.ledger.execute(&capture).await.unwrap();
    assert_eq!(count(&pool).await, 1);
    let payload: Vec<u8> = sqlx::query_scalar(
        "SELECT payload FROM core_events.outbox WHERE event_type='notifications.balance.v1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let fact: Fact = serde_json::from_slice(&payload).unwrap();
    assert!(matches!(
        fact.evidence,
        Evidence::Balance {
            before_credits: 200,
            after_credits: 50,
            threshold_credits: 100,
            ..
        }
    ));
    assert!(!String::from_utf8(payload).unwrap().contains('@'));
    f.ledger
        .execute(&request(
            "still-low",
            Action::Charge {
                account_id: f.account,
                amount_units: units(10),
            },
        ))
        .await
        .unwrap();
    assert_eq!(count(&pool).await, 1);
    credit(&f, "topup", 100).await;
    assert_eq!(f.ledger.execute(&capture).await.unwrap(), original);
    assert_eq!(count(&pool).await, 1);
    f.ledger
        .execute(&request(
            "cross-again",
            Action::Charge {
                account_id: f.account,
                amount_units: units(41),
            },
        ))
        .await
        .unwrap();
    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn full_notification_capacity_rolls_back_money_then_same_key_can_succeed(pool: PgPool) {
    let f = fresh(&pool).await;
    credit(&f, "fund", 200).await;
    sqlx::query("UPDATE core_events.capacity SET max_events=1")
        .execute(&pool)
        .await
        .unwrap();
    f.ledger
        .execute(&request(
            "first",
            Action::Charge {
                account_id: f.account,
                amount_units: units(101),
            },
        ))
        .await
        .unwrap();
    credit(&f, "topup", 101).await;
    let second = request(
        "second",
        Action::Charge {
            account_id: f.account,
            amount_units: units(101),
        },
    );
    assert!(f.ledger.execute(&second).await.is_err());
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        200
    );
    let operations:i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_operations WHERE scope='notification-tests' AND operation_key='second'").fetch_one(&pool).await.unwrap();
    assert_eq!(operations, 0);
    let events = EventStore::from_pool(pool.clone());
    let delivery = events
        .pull(SERVICE, CONSUMER, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    events
        .acknowledge(SERVICE, CONSUMER, delivery.event.id, &delivery.lease_token)
        .await
        .unwrap();
    f.ledger.execute(&second).await.unwrap();
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        99
    );
    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn observation_replay_in_the_same_transaction_cannot_invent_a_later_crossing(pool: PgPool) {
    let f = fresh(&pool).await;
    credit(&f, "fund", 120).await;
    let first = request(
        "first",
        Action::Charge {
            account_id: f.account,
            amount_units: units(10),
        },
    );
    let second = request(
        "second",
        Action::Charge {
            account_id: f.account,
            amount_units: units(20),
        },
    );
    let mut tx = pool.begin().await.unwrap();
    let old = ledger::post_in_transaction(&mut tx, &first).await.unwrap();
    ledger::post_in_transaction(&mut tx, &second).await.unwrap();
    assert_eq!(
        ledger::post_in_transaction(&mut tx, &first).await.unwrap(),
        old
    );
    tx.commit().await.unwrap();
    assert_eq!(count(&pool).await, 1);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        90
    );
}

#[sqlx::test(migrations = false)]
async fn repeatable_read_is_rejected_and_uncommitted_money_and_event_disappear(pool: PgPool) {
    let f = fresh(&pool).await;
    credit(&f, "fund", 120).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        ledger::post_in_transaction(
            &mut tx,
            &request(
                "reject-isolation",
                Action::Charge {
                    account_id: f.account,
                    amount_units: units(30)
                }
            )
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(count(&pool).await, 0);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        120
    );
}

#[sqlx::test(migrations = false)]
async fn observer_contention_rolls_back_money_and_same_operation_can_be_retried(pool: PgPool) {
    use sha2::{Digest, Sha256};
    let f = fresh(&pool).await;
    credit(&f, "fund", 120).await;
    let mut digest = Sha256::new();
    digest.update(b"lmm.notifications.wallet.v1\0");
    digest.update(f.account.to_be_bytes());
    let key = i64::from_be_bytes(digest.finalize()[..8].try_into().unwrap());
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let cross = request(
        "contended",
        Action::Charge {
            account_id: f.account,
            amount_units: units(30),
        },
    );
    assert!(f.ledger.execute(&cross).await.is_err());
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        120
    );
    assert_eq!(count(&pool).await, 0);
    blocker.rollback().await.unwrap();
    f.ledger.execute(&cross).await.unwrap();
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        90
    );
    assert_eq!(count(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn preferences_preserve_unrelated_keys_and_require_actual_admin_role_and_verified_destination(
    pool: PgPool,
) {
    let f = fresh(&pool).await;
    let update: SettingsUpdate =
        serde_json::from_value(json!({"quota_warning_enabled":false,"quota_warning_threshold":0}))
            .unwrap();
    let view = f
        .identity
        .update_notification_settings(&f.session, &update)
        .await
        .unwrap();
    assert_eq!(view["quota_warning_enabled"], false);
    assert_eq!(view["admin_notifications_allowed"], false);
    let raw: String =
        sqlx::query_scalar("SELECT setting FROM core_identity.user_profiles WHERE user_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let saved: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(saved["language"], "ja");
    assert_eq!(saved["allow_key_bypass_ip_policy"], false);
    let admin: SettingsUpdate =
        serde_json::from_value(json!({"channel_notify_enabled":true})).unwrap();
    assert!(matches!(
        f.identity
            .update_notification_settings(&f.session, &admin)
            .await,
        Err(IdentityError::Forbidden)
    ));
    let address: SettingsUpdate =
        serde_json::from_value(json!({"notification_email":"unverified@example.invalid"})).unwrap();
    assert!(matches!(
        f.identity
            .update_notification_settings(&f.session, &address)
            .await,
        Err(IdentityError::Invalid)
    ));
    let root = f.identity.bootstrap_user(2, 6).await.unwrap();
    let ordinary_admin = f.identity.bootstrap_user(3, 5).await.unwrap();
    assert_eq!(
        f.identity
            .notification_settings(&root.secret)
            .await
            .unwrap()["channel_notify_enabled"],
        true
    );
    assert_eq!(
        f.identity
            .notification_settings(&ordinary_admin.secret)
            .await
            .unwrap()["channel_notify_enabled"],
        false
    );
}

fn mail(port: u16) -> Mailer {
    Mailer::open(&MailConfig::Smtp(crate::humangrant::mail::SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        from: "sender@example.invalid".into(),
        tls: crate::humangrant::mail::SmtpTls::Plaintext,
        username: None,
        password_file: None,
        allow_insecure_loopback: true,
    }))
    .unwrap()
}
async fn smtp_once(listener: TcpListener) -> String {
    let (stream, _) = listener.accept().await.unwrap();
    let mut stream = BufReader::new(stream);
    stream
        .get_mut()
        .write_all(b"220 local.invalid ESMTP\r\n")
        .await
        .unwrap();
    let mut body = String::new();
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).await.unwrap() == 0 {
            break;
        }
        if line.starts_with("DATA") {
            stream.get_mut().write_all(b"354 data\r\n").await.unwrap();
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                if line == ".\r\n" {
                    break;
                }
                body.push_str(&line);
            }
            stream
                .get_mut()
                .write_all(b"250 accepted\r\n")
                .await
                .unwrap();
            // Deliberately close the accepted transport: the inbox receipt must
            // prevent a second socket even if the transport outcome is unknown.
            break;
        } else {
            stream
                .get_mut()
                .write_all(b"250 local.invalid\r\n")
                .await
                .unwrap();
        }
    }
    drop(stream);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err()
    );
    body
}

#[sqlx::test(migrations = false)]
async fn committed_attempt_prevents_resend_after_smtp_and_acknowledgement_failure(pool: PgPool) {
    let f = fresh(&pool).await;
    credit(&f, "fund", 120).await;
    f.ledger
        .execute(&request(
            "cross",
            Action::Charge {
                account_id: f.account,
                amount_units: units(30),
            },
        ))
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let capture = tokio::spawn(smtp_once(listener));
    let runtime = NotificationRuntime {
        pool: pool.clone(),
        events: EventStore::from_pool(pool.clone()),
        mail: mail(port),
    };
    let first = runtime
        .events
        .pull(SERVICE, CONSUMER, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM core_events.capacity WHERE singleton FOR UPDATE")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    assert!(runtime.deliver(&first).await.is_err());
    blocker.rollback().await.unwrap();
    sqlx::query(
        "UPDATE core_events.deliveries SET available_at=clock_timestamp()-interval '1 second'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let again = runtime
        .events
        .pull(SERVICE, CONSUMER, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    runtime.deliver(&again).await.unwrap();
    let body = tokio::time::timeout(Duration::from_secs(2), capture)
        .await
        .unwrap()
        .unwrap();
    assert!(body.contains("verified@example.invalid"));
    let response: Vec<u8> = sqlx::query_scalar(
        "SELECT response FROM core_events.inbox WHERE method='notifications.delivery.result'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response).unwrap()["status"],
        "unknown"
    );
    assert!(
        runtime
            .events
            .pull(SERVICE, CONSUMER, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = false)]
async fn unsubscribe_or_unverified_address_suppresses_already_queued_delivery(pool: PgPool) {
    let f = fresh(&pool).await;
    credit(&f, "fund", 120).await;
    f.ledger
        .execute(&request(
            "cross",
            Action::Charge {
                account_id: f.account,
                amount_units: units(30),
            },
        ))
        .await
        .unwrap();
    let update: SettingsUpdate =
        serde_json::from_value(json!({"quota_warning_enabled":false})).unwrap();
    f.identity
        .update_notification_settings(&f.session, &update)
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let runtime = NotificationRuntime {
        pool: pool.clone(),
        events: EventStore::from_pool(pool.clone()),
        mail: mail(listener.local_addr().unwrap().port()),
    };
    let delivery = runtime
        .events
        .pull(SERVICE, CONSUMER, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    runtime.deliver(&delivery).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let response: Vec<u8> = sqlx::query_scalar(
        "SELECT response FROM core_events.inbox WHERE method='notifications.delivery.result'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response).unwrap()["status"],
        "suppressed"
    );
    let enabled: SettingsUpdate =
        serde_json::from_value(json!({"quota_warning_enabled":true})).unwrap();
    f.identity
        .update_notification_settings(&f.session, &enabled)
        .await
        .unwrap();
    credit(&f, "topup", 30).await;
    f.ledger
        .execute(&request(
            "cross-unverified",
            Action::Charge {
                account_id: f.account,
                amount_units: units(30),
            },
        ))
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET email_login_allowed=FALSE WHERE user_id=1")
        .execute(&pool)
        .await
        .unwrap();
    let delivery = runtime
        .events
        .pull(SERVICE, CONSUMER, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    runtime.deliver(&delivery).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let suppressed:i64 = sqlx::query_scalar("SELECT count(*) FROM core_events.inbox WHERE method='notifications.delivery.result' AND convert_from(response,'UTF8')::jsonb->>'status'='suppressed'").fetch_one(&pool).await.unwrap();
    assert_eq!(suppressed, 2);
}

#[sqlx::test(migrations = false)]
async fn native_runtime_can_publish_without_local_worker_or_console_flags(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(1, 4).await.unwrap().secret;
    let account = identity.authorize(&session).await.unwrap().owner_account_id;
    Ledger::from_pool(pool.clone())
        .open_wallet(account)
        .await
        .unwrap();
    let role = crate::test_runtime::RuntimeRole::create(&pool).await;
    crate::runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    let ledger = Ledger::from_pool(role.pool.clone());
    // The policy row does not exist yet: the ordinary restricted writer must
    // still be able to read that fact and commit its authorized operation.
    ledger
        .execute(&request(
            "fund",
            Action::Credit {
                account_id: account,
                amount_units: units(120),
                payment_reference: "synthetic:fund".into(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(count(&pool).await, 0);
    let narrow: bool = sqlx::query_scalar("SELECT NOT has_table_privilege('core_console.options','UPDATE') AND NOT has_table_privilege('core_events.subscriptions','INSERT') AND NOT has_column_privilege('core_events.subscriptions','max_pending','UPDATE') AND NOT has_table_privilege('core_events.deliveries','UPDATE')")
        .fetch_one(&role.pool).await.unwrap();
    assert!(narrow);
    install_policy(
        &pool,
        &PolicyInstallation {
            expected_revision: None,
            policy: policy(),
        },
    )
    .await
    .unwrap();
    // This same role has no optional worker/consumer grants; the durable
    // owner's policy still requires its transaction to append the crossing.
    ledger
        .execute(&request(
            "cross",
            Action::Charge {
                account_id: account,
                amount_units: units(30),
            },
        ))
        .await
        .unwrap();
    assert_eq!(count(&pool).await, 1);
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 90);
    assert!(matches!(
        install_policy(
            &role.pool,
            &PolicyInstallation {
                expected_revision: Some(1),
                policy: policy(),
            }
        )
        .await,
        Err(_)
    ));
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn missing_publisher_permissions_reject_startup_and_roll_back_the_same_money_write(
    pool: PgPool,
) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(1, 4).await.unwrap().secret;
    let account = identity.authorize(&session).await.unwrap().owner_account_id;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    ledger
        .execute(&request(
            "fund",
            Action::Credit {
                account_id: account,
                amount_units: units(120),
                payment_reference: "synthetic:fund".into(),
            },
        ))
        .await
        .unwrap();
    install_policy(
        &pool,
        &PolicyInstallation {
            expected_revision: None,
            policy: policy(),
        },
    )
    .await
    .unwrap();
    let role = crate::test_runtime::RuntimeRole::create(&pool).await;
    let writer = Ledger::from_pool(role.pool.clone());
    let cross = request(
        "cross",
        Action::Charge {
            account_id: account,
            amount_units: units(30),
        },
    );
    for permission in [
        "SELECT ON core_console.options",
        "SELECT ON core_events.subscriptions",
        "INSERT ON core_events.outbox",
        "INSERT ON core_events.deliveries",
        "UPDATE(pending) ON core_events.subscriptions",
        "UPDATE(response) ON core_events.inbox",
        "USAGE ON SEQUENCE core_events.outbox_id_seq",
    ] {
        sqlx::query(&format!("REVOKE {permission} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            crate::runtime_database::check(&role.pool, Default::default()).await,
            Err(crate::runtime_database::RuntimeDatabaseError::MissingPrivileges),
            "{permission}"
        );
        // Direct library invocation also fails closed; privilege failure must
        // never be interpreted as an absent/disabled notification policy.
        assert!(writer.execute(&cross).await.is_err(), "{permission}");
        assert_eq!(ledger.balance(account).await.unwrap().available_units, 120);
        assert_eq!(count(&pool).await, 0);
        sqlx::query(&format!("GRANT {permission} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        crate::runtime_database::check(&role.pool, Default::default())
            .await
            .unwrap();
    }
    writer.execute(&cross).await.unwrap();
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 90);
    assert_eq!(count(&pool).await, 1);
    role.cleanup().await;
}
