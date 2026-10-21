//! Independent signing plus real identity/browser/MFA state in disposable PG.
//! Fixture DDL is explicit test installation, not an upgrade acceptance claim.
use super::*;
use crate::identity::{
    BrowserAuth, BrowserAuthError, BrowserBundle, BrowserLoginOutcome, BrowserRegistration,
    BrowserSessionMetadata, MfaCompletion, MfaError,
};
use std::{collections::BTreeMap, fmt::Write};
use totp_rs::{Algorithm, Builder, Secret};

const TOKEN: &str = "123456789:synthetic-only-widget-signing-key";
const PASSWORD: &str = "telegram-test-47";
const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_BINDING: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const MFA_BINDING: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SUBJECT: &str = "9223372036854775807";

struct Fixture {
    pool: PgPool,
    store: IdentityStore,
    browser: BrowserAuth,
    owner: BrowserBundle,
}

fn metadata() -> BrowserSessionMetadata {
    BrowserSessionMetadata {
        ip: "192.0.2.31".into(),
        user_agent: "synthetic Telegram isolated PostgreSQL test".into(),
    }
}

fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}

fn sid(bundle: &BrowserBundle) -> &str {
    bundle.data["session"]["sid"].as_str().unwrap()
}

fn user(bundle: &BrowserBundle) -> i64 {
    bundle.data["user"]["id"].as_i64().unwrap()
}

async fn register(browser: &BrowserAuth, name: &str) -> BrowserBundle {
    browser
        .register(&BrowserRegistration {
            username: name,
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    browser.login(name, PASSWORD, &metadata()).await.unwrap()
}

async fn fixture(pool: PgPool) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    let installed: bool =
        sqlx::query_scalar("SELECT to_regclass('core_telegram_auth.bindings') IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        installed,
        "compiled fresh installer must include Telegram component"
    );
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys(
            "telegram-test".into(),
            vec![("telegram-test".into(), vec![47; 32])],
        )
        .unwrap()
        .with_telegram_login(TelegramLogin::new("lmm_test_bot", TOKEN).unwrap())
        .unwrap();
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    let owner = register(&browser, "telegramowner").await;
    Fixture {
        pool,
        store,
        browser,
        owner,
    }
}

async fn epoch(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(pool)
        .await
        .unwrap()
}

// ring signs the Widget fixture independently of production's RustCrypto HMAC.
fn signed_at(subject: &str, label: &str, at: i64) -> TelegramAuthorization {
    let mut fields = BTreeMap::from([
        ("id".to_owned(), subject.to_owned()),
        ("auth_date".to_owned(), at.to_string()),
        ("first_name".to_owned(), format!("Synthetic {label}")),
        // Matching a local username must never become an identity claim.
        ("username".to_owned(), "telegramowner".to_owned()),
    ]);
    let message = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let key = ring::digest::digest(&ring::digest::SHA256, TOKEN.as_bytes());
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_ref());
    let tag = ring::hmac::sign(&key, message.as_bytes());
    let mut hash = String::with_capacity(64);
    for byte in tag.as_ref() {
        write!(&mut hash, "{byte:02x}").unwrap();
    }
    fields.insert("hash".into(), hash);
    serde_json::from_value(serde_json::to_value(fields).unwrap()).unwrap()
}

