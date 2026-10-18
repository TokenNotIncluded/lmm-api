//! Standalone extraction of the two frozen v7 catalog socket tests.
//! Assertions, fixed-main oracle and fingerprint helper are preserved verbatim.
//! The synthetic identity-only fixture below intentionally creates no history
//! schema or rows. Extraction changes input; this split must be independently
//! compiled and rerun. This is router/socket evidence, not final binary/browser.
use lmm_core::{core_schema, identity::IdentityStore};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;

struct Fixture {
    identity: IdentityStore,
    user: String,
    admin: String,
}
async fn fixture(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(9101, 1).await.unwrap().secret;
    let admin = identity.bootstrap_user(9102, 5).await.unwrap().secret;
    Fixture {
        identity,
        user,
        admin,
    }
}

/// Fingerprint every Core table and sequence, rather than asserting a few row
/// counts. No source or credential values are printed or persisted by this helper.
async fn fingerprint(pool: &PgPool) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let rows=sqlx::query("SELECT quote_ident(n.nspname)||'.'||quote_ident(c.relname) AS name,c.relkind::text AS kind FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core_%' AND c.relkind IN ('r','S') ORDER BY 1").fetch_all(pool).await.unwrap();
    for row in rows {
        let name: String = row.get("name");
        let kind: String = row.get("kind");
        let sql = if kind == "S" {
            format!("SELECT md5(last_value::text || chr(58) || is_called::text) FROM {name}")
        } else {
            format!(
                "SELECT md5(COALESCE(string_agg(v,E'\\n' ORDER BY v),'')) FROM (SELECT row_to_json(t)::text v FROM {name} t) q"
            )
        };
        let hash: String = sqlx::query_scalar(&sql).fetch_one(pool).await.unwrap();
        result.insert(name, hash);
    }
    result
}

// Additional independent HTTP checks. Actual loopback sockets and real PG;
// library/router input, not the final application binary or browser.
struct DiscoveryHttp {
    base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for DiscoveryHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn discovery_http(store: lmm_core::identity::PermissionReadStore) -> DiscoveryHttp {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, lmm_core::identity::permission_read_router(store))
            .await
            .unwrap();
    });
    DiscoveryHttp { base, task }
}
async fn discovery_response(request: reqwest::RequestBuilder, status: u16) -> serde_json::Value {
    let response = request.send().await.unwrap();
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["vary"], "Authorization");
    let value: serde_json::Value = response.json().await.unwrap();
    if status != 200 {
        assert_ne!(value.get("success"), Some(&serde_json::Value::Bool(true)));
    }
    value
}
fn discovery_expected_catalog() -> serde_json::Value {
    // Independently transcribed from fixed main 5999dc656's resources_* and role.go.
    let resources = serde_json::json!([
        {"resource":"acquisition","label_key":"User acquisition","actions":[
            {"action":"read","label_key":"View acquisition summaries","description_key":"View aggregate channel and campaign results."},
            {"action":"write","label_key":"Manage promotion links","description_key":"Create, archive, and delete promotion links."},
            {"action":"details","label_key":"View acquisition account details","description_key":"View account-level source records."},
            {"action":"export","label_key":"Export acquisition details","description_key":"Export account-level source records."}]},
        {"resource":"channel","label_key":"Channel Management","actions":[
            {"action":"read","label_key":"Read channels","description_key":"View channel lists and details without secrets."},
            {"action":"operate","label_key":"Operate channels","description_key":"Test channels, refresh balances, and enable/disable individual, batch, or tagged channels."},
            {"action":"write","label_key":"Edit channel routing","description_key":"Edit non-sensitive settings such as models, groups, and routing rules."},
            {"action":"sensitive_write","label_key":"Edit sensitive channel settings","description_key":"Create channels or edit keys, base URLs, and overrides."},
            {"action":"secret_view","label_key":"View channel secrets","description_key":"Reserved for viewing complete channel keys after secure verification."}]}
    ]);
    let admin = serde_json::json!({"acquisition":{"read":true,"write":true,"details":false,"export":false},"channel":{"read":true,"operate":true,"write":true,"sensitive_write":false,"secret_view":false}});
    let root = serde_json::json!({"acquisition":{"read":true,"write":true,"details":true,"export":true},"channel":{"read":true,"operate":true,"write":true,"sensitive_write":true,"secret_view":true}});
    serde_json::json!({"resources":resources,"roles":[
        {"key":"root","name":"Root","built_in":true,"superuser":true,"grants":root},
        {"key":"admin","name":"Admin","built_in":true,"superuser":false,"grants":admin}
    ]})
}

