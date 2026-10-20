//! Independent synthetic acceptance. Copy only this test and the separately
//! pinned schema fixture into a frozen candidate; never edit business source.
//! Source presence is not a test result. Requires a fresh SQLx test database.
#[path = "common/history_scope.rs"]
mod history_scope;
use lmm_core::{
    core_schema,
    finance_export::{self, ExportFormat},
    history_read::{HistoryError, HistoryQuery, HistoryStore, ReadRoute, projection},
    identity::{IdentityStore, PermissionReadStore},
};
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn main_numeric_metadata_contract_keeps_long_integral_decimals_exact() {
    let accepted = [
        ("1.0000000000000000000000000000000000000000000000000", 1),
        (
            "9007199254740991.000000000000000000000000000",
            9_007_199_254_740_991,
        ),
        ("500000.00000000000000000000000000000000000", 500000),
        ("0.000000000000000001e18", 1),
        ("5e5", 500000),
        ("9007199254740991", 9_007_199_254_740_991),
    ];
    for (raw, expected) in accepted {
        assert!(raw.len() <= 64);
        assert_eq!(
            projection::captured_basis(&format!("{{\"pricing_credits_per_usd\":{raw}}}")),
            Some(expected),
            "valid raw={raw}"
        );
    }
    for raw in [
        "0",
        "-1",
        "1.0000000000000000000000000000000000000000000000001",
        "9007199254740992",
        "1e19",
        "1e-19",
        "true",
        "null",
        "\"500000\"",
    ] {
        assert_eq!(
            projection::captured_basis(&format!("{{\"pricing_credits_per_usd\":{raw}}}")),
            None,
            "invalid raw={raw}"
        );
    }
    let base = "{\"pricing_credits_per_usd\":1,\"padding\":\"\"}";
    let at_limit = base.replacen(
        "\"\"",
        &format!("\"{}\"", "x".repeat(65536 - base.len())),
        1,
    );
    assert_eq!(at_limit.len(), 65536);
    assert_eq!(projection::captured_basis(&at_limit), Some(1));
    assert_eq!(projection::captured_basis(&(at_limit + " ")), None);
}

#[test]
fn money_is_exact_and_unknown_does_not_serialize_as_zero() {
    for (q, k, expected) in [
        (1, 3, "0.333333333333333333"),
        (2, 3, "0.666666666666666667"),
        (1, projection::MAX_BASIS, "0.000000000000000111"),
        (3_359_744, 3_359_744, "1"),
        (0, 7, "0"),
    ] {
        let value = projection::log_money(Some(q), 2, Some(k));
        assert_eq!(value.quota_usd.as_deref(), Some(expected));
    }
    for (q, t, k, label) in [
        (None, 2, Some(1), "invalid_stored_quota"),
        (Some(-1), 2, Some(1), "missing_credit_basis"),
        (Some(0), 2, None, "missing_credit_basis"),
        (Some(5), 3, Some(1), "not_consumption"),
    ] {
        let value = projection::log_money(q, t, k);
        assert!(!value.quota_money_available);
        assert_eq!(value.quota_money_basis, label);
        assert!(
            serde_json::to_value(value)
                .unwrap()
                .get("quota_usd")
                .is_none()
        );
    }
}

struct Fixture {
    identity: IdentityStore,
    store: HistoryStore,
    user: String,
    admin: String,
    owner: i64,
    other: i64,
}
async fn fixture(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(9101, 1).await.unwrap().secret;
    let admin = identity.bootstrap_user(9102, 5).await.unwrap().secret;
    let owner =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=9101")
            .fetch_one(pool)
            .await
            .unwrap();
    let other =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=9102")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES ('core.history-source.v1',repeat('a',64),repeat('b',64),'independent-synthetic',1700000000,true)").execute(pool).await.unwrap();
    history_scope::install(pool).await;
    Fixture {
        identity,
        store: HistoryStore::new(pool.clone()),
        user,
        admin,
        owner,
        other,
    }
}
async fn row(
    pool: &PgPool,
    id: i64,
    user: i64,
    owner: i64,
    quota: Option<i64>,
    basis: Option<i64>,
    model: &str,
) {
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,pricing_credits_per_usd,username,model_name,token_name,channel_id,group_name,request_id,upstream_request_id) VALUES(1,$1,$2,repeat('c',64),$3,$4,1700000000,2,$5,$6,'synthetic',$7,'key',8,'group','request','upstream')")
        .bind(id).bind(format!("independent-{id}")).bind(user).bind(owner).bind(quota).bind(basis).bind(model).execute(pool).await.unwrap();
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

