//! Independent synthetic PG and actual loopback-HTTP acceptance of the frozen
//! narrow display-name operation. No business implementation is modified here.
//! HTTP socket evidence is library/router evidence, not a final binary sign-off.
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    identity::{
        IdentityStore,
        admin_users::{AdminUsersStore, ProfileError, ProfileUpdateRequest},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::BTreeMap, time::Duration};

struct Fixture {
    identity: IdentityStore,
    admin: String,
    root: String,
    user: String,
    store: AdminUsersStore,
    admin_id: i64,
    target_id: i64,
    root_id: i64,
}
async fn fixture(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(9201, 5).await.unwrap().secret;
    let user = identity.bootstrap_user(9202, 1).await.unwrap().secret;
    let root = identity.bootstrap_user(9203, 6).await.unwrap().secret;
    sqlx::query("UPDATE core_identity.user_profiles SET display_name='before' WHERE user_id=9202")
        .execute(pool)
        .await
        .unwrap();
    Fixture {
        store: AdminUsersStore::new(identity.clone()),
        identity,
        admin,
        root,
        user,
        admin_id: 9201,
        target_id: 9202,
        root_id: 9203,
    }
}
fn change(version: &str, old: &str, new: &str) -> ProfileUpdateRequest {
    ProfileUpdateRequest {
        expected_auth_version: version.into(),
        expected_display_name: old.into(),
        display_name: new.into(),
    }
}
async fn data_fingerprint(pool: &PgPool) -> BTreeMap<String, String> {
    let names:Vec<String>=sqlx::query_scalar("SELECT quote_ident(n.nspname)||'.'||quote_ident(c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core_%' AND c.relkind='r' ORDER BY 1").fetch_all(pool).await.unwrap();
    let mut result = BTreeMap::new();
    for name in names {
        let hash:String=sqlx::query_scalar(&format!("SELECT md5(COALESCE(string_agg(v,E'\\n' ORDER BY v),'')) FROM (SELECT row_to_json(t)::text v FROM {name} t) q")).fetch_one(pool).await.unwrap();
        result.insert(name, hash);
    }
    result
}
async fn sequences(pool: &PgPool) -> BTreeMap<String, (i64, bool)> {
    let names:Vec<String>=sqlx::query_scalar("SELECT quote_ident(n.nspname)||'.'||quote_ident(c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core_%' AND c.relkind='S' ORDER BY 1").fetch_all(pool).await.unwrap();
    let mut result = BTreeMap::new();
    for name in names {
        let value: (i64, bool) =
            sqlx::query_as(&format!("SELECT last_value,is_called FROM {name}"))
                .fetch_one(pool)
                .await
                .unwrap();
        result.insert(name, value);
    }
    result
}
fn digest(name: &str) -> String {
    format!("{:x}", Sha256::digest(name.as_bytes()))
}
async fn assert_no_mutation(
    pool: &PgPool,
    before: &BTreeMap<String, String>,
    seq: &BTreeMap<String, (i64, bool)>,
) {
    assert_eq!(*before, data_fingerprint(pool).await);
    assert_eq!(*seq, sequences(pool).await);
}
async fn audit_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='admin.user.display_name.update'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
async fn import_fact(pool: &PgPool, id: i64) {
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES($1,decode(repeat('a',64),'hex'),decode(repeat('b',64),'hex'),decode(repeat('c',64),'hex'),decode(repeat('d',64),'hex'),'independent-profile',decode(repeat('e',64),'hex'),(SELECT oid FROM pg_database WHERE datname=current_database()),decode(repeat('f',64),'hex'),decode(repeat('1',64),'hex'),true,false,true,true,decode(repeat('2',64),'hex'),decode(repeat('3',64),'hex'),'admin',false,1)").bind(id).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn profile_change_touches_only_name_and_digest_audit_without_auth_generation_or_money(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let mut before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    let original = f.store.user_profile(&f.admin, f.target_id).await.unwrap();
    let result = f
        .store
        .update_user_profile(
            &f.admin,
            f.target_id,
            &change(&original.auth_version, "before", "  新名字 💡  "),
        )
        .await
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.profile.display_name, "  新名字 💡  ");
    assert_eq!(result.profile.auth_version, original.auth_version);
    assert_eq!(result.profile.active, original.active);
    assert_eq!(result.profile.account_active, original.account_active);
    assert_eq!(audit_count(&pool).await, 1);
    let audit: Value = sqlx::query_scalar(
        "SELECT target FROM core_identity.audit WHERE action='admin.user.display_name.update'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        audit,
        json!({"user_id":f.target_id,"field":"display_name","before_sha256":digest("before"),"after_sha256":digest("  新名字 💡  "),"target_auth_version":original.auth_version})
    );
    assert!(!audit.to_string().contains("新名字"));
    let mut after = data_fingerprint(&pool).await;
    before.remove("core_identity.user_profiles");
    before.remove("core_identity.audit");
    after.remove("core_identity.user_profiles");
    after.remove("core_identity.audit");
    assert_eq!(before, after);
    let mut seq_after = sequences(&pool).await;
    let audit_after = seq_after
        .remove("core_identity.audit_id_seq")
        .expect("audit sequence");
    let mut seq_before = seq;
    let audit_before = seq_before.remove("core_identity.audit_id_seq").unwrap();
    assert_eq!(audit_after.0, audit_before.0 + 1);
    assert_eq!(seq_before, seq_after);
}

