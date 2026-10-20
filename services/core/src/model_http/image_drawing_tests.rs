//! Real browser confirmation and original Core marketplace authority, with the
//! same real image funds path and counting loopback provider as native-Key tests.
use super::tests::{Reply, Supplier, image_service_for_pool, input};
use super::*;
use crate::{
    billing::{
        Credits,
        ledger::{Action, Ledger, Outcome as LedgerOutcome, Request as LedgerRequest},
    },
    identity::{BrowserBundle, BrowserRegistration, BrowserSessionMetadata, IdentityStore},
    tool_market::{
        Owner,
        credit_access::IdRequest,
        credit_test_fixture::{BuiltinFixture, builtin_with_session},
    },
};
use std::sync::atomic::Ordering;

const PASSWORD: &str = "drawing-password";
struct BrowserFixture {
    pool: PgPool,
    identity: IdentityStore,
    browser: BrowserAuth,
    bundle: BrowserBundle,
    secret: String,
    user: i64,
    account: i64,
    ledger: Ledger,
}
async fn login(browser: &BrowserAuth, username: &str) -> BrowserBundle {
    browser
        .login(
            username,
            PASSWORD,
            &BrowserSessionMetadata {
                ip: "192.0.2.7".into(),
                user_agent: "synthetic drawing PostgreSQL test".into(),
            },
        )
        .await
        .unwrap()
}
async fn fixture(pool: PgPool) -> BrowserFixture {
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::check(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys(
            "drawing-test".into(),
            vec![("drawing-test".into(), vec![47; 32])],
        )
        .unwrap();
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "drawingowner",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let bundle = login(&browser, "drawingowner").await;
    let secret = bundle.data["access_token"].as_str().unwrap().to_owned();
    let actor = identity.authorize(&secret).await.unwrap();
    assert_eq!(actor.credential_kind, CredentialKind::Session);
    sqlx::query("INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES($1,TRUE,'synthetic-drawing-pg-test')")
        .bind(actor.user_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,'allowed',TRUE,'synthetic-drawing-pg-test')")
        .bind(actor.user_id).execute(&pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    let credited = ledger
        .execute(&LedgerRequest {
            scope: "drawing-image-pg-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(actor.user_id),
            reason: "Synthetic funds in disposable drawing PostgreSQL".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(10000).unwrap(),
                payment_reference: "synthetic:drawing-pg-test".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(credited, LedgerOutcome::Posted { .. }));
    BrowserFixture {
        pool,
        identity,
        browser,
        bundle,
        secret,
        user: actor.user_id,
        account: actor.owner_account_id,
        ledger,
    }
}
fn drawing_input() -> Value {
    let mut drawing = input(3);
    drawing.group = "allowed".into();
    drawing.prompt = "<scene>&light>\u{2028}line\u{2029}paragraph".into();
    json!(drawing)
}
async fn builtin(f: &BrowserFixture, raw: &Value) -> BuiltinFixture {
    builtin_with_session(
        &f.pool,
        &f.secret,
        "drawing-tool",
        "drawing-test-v1",
        "drawing.generate",
        &"a".repeat(64),
        &crate::builtins::input_digest(raw).unwrap(),
    )
    .await
}
async fn no_model_hold(f: &BrowserFixture) {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.available_units, 10000);
    assert_eq!(balance.reserved_units, 0);
}
async fn revoke(f: &BrowserFixture, builtin: &BuiltinFixture, operation_id: &str) {
    builtin
        .store
        .revoke(
            &f.secret,
            &IdRequest {
                owner: Owner {
                    user: f.user.to_string(),
                    account: f.user.to_string(),
                },
                operation_id: operation_id.into(),
                id: builtin.context.grant_id.clone(),
            },
            false,
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn image_drawing_real_cookie_confirmation_dispatch_and_context_receipt(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let raw = drawing_input();
    let builtin = builtin(&f, &raw).await;
    assert_eq!(builtin.secret, f.secret);
    let preview = service
        .preview(&f.secret, &builtin.context, raw.clone())
        .await
        .unwrap();
    assert_eq!(preview.state, "previewed");
    assert_eq!(preview.reservation_credits, 6000);
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    // A raw operation id, valid browser cookie or arbitrary 64-byte CSRF alone
    // cannot turn a preview into spending permission.
    assert_eq!(
        service
            .confirm(
                &f.bundle.refresh_token,
                &preview.operation_id,
                &"0".repeat(64)
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .confirm(
                "invalid-browser-cookie",
                &preview.operation_id,
                &"0".repeat(64)
            )
            .await
            .unwrap_err(),
        Error::Unauthorized
    );
    let alternate = login(&f.browser, "drawingowner").await;
    let alternate_secret = alternate.data["access_token"].as_str().unwrap();
    assert_eq!(
        f.identity
            .authorize(alternate_secret)
            .await
            .unwrap()
            .user_id,
        f.user
    );
    assert_eq!(
        service
            .execute_for_context(alternate_secret, &preview.operation_id, &builtin.context)
            .await
            .unwrap_err(),
        Error::Missing
    );
    let (page, csrf) = service
        .confirmation_page(&f.bundle.refresh_token, &preview.operation_id)
        .await
        .unwrap();
    assert_eq!(page.preview_digest, preview.preview_digest);
    assert_eq!(page.input.prompt, raw["prompt"].as_str().unwrap());
    assert_eq!(
        service
            .confirm(&alternate.refresh_token, &preview.operation_id, &csrf)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .confirm(
                &f.bundle.refresh_token,
                &preview.operation_id,
                &"0".repeat(64)
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_model_hold(&f).await;
    let confirmed = service
        .confirm(&f.bundle.refresh_token, &preview.operation_id, &csrf)
        .await
        .unwrap();
    assert_eq!(confirmed.state, "confirmed");
    assert!(confirmed.output.is_none());
    no_model_hold(&f).await;
    let mut wrong_context = builtin.context.clone();
    wrong_context.dispatch_id = "b".repeat(64);
    assert_eq!(
        service
            .query_for_context(&f.secret, &preview.operation_id, &wrong_context)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .execute_for_context(&f.secret, &preview.operation_id, &wrong_context)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let receipt = service
        .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(charge.key_id.is_none());
    assert!(charge.oauth_source.is_none());
    assert_eq!(charge.owner_account_id, f.account);
    assert_eq!(charge.payer_account_id, f.account);
    assert_eq!(charge.reserved, 6000);
    assert_eq!(charge.settled, 2000);
    assert_eq!(charge.state, "settled");
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
    // A completed result is historical evidence, not a new grant execution.
    // Revocation would make validate_builtin_context fail; query and idempotent
    // execute must still return the already-paid original receipt.
    revoke(&f, &builtin, "revoke-after-completion").await;
    let query = service
        .query_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    let repeated = service
        .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(query.output, receipt.output);
    assert_eq!(repeated.output, receipt.output);
    assert_eq!(repeated.charged_credits, Some(2000));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        8000
    );
}

#[sqlx::test(migrations = false)]
async fn image_drawing_revoked_original_consent_rejects_before_model_hold(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let raw = drawing_input();
    let builtin = builtin(&f, &raw).await;
    let preview = service
        .preview(&f.secret, &builtin.context, raw)
        .await
        .unwrap();
    let (_, csrf) = service
        .confirmation_page(&f.bundle.refresh_token, &preview.operation_id)
        .await
        .unwrap();
    service
        .confirm(&f.bundle.refresh_token, &preview.operation_id, &csrf)
        .await
        .unwrap();
    revoke(&f, &builtin, "revoke-before-image-dispatch").await;
    assert_eq!(
        service
            .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    no_model_hold(&f).await;
    let receipt = service
        .query_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(receipt.state, "confirmed");
    assert!(receipt.output.is_none());
    assert!(receipt.charged_credits.is_none());
}

#[sqlx::test(migrations = false)]
async fn image_drawing_restricted_runtime_role_confirms_and_settles_original_grant(pool: PgPool) {
    let f = fixture(pool).await;
    let raw = drawing_input();
    let builtin = builtin(&f, &raw).await;
    let role = super::tests::runtime_role::RuntimeRole::create(&f.pool).await;
    for capability in ["core_charging", "core_model", "core_image"] {
        role.optional(capability).await;
    }
    crate::runtime_database::check(
        &role.pool,
        crate::runtime_database::RequiredCapabilities {
            models: true,
            images: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&role.pool, &supplier);
    let preview = service
        .preview(&f.secret, &builtin.context, raw)
        .await
        .unwrap();
    assert_eq!(preview.state, "previewed");
    no_model_hold(&f).await;
    let (_, csrf) = service
        .confirmation_page(&f.bundle.refresh_token, &preview.operation_id)
        .await
        .unwrap();
    let confirmed = service
        .confirm(&f.bundle.refresh_token, &preview.operation_id, &csrf)
        .await
        .unwrap();
    assert_eq!(confirmed.state, "confirmed");
    no_model_hold(&f).await;
    let receipt = service
        .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.reservation_credits, 6000);
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(charge.key_id.is_none());
    assert!(charge.oauth_source.is_none());
    assert_eq!(charge.owner_account_id, f.account);
    assert_eq!(charge.payer_account_id, f.account);
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.reserved, 6000);
    assert_eq!(charge.settled, 2000);
    let ledger = Ledger::from_pool(role.pool.clone());
    let balance = ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
    let queried = service
        .query_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(queried.output, receipt.output);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(service.drain(std::time::Duration::from_secs(1)).await, 0);
    drop(ledger);
    drop(service);
    role.cleanup().await;
}

#[path = "image_drawing_reference_tests.rs"]
mod references;
