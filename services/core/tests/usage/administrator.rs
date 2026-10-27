use super::*;

#[sqlx::test(migrations = false)]
async fn administrator_data_aggregates_real_users_accounts_and_refunds(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = f.identity.bootstrap_user(9, 5).await.unwrap();
    let other = f.identity.bootstrap_user(8, 0).await.unwrap();
    let other_account = f
        .identity
        .authorize(&other.secret)
        .await
        .unwrap()
        .owner_account_id;
    let other_key = f
        .identity
        .issue_key(
            &other.secret,
            Account {
                kind: AccountKind::Personal,
                id: 8,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    credit(&f.ledger, other_account, "other-credit").await;
    settled(&f, &f.key.secret, "personal", Outcome::Completed).await;
    f.charging
        .refund("personal", "partial-refund", 20, "private test refund")
        .await
        .unwrap();
    settled(&f, &other_key.secret, "other-user", Outcome::Failed).await;
    let team = f.identity.create_team(&f.session.secret).await.unwrap();
    let team_key = f
        .identity
        .issue_key(
            &f.session.secret,
            Account {
                kind: AccountKind::Team,
                id: team,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    let team_account = f
        .identity
        .authorize(&team_key.secret)
        .await
        .unwrap()
        .owner_account_id;
    credit(&f.ledger, team_account, "team-credit").await;
    settled(&f, &team_key.secret, "team-owned", Outcome::Completed).await;
    f.charging
        .reserve(&f.key.secret, &spec("still-reserved"), &WORKER)
        .await
        .unwrap();

    // A reviewed archive row stays historical; the native endpoint must not
    // reinterpret its raw quota using today's Core credit scale.
    let source: i64 = sqlx::query_scalar(
        "INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete)
         VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'private-test',1,true) RETURNING source_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,model_name)
         VALUES($1,1,'private-history-event',repeat('c',64),7,$2,floor(extract(epoch FROM clock_timestamp()))::bigint,2,9000000,'test-model')",
    )
    .bind(source)
    .bind(f.account)
    .execute(&pool)
    .await
    .unwrap();

    let (status, response) = get(&f.app, "/api/data", Some(&admin.secret)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["success"], true);
    assert_eq!(response["availability"], "partial");
    assert_eq!(response["coverage"]["native_requests"], true);
    assert_eq!(response["coverage"]["historical_imports"], false);
    assert_eq!(response["coverage"]["complete"], false);
    let rows = response["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .map(|r| r["count"].as_i64().unwrap())
            .sum::<i64>(),
        3
    );
    assert_eq!(
        rows.iter()
            .map(|r| r["token_used"].as_i64().unwrap())
            .sum::<i64>(),
        90
    );
    assert_eq!(
        rows.iter()
            .map(|r| r["quota"].as_i64().unwrap())
            .sum::<i64>(),
        127
    );
    for row in rows {
        assert_eq!(row["usage_scope"], "administrator_all_accounts");
        assert_eq!(row["gross_settled_credits"], 49);
        assert_eq!(row["quota_money_available"], true);
        assert_eq!(row["quota_money_basis"], "captured_credits_per_usd");
        assert!(row.get("request_id").is_none());
    }
    let personal = rows
        .iter()
        .find(|row| row["user_id"] == 7 && row["payer_kind"] == "personal")
        .unwrap();
    assert_eq!(personal["refunded_credits"], 20);
    assert_eq!(personal["quota"], 29);
    assert_eq!(personal["quota_usd"], "0.000058");
    assert!(
        rows.iter()
            .any(|row| row["user_id"] == 8 && row["quota"] == 49)
    );
    assert!(
        rows.iter()
            .any(|row| row["user_id"] == 7 && row["payer_kind"] == "team")
    );

    let own = ok(&f, "/api/data/self").await;
    assert_eq!(own.as_array().unwrap().len(), 1);
    assert_eq!(own[0]["quota"], 29);
    assert_eq!(own[0]["usage_scope"], "actor_personal_key");

    let username: String =
        sqlx::query_scalar("SELECT username FROM core_identity.user_profiles WHERE user_id=8")
            .fetch_one(&pool)
            .await
            .unwrap();
    let response = get(
        &f.app,
        &format!("/api/data/?username={username}"),
        Some(&admin.secret),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(response.1["data"].as_array().unwrap().len(), 1);
    assert_eq!(response.1["data"][0]["user_id"], 8);
    let response = get(
        &f.app,
        "/api/data?username=nonexistent-user",
        Some(&admin.secret),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"], json!([]));
    assert_eq!(response.1["coverage"]["complete"], false);
}

#[sqlx::test(migrations = false)]
async fn administrator_data_enforces_current_platform_role_and_session(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = f.identity.bootstrap_user(9, 5).await.unwrap();
    let high_service_level_user = f.identity.bootstrap_user(10, 4).await.unwrap();
    let key = f
        .identity
        .issue_key(
            &admin.secret,
            Account {
                kind: AccountKind::Personal,
                id: 9,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        get(&f.app, "/api/data", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    for secret in [
        &f.session.secret,
        &high_service_level_user.secret,
        &key.secret,
    ] {
        let (status, response) = get(&f.app, "/api/data", Some(secret)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        assert!(response.get("data").is_none());
    }
    assert_eq!(
        get(&f.app, "/api/data", Some(&admin.secret)).await.0,
        StatusCode::OK
    );
    let version_before: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=9")
            .fetch_one(&pool)
            .await
            .unwrap();
    let version_after: i64 = sqlx::query_scalar(
        "UPDATE core_identity.users SET platform_role='user' WHERE id=9 RETURNING auth_version",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // Role changes revoke existing sessions through the identity generation.
    assert_eq!(version_after, version_before + 1);
    let (status, response) = get(&f.app, "/api/data", Some(&admin.secret)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{response}");
    assert_eq!(response["code"], "AUTH_UNAUTHORIZED");
    assert!(response.get("data").is_none());
}

#[sqlx::test(migrations = false)]
async fn administrator_data_refuses_incomplete_receipts_instead_of_reporting_zero(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = f.identity.bootstrap_user(9, 5).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved,outcome)
        VALUES('missing-receipt',7,$1,$2,$2,$3,$3,$3,$4,floor(extract(epoch FROM clock_timestamp()))::bigint,253402300798,253402300799,'settled',0,'completed')")
        .bind(f.key.id).bind(f.account).bind(vec![3u8;32])
        .bind(serde_json::to_value(spec("unused").price).unwrap())
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (status, response) = get(&f.app, "/api/data", Some(&admin.secret)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response["code"], "USAGE_UNAVAILABLE");
    assert!(response.get("data").is_none());
    let username: String =
        sqlx::query_scalar("SELECT username FROM core_identity.user_profiles WHERE user_id=9")
            .fetch_one(&pool)
            .await
            .unwrap();
    let (status, response) = get(
        &f.app,
        &format!("/api/data?username={username}"),
        Some(&admin.secret),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["data"], json!([]));
}