#[sqlx::test(migrations = false)]
async fn no_op_stale_cas_and_validation_refusals_do_not_allocate_audit_or_sequence(pool: PgPool) {
    let f = fixture(&pool).await;
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(
        !f.store
            .update_user_profile(&f.admin, f.target_id, &change(&version, "before", "before"))
            .await
            .unwrap()
            .changed
    );
    for input in [
        change("999", "before", "before"),
        change(&version, "stale", "before"),
    ] {
        assert!(matches!(
            f.store
                .update_user_profile(&f.admin, f.target_id, &input)
                .await,
            Err(ProfileError::Conflict)
        ));
    }
    for input in [
        change("01", "before", "new"),
        change("9223372036854775808", "before", "new"),
        change(&version, "before", &"💡".repeat(21)),
        change(&version, "before", "line\nnext"),
        change(&version, "before", "\u{85}"),
    ] {
        assert!(matches!(
            f.store
                .update_user_profile(&f.admin, f.target_id, &input)
                .await,
            Err(ProfileError::Invalid)
        ));
    }
    assert_no_mutation(&pool, &before, &seq).await;
}

#[sqlx::test(migrations = false)]
async fn audit_fault_rolls_back_all_business_rows_but_retains_one_normal_sequence_hole(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    sqlx::raw_sql("CREATE FUNCTION core_identity.independent_audit_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='admin.user.display_name.update' THEN RAISE EXCEPTION 'independent intentional audit failure' USING ERRCODE='P0001'; END IF; RETURN NEW; END $$; CREATE TRIGGER independent_audit_fault AFTER INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION core_identity.independent_audit_fault();").execute(&pool).await.unwrap();
    let before = data_fingerprint(&pool).await;
    let mut seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                &f.admin,
                f.target_id,
                &change(&version, "before", "must rollback")
            )
            .await,
        Err(ProfileError::Unavailable)
    ));
    assert_eq!(before, data_fingerprint(&pool).await);
    assert_eq!(audit_count(&pool).await, 0);
    let mut after = sequences(&pool).await;
    let old = seq.remove("core_identity.audit_id_seq").unwrap();
    let new = after.remove("core_identity.audit_id_seq").unwrap();
    assert_eq!(
        new.0,
        old.0 + 1,
        "sequence allocation survives the failed transaction by PostgreSQL design"
    );
    assert_eq!(seq, after);
    println!(
        "independent audit fault: business rows rolled back; audit sequence advanced once (normal nontransactional hole)"
    );
}

