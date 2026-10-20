//! Actual Session/market grant/browser confirmation and multipart supplier call.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
fn upload() -> DrawingUpload {
    let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==";
    let bytes = STANDARD.decode(data).unwrap();
    DrawingUpload {
        media_type: "image/png".into(),
        sha256: hex(&Sha256::digest(&bytes)),
        byte_length: bytes.len() as u64,
        data_base64: data.into(),
    }
}
fn arguments(image: &DrawingUpload) -> Value {
    let mut raw = drawing_input();
    raw["n"] = json!(1);
    raw["reference_images"] = json!([{"media_type":image.media_type,"sha256":image.sha256,"byte_length":image.byte_length}]);
    raw
}
async fn reference_builtin(f: &BrowserFixture, raw: &Value) -> BuiltinFixture {
    builtin_with_session(
        &f.pool,
        &f.secret,
        "drawing-tool",
        "ca3a1996-723d-5695-b29e-40e3bef58644",
        "drawing.generate",
        "abd718715e097d729e362afd0f5cdf19ae664886a878902e1c38ac31a2c2964e",
        &crate::builtins::input_digest(raw).unwrap(),
    )
    .await
}
#[sqlx::test(migrations = false)]
async fn image_drawing_reference_edit_freezes_bytes_confirms_price_and_dispatches_once(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let image = upload();
    let raw = arguments(&image);
    let builtin = reference_builtin(&f, &raw).await;
    assert!(matches!(
        service
            .preview(&f.secret, &builtin.context, raw.clone())
            .await,
        Err(Error::Invalid)
    ));
    let preview = service
        .preview_with_uploads(
            &f.secret,
            &builtin.context,
            raw.clone(),
            vec![image.clone()],
        )
        .await
        .unwrap();
    assert_eq!(preview.reservation_credits, 2000);
    assert_eq!(preview.input.reference_images.len(), 1);
    assert!(
        !serde_json::to_string(&preview)
            .unwrap()
            .contains(&image.data_base64)
    );
    let persisted: Value =
        sqlx::query_scalar("SELECT input FROM core_image.operations WHERE id=$1")
            .bind(&preview.operation_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(persisted["uploads"][0]["data_base64"], image.data_base64);
    assert_eq!(persisted["arguments"], raw);
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let (_, csrf) = service
        .confirmation_page(&f.bundle.refresh_token, &preview.operation_id)
        .await
        .unwrap();
    service
        .confirm(&f.bundle.refresh_token, &preview.operation_id, &csrf)
        .await
        .unwrap();
    let receipt = service
        .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.charged_credits, Some(2000));
    assert!(
        !serde_json::to_string(&receipt)
            .unwrap()
            .contains(&image.data_base64)
    );
    let replay = service
        .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&receipt).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
    let endpoint: String = sqlx::query_scalar(
        "SELECT price#>>'{image,request,endpoint}' FROM core_image.operations WHERE id=$1",
    )
    .bind(&preview.operation_id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(endpoint, "edits");
}
#[sqlx::test(migrations = false)]
async fn image_drawing_reference_mismatch_rejected_before_preview_hold_or_send(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let mut image = upload();
    let raw = arguments(&image);
    let builtin = reference_builtin(&f, &raw).await;
    image.sha256 = "0".repeat(64);
    assert!(matches!(
        service
            .preview_with_uploads(&f.secret, &builtin.context, raw, vec![image])
            .await,
        Err(Error::Invalid)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_image.operations")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
}
#[sqlx::test(migrations = false)]
async fn image_drawing_reference_unknown_never_resends_original_edit(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::Truncated).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let image = upload();
    let raw = arguments(&image);
    let builtin = reference_builtin(&f, &raw).await;
    let preview = service
        .preview_with_uploads(&f.secret, &builtin.context, raw, vec![image])
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
    assert!(matches!(
        service
            .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
            .await,
        Err(Error::Pending)
    ));
    for _ in 0..3 {
        let receipt = service
            .execute_for_context(&f.secret, &preview.operation_id, &builtin.context)
            .await
            .unwrap();
        assert_eq!(receipt.state, "dispatching");
        assert!(receipt.output.is_none());
        assert!(receipt.charged_credits.is_none());
    }
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.reserved_units, 2000);
}

#[sqlx::test(migrations = false)]
async fn image_drawing_old_generation_grant_cannot_authorize_new_reference_edit(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let image = upload();
    let raw = arguments(&image);
    // A genuine persisted old-schema quote/grant, not untrusted context fields.
    let old = builtin(&f, &raw).await;
    assert!(matches!(
        service
            .preview_with_uploads(&f.secret, &old.context, raw, vec![image])
            .await,
        Err(Error::Forbidden)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_image.operations")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
}