async fn signed(f: &Fixture, subject: &str, label: &str) -> TelegramAuthorization {
    signed_at(subject, label, epoch(&f.pool).await)
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    // table is selected only by literal test callers, never a protocol value.
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn bind(f: &Fixture, owner: &BrowserBundle, subject: &str, label: &str) {
    let flow = f
        .browser
        .begin_telegram_bind(access(owner), BINDING, Some(sid(owner)))
        .await
        .unwrap();
    f.browser
        .finish_telegram_bind(&flow.flow_token, BINDING, &signed(f, subject, label).await)
        .await
        .unwrap();
}

async fn login(f: &Fixture, proof: &TelegramAuthorization, binding: &str) -> BrowserBundle {
    let flow = f.browser.begin_telegram_login(binding).await.unwrap();
    match f
        .browser
        .finish_telegram_login(&flow.flow_token, binding, proof, &metadata(), MFA_BINDING)
        .await
        .unwrap()
    {
        BrowserLoginOutcome::Session(bundle) => bundle,
        BrowserLoginOutcome::Challenge(_) => panic!("factor not required in this fixture"),
    }
}

#[sqlx::test(migrations = false)]
async fn telegram_explicit_binding_preserves_64_bit_subject_and_logs_into_exact_owner(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    bind(&f, &f.owner, SUBJECT, "bind-64-bit").await;
    let stored: (String, i64) = sqlx::query_as(
        "SELECT subject,user_id FROM core_telegram_auth.bindings WHERE revoked_at IS NULL",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(stored.0, SUBJECT);
    assert_eq!(stored.1, user(&f.owner));
    let bundle = login(
        &f,
        &signed(&f, SUBJECT, "login-64-bit").await,
        OTHER_BINDING,
    )
    .await;
    let actor = f.store.authorize(access(&bundle)).await.unwrap();
    assert_eq!(actor.credential_kind, CredentialKind::Session);
    assert_eq!(actor.user_id, user(&f.owner));
    assert!(sid(&bundle) != sid(&f.owner));
    let audit: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.audit WHERE actor_user_id=$1 AND action='session.telegram_browser_login')")
        .bind(actor.user_id).fetch_one(&f.pool).await.unwrap();
    assert!(audit);
    assert_eq!(count(&f.pool, "core_identity.users").await, 1);
}

#[sqlx::test(migrations = false)]
async fn telegram_unbound_assertion_cannot_register_match_username_or_replay_into_binding(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let proof = signed(&f, SUBJECT, "unbound").await;
    let families = count(&f.pool, "core_identity.web_sessions").await;
    let credentials = count(&f.pool, "core_identity.credentials").await;
    let flow = f.browser.begin_telegram_login(BINDING).await.unwrap();
    let result = f
        .browser
        .finish_telegram_login(&flow.flow_token, BINDING, &proof, &metadata(), MFA_BINDING)
        .await;
    assert!(matches!(
        result,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(count(&f.pool, "core_identity.users").await, 1);
    assert_eq!(count(&f.pool, "core_identity.web_sessions").await, families);
    assert_eq!(
        count(&f.pool, "core_identity.credentials").await,
        credentials
    );
    assert_eq!(count(&f.pool, "core_telegram_auth.bindings").await, 0);
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 1);
    let bind_flow = f
        .browser
        .begin_telegram_bind(access(&f.owner), OTHER_BINDING, None)
        .await
        .unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_bind(&bind_flow.flow_token, OTHER_BINDING, &proof)
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(count(&f.pool, "core_telegram_auth.bindings").await, 0);
}

#[sqlx::test(migrations = false)]
async fn telegram_flow_is_cookie_and_purpose_bound_before_claim_and_replay_is_global(pool: PgPool) {
    let f = fixture(pool).await;
    let proof = signed(&f, SUBJECT, "cookie-purpose-bind").await;
    let flow = f
        .browser
        .begin_telegram_bind(access(&f.owner), BINDING, None)
        .await
        .unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_bind(&flow.flow_token, OTHER_BINDING, &proof)
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Unauthorized))
    ));
    assert!(matches!(
        f.browser
            .finish_telegram_login(&flow.flow_token, BINDING, &proof, &metadata(), MFA_BINDING)
            .await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 0);
    f.browser
        .finish_telegram_bind(&flow.flow_token, BINDING, &proof)
        .await
        .unwrap();
    assert!(
        f.browser
            .finish_telegram_bind(&flow.flow_token, BINDING, &proof)
            .await
            .is_err()
    );
    let new_login = f.browser.begin_telegram_login(OTHER_BINDING).await.unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_login(
                &new_login.flow_token,
                OTHER_BINDING,
                &proof,
                &metadata(),
                MFA_BINDING
            )
            .await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    let login_proof = signed(&f, SUBJECT, "cookie-purpose-login").await;
    let bundle = login(&f, &login_proof, OTHER_BINDING).await;
    assert_eq!(user(&bundle), user(&f.owner));
    let repeat = f.browser.begin_telegram_login(BINDING).await.unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_login(
                &repeat.flow_token,
                BINDING,
                &login_proof,
                &metadata(),
                MFA_BINDING
            )
            .await,
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 2);
}

