//! Real CookieJar login/rotation and PostgreSQL original-operation recovery.
use super::*;

const ROTATE: &str = r#"
import http.cookiejar
import json
import sys
import urllib.request
config=json.load(sys.stdin)
jar=http.cookiejar.LWPCookieJar(config['jar'])
jar.load(ignore_discard=True,ignore_expires=True)
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPCookieProcessor(jar))
request=urllib.request.Request(config['origin']+'/api/user/auth/'+config.get('action','refresh'),data=b'{}',headers={'Content-Type':'application/json','Origin':config['origin']},method='POST')
with opener.open(request,timeout=10) as response:
    body=json.loads(response.read(131073))
    status=response.status
jar.save(ignore_discard=True,ignore_expires=True)
print(json.dumps({'status':status,'access':body.get('data',{}).get('access_token')}))
"#;

struct RefreshFixture {
    f: BrowserFixture,
    supplier: Supplier,
    service: ImageService,
    _server: HttpServer,
    jar: PrivateJar,
    origin: String,
    access: String,
}
impl RefreshFixture {
    async fn new(pool: PgPool, reply: Reply) -> Self {
        let f = fixture(pool).await;
        let supplier = Supplier::start(reply).await;
        let service = image_service_for_pool(&f.pool, &supplier);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let auth = crate::legacy_auth::LegacyAuthConfig::new(&origin, &[43; 32])
            .unwrap()
            .with_insecure_local_http()
            .unwrap()
            .with_legal_acceptance_required(false);
        let app = crate::legacy_auth::router(f.identity.clone(), auth).merge(
            crate::model_http::image_http::browser_router_with_local_http(
                service.clone(),
                origin.clone(),
                true,
            )
            .unwrap(),
        );
        let server = HttpServer(tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        }));
        let jar = PrivateJar::new();
        let login = python(LOGIN, json!({"origin":origin,"jar":jar.path})).await;
        assert_eq!(login["status"], 200);
        let access = login["access"].as_str().unwrap().to_owned();
        jar.assert_private();
        Self {
            f,
            supplier,
            service,
            _server: server,
            jar,
            origin,
            access,
        }
    }
    async fn rotate(&mut self) {
        let old = self.access.clone();
        let old_actor = self.f.identity.authorize(&old).await.unwrap();
        let result = python(ROTATE, json!({"origin":self.origin,"jar":self.jar.path})).await;
        assert_eq!(result["status"], 200);
        self.access = result["access"].as_str().unwrap().to_owned();
        let actor = self.f.identity.authorize(&self.access).await.unwrap();
        assert_ne!(old_actor.credential_id, actor.credential_id);
        assert_eq!(old_actor.user_id, actor.user_id);
        assert!(self.f.identity.authorize(&old).await.is_err());
        let same:bool = sqlx::query_scalar("SELECT a.sid=b.sid FROM core_identity.web_session_credentials a JOIN core_identity.web_session_credentials b ON b.credential_id=$2 WHERE a.credential_id=$1")
            .bind(old_actor.credential_id).bind(actor.credential_id).fetch_one(&self.f.pool).await.unwrap();
        assert!(same);
        self.jar.assert_private();
    }
    async fn original(&self) -> (BuiltinFixture, Value, Preview) {
        let raw = drawing_input();
        let b = builtin_with_session(
            &self.f.pool,
            &self.access,
            "drawing-tool",
            "drawing-test-v1",
            "drawing.generate",
            &"a".repeat(64),
            &crate::builtins::input_digest(&raw).unwrap(),
        )
        .await;
        let preview = self
            .service
            .preview(&self.access, &b.context, raw.clone())
            .await
            .unwrap();
        (b, raw, preview)
    }
    async fn confirm(&self, p: &Preview, raw: &Value) {
        let page = python(NEGATIVE_REQUESTS,json!({"origin":self.origin,"jar":self.jar.path,"access":self.access,"path":p.confirmation_path,"operation_id":p.operation_id,"prompt":raw["prompt"]})).await;
        assert_eq!(page["page_status"], 200);
        assert_eq!(page["wrong_origin_status"], 403);
        assert_eq!(page["wrong_csrf_status"], 403);
        let confirmed = python(CONFIRM,json!({"origin":self.origin,"jar":self.jar.path,"path":p.confirmation_path,"csrf":page["csrf"]})).await;
        assert_eq!(confirmed["status"], 200);
        assert_eq!(confirmed["operation_id"], p.operation_id);
    }
    async fn operation_count(&self, b: &BuiltinFixture) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM core_image.operations WHERE source_kind='drawing' AND context->>'call_id'=$1 AND context->>'dispatch_id'=$2")
            .bind(&b.context.call_id).bind(&b.context.dispatch_id).fetch_one(&self.f.pool).await.unwrap()
    }
}

