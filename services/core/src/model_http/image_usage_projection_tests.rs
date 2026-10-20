//! Real drawing/native image charges through the actual usage HTTP projection.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request as HttpRequest, StatusCode},
};
use tower::ServiceExt;

async fn usage(f: &BrowserFixture, path: &str) -> (StatusCode, Value) {
    let app = crate::usage::router(crate::usage::UsageStore::new(f.pool.clone()));
    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri(path)
                .header("Authorization", format!("Bearer {}", f.secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let code = response.status();
    let value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    (code, value)
}
async fn complete_drawing(f: &BrowserFixture, service: &ImageService) -> Receipt {
    let raw = drawing_input();
    let authority = builtin(f, &raw).await;
    let preview = service
        .preview(&f.secret, &authority.context, raw)
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
    service
        .execute_for_context(&f.secret, &preview.operation_id, &authority.context)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn image_usage_mixed_drawing_native_images_and_text_preserve_real_receipts(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let drawing = complete_drawing(&f, &service).await;
    assert_eq!(drawing.state, "completed");
    let key = f
        .identity
        .issue_key(
            &f.secret,
            crate::accounts::Account {
                kind: crate::accounts::AccountKind::Personal,
                id: f.user,
            },
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'usage-image-key','allowed')").bind(key.id).execute(&f.pool).await.unwrap();
    let native = service
        .execute_api(
            &key.secret,
            &"a".repeat(64),
            &["allowed".into()],
            input(1),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(native.state, "completed");
    let worker = [13; 32];
    let request = crate::billing::charging::Request {
        id: "usage-mixed-text".into(),
        fingerprint: [14; 32],
        reserve: 100,
        lease_seconds: 60,
        maximum_seconds: 120,
        price: Price {
            image: None,
            tiered: None,
            upstream_cost_group_ppm: None,
            version: "usage-image-text-v1".into(),
            model: "local-model".into(),
            group: "allowed".into(),
            input_per_million: 1_000_000,
            cached_per_million: 0,
            output_per_million: 1_000_000,
        },
    };
    service
        .charging
        .reserve(&key.secret, &request, &worker)
        .await
        .unwrap();
    service.charging.start(&request.id, &worker).await.unwrap();
    let text_usage = Usage {
        input: 2,
        cached: 0,
        output: 3,
        ..Default::default()
    };
    service
        .charging
        .settle(&request.id, &worker, text_usage, Outcome::Completed)
        .await
        .unwrap();
    let before = f.ledger.balance(f.account).await.unwrap();
    let (code, logs) = usage(&f, "/api/log/self?p=1&page_size=100").await;
    assert_eq!(code, StatusCode::OK, "{logs}");
    assert_eq!(logs["data"]["total"], 3);
    let rows = logs["data"]["items"].as_array().unwrap();
    for id in [&drawing.operation_id, &native.operation_id] {
        let row = rows
            .iter()
            .find(|r| r["request_id"] == id.as_str())
            .unwrap();
        assert_eq!(row["quota"], 2000);
        assert_eq!(row["pricing_dimension"], "image");
        assert_eq!(row["output_image_count"], 1);
        assert_eq!(row["token_usage_available"], false);
        assert!(row.get("prompt_tokens").is_none());
        assert!(row.get("completion_tokens").is_none());
        assert!(!row.to_string().contains("provider_receipt_sha256"));
        let other: Value = serde_json::from_str(row["other"].as_str().unwrap()).unwrap();
        assert_eq!(
            other["credential_source"],
            if id == &drawing.operation_id {
                "drawing_session"
            } else {
                "api_key"
            }
        );
    }
    let text = rows.iter().find(|r| r["request_id"] == request.id).unwrap();
    assert_eq!(text["quota"], 5);
    assert_eq!(text["prompt_tokens"], 2);
    let (code, stats) = usage(&f, "/api/log/self/stat").await;
    assert_eq!(code, StatusCode::OK, "{stats}");
    assert_eq!(stats["data"]["net_consumption_credits"], 4005);
    assert_eq!(stats["data"]["output_image_count"], 2);
    assert_eq!(stats["data"]["reserved_credits"], 0);
    let (code, buckets) = usage(&f, "/api/data/self").await;
    assert_eq!(code, StatusCode::OK, "{buckets}");
    let image_bucket = buckets["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["model_name"] == "dall-e-3")
        .unwrap();
    assert!(image_bucket["token_used"].is_null());
    assert_eq!(image_bucket["output_image_count"], 2);
    assert_eq!(before, f.ledger.balance(f.account).await.unwrap());
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 2);
}

#[sqlx::test(migrations = false)]
async fn image_usage_unknown_is_nullable_consumption_with_original_hold_no_redispatch(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::Truncated).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let raw = drawing_input();
    let authority = builtin(&f, &raw).await;
    let preview = service
        .preview(&f.secret, &authority.context, raw)
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
    assert_eq!(
        service
            .execute_for_context(&f.secret, &preview.operation_id, &authority.context)
            .await
            .unwrap_err(),
        Error::Pending
    );
    let receipt = service
        .query_for_context(&f.secret, &preview.operation_id, &authority.context)
        .await
        .unwrap();
    assert_ne!(receipt.state, "completed");
    assert!(receipt.charged_credits.is_none());
    let before = f.ledger.balance(f.account).await.unwrap();
    let (code, logs) = usage(&f, "/api/log/self").await;
    assert_eq!(code, StatusCode::OK, "{logs}");
    assert_eq!(logs["data"]["total"], 1);
    let row = &logs["data"]["items"][0];
    assert!(row["quota"].is_null());
    assert!(row["net_consumption_credits"].is_null());
    assert!(row["output_image_count"].is_null());
    assert_eq!(row["quota_money_available"], false);
    let other: Value = serde_json::from_str(row["other"].as_str().unwrap()).unwrap();
    assert_eq!(other["reserved_credits"], 6000);
    assert_eq!(other["usage_status"], "incomplete");
    let (code, stats) = usage(&f, "/api/log/self/stat").await;
    assert_eq!(code, StatusCode::OK, "{stats}");
    assert_eq!(stats["data"]["reserved_credits"], 6000);
    assert_eq!(stats["data"]["incomplete_count"], 1);
    assert_eq!(stats["data"]["gross_settled_credits"], 0);
    assert_eq!(before, f.ledger.balance(f.account).await.unwrap());
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test(migrations = false)]
async fn image_usage_corrupt_final_image_evidence_fails_closed_not_empty_history(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let receipt = complete_drawing(&f, &service).await;
    assert_eq!(receipt.state, "completed");
    assert_eq!(usage(&f, "/api/log/self").await.0, StatusCode::OK);
    let mut tx = f.pool.begin().await.unwrap();
    // Installation-owner-only corruption in this disposable fixture. The public
    // reader must compare persisted image usage with its immutable final event.
    sqlx::query("SET LOCAL session_replication_role=replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_charging.usage_events SET detail=jsonb_set(detail,'{usage,image,provider_receipt_sha256}',to_jsonb(array_fill(9,ARRAY[32]))) WHERE request_id=$1 AND event_id='final'").bind(&receipt.operation_id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    for path in ["/api/log/self", "/api/log/self/stat", "/api/data/self"] {
        let (code, value) = usage(&f, path).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{value}");
        assert_eq!(value["code"], "USAGE_UNAVAILABLE");
    }
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
}
