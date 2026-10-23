//! Pure export contracts plus real, isolated PostgreSQL scope/receipt tests.
use super::*;
use crate::{
    accounts::{Account, AccountKind},
    funding::FundingPolicy,
    identity::IdentityStore,
};
use axum::{body::to_bytes, http::Request as HttpRequest};
use std::sync::atomic::{AtomicI64, Ordering};
use tower::ServiceExt;

#[test]
fn csv_quotes_delimiters_and_preserves_unknown_and_real_zero() {
    let mut csv = Csv::new();
    csv.row(&[
        text("a,b"),
        text("line\n\"next\""),
        optional::<i64>(None),
        optional(Some(0)),
    ])
    .unwrap();
    assert_eq!(
        String::from_utf8(csv.bytes).unwrap(),
        "\u{feff}\"a,b\",\"line\n\"\"next\"\"\",\"\",\"0\"\r\n"
    );
    for dangerous in [
        "=1+1",
        "+SUM(A1)",
        " -1+2",
        "\t@SUM(A1)",
        "\r=cmd",
        "\u{feff}=cmd",
        "\u{a0}+cmd",
    ] {
        assert_eq!(text(dangerous), format!("'{dangerous}"));
    }
    assert_eq!(text("正常中文"), "正常中文");
}

#[test]
fn explicit_bounded_window_and_hard_byte_limit() {
    assert!(window(Some(0), Some(WINDOW)).is_ok());
    for (start, end) in [
        (None, Some(1)),
        (Some(0), None),
        (Some(-1), Some(0)),
        (Some(2), Some(1)),
        (Some(0), Some(WINDOW + 1)),
        (Some(i64::MAX), Some(i64::MAX)),
    ] {
        assert_eq!(window(start, end), Err(ExportError::Invalid));
    }
    let mut csv = Csv {
        bytes: vec![0; 7],
        limit: 8,
    };
    csv.append(b"a").unwrap();
    assert_eq!(csv.append(b"b"), Err(ExportError::TooDense));
    assert_eq!(csv.bytes.len(), 8);
}

#[test]
fn search_export_keeps_unknown_distinct_from_zero_and_real_measurements() {
    use crate::routing::{SearchProvider, SearchUsage};
    assert_eq!(search_cells(None).unwrap(), ["", "", "", "", "", ""]);
    let usage = SearchUsage {
        provider: SearchProvider::XaiResponsesV1,
        calls: Some(0),
        queries: None,
        grounded_requests: None,
        x_posts_fetched: Some(2),
        x_users_fetched: Some(0),
    };
    assert_eq!(
        search_cells(Some(usage)).unwrap(),
        ["xai_responses_v1", "0", "", "", "2", "0"]
    );
}