#[sqlx::test(migrations = false)]
async fn image_drawing_refresh_family_cookie_rotation_recovers_original_and_charges_once(
    pool: PgPool,
) {
    let mut h = RefreshFixture::new(pool, Reply::OneImage).await;
    let (b, raw, p) = h.original().await;
    let frozen:(i64,Vec<u8>,i64,i64)=sqlx::query_as("SELECT session_id,fingerprint,created_at,expires_at FROM core_image.operations WHERE id=$1").bind(&p.operation_id).fetch_one(&h.f.pool).await.unwrap();
    h.rotate().await;
    let by_id = h
        .service
        .query_for_context(&h.access, &p.operation_id, &b.context)
        .await
        .unwrap();
    let by_context = h
        .service
        .query_by_context(&h.access, &b.context)
        .await
        .unwrap();
    assert_eq!(by_id.operation_id, p.operation_id);
    assert_eq!(by_context.operation_id, p.operation_id);
    assert_eq!(by_id.state, "previewed");
    let replay = h
        .service
        .preview(&h.access, &b.context, raw.clone())
        .await
        .unwrap();
    assert_eq!(replay.operation_id, p.operation_id);
    assert_eq!(replay.preview_digest, p.preview_digest);
    assert_eq!(replay.expires_at, p.expires_at);
    let after:(i64,Vec<u8>,i64,i64)=sqlx::query_as("SELECT session_id,fingerprint,created_at,expires_at FROM core_image.operations WHERE id=$1").bind(&p.operation_id).fetch_one(&h.f.pool).await.unwrap();
    assert_eq!(frozen, after);
    assert_eq!(h.operation_count(&b).await, 1);
    no_model_hold(&h.f).await;
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 0);
    h.confirm(&p, &raw).await;
    h.rotate().await;
    let receipt = h
        .service
        .execute_for_context(&h.access, &p.operation_id, &b.context)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 1);
    b.store
        .revoke(
            &h.access,
            &IdRequest {
                owner: Owner {
                    user: h.f.user.to_string(),
                    account: h.f.user.to_string(),
                },
                operation_id: "revoke-completed-refresh".into(),
                id: b.context.grant_id.clone(),
            },
            false,
        )
        .await
        .unwrap();
    h.rotate().await;
    let recovered = h
        .service
        .query_by_context(&h.access, &b.context)
        .await
        .unwrap();
    assert_eq!(recovered.output, receipt.output);
    let repeated = h
        .service
        .execute_for_context(&h.access, &p.operation_id, &b.context)
        .await
        .unwrap();
    assert_eq!(repeated.output, receipt.output);
    assert_eq!(h.operation_count(&b).await, 1);
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 1);
    let balance = h.f.ledger.balance(h.f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
}

#[sqlx::test(migrations = false)]
async fn image_drawing_refresh_family_unknown_keeps_original_hold_and_never_redispatches(
    pool: PgPool,
) {
    let mut h = RefreshFixture::new(pool, Reply::Truncated).await;
    let (b, raw, p) = h.original().await;
    h.confirm(&p, &raw).await;
    assert_eq!(
        h.service
            .execute_for_context(&h.access, &p.operation_id, &b.context)
            .await
            .unwrap_err(),
        Error::Pending
    );
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 1);
    h.rotate().await;
    for _ in 0..2 {
        let receipt = h
            .service
            .query_by_context(&h.access, &b.context)
            .await
            .unwrap();
        assert_eq!(receipt.operation_id, p.operation_id);
        assert!(receipt.output.is_none());
        assert_ne!(receipt.state, "confirmed");
        let observed = h
            .service
            .execute_for_context(&h.access, &p.operation_id, &b.context)
            .await
            .unwrap();
        assert_eq!(observed.operation_id, p.operation_id);
    }
    let charge = h
        .service
        .charging
        .resolve(&p.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.state, "unknown");
    assert_eq!(charge.reserved, 6000);
    let balance = h.f.ledger.balance(h.f.account).await.unwrap();
    assert_eq!(balance.reserved_units, 6000);
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(h.operation_count(&b).await, 1);
}