#[sqlx::test(migrations = false)]
async fn telegram_flow_rejects_preissued_assertion_expiry_and_mismatched_family(pool: PgPool) {
    let f = fixture(pool).await;
    let other = f
        .browser
        .login("telegramowner", PASSWORD, &metadata())
        .await
        .unwrap();
    assert!(matches!(
        f.browser
            .begin_telegram_bind(access(&f.owner), BINDING, Some(sid(&other)))
            .await,
        Err(BrowserAuthError::SessionMismatch)
    ));
    for invalid in ["", "short", &"g".repeat(64), &"1".repeat(65)] {
        assert!(f.browser.begin_telegram_login(invalid).await.is_err());
    }
    let flow = f
        .browser
        .begin_telegram_bind(access(&f.owner), BINDING, None)
        .await
        .unwrap();
    let early = signed_at(SUBJECT, "preissued", epoch(&f.pool).await - 60);
    assert!(matches!(
        f.browser
            .finish_telegram_bind(&flow.flow_token, BINDING, &early)
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 0);
    sqlx::query("UPDATE core_telegram_auth.flows SET created_at=statement_timestamp()-interval '301 seconds',expires_at=statement_timestamp()-interval '1 second' WHERE token_digest=$1")
        .bind(digest(&flow.flow_token).unwrap()).execute(&f.pool).await.unwrap();
    let proof = signed(&f, SUBJECT, "expired-flow").await;
    assert!(matches!(
        f.browser
            .finish_telegram_bind(&flow.flow_token, BINDING, &proof)
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(count(&f.pool, "core_telegram_auth.bindings").await, 0);
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn telegram_bind_survives_refresh_only_within_original_live_family(pool: PgPool) {
    let f = fixture(pool).await;
    let flow = f
        .browser
        .begin_telegram_bind(access(&f.owner), BINDING, Some(sid(&f.owner)))
        .await
        .unwrap();
    let refreshed = f
        .browser
        .refresh(&f.owner.refresh_token, Some(sid(&f.owner)))
        .await
        .unwrap();
    assert!(sid(&refreshed) == sid(&f.owner));
    assert!(f.store.authorize(access(&f.owner)).await.is_err());
    let proof = signed(&f, SUBJECT, "refreshed-family").await;
    // There is deliberately no access or refresh argument at callback time.
    f.browser
        .finish_telegram_bind(&flow.flow_token, BINDING, &proof)
        .await
        .unwrap();
    let bound: i64 = sqlx::query_scalar(
        "SELECT user_id FROM core_telegram_auth.bindings WHERE subject=$1 AND revoked_at IS NULL",
    )
    .bind(SUBJECT)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(bound, user(&refreshed));
}

#[sqlx::test(migrations = false)]
async fn telegram_bind_rejects_logout_generation_and_disabled_authority(pool: PgPool) {
    let f = fixture(pool).await;
    for (index, mode) in ["logout", "generation", "user_disabled", "account_disabled"]
        .into_iter()
        .enumerate()
    {
        let owner = if index == 0 {
            f.browser
                .login("telegramowner", PASSWORD, &metadata())
                .await
                .unwrap()
        } else {
            register(&f.browser, &format!("telegramstale{index}")).await
        };
        let flow = f
            .browser
            .begin_telegram_bind(access(&owner), BINDING, None)
            .await
            .unwrap();
        match mode {
            "logout" => f
                .browser
                .logout(Some(&owner.refresh_token), None, Some(sid(&owner)))
                .await
                .unwrap(),
            "generation" => {
                sqlx::query(
                    "UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1",
                )
                .bind(user(&owner))
                .execute(&f.pool)
                .await
                .unwrap();
            }
            "user_disabled" => {
                sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=$1")
                    .bind(user(&owner))
                    .execute(&f.pool)
                    .await
                    .unwrap();
            }
            "account_disabled" => {
                sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)")
                .bind(user(&owner)).execute(&f.pool).await.unwrap();
            }
            _ => unreachable!(),
        }
        let proof = signed(&f, &(4_294_967_296_u64 + index as u64).to_string(), mode).await;
        assert!(
            matches!(
                f.browser
                    .finish_telegram_bind(&flow.flow_token, BINDING, &proof)
                    .await,
                Err(BrowserAuthError::Identity(IdentityError::Unauthorized))
            ),
            "stale authority: {mode}"
        );
        let consumed: bool = sqlx::query_scalar(
            "SELECT consumed_at IS NOT NULL FROM core_telegram_auth.flows WHERE token_digest=$1",
        )
        .bind(digest(&flow.flow_token).unwrap())
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert!(consumed);
    }
    assert_eq!(count(&f.pool, "core_telegram_auth.bindings").await, 0);
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 4);
}

#[sqlx::test(migrations = false)]
async fn telegram_binding_cannot_steal_subject_replace_active_user_or_claim_revoked_owner(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    bind(&f, &f.owner, SUBJECT, "original-owner").await;
    let other = register(&f.browser, "telegramother").await;
    let flow = f
        .browser
        .begin_telegram_bind(access(&other), OTHER_BINDING, None)
        .await
        .unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_bind(
                &flow.flow_token,
                OTHER_BINDING,
                &signed(&f, SUBJECT, "steal-active").await
            )
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Conflict))
    ));
    let replace = f
        .browser
        .begin_telegram_bind(access(&f.owner), BINDING, None)
        .await
        .unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_bind(
                &replace.flow_token,
                BINDING,
                &signed(&f, "4294967296", "replace-active").await
            )
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Conflict))
    ));
    // Simulate an installer-owned historical revocation, not an unbind API.
    sqlx::query(
        "UPDATE core_telegram_auth.bindings SET revoked_at=clock_timestamp() WHERE subject=$1",
    )
    .bind(SUBJECT)
    .execute(&f.pool)
    .await
    .unwrap();
    let steal = f
        .browser
        .begin_telegram_bind(access(&other), OTHER_BINDING, None)
        .await
        .unwrap();
    assert!(matches!(
        f.browser
            .finish_telegram_bind(
                &steal.flow_token,
                OTHER_BINDING,
                &signed(&f, SUBJECT, "steal-revoked").await
            )
            .await,
        Err(BrowserAuthError::Identity(IdentityError::Conflict))
    ));
    bind(&f, &f.owner, SUBJECT, "restore-original-owner").await;
    assert_eq!(count(&f.pool, "core_telegram_auth.bindings").await, 1);
    let owner: i64 = sqlx::query_scalar(
        "SELECT user_id FROM core_telegram_auth.bindings WHERE subject=$1 AND revoked_at IS NULL",
    )
    .bind(SUBJECT)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(owner, user(&f.owner));
}