#[sqlx::test(migrations = false)]
async fn two_concurrent_cas_updates_have_exactly_one_winner_and_one_audit(pool: PgPool) {
    let f = fixture(&pool).await;
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    let left_request = change(&version, "before", "left");
    let right_request = change(&version, "before", "right");
    let (left, right) = tokio::join!(
        f.store
            .update_user_profile(&f.admin, f.target_id, &left_request),
        f.store
            .update_user_profile(&f.root, f.target_id, &right_request)
    );
    let winners = [left.as_ref().ok(), right.as_ref().ok()]
        .into_iter()
        .flatten()
        .filter(|v| v.changed)
        .count();
    assert_eq!(winners, 1);
    assert_eq!(
        [left.as_ref().err(), right.as_ref().err()]
            .into_iter()
            .flatten()
            .filter(|v| **v == ProfileError::Conflict)
            .count(),
        1
    );
    assert_eq!(audit_count(&pool).await, 1);
    let actual = f.store.user_profile(&f.admin, f.target_id).await.unwrap();
    assert!(matches!(actual.display_name.as_str(), "left" | "right"));
    assert_eq!(actual.auth_version, version);
}

async fn wait_for_writer_lock(pool: &PgPool, fragment: &str) {
    for _ in 0..30 {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND wait_event_type='Lock' AND query LIKE $1)")
            .bind(format!("%{fragment}%")).fetch_one(pool).await.unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the independent concurrent transaction never observed the expected real row-lock wait");
}