#[sqlx::test(migrations = false)]
async fn image_drawing_refresh_family_rejects_other_family_actor_context_and_disabled_user(
    pool: PgPool,
) {
    let mut h = RefreshFixture::new(pool, Reply::OneImage).await;
    let (b, raw, p) = h.original().await;
    h.rotate().await;
    let other = login(&h.f.browser, "drawingowner").await;
    let other_access = other.data["access_token"].as_str().unwrap();
    assert_eq!(
        h.service
            .query_for_context(other_access, &p.operation_id, &b.context)
            .await
            .unwrap_err(),
        Error::Missing
    );
    assert_eq!(
        h.service
            .preview(other_access, &b.context, raw.clone())
            .await
            .unwrap_err(),
        Error::Missing
    );
    assert_eq!(
        h.service
            .execute_for_context(other_access, &p.operation_id, &b.context)
            .await
            .unwrap_err(),
        Error::Missing
    );
    assert_eq!(
        h.service
            .confirmation_page(&other.refresh_token, &p.operation_id)
            .await
            .unwrap_err(),
        Error::Missing
    );
    assert_eq!(
        h.service
            .confirm(&other.refresh_token, &p.operation_id, &"0".repeat(64))
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    for field in [
        "call_id",
        "dispatch_id",
        "tool_id",
        "version_id",
        "grant_id",
        "input_digest",
    ] {
        let mut value = serde_json::to_value(&b.context).unwrap();
        value[field] = json!("f".repeat(64));
        let wrong: BuiltinContext = serde_json::from_value(value).unwrap();
        assert_eq!(
            h.service
                .query_for_context(&h.access, &p.operation_id, &wrong)
                .await
                .unwrap_err(),
            Error::Forbidden
        );
    }
    h.f.browser
        .register(&BrowserRegistration {
            username: "drawingother",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let different = login(&h.f.browser, "drawingother").await;
    assert_eq!(
        h.service
            .query(
                different.data["access_token"].as_str().unwrap(),
                &p.operation_id
            )
            .await
            .unwrap_err(),
        Error::Missing
    );
    assert_eq!(h.operation_count(&b).await, 1);
    no_model_hold(&h.f).await;
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=$1")
        .bind(h.f.user)
        .execute(&h.f.pool)
        .await
        .unwrap();
    assert_eq!(
        h.service
            .query(&h.access, &p.operation_id)
            .await
            .unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test(migrations = false)]
async fn image_drawing_refresh_family_logout_denies_old_and_current_bearers(pool: PgPool) {
    let mut h = RefreshFixture::new(pool, Reply::OneImage).await;
    let (b, _, p) = h.original().await;
    let old = h.access.clone();
    h.rotate().await;
    let out = python(
        ROTATE,
        json!({"origin":h.origin,"jar":h.jar.path,"action":"logout"}),
    )
    .await;
    assert_eq!(out["status"], 200);
    assert_eq!(
        h.service
            .query(&h.access, &p.operation_id)
            .await
            .unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(
        h.service.query(&old, &p.operation_id).await.unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(h.operation_count(&b).await, 1);
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 0);
    no_model_hold(&h.f).await;
}

#[sqlx::test(migrations = false)]
async fn image_drawing_refresh_family_parallel_previews_keep_one_original_dispatch(pool: PgPool) {
    let mut h = RefreshFixture::new(pool, Reply::OneImage).await;
    let raw = drawing_input();
    let b = builtin_with_session(
        &h.f.pool,
        &h.access,
        "drawing-tool",
        "drawing-test-v1",
        "drawing.generate",
        &"a".repeat(64),
        &crate::builtins::input_digest(&raw).unwrap(),
    )
    .await;
    let second = image_service_for_pool(&h.f.pool, &h.supplier);
    let mut original: Option<Preview> = None;
    for _ in 0..2 {
        let (left, right) = tokio::join!(
            h.service.preview(&h.access, &b.context, raw.clone()),
            second.preview(&h.access, &b.context, raw.clone())
        );
        assert!(left.is_ok() || right.is_ok());
        for out in [left, right] {
            match out {
                Ok(value) => {
                    if let Some(prior) = &original {
                        assert_eq!(value.operation_id, prior.operation_id);
                    } else {
                        original = Some(value);
                    }
                }
                Err(error) => assert!(matches!(
                    error,
                    Error::Pending | Error::Retry | Error::Conflict
                )),
            }
        }
        assert_eq!(
            h.service
                .query_by_context(&h.access, &b.context)
                .await
                .unwrap()
                .operation_id,
            original.as_ref().unwrap().operation_id
        );
        h.rotate().await;
    }
    let p = original.unwrap();
    assert_eq!(h.operation_count(&b).await, 1);
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 0);
    no_model_hold(&h.f).await;
    h.confirm(&p, &raw).await;
    let (left, right) = tokio::join!(
        h.service
            .execute_for_context(&h.access, &p.operation_id, &b.context),
        second.execute_for_context(&h.access, &p.operation_id, &b.context)
    );
    assert!(left.is_ok() || right.is_ok());
    for out in [left, right] {
        match out {
            Ok(receipt) => {
                assert_eq!(receipt.operation_id, p.operation_id);
                if receipt.state == "completed" {
                    assert_eq!(receipt.charged_credits, Some(2000));
                } else {
                    assert!(matches!(
                        receipt.state.as_str(),
                        "dispatching" | "billing_pending" | "unknown"
                    ));
                    assert!(receipt.charged_credits.is_none());
                    assert!(receipt.output.is_none());
                }
            }
            Err(error) => assert!(matches!(
                error,
                Error::Pending | Error::Retry | Error::Conflict
            )),
        }
    }
    let terminal = h
        .service
        .query_for_context(&h.access, &p.operation_id, &b.context)
        .await
        .unwrap();
    assert_eq!(terminal.state, "completed");
    assert_eq!(terminal.charged_credits, Some(2000));
    assert!(terminal.output.is_some());
    assert_eq!(h.operation_count(&b).await, 1);
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 1);
    let balance = h.f.ledger.balance(h.f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
}

#[sqlx::test(migrations = false)]
async fn image_drawing_refresh_family_preserves_exact_native_session_execution(pool: PgPool) {
    let h = RefreshFixture::new(pool, Reply::OneImage).await;
    let native =
        h.f.identity
            .login(&crate::identity::LoginRequest {
                login_name: "drawingowner".into(),
                password: PASSWORD.into(),
            })
            .await
            .unwrap();
    let other =
        h.f.identity
            .login(&crate::identity::LoginRequest {
                login_name: "drawingowner".into(),
                password: PASSWORD.into(),
            })
            .await
            .unwrap();
    let raw = drawing_input();
    let b = builtin_with_session(
        &h.f.pool,
        &native.secret,
        "native-drawing-tool",
        "drawing-test-v1",
        "drawing.generate",
        &"a".repeat(64),
        &crate::builtins::input_digest(&raw).unwrap(),
    )
    .await;
    let p = h
        .service
        .preview(&native.secret, &b.context, raw.clone())
        .await
        .unwrap();
    assert_eq!(
        h.service
            .query(&other.secret, &p.operation_id)
            .await
            .unwrap_err(),
        Error::Missing
    );
    assert_eq!(
        h.service
            .query(&h.access, &p.operation_id)
            .await
            .unwrap_err(),
        Error::Missing
    );
    // Native operations retain their prior same-user cookie-confirmation policy;
    // execution still requires exactly the original native Session credential.
    h.confirm(&p, &raw).await;
    assert_eq!(
        h.service
            .execute_for_context(&other.secret, &p.operation_id, &b.context)
            .await
            .unwrap_err(),
        Error::Missing
    );
    let paid = h
        .service
        .execute_for_context(&native.secret, &p.operation_id, &b.context)
        .await
        .unwrap();
    assert_eq!(paid.charged_credits, Some(2000));
    assert_eq!(h.supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(h.operation_count(&b).await, 1);
}