#[sqlx::test(migrations = false)]
async fn telegram_same_subject_concurrent_bind_has_exactly_one_owner(pool: PgPool) {
    let f = fixture(pool).await;
    let other = register(&f.browser, "telegramother").await;
    let first = f
        .browser
        .begin_telegram_bind(access(&f.owner), BINDING, None)
        .await
        .unwrap();
    let second = f
        .browser
        .begin_telegram_bind(access(&other), OTHER_BINDING, None)
        .await
        .unwrap();
    let first_proof = signed(&f, SUBJECT, "concurrent-first").await;
    let second_proof = signed(&f, SUBJECT, "concurrent-second").await;
    let barrier = tokio::sync::Barrier::new(2);
    let (a, b) = tokio::join!(
        async {
            barrier.wait().await;
            f.browser
                .finish_telegram_bind(&first.flow_token, BINDING, &first_proof)
                .await
        },
        async {
            barrier.wait().await;
            f.browser
                .finish_telegram_bind(&second.flow_token, OTHER_BINDING, &second_proof)
                .await
        },
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        if a.is_err() { a } else { b },
        Err(BrowserAuthError::Identity(IdentityError::Conflict))
    ));
    let bound: i64 = sqlx::query_scalar(
        "SELECT user_id FROM core_telegram_auth.bindings WHERE subject=$1 AND revoked_at IS NULL",
    )
    .bind(SUBJECT)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(bound == user(&f.owner) || bound == user(&other));
    assert_eq!(count(&f.pool, "core_telegram_auth.bindings").await, 1);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='identity.telegram_bind'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
}