#[sqlx::test(migrations = false)]
async fn real_http_catalog_exact_main_oracle_native_capabilities_and_credential_denials(
    pool: PgPool,
) {
    use lmm_core::{
        accounts::{Account, AccountKind},
        funding::FundingPolicy,
    };
    let f = fixture(&pool).await;
    let root = f.identity.bootstrap_user(9103, 6).await.unwrap().secret;
    let key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Personal,
                id: 9102,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let http = discovery_http(lmm_core::identity::PermissionReadStore::new(
        f.identity.clone(),
    ))
    .await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let expected = discovery_expected_catalog();
    let before = fingerprint(&pool).await;
    for (secret, uid, index) in [
        (f.admin.as_str(), "9102", 1usize),
        (root.as_str(), "9103", 0usize),
    ] {
        let native = discovery_response(
            client
                .get(format!("{}/core/v1/authz/catalog", http.base))
                .bearer_auth(secret),
            200,
        )
        .await;
        assert_eq!(native, expected);
        let legacy = discovery_response(
            client
                .get(format!("{}/api/authz/catalog", http.base))
                .bearer_auth(secret),
            200,
        )
        .await;
        assert_eq!(
            legacy,
            serde_json::json!({"success":true,"message":"","data":expected})
        );
        let caps = discovery_response(
            client
                .get(format!("{}/core/v1/authz/capabilities", http.base))
                .bearer_auth(secret),
            200,
        )
        .await;
        assert_eq!(
            caps,
            serde_json::json!({"version":1,"contract":"core.permissions.v1","user_id":uid,"admin_permissions":expected["roles"][index]["grants"],"policy_revision":null,"policy_sha256":null})
        );
    }
    for route in [
        "/core/v1/authz/catalog",
        "/core/v1/authz/capabilities",
        "/api/authz/catalog",
    ] {
        let url = format!("{}{route}", http.base);
        for (secret, status) in [
            (f.user.as_str(), 403),
            (key.secret.as_str(), 403),
            ("host_unknown_123456789012345678901234567890", 401),
        ] {
            discovery_response(client.get(&url).bearer_auth(secret), status).await;
        }
        discovery_response(client.get(&url), 401).await;
        discovery_response(
            client
                .get(format!("{url}?unexpected=1"))
                .bearer_auth(&f.admin),
            400,
        )
        .await;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", f.admin).parse().unwrap(),
        );
        headers.append(
            reqwest::header::AUTHORIZATION,
            "Bearer duplicate".parse().unwrap(),
        );
        discovery_response(client.get(&url).headers(headers), 401).await;
    }
    assert_eq!(before, fingerprint(&pool).await);
}

async fn discovery_imported(pool: &PgPool, id: i64) {
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES($1,decode(repeat('a',64),'hex'),decode(repeat('b',64),'hex'),decode(repeat('c',64),'hex'),decode(repeat('d',64),'hex'),'independent-discovery',decode(repeat('e',64),'hex'),(SELECT oid FROM pg_database WHERE datname=current_database()),decode(repeat('f',64),'hex'),decode(repeat('1',64),'hex'),true,false,true,true,decode(repeat('2',64),'hex'),decode(repeat('3',64),'hex'),'admin',false,1)").bind(id).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn real_http_imported_policy_complete_deny_missing_hash_and_storage_boundaries(pool: PgPool) {
    use lmm_core::identity::{canonical_policy_payload, complete_policy_snapshot, policy_sha256};
    let f = fixture(&pool).await;
    let missing = f.identity.bootstrap_user(9104, 5).await.unwrap().secret;
    let bad = f.identity.bootstrap_user(9105, 5).await.unwrap().secret;
    for id in [9102, 9104, 9105] {
        discovery_imported(&pool, id).await;
    }
    let entries: Vec<(String, String)> = [
        "ChannelRead",
        "ChannelOperate",
        "ChannelWrite",
        "ChannelSensitiveWrite",
        "ChannelSecretView",
        "AcquisitionRead",
        "AcquisitionWrite",
        "AcquisitionDetails",
        "AcquisitionExport",
    ]
    .into_iter()
    .map(|name| (name.into(), "deny".into()))
    .collect();
    let snapshot = complete_policy_snapshot(9102, 7, &entries).unwrap();
    let payload = canonical_policy_payload(&snapshot).unwrap();
    let hash = policy_sha256(&snapshot).unwrap();
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(9102,7,'core.permissions.v1',$1,$2,true)").bind(hash.to_vec()).bind(payload).execute(&pool).await.unwrap();
    for (action, effect) in entries {
        sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES(9102,$1,$2)").bind(action).bind(effect).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(9105,1,'core.permissions.v1',decode(repeat('0',64),'hex'),convert_to('{}','UTF8'),true)").execute(&pool).await.unwrap();
    let http = discovery_http(lmm_core::identity::PermissionReadStore::new(
        f.identity.clone(),
    ))
    .await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let before = fingerprint(&pool).await;
    for route in [
        "/core/v1/authz/catalog",
        "/core/v1/authz/capabilities",
        "/api/authz/catalog",
    ] {
        for secret in [&missing, &bad] {
            discovery_response(
                client
                    .get(format!("{}{route}", http.base))
                    .bearer_auth(secret),
                403,
            )
            .await;
        }
    }
    let actual = discovery_response(
        client
            .get(format!("{}/core/v1/authz/capabilities", http.base))
            .bearer_auth(&f.admin),
        200,
    )
    .await;
    let deny = serde_json::json!({"acquisition":{"read":false,"write":false,"details":false,"export":false},"channel":{"read":false,"operate":false,"write":false,"sensitive_write":false,"secret_view":false}});
    let hash_string: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        actual,
        serde_json::json!({"version":1,"contract":"core.permissions.v1","user_id":"9102","admin_permissions":deny,"policy_revision":"7","policy_sha256":hash_string})
    );
    assert_eq!(before, fingerprint(&pool).await);
    sqlx::query("ALTER TABLE core_identity.permission_policies RENAME TO independent_http_unavailable_policies").execute(&pool).await.unwrap();
    let before = fingerprint(&pool).await;
    for route in [
        "/core/v1/authz/catalog",
        "/core/v1/authz/capabilities",
        "/api/authz/catalog",
    ] {
        discovery_response(
            client
                .get(format!("{}{route}", http.base))
                .bearer_auth(&f.admin),
            503,
        )
        .await;
    }
    assert_eq!(before, fingerprint(&pool).await);
}
