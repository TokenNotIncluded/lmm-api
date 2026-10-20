use super::*;
use crate::identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore};
use serde_json::json;
use sqlx::PgPool;
#[test]
fn closed_read_arguments_reject_authority_and_unknown_names() {
    assert!(TypedReadArgs::parse("get_account_access", br#"{}"#).is_ok());
    for input in [
        br#"{"user_id":1}"#.as_slice(),
        br#"{"payer":1}"#,
        br#"{"root":true}"#,
    ] {
        assert!(TypedReadArgs::parse("get_account_access", input).is_err());
    }
    assert!(TypedReadArgs::parse("arbitrary_sql", br#"{}"#).is_err());
    assert!(TypedReadArgs::parse("list_my_api_keys", br#"{"secret":"x"}"#).is_err());
}
async fn browser(pool: &PgPool, name: &str) -> (IdentityStore, String, i64) {
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![44; 32])])
        .unwrap();
    let browser = BrowserAuth::new(store.clone(), &[45; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: name,
            password: "test-password123",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            name,
            "test-password123",
            &BrowserSessionMetadata {
                ip: "203.0.113.1".into(),
                user_agent: "synthetic-browser".into(),
            },
        )
        .await
        .unwrap();
    (
        store,
        bundle.data["access_token"].as_str().unwrap().to_owned(),
        bundle.data["user"]["id"].as_i64().unwrap(),
    )
}
#[sqlx::test(migrations = false)]
async fn read_scope_and_credential_revocation_are_live(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let (store, secret, user) = browser(&pool, "tool-alice").await;
    let (_, bob_secret, bob) = browser(&pool, "tool-bob").await;
    let mut tx = pool.begin().await.unwrap();
    let context = context::from_session(&mut tx, &secret).await.unwrap();
    let own = read_in_transaction(
        &mut tx,
        &context,
        &TypedReadArgs::UserOverview(account::UserTarget::default()),
    )
    .await
    .unwrap();
    assert_eq!(own.output["user"]["id"], user);
    assert!(
        read_in_transaction(
            &mut tx,
            &context,
            &TypedReadArgs::UserOverview(account::UserTarget {
                user_id: Some(bob),
                identifier: None
            })
        )
        .await
        .is_err()
    );
    let keys = read_in_transaction(
        &mut tx,
        &context,
        &TypedReadArgs::ListMyApiKeys(keys::ListMyApiKeys::default()),
    )
    .await
    .unwrap();
    assert_eq!(keys.output["credentials_returned"], false);
    let encoded = serde_json::to_string(&keys.output).unwrap();
    assert!(!encoded.contains(&secret));
    assert!(!encoded.contains(&bob_secret));
    let risk = read_in_transaction(&mut tx, &context, &TypedReadArgs::RegistrationRisk)
        .await
        .unwrap();
    assert_eq!(
        risk.output,
        json!({"ok":false,"status":"verification_unavailable"})
    );
    let support = read_in_transaction(
        &mut tx,
        &context,
        &TypedReadArgs::TechnicalSupportEligibility,
    )
    .await
    .unwrap();
    assert_eq!(support.output["eligible"], false);
    tx.commit().await.unwrap();
    store
        .revoke(&secret, context.credential_id())
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(context.revalidate(&mut tx).await.is_err());
    tx.rollback().await.unwrap();
}

#[test]
fn implemented_manifest_is_closed_and_self_hashed() {
    use sha2::{Digest, Sha256};
    let manifest = implemented_manifest().unwrap();
    for entry in manifest.as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let arguments = if name == "get_model_pricing" {
            br#"{"model_id":"synthetic-model"}"#.as_slice()
        } else {
            br#"{}"#
        };
        assert!(TypedReadArgs::parse(name, arguments).is_ok());
        let digest = Sha256::digest(serde_json::to_vec(&entry["parameters"]).unwrap());
        let digest: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(entry["schema_sha256"], digest);
        assert_eq!(entry["parameters"]["additionalProperties"], false);
        let price = Sha256::digest(serde_json::to_vec(&entry["price"]).unwrap());
        let price: String = price.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(entry["price_sha256"], price);
        assert_eq!(entry["domain"], "registered_core");
        assert_eq!(entry["effect"], "read_only");
    }
}
