//! Explicit PG18 evidence acquisition for component 27. This does not execute an
//! upgrade, accept a caller fingerprint, or weaken the reviewed 25→26 pins.
use super::*;

#[sqlx::test]
async fn routing_admin_predecessor26_catalog_evidence(pool: sqlx::PgPool) {
    assert_eq!(COMPONENTS.len(), 27);
    assert_eq!(COMPONENTS[25].0, "telegram-auth");
    assert_eq!(COMPONENTS[26].0, "routing-admin");
    let mut tx = pool.begin().await.unwrap();
    for (_, schema) in &COMPONENTS[..26] {
        sqlx::raw_sql(schema).execute(&mut *tx).await.unwrap();
    }
    sqlx::raw_sql(CONTRACT).execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(UPGRADE_METADATA)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(ADDITIVE_UPGRADE_METADATA)
        .execute(&mut *tx)
        .await
        .unwrap();
    IdentityStore::record_schema_contract(&mut tx)
        .await
        .unwrap();
    crate::identity::grants::record_schema_contract(&mut tx)
        .await
        .unwrap();
    for (name, schema) in &COMPONENTS[..26] {
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES ($1,$2)")
            .bind(name)
            .bind(Sha256::digest(schema.as_bytes()).to_vec())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let predecessor_catalog = catalog(&mut tx).await.unwrap();
    sqlx::query("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES ($1)")
        .bind(&predecessor_catalog)
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut manifest = schema_manifest();
    manifest.components.truncate(26);
    let raw = serde_json::to_vec(&manifest).unwrap();
    let pg: String = sqlx::query_scalar("SHOW server_version_num")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    println!("ROUTING_ADMIN_PREDECESSOR26_PG={pg}");
    println!(
        "ROUTING_ADMIN_PREDECESSOR26_MANIFEST={}",
        String::from_utf8(raw.clone()).unwrap()
    );
    println!(
        "ROUTING_ADMIN_PREDECESSOR26_MANIFEST_SHA256={:x}",
        Sha256::digest(&raw)
    );
    println!(
        "ROUTING_ADMIN_PREDECESSOR26_CATALOG_SHA256={}",
        predecessor_catalog
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    tx.rollback().await.unwrap();
    // Intentionally fail until an independently inspected PG18 result is pinned
    // in the new 26→27 offline procedure and this probe becomes a regression.
    panic!("ROUTING_ADMIN_26_TO_27_PIN_PENDING: evidence only; upgrade is not accepted");
}