#[sqlx::test(migrations = false)]
async fn revocation_committed_while_patch_waits_on_actor_lock_prevents_mutation(pool: PgPool) {
    let f = fixture(&pool).await;
    let activity_limit: String = sqlx::query_scalar("SHOW track_activity_query_size")
        .fetch_one(&pool)
        .await
        .unwrap();
    println!(
        "independent actor-revoke lock observer: track_activity_query_size={activity_limit}; pinned AUTH=1055 bytes; matching unique prefix plus real Lock wait"
    );
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=$1 AND kind='session'").bind(f.admin_id).execute(&mut *tx).await.unwrap();
    let task_store = f.store.clone();
    let actor = f.admin.clone();
    let id = f.target_id;
    let task = tokio::spawn(async move {
        task_store
            .update_user_profile(&actor, id, &change(&version, "before", "forbidden"))
            .await
    });
    // AUTH is 1055 bytes before the suffix; pg_stat_activity commonly
    // truncates at 1KiB, so require the unique AUTH prefix plus a real Lock wait.
    wait_for_writer_lock(
        &pool,
        "SELECT c.id,c.kind,c.user_id,c.user_version,c.owner_account_id",
    )
    .await;
    tx.commit().await.unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        task.await.unwrap(),
        Err(ProfileError::Unauthorized)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
    assert_eq!(audit_count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn target_promoted_while_patch_waits_is_rechecked_and_never_modified(pool: PgPool) {
    let f = fixture(&pool).await;
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=$1")
        .bind(f.target_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let store = f.store.clone();
    let actor = f.admin.clone();
    let target = f.target_id;
    let task = tokio::spawn(async move {
        store
            .update_user_profile(&actor, target, &change(&version, "before", "forbidden"))
            .await
    });
    wait_for_writer_lock(
        &pool,
        "SELECT personal_account_id,platform_role,auth_version,active FROM core_identity.users",
    )
    .await;
    tx.commit().await.unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(task.await.unwrap(), Err(ProfileError::Forbidden)));
    assert_no_mutation(&pool, &before, &seq).await;
}

#[sqlx::test(migrations = false)]
async fn disabled_ordinary_target_metadata_repair_never_activates_or_changes_credentials(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=$1")
        .bind(f.target_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.accounts SET active=false WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)").bind(f.target_id).execute(&pool).await.unwrap();
    let profile = f.store.user_profile(&f.root, f.target_id).await.unwrap();
    assert!(!profile.active);
    assert!(!profile.account_active);
    let mut before = data_fingerprint(&pool).await;
    let result = f
        .store
        .update_user_profile(
            &f.root,
            f.target_id,
            &change(&profile.auth_version, "before", "repaired"),
        )
        .await
        .unwrap();
    assert!(result.changed);
    assert!(!result.profile.active);
    assert!(!result.profile.account_active);
    assert_eq!(profile.auth_version, result.profile.auth_version);
    let mut after = data_fingerprint(&pool).await;
    for name in ["core_identity.user_profiles", "core_identity.audit"] {
        before.remove(name);
        after.remove(name);
    }
    assert_eq!(before, after);
    assert_eq!(audit_count(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn ordinary_api_key_host_stale_mfa_and_privileged_target_refusals_are_write_free(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Personal,
                id: f.admin_id,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    for (secret, expected) in [
        (f.user.as_str(), ProfileError::Forbidden),
        (key.secret.as_str(), ProfileError::Forbidden),
        ("host-token", ProfileError::Unauthorized),
        ("legacy-unsupported-pat", ProfileError::Unauthorized),
    ] {
        assert_eq!(
            f.store
                .update_user_profile(
                    secret,
                    f.target_id,
                    &change(&version, "before", "forbidden")
                )
                .await
                .err(),
            Some(expected)
        );
    }
    for target in [f.admin_id, f.root_id] {
        assert!(matches!(
            f.store
                .update_user_profile(&f.root, target, &change("1", "irrelevant", "forbidden"))
                .await,
            Err(ProfileError::Forbidden)
        ));
    }
    assert_no_mutation(&pool, &before, &seq).await;
    sqlx::query(
        "UPDATE core_identity.user_profiles SET second_factor_required=true WHERE user_id=$1",
    )
    .bind(f.admin_id)
    .execute(&pool)
    .await
    .unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                &f.admin,
                f.target_id,
                &change(&version, "before", "forbidden")
            )
            .await,
        Err(ProfileError::Unauthorized)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
    sqlx::query(
        "UPDATE core_identity.user_profiles SET second_factor_required=false WHERE user_id=$1",
    )
    .bind(f.admin_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1")
        .bind(f.admin_id)
        .execute(&pool)
        .await
        .unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                &f.admin,
                f.target_id,
                &change(&version, "before", "forbidden")
            )
            .await,
        Err(ProfileError::Unauthorized)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
}

#[sqlx::test(migrations = false)]
async fn incomplete_import_policy_and_policy_storage_fault_reject_profile_writes(pool: PgPool) {
    let f = fixture(&pool).await;
    let version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    import_fact(&pool, f.admin_id).await;
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                &f.admin,
                f.target_id,
                &change(&version, "before", "forbidden")
            )
            .await,
        Err(ProfileError::Forbidden)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES($1,1,'core.permissions.v1',decode(repeat('0',64),'hex'),convert_to('{}','UTF8'),true)").bind(f.admin_id).execute(&pool).await.unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                &f.admin,
                f.target_id,
                &change(&version, "before", "forbidden")
            )
            .await,
        Err(ProfileError::Forbidden)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
    sqlx::query(
        "ALTER TABLE core_identity.permission_policies RENAME TO independent_unavailable_policies",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                &f.admin,
                f.target_id,
                &change(&version, "before", "forbidden")
            )
            .await,
        Err(ProfileError::Unavailable)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
}

#[sqlx::test(migrations = false)]
async fn current_generation_primary_session_still_cannot_bypass_required_mfa(pool: PgPool) {
    let f = fixture(&pool).await;
    let target_version = f
        .store
        .user_profile(&f.admin, f.target_id)
        .await
        .unwrap()
        .auth_version;
    sqlx::query(
        "UPDATE core_identity.user_profiles SET second_factor_required=true WHERE user_id=$1",
    )
    .bind(f.admin_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO core_identity.mfa_factors(user_id,revision,enabled,key_id,encrypted_secret) VALUES($1,1,true,'independent-test-only',decode(repeat('0',98),'hex'))").bind(f.admin_id).execute(&pool).await.unwrap();
    // Deliberately construct a current-version but primary-only persisted
    // Session in this owner-controlled synthetic DB. This isolates assurance
    // rejection from stale-generation rejection; it is not a real login proof.
    let secret = "lmms_independent_primary_only_current_version_1234567890";
    let id:i64=sqlx::query_scalar("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=$2 RETURNING id")
        .bind(Sha256::digest(secret.as_bytes()).to_vec()).bind(f.admin_id).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,factor_level,method,authenticated_at) SELECT $1,id,auth_version,0,1,'primary',clock_timestamp() FROM core_identity.users WHERE id=$2")
        .bind(id).bind(f.admin_id).execute(&pool).await.unwrap();
    let versions_match:bool=sqlx::query_scalar("SELECT c.user_version=u.auth_version AND p.user_version=u.auth_version FROM core_identity.credentials c JOIN core_identity.users u ON u.id=c.user_id JOIN core_identity.session_authentication p ON p.credential_id=c.id WHERE c.id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert!(versions_match);
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    assert!(matches!(
        f.store
            .update_user_profile(
                secret,
                f.target_id,
                &change(&target_version, "before", "forbidden")
            )
            .await,
        Err(ProfileError::Unauthorized)
    ));
    assert!(matches!(
        f.store.user_profile(secret, f.target_id).await,
        Err(ProfileError::Unauthorized)
    ));
    assert_no_mutation(&pool, &before, &seq).await;
}

struct HttpServer {
    base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(store: AdminUsersStore) -> HttpServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, lmm_core::identity::admin_users::router(store))
            .await
            .unwrap();
    });
    HttpServer { base, task }
}
async fn response(request: reqwest::RequestBuilder, status: u16, code: Option<&str>) -> Value {
    let response = request.send().await.unwrap();
    assert_eq!(response.status().as_u16(), status);
    for (key, value) in [
        ("cache-control", "private, no-store"),
        ("vary", "Authorization"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
    ] {
        assert_eq!(
            response.headers().get(key).unwrap().to_str().unwrap(),
            value
        );
    }
    let value: Value = response.json().await.unwrap();
    if let Some(code) = code {
        assert_eq!(value, json!({"error":{"code":code}}));
    }
    value
}

#[sqlx::test(migrations = false)]
async fn real_http_contract_strict_json_limits_private_headers_double_cas_and_missing_profile(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let http = server(f.store.clone()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let path = format!("{}/core/v1/admin/user-profiles/{}", http.base, f.target_id);
    let caps = response(
        client
            .get(format!(
                "{}/core/v1/admin/user-profiles/capabilities",
                http.base
            ))
            .bearer_auth(&f.admin),
        200,
        None,
    )
    .await;
    assert_eq!(
        caps,
        json!({"version":1,"contract":"core.admin-user-profile.v1","operations":{"read":true,"update_display_name":true},"target_platform_roles":["user"],"display_name_limits":{"max_code_points":20,"max_utf8_bytes":80,"controls_allowed":false},"preconditions":["auth_version","display_name"]})
    );
    let initial = response(client.get(&path).bearer_auth(&f.admin), 200, None).await;
    assert_eq!(initial["user_id"], f.target_id.to_string());
    assert_eq!(initial["display_name"], "before");
    let version = initial["auth_version"].as_str().unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    for body in [
        json!({"expected_auth_version":version,"expected_display_name":"before","display_name":"new","platform_role":"superadmin"}).to_string(),
        format!(r#"{{"expected_auth_version":"{version}","expected_auth_version":"2","expected_display_name":"before","display_name":"new"}}"#),
        format!(r#"{{"expected_auth_version":"{version}","expected_display_name":"before","display_name":"new","display_name":"second"}}"#),
        json!({"expected_auth_version":1,"expected_display_name":"before","display_name":"new"}).to_string(),
        json!({"expected_auth_version":version,"expected_display_name":"before"}).to_string(),
        json!({"expected_auth_version":version,"expected_display_name":"x".repeat(5000),"display_name":"new"}).to_string(),
    ] {
        response(client.patch(&path).bearer_auth(&f.admin).header("content-type","application/json").body(body),400,Some("ADMIN_PROFILE_INVALID")).await;
    }
    response(client.patch(format!("{path}?unexpected=1")).bearer_auth(&f.admin).json(&json!({"expected_auth_version":version,"expected_display_name":"before","display_name":"new"})),400,Some("ADMIN_PROFILE_INVALID")).await;
    response(client.patch(&path).json(&json!({"expected_auth_version":version,"expected_display_name":"before","display_name":"new"})),401,Some("ADMIN_PROFILE_UNAUTHORIZED")).await;
    response(client.patch(&path).bearer_auth(&f.user).json(&json!({"expected_auth_version":version,"expected_display_name":"before","display_name":"new"})),403,Some("ADMIN_PROFILE_FORBIDDEN")).await;
    response(client.patch(format!("{}/core/v1/admin/user-profiles/{}",http.base,f.root_id)).bearer_auth(&f.root).json(&json!({"expected_auth_version":"1","expected_display_name":"irrelevant","display_name":"new"})),403,Some("ADMIN_PROFILE_FORBIDDEN")).await;
    assert_no_mutation(&pool, &before, &seq).await;
    let changed=response(client.patch(&path).bearer_auth(&f.admin).json(&json!({"expected_auth_version":version,"expected_display_name":"before","display_name":"after"})),200,None).await;
    assert_eq!(changed["changed"], true);
    assert_eq!(changed["profile"]["display_name"], "after");
    assert_eq!(changed["profile"]["auth_version"], version);
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    response(client.patch(&path).bearer_auth(&f.admin).json(&json!({"expected_auth_version":version,"expected_display_name":"before","display_name":"after"})),409,Some("ADMIN_PROFILE_CONFLICT")).await;
    response(client.patch(&path).bearer_auth(&f.admin).json(&json!({"expected_auth_version":"999","expected_display_name":"after","display_name":"after"})),409,Some("ADMIN_PROFILE_CONFLICT")).await;
    let noop=response(client.patch(&path).bearer_auth(&f.admin).json(&json!({"expected_auth_version":version,"expected_display_name":"after","display_name":"after"})),200,None).await;
    assert_eq!(noop["changed"], false);
    assert_no_mutation(&pool, &before, &seq).await;
    sqlx::query("DELETE FROM core_identity.user_profiles WHERE user_id=$1")
        .bind(f.target_id)
        .execute(&pool)
        .await
        .unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    response(
        client.get(&path).bearer_auth(&f.admin),
        404,
        Some("ADMIN_PROFILE_NOT_FOUND"),
    )
    .await;
    response(client.patch(&path).bearer_auth(&f.admin).json(&json!({"expected_auth_version":version,"expected_display_name":"after","display_name":"new"})),404,Some("ADMIN_PROFILE_NOT_FOUND")).await;
    response(
        client
            .get(format!(
                "{}/core/v1/admin/user-profiles/99999999",
                http.base
            ))
            .bearer_auth(&f.admin),
        404,
        Some("ADMIN_PROFILE_NOT_FOUND"),
    )
    .await;
    assert_no_mutation(&pool, &before, &seq).await;
}

#[sqlx::test(migrations = false)]
async fn real_http_audit_failure_returns_503_with_business_rollback_and_observed_sequence_hole(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let http = server(f.store.clone()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let path = format!("{}/core/v1/admin/user-profiles/{}", http.base, f.target_id);
    let initial = response(client.get(&path).bearer_auth(&f.admin), 200, None).await;
    sqlx::raw_sql("CREATE FUNCTION core_identity.independent_http_audit_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='admin.user.display_name.update' THEN RAISE EXCEPTION 'independent intentional HTTP audit failure' USING ERRCODE='P0001'; END IF; RETURN NEW; END $$; CREATE TRIGGER independent_http_audit_fault AFTER INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION core_identity.independent_http_audit_fault();").execute(&pool).await.unwrap();
    let before = data_fingerprint(&pool).await;
    let seq = sequences(&pool).await;
    response(client.patch(&path).bearer_auth(&f.admin).json(&json!({"expected_auth_version":initial["auth_version"],"expected_display_name":"before","display_name":"must rollback"})),503,Some("ADMIN_PROFILE_UNAVAILABLE")).await;
    assert_eq!(before, data_fingerprint(&pool).await);
    assert_eq!(
        sequences(&pool).await["core_identity.audit_id_seq"].0,
        seq["core_identity.audit_id_seq"].0 + 1
    );
    let readback = response(client.get(&path).bearer_auth(&f.admin), 200, None).await;
    assert_eq!(readback, initial);
    println!(
        "independent HTTP fault: 503 and exact profile readback; data rolled back; one normal audit sequence hole"
    );
}