#[sqlx::test(migrations = false)]
async fn independent_filters_page_total_and_money_share_scope_and_never_write(pool: PgPool) {
    let f = fixture(&pool).await;
    row(&pool, 1, 9101, f.owner, Some(1), Some(3), "selected").await;
    row(&pool, 2, 9101, f.owner, Some(1), Some(6), "selected").await;
    row(&pool, 3, 9101, f.owner, Some(7), None, "different").await;
    row(&pool, 4, 9102, f.other, Some(99), None, "selected").await;
    let before = fingerprint(&pool).await;
    let query = HistoryQuery {
        model_name: Some("selected".into()),
        token_name: Some("key".into()),
        group: Some("group".into()),
        request_id: Some("request".into()),
        start_timestamp: Some(1700000000),
        end_timestamp: Some(1700000000),
        page_size: Some(1),
        ..Default::default()
    };
    let first = f
        .store
        .read(&f.user, ReadRoute::SelfSession, &query)
        .await
        .unwrap();
    assert_eq!((first.total, first.items.len()), (2, 1));
    assert_eq!(first.items[0].source_log_id, 2);
    assert_eq!(first.summary.raw_consumption_quota, "2");
    assert_eq!(first.summary.money.quota_usd.as_deref(), Some("0.5"));
    let second = f
        .store
        .read(
            &f.user,
            ReadRoute::SelfSession,
            &HistoryQuery {
                p: Some(2),
                ..query.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(second.items[0].source_log_id, 1);
    assert_eq!(
        serde_json::to_value(&first.summary).unwrap(),
        serde_json::to_value(&second.summary).unwrap()
    );
    for item in &first.items {
        assert_eq!(item.user_id, 9101);
        assert!(item.channel.is_none());
        assert!(item.upstream_request_id.is_none());
    }
    let all = f
        .store
        .read(&f.admin, ReadRoute::Administrator, &query)
        .await
        .unwrap();
    assert_eq!(all.total, 3);
    assert!(!all.summary.money.quota_money_available);
    assert!(all.summary.money.quota_usd.is_none());
    assert_eq!(before, fingerprint(&pool).await);
}

#[sqlx::test(migrations = false)]
async fn independent_admin_denials_and_storage_failure_are_not_empty_success(pool: PgPool) {
    let f = fixture(&pool).await;
    let permissions = PermissionReadStore::new(f.identity.clone());
    assert!(permissions.catalog(&f.admin).await.is_ok());
    let before = fingerprint(&pool).await;
    for secret in [&f.user, "lmmk_unknown", "host-token", "", "lmms_unknown"] {
        assert!(permissions.catalog(secret).await.is_err());
        assert!(
            f.store
                .read(secret, ReadRoute::Administrator, &Default::default())
                .await
                .is_err()
        );
    }
    assert_eq!(before, fingerprint(&pool).await);
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=9102")
        .execute(&pool)
        .await
        .unwrap();
    assert!(permissions.catalog(&f.admin).await.is_err());
    assert_eq!(
        f.store
            .read(&f.admin, ReadRoute::Administrator, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Unauthorized
    );
    sqlx::query("DROP SCHEMA core_history CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .read(&f.user, ReadRoute::SelfSession, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}

#[sqlx::test(migrations = false)]
async fn independent_imported_policy_missing_malformed_and_storage_are_distinct(pool: PgPool) {
    use lmm_core::identity::admin_users::AdminUsersError;
    let f = fixture(&pool).await;
    let permissions = PermissionReadStore::new(f.identity.clone());
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES(9102,decode(repeat('a',64),'hex'),decode(repeat('b',64),'hex'),decode(repeat('c',64),'hex'),decode(repeat('d',64),'hex'),'independent-policy-negative',decode(repeat('e',64),'hex'),(SELECT oid FROM pg_database WHERE datname=current_database()),decode(repeat('f',64),'hex'),decode(repeat('1',64),'hex'),true,false,true,true,decode(repeat('2',64),'hex'),decode(repeat('3',64),'hex'),'admin',false,1)").execute(&pool).await.unwrap();
    let before = fingerprint(&pool).await;
    assert!(matches!(
        permissions.catalog(&f.admin).await,
        Err(AdminUsersError::Forbidden)
    ));
    assert!(matches!(
        permissions.capabilities(&f.admin).await,
        Err(AdminUsersError::Forbidden)
    ));
    assert_eq!(
        f.store
            .read(&f.admin, ReadRoute::Administrator, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    assert_eq!(before, fingerprint(&pool).await);
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(9102,1,'core.permissions.v1',decode(repeat('0',64),'hex'),convert_to('{}','UTF8'),true)").execute(&pool).await.unwrap();
    assert!(matches!(
        permissions.catalog(&f.admin).await,
        Err(AdminUsersError::Forbidden)
    ));
    assert!(matches!(
        permissions.capabilities(&f.admin).await,
        Err(AdminUsersError::Forbidden)
    ));
    assert_eq!(
        f.store
            .read(&f.admin, ReadRoute::Administrator, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    sqlx::query("DROP TABLE core_identity.permission_policies CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        permissions.catalog(&f.admin).await,
        Err(AdminUsersError::Unavailable)
    ));
    assert_eq!(
        f.store
            .read(&f.admin, ReadRoute::Administrator, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}

#[sqlx::test(migrations = false)]
async fn independent_query_bounds_reject_without_changing_any_core_state(pool: PgPool) {
    let f = fixture(&pool).await;
    let before = fingerprint(&pool).await;
    let invalid = vec![
        HistoryQuery {
            p: Some(0),
            ..Default::default()
        },
        HistoryQuery {
            p: Some(100001),
            ..Default::default()
        },
        HistoryQuery {
            page_size: Some(0),
            ..Default::default()
        },
        HistoryQuery {
            page_size: Some(101),
            ..Default::default()
        },
        HistoryQuery {
            start_timestamp: Some(-1),
            ..Default::default()
        },
        HistoryQuery {
            start_timestamp: Some(10),
            end_timestamp: Some(9),
            ..Default::default()
        },
        HistoryQuery {
            end_timestamp: Some(253402300800),
            ..Default::default()
        },
        HistoryQuery {
            r#type: Some(8),
            ..Default::default()
        },
        HistoryQuery {
            channel: Some(-1),
            ..Default::default()
        },
        HistoryQuery {
            model_name: Some("x".repeat(257)),
            ..Default::default()
        },
        HistoryQuery {
            model_name: Some("bad\nmodel".into()),
            ..Default::default()
        },
    ];
    for query in invalid {
        assert_eq!(
            f.store
                .read(&f.user, ReadRoute::SelfSession, &query)
                .await
                .unwrap_err(),
            HistoryError::Invalid
        );
    }
    for query in [
        HistoryQuery {
            username: Some("other".into()),
            ..Default::default()
        },
        HistoryQuery {
            channel: Some(8),
            ..Default::default()
        },
        HistoryQuery {
            upstream_request_id: Some("upstream".into()),
            ..Default::default()
        },
    ] {
        assert_eq!(
            f.store
                .read(&f.user, ReadRoute::SelfSession, &query)
                .await
                .unwrap_err(),
            HistoryError::Forbidden
        );
    }
    assert_eq!(before, fingerprint(&pool).await);
}

#[sqlx::test(migrations = false)]
async fn independent_real_keys_cannot_cross_credential_scope_or_enter_session_admin_routes(
    pool: PgPool,
) {
    use lmm_core::{
        accounts::{Account, AccountKind},
        funding::FundingPolicy,
    };
    let f = fixture(&pool).await;
    let personal = Account {
        kind: AccountKind::Personal,
        id: 9101,
    };
    let key = f
        .identity
        .issue_key(&f.user, personal, FundingPolicy::new_personal_key(), 3600)
        .await
        .unwrap();
    let second = f
        .identity
        .issue_key(&f.user, personal, FundingPolicy::new_personal_key(), 3600)
        .await
        .unwrap();
    let admin_key = f
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
    row(&pool, 1, 9101, f.owner, Some(1), Some(3), "selected").await;
    row(&pool, 2, 9101, f.owner, Some(100), Some(1), "selected").await;
    sqlx::query("UPDATE core_history.log_records SET token_id=CASE source_log_id WHEN 1 THEN $1 ELSE $2 END").bind(key.id).bind(second.id).execute(&pool).await.unwrap();
    history_scope::install(&pool).await;
    let before = fingerprint(&pool).await;
    let selected = f
        .store
        .read(&key.secret, ReadRoute::ReadOnlyKey, &Default::default())
        .await
        .unwrap();
    assert_eq!(selected.total, 1);
    assert_eq!(selected.items[0].source_log_id, 1);
    assert_eq!(selected.summary.raw_consumption_quota, "1");
    assert_eq!(
        selected.summary.money.quota_usd.as_deref(),
        Some("0.333333333333333333")
    );
    assert_eq!(
        f.store
            .read(&key.secret, ReadRoute::SelfSession, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    assert_eq!(
        f.store
            .read(&f.user, ReadRoute::ReadOnlyKey, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    assert_eq!(
        f.store
            .read(
                &admin_key.secret,
                ReadRoute::Administrator,
                &Default::default()
            )
            .await
            .unwrap_err(),
        HistoryError::Forbidden
    );
    let permissions = PermissionReadStore::new(f.identity.clone());
    assert!(matches!(
        permissions.catalog(&admin_key.secret).await,
        Err(lmm_core::identity::admin_users::AdminUsersError::Forbidden)
    ));
    assert_eq!(before, fingerprint(&pool).await);
}

#[sqlx::test(migrations = false)]
async fn independent_unknown_raw_quota_and_lossless_numbers(pool: PgPool) {
    let f = fixture(&pool).await;
    for (id, q, k) in [
        (1, Some(3), Some(3)),
        (2, None, Some(3)),
        (3, Some(-1), Some(3)),
        (4, Some(5), None),
    ] {
        row(&pool, id, 9101, f.owner, q, k, "chosen").await;
    }
    let result = f
        .store
        .read(&f.user, ReadRoute::SelfSession, &Default::default())
        .await
        .unwrap();
    assert_eq!(result.summary.raw_consumption_quota, "7");
    assert_eq!(result.summary.invalid_quota_records, 1);
    assert_eq!(result.summary.unpriced_quota_records, 3);
    assert!(!result.summary.money.quota_money_available);
    assert!(result.summary.money.quota_usd.is_none());
    row(
        &pool,
        5,
        9101,
        f.owner,
        Some(9_007_199_254_740_992),
        Some(1),
        "unsafe",
    )
    .await;
    assert_eq!(
        f.store
            .read(&f.user, ReadRoute::SelfSession, &Default::default())
            .await
            .unwrap_err(),
        HistoryError::Unavailable
    );
}

#[sqlx::test(migrations = false)]
async fn independent_export_allowlist_unknown_and_csv_formula_encoding(pool: PgPool) {
    let f = fixture(&pool).await;
    row(&pool, 1, 9101, f.owner, Some(11), None, "=SUM(1,2)").await;
    sqlx::query("UPDATE core_history.log_records SET username=E' \\t@formula',token_name='a\"b,c',request_id='正常中文'").execute(&pool).await.unwrap();
    let before = fingerprint(&pool).await;
    let document =
        finance_export::export(&f.store, &f.admin, &Default::default(), ExportFormat::Json)
            .await
            .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&document.bytes).unwrap();
    assert_eq!(value["complete_financial_export"], false);
    assert_eq!(
        value["included_families"],
        serde_json::json!(["historical_usage"])
    );
    assert_eq!(value["usage"][0]["quota"], 11);
    assert_eq!(value["usage"][0]["quota_money_available"], false);
    assert!(value["usage"][0].get("quota_usd").is_none());
    let allowed: BTreeSet<&str> = [
        "source_log_id",
        "user_id",
        "token_id",
        "created_at",
        "type",
        "username",
        "token_name",
        "model_name",
        "quota",
        "prompt_tokens",
        "completion_tokens",
        "group",
        "request_id",
        "channel",
        "upstream_request_id",
        "quota_usd",
        "quota_money_available",
        "quota_money_basis",
    ]
    .into_iter()
    .collect();
    for key in value["usage"][0].as_object().unwrap().keys() {
        assert!(
            allowed.contains(key.as_str()),
            "unexpected public field {key}"
        );
    }
    let csv = finance_export::export(&f.store, &f.admin, &Default::default(), ExportFormat::Csv)
        .await
        .unwrap();
    let csv = String::from_utf8(csv.bytes).unwrap();
    assert!(csv.contains("\"'=SUM(1,2)\""));
    assert!(csv.contains("\"' \t@formula\""));
    assert!(csv.contains("\"a\"\"b,c\""));
    assert!(csv.contains("正常中文"));
    assert!(csv.ends_with("\r\n"));
    assert!(!csv.contains("source_archive_sha256"));
    assert!(
        finance_export::export(&f.store, &f.user, &Default::default(), ExportFormat::Json)
            .await
            .is_err()
    );
    assert_eq!(
        finance_export::export(
            &f.store,
            &f.admin,
            &HistoryQuery {
                p: Some(2),
                ..Default::default()
            },
            ExportFormat::Json
        )
        .await
        .err(),
        Some(HistoryError::Invalid)
    );
    assert_eq!(before, fingerprint(&pool).await);
}

#[sqlx::test(migrations = false)]
async fn independent_concurrent_ownership_and_amount_changes_never_split_snapshot(pool: PgPool) {
    let f = fixture(&pool).await;
    row(&pool, 1, 9101, f.owner, Some(3), Some(3), "selected").await;
    let writer_pool = pool.clone();
    let (owner, other) = (f.owner, f.other);
    let writer = tokio::spawn(async move {
        for n in 0..80 {
            let (user, account, quota) = if n % 2 == 0 {
                (9102, other, 300)
            } else {
                (9101, owner, 3)
            };
            sqlx::query(
                "UPDATE core_history.log_records SET user_id=$1,owner_account_id=$2,quota=$3",
            )
            .bind(user)
            .bind(account)
            .bind(quota)
            .execute(&writer_pool)
            .await
            .unwrap();
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..80 {
        let result = f
            .store
            .read(&f.user, ReadRoute::SelfSession, &Default::default())
            .await
            .unwrap();
        match result.total {
            0 => {
                assert!(result.items.is_empty());
                assert_eq!(result.summary.raw_consumption_quota, "0");
                assert_eq!(result.summary.money.quota_usd.as_deref(), Some("0"));
            }
            1 => {
                assert_eq!(result.items.len(), 1);
                assert_eq!(result.items[0].user_id, 9101);
                assert_eq!(result.items[0].quota, Some(3));
                assert_eq!(result.summary.raw_consumption_quota, "3");
                assert_eq!(result.summary.money.quota_usd.as_deref(), Some("1"));
            }
            total => panic!("impossible total {total}"),
        }
    }
    writer.await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn independent_record_basis_and_total_export_byte_bounds_are_hard(pool: PgPool) {
    let f = fixture(&pool).await;
    for id in 1..=64 {
        row(&pool, id, 9101, f.owner, Some(1), Some(id), "selected").await;
    }
    let allowed = f
        .store
        .read(&f.user, ReadRoute::SelfSession, &Default::default())
        .await
        .unwrap();
    assert!(allowed.summary.money.quota_money_available);
    assert!(!allowed.summary.quota_projection_truncated);
    row(&pool, 65, 9101, f.owner, Some(1), Some(65), "selected").await;
    let bounded = f
        .store
        .read(&f.user, ReadRoute::SelfSession, &Default::default())
        .await
        .unwrap();
    assert_eq!(bounded.summary.money.quota_money_basis, "projection_limit");
    assert!(bounded.summary.money.quota_usd.is_none());
    assert!(bounded.summary.quota_projection_truncated);
    sqlx::query("DELETE FROM core_history.log_records")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,pricing_credits_per_usd) SELECT 1,n,'independent-dense-'||n,repeat('d',64),9101,$1,1700000000,2,1,3 FROM generate_series(1,10000) n").bind(f.owner).execute(&pool).await.unwrap();
    let full = f
        .store
        .read(&f.user, ReadRoute::SelfSession, &Default::default())
        .await
        .unwrap();
    assert_eq!(full.total, 10000);
    assert_eq!(
        full.summary.money.quota_usd.as_deref(),
        Some("3333.333333333333333333")
    );
    assert!(!full.summary.quota_projection_truncated);
    assert!(
        finance_export::export(&f.store, &f.admin, &Default::default(), ExportFormat::Json)
            .await
            .is_ok()
    );
    row(&pool, 10001, 9101, f.owner, Some(1), Some(3), "selected").await;
    let over = f
        .store
        .read(&f.user, ReadRoute::SelfSession, &Default::default())
        .await
        .unwrap();
    assert_eq!(over.total, 10001);
    assert!(over.summary.quota_projection_truncated);
    assert!(over.summary.money.quota_usd.is_none());
    assert_eq!(
        finance_export::export(&f.store, &f.admin, &Default::default(), ExportFormat::Json)
            .await
            .err(),
        Some(HistoryError::TooDense)
    );
    sqlx::query("DELETE FROM core_history.log_records WHERE source_log_id=10001")
        .execute(&pool)
        .await
        .unwrap();
    // Valid persisted bytes can expand sixfold when escaped into JSON. This
    // checks the complete document bound through the public export operation.
    sqlx::query("UPDATE core_history.log_records SET username=repeat(chr(1),256),token_name=repeat(chr(1),256),model_name=repeat(chr(1),256),group_name=repeat(chr(1),256),request_id=repeat(chr(1),256),upstream_request_id=repeat(chr(1),256)").execute(&pool).await.unwrap();
    let before = fingerprint(&pool).await;
    assert_eq!(
        finance_export::export(&f.store, &f.admin, &Default::default(), ExportFormat::Json)
            .await
            .err(),
        Some(HistoryError::TooDense)
    );
    assert_eq!(before, fingerprint(&pool).await);
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