#[sqlx::test(migrations = false)]
async fn telegram_same_assertion_concurrent_login_issues_at_most_one_family(pool: PgPool) {
    let f = fixture(pool).await;
    bind(&f, &f.owner, SUBJECT, "concurrent-login-bind").await;
    let families = count(&f.pool, "core_identity.web_sessions").await;
    let first = f.browser.begin_telegram_login(BINDING).await.unwrap();
    let second = f.browser.begin_telegram_login(OTHER_BINDING).await.unwrap();
    let proof = signed(&f, SUBJECT, "concurrent-login").await;
    let metadata = metadata();
    let barrier = tokio::sync::Barrier::new(2);
    let (a, b) = tokio::join!(
        async {
            barrier.wait().await;
            f.browser
                .finish_telegram_login(&first.flow_token, BINDING, &proof, &metadata, MFA_BINDING)
                .await
        },
        async {
            barrier.wait().await;
            f.browser
                .finish_telegram_login(
                    &second.flow_token,
                    OTHER_BINDING,
                    &proof,
                    &metadata,
                    MFA_BINDING,
                )
                .await
        },
    );
    assert_eq!(
        usize::from(matches!(&a, Ok(BrowserLoginOutcome::Session(_))))
            + usize::from(matches!(&b, Ok(BrowserLoginOutcome::Session(_)))),
        1
    );
    assert!(matches!(
        if a.is_err() { a } else { b },
        Err(MfaError::Identity(IdentityError::Unauthorized))
    ));
    assert_eq!(
        count(&f.pool, "core_identity.web_sessions").await,
        families + 1
    );
    assert_eq!(count(&f.pool, "core_telegram_auth.assertions").await, 2);
}

#[sqlx::test(migrations = false)]
async fn telegram_required_totp_returns_only_challenge_until_bound_local_factor_completes(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    bind(&f, &f.owner, SUBJECT, "mfa-bind").await;
    let setup = f.browser.mfa_setup(access(&f.owner)).await.unwrap();
    let secret = Secret::try_from_base32(&setup.secret).unwrap();
    let code = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(u64::try_from(epoch(&f.pool).await).unwrap())
        .to_string();
    let enabled = f.browser.mfa_enable(access(&f.owner), &code).await.unwrap();
    assert!(sid(&enabled) == sid(&f.owner));
    let families = count(&f.pool, "core_identity.web_sessions").await;
    let credentials = count(&f.pool, "core_identity.credentials").await;
    let flow = f.browser.begin_telegram_login(OTHER_BINDING).await.unwrap();
    let proof = signed(&f, SUBJECT, "mfa-login").await;
    let challenge = match f
        .browser
        .finish_telegram_login(
            &flow.flow_token,
            OTHER_BINDING,
            &proof,
            &metadata(),
            MFA_BINDING,
        )
        .await
        .unwrap()
    {
        BrowserLoginOutcome::Challenge(challenge) => challenge,
        BrowserLoginOutcome::Session(_) => {
            panic!("Telegram primary proof bypassed required local MFA")
        }
    };
    assert!(challenge.require_2fa);
    assert_eq!(count(&f.pool, "core_identity.web_sessions").await, families);
    assert_eq!(
        count(&f.pool, "core_identity.credentials").await,
        credentials
    );
    let public = serde_json::to_value(&challenge).unwrap();
    assert!(public.get("access_token").is_none());
    assert!(public.get("refresh_token").is_none());
    let completion = MfaCompletion {
        flow_token: challenge.flow_token.clone(),
        code: setup.backup_codes[0].clone(),
    };
    assert!(matches!(
        f.browser.complete_mfa_login(&completion, BINDING).await,
        Err(MfaError::FlowExpired)
    ));
    assert_eq!(count(&f.pool, "core_identity.web_sessions").await, families);
    let bundle = f
        .browser
        .complete_mfa_login(&completion, MFA_BINDING)
        .await
        .unwrap();
    assert_eq!(user(&bundle), user(&f.owner));
    let actor = f.store.authorize(access(&bundle)).await.unwrap();
    let assurance: (i16,String) = sqlx::query_as("SELECT factor_level,method FROM core_identity.session_authentication WHERE credential_id=$1")
        .bind(actor.credential_id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(assurance, (2, "recovery".into()));
    assert_eq!(
        count(&f.pool, "core_identity.web_sessions").await,
        families + 1
    );
    assert!(
        f.browser
            .complete_mfa_login(&completion, MFA_BINDING)
            .await
            .is_err()
    );
}