#[tokio::test]
async fn user_and_global_limits_survive_until_body_drop_and_release_on_cancellation() {
    let gate = Arc::new(ExportGate::new(2));
    let first = gate.acquire(1).unwrap();
    assert!(matches!(gate.acquire(1), Err(ExportError::Busy)));
    let second = gate.acquire(2).unwrap();
    assert!(matches!(gate.acquire(3), Err(ExportError::Busy)));
    let mut stream = body(vec![0; CHUNK_BYTES * 4], first, Duration::from_secs(1));
    assert_eq!(stream.next().await.unwrap().unwrap().len(), CHUNK_BYTES);
    assert!(matches!(gate.acquire(1), Err(ExportError::Busy)));
    drop(stream); // Browser drops the response before its last chunk.
    let released = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(lease) = gate.acquire(1) {
                return lease;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(released);
    drop(second);
    let lease = gate.acquire(3).unwrap();
    let task = tokio::spawn(async move {
        let _lease = lease;
        std::future::pending::<()>().await;
    });
    task.abort();
    let _ = task.await;
    assert!(gate.acquire(3).is_ok());
}

#[tokio::test]
async fn stalled_transfer_releases_admission_and_cannot_end_as_a_successful_partial_csv() {
    let gate = Arc::new(ExportGate::new(1));
    let stream = body(
        vec![0; CHUNK_BYTES * 3],
        gate.acquire(1).unwrap(),
        Duration::from_millis(5),
    );
    let released = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(lease) = gate.acquire(1) {
                return lease;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(released);
    assert!(
        to_bytes(Body::from_stream(stream), MAX_BYTES)
            .await
            .is_err()
    );
}

struct Fixture {
    user: i64,
    owner: i64,
    key: i64,
    session: String,
    key_secret: String,
    at: i64,
}
async fn identity(pool: &PgPool, level: i16) -> Fixture {
    static NEXT_USER: AtomicI64 = AtomicI64::new(1000);
    let user = NEXT_USER.fetch_add(1, Ordering::Relaxed);
    let store = IdentityStore::from_pool(pool.clone());
    let session = store.bootstrap_user(user, level).await.unwrap();
    let owner = store
        .authorize(&session.secret)
        .await
        .unwrap()
        .owner_account_id;
    let key = store
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: user,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let at = sqlx::query_scalar("SELECT floor(extract(epoch FROM now()))::bigint")
        .fetch_one(pool)
        .await
        .unwrap();
    Fixture {
        user,
        owner,
        key: key.id,
        session: session.secret,
        key_secret: key.secret,
        at,
    }
}
async fn pending(pool: &PgPool, fixture: &Fixture, prefix: &str, count: i64, model: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let price = json!({"version":"csv-fixture", "model":model, "group":"default", "input_per_million":0, "cached_per_million":0, "output_per_million":0});
    sqlx::query(r#"INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved)
      SELECT $1||lpad(g::text,6,'0'),$2,$3,$4,$4,decode(repeat('03',32),'hex'),decode(repeat('03',32),'hex'),decode(repeat('03',32),'hex'),$5,$6,$6+300,$6+600,'reserved',1 FROM generate_series(1,$7::bigint) g"#)
        .bind(prefix).bind(fixture.user).bind(fixture.key).bind(fixture.owner).bind(price).bind(fixture.at).bind(count).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}
async fn request(
    pool: &PgPool,
    path: &str,
    secret: Option<&str>,
    at: i64,
    suffix: &str,
) -> Response {
    let mut request = HttpRequest::builder().uri(format!(
        "{path}?start_timestamp={}&end_timestamp={at}{suffix}",
        at - 60
    ));
    if let Some(secret) = secret {
        request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    router(UsageStore::new(pool.clone()))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn self_csv_exports_all_filtered_rows_and_never_another_actor_or_key(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let own = identity(&pool, 1).await;
    let other = identity(&pool, 1).await;
    pending(&pool, &own, "own-", 121, "chosen").await;
    pending(&pool, &own, "excluded-", 1, "other-model").await;
    pending(&pool, &other, "another-user-", 1, "chosen").await;
    let response = request(
        &pool,
        "/api/log/self/export",
        Some(&own.session),
        own.at,
        "&model_name=chosen&group=default",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-export-rows"], "121");
    assert_eq!(
        response.headers()["x-export-source"],
        "native_personal_usage"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let expected_bytes: usize = response.headers()["x-export-bytes"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let body = to_bytes(response.into_body(), MAX_BYTES).await.unwrap();
    assert_eq!(body.len(), expected_bytes);
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(body.matches("\r\n").count(), 122); // Header + all rows, beyond page size 100.
    assert!(body.contains("own-000001") && body.contains("own-000121"));
    assert!(!body.contains("another-user-") && !body.contains("excluded-"));
    assert!(!body.contains("fingerprint") && !body.contains("worker_digest"));
    for (secret, suffix, expected) in [
        (None, "", StatusCode::UNAUTHORIZED),
        (Some(own.key_secret.as_str()), "", StatusCode::FORBIDDEN),
        (Some(own.session.as_str()), "&p=2", StatusCode::BAD_REQUEST),
        (
            Some(own.session.as_str()),
            "&page_size=20",
            StatusCode::BAD_REQUEST,
        ),
        (
            Some(own.session.as_str()),
            "&username=another",
            StatusCode::BAD_REQUEST,
        ),
        (
            Some(own.session.as_str()),
            "&channel=1",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            request(&pool, "/api/log/self/export", secret, own.at, suffix)
                .await
                .status(),
            expected
        );
    }
}

#[sqlx::test(migrations = false)]
async fn corrupt_receipts_and_excess_density_return_errors_without_csv(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let own = identity(&pool, 1).await;
    pending(&pool, &own, "dense-", MAX_ROWS as i64 + 1, "dense").await;
    let response = request(
        &pool,
        "/api/log/self/export",
        Some(&own.session),
        own.at,
        "&model_name=dense",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["code"], "USAGE_EXPORT_TOO_DENSE");
    pending(&pool, &own, "bad-", 1, "bad").await;
    sqlx::query("UPDATE core_charging.requests SET state='streaming',revision=revision+1 WHERE id='bad-000001'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_charging.requests SET state='settled',outcome='completed',settled=0,revision=revision+1 WHERE id='bad-000001'").execute(&pool).await.unwrap();
    let response = request(
        &pool,
        "/api/log/self/export",
        Some(&own.session),
        own.at,
        "&model_name=bad",
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
}

#[sqlx::test(migrations = false)]
async fn administrator_csv_uses_its_existing_historical_authority_and_filters(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let user = identity(&pool, 1).await;
    let admin = identity(&pool, 5).await;
    assert_eq!(
        request(&pool, "/api/log/export", Some(&user.session), user.at, "")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &pool,
            "/api/log/export",
            Some(&admin.key_secret),
            admin.at,
            ""
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&pool, "/api/log/export", Some(&admin.session), admin.at, "")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    sqlx::query("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'csv-test-only',1700000000,true)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,quota,pricing_credits_per_usd,username,model_name,token_name,channel_id) VALUES(1,1,'csv-test-1',repeat('c',64),$1,$2,$3,2,0,500000,'chosen-user','chosen-model','=SUM(A1)',9),(1,2,'csv-test-2',repeat('d',64),$1,$2,$3,2,7,NULL,'other-user','chosen-model','normal',10)")
        .bind(user.user).bind(user.owner).bind(admin.at).execute(&pool).await.unwrap();
    let response = request(
        &pool,
        "/api/log/export",
        Some(&admin.session),
        admin.at,
        "&username=chosen-user&channel=9&model_name=chosen-model",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-export-source"], "legacy_log_only");
    assert_eq!(response.headers()["x-export-rows"], "1");
    let body = String::from_utf8(
        to_bytes(response.into_body(), MAX_BYTES)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("chosen-user") && body.contains("\"'=SUM(A1)\""));
    assert!(!body.contains("other-user"));
    assert!(body.contains("\"0\",\"true\",\"captured_credits_per_usd\""));
}
