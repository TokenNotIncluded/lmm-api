//! Restored executor regression from the pre-existing admin draft. These direct
//! synthetic inserts exercise transaction mechanics, not actual A2 import proof.
use super::*;
use crate::identity::IdentityStore;

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn request(approval: &Approval) -> ApplyAdminActivationRequest {
    ApplyAdminActivationRequest {
        confirmation_sha256: encode_hex_32(&payload_sha256(approval)),
        approval: approval_to_dto(approval),
    }
}

#[sqlx::test(migrations = false)]
async fn inactive_root_activates_once_on_postgres(pool: sqlx::PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let oid: i64 =
        sqlx::query_scalar("SELECT oid::bigint FROM pg_database WHERE datname=current_database()")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    let identifier: i64 = sqlx::query_scalar("SELECT system_identifier FROM pg_control_system()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let nonce = hash(9);
    sqlx::query("SELECT set_config('lmm.migration_instance_nonce',$1,false)")
        .bind(encode_hex_32(&nonce))
        .execute(&mut *conn)
        .await
        .unwrap();
    let entries = permissions::Action::CATALOG
        .iter()
        .map(|a| (a.catalog_name().unwrap().to_owned(), "allow".to_owned()))
        .collect::<Vec<_>>();
    let (snapshot, policy) = snapshot_from_binding(11, 3, &entries).unwrap();
    let approval = Approval {
        id: hash(1),
        approver: hash(2),
        expiry_unix: 4_000_000_000,
        source: FrozenSource {
            archive_sha256: hash(3),
            a2_semantic_sha256: hash(4),
            a2_file_sha256: hash(5),
            a2_sql_sha256: hash(6),
            migration_id: "synthetic-executor-mig-7".into(),
            user_id: 11,
            personal_account_id: 12,
            enabled: true,
            deleted: false,
            password_verifier_recognized: true,
            password_login_enabled: true,
            password_hash_sha256: digest_text("synthetic-password-verifier-not-a-credential"),
            login_name_sha256: digest_text("legacy-11"),
            second_factor_required: false,
            role: PlatformRole::Root,
            auth_version: 4,
        },
        target: TargetInstance {
            system_id: system_id(identifier),
            database_oid: oid,
            instance_nonce: nonce,
            import_run: import_run_digest("synthetic-executor-mig-7"),
        },
        policy,
        policy_snapshot: snapshot,
    };
    sqlx::query("INSERT INTO core_identity.accounts(id,kind,active) OVERRIDING SYSTEM VALUE VALUES (12,'personal',FALSE)")
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.users(id,personal_account_id,platform_role,active,auth_version) VALUES (11,12,'superadmin',FALSE,4)")
        .execute(&mut *conn).await.unwrap();
    // True A2 dormant gate is FALSE; the frozen source global flag is TRUE.
    sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,password_login_allowed,second_factor_required) VALUES (11,'root-import',FALSE,FALSE)")
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ('legacy-11',11,'synthetic-password-verifier-not-a-credential')")
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES (11,$1,$2,$3,$4,$5,$6,$7,$8,$9,TRUE,FALSE,TRUE,TRUE,$10,$11,'superadmin',FALSE,4)")
        .bind(approval.source.archive_sha256.as_slice()).bind(approval.source.a2_semantic_sha256.as_slice())
        .bind(approval.source.a2_file_sha256.as_slice()).bind(approval.source.a2_sql_sha256.as_slice())
        .bind(&approval.source.migration_id).bind(approval.target.system_id.as_slice()).bind(oid)
        .bind(nonce.as_slice()).bind(approval.target.import_run.as_slice())
        .bind(approval.source.password_hash_sha256.as_slice()).bind(approval.source.login_name_sha256.as_slice())
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES($1,$2,$3,$4)")
        .bind(&approval.source.migration_id).bind(encode_hex_32(&approval.source.archive_sha256))
        .bind(encode_hex_32(&approval.source.a2_semantic_sha256))
        .bind(serde_json::json!({"mapping_file_sha256":encode_hex_32(&approval.source.a2_file_sha256)}))
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) VALUES ($1,'session',11,4,12,clock_timestamp()+interval '1 hour')")
        .bind(hash(7).as_slice()).execute(&mut *conn).await.unwrap();
    let mut wrong = approval.clone();
    wrong.source.archive_sha256 = hash(99);
    assert!(matches!(
        apply_admin_activation(&mut conn, &request(&wrong)).await,
        Err(ApprovalError::SourceMismatch)
    ));
    let applied = apply_admin_activation(&mut conn, &request(&approval))
        .await
        .unwrap();
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.auth_version, Some(5));
    let replay = apply_admin_activation(&mut conn, &request(&approval))
        .await
        .unwrap();
    assert_eq!(replay.status, "replay");
    assert_eq!(replay.user_id, 11);
    let mut other = approval.clone();
    other.id = hash(12);
    assert!(matches!(
        apply_admin_activation(&mut conn, &request(&other)).await,
        Err(ApprovalError::CasConflict)
    ));
    let (active,version,role,account_active,factor,revoked,receipts):(bool,i64,String,bool,bool,bool,i64) =
        sqlx::query_as("SELECT u.active,u.auth_version,u.platform_role,a.active,p.second_factor_required,(SELECT revoked_at IS NOT NULL FROM core_identity.credentials c WHERE c.user_id=u.id),(SELECT count(*) FROM core_identity.admin_activation_receipts) FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=11")
        .fetch_one(&mut *conn).await.unwrap();
    assert!(active && account_active && !factor && revoked);
    assert_eq!((version, role.as_str(), receipts), (5, "superadmin", 1));
    // Full snapshot is persisted in the same transaction and is runtime-readable.
    let loaded = permissions::load_complete_policy(&mut conn, 11)
        .await
        .unwrap();
    assert_eq!(loaded.sha256, approval.policy.sha256);
    let store = IdentityStore::from_pool(pool);
    store.check_schema().await.unwrap();
}
