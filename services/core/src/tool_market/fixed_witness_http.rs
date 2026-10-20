//! A separate, explicitly reviewed execution witness role. Never shares the
//! ordinary market host bearer credential or accepts a claimed role in JSON.
use super::{
    credit_access::CreditStore,
    credit_execution::{Receipt, ReleaseRequest, SettleRequest},
    *,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;
#[derive(Clone)]
struct Service {
    store: CreditStore,
    role: String,
    token: [u8; 32],
    slots: Arc<Semaphore>,
}
impl Service {
    fn check(&self, headers: &HeaderMap) -> Result<()> {
        let one = |key: &str| -> Result<String> {
            let mut all = headers.get_all(key).iter();
            let first = all.next().ok_or(Error::Unauthorized)?;
            if all.next().is_some() {
                return Err(Error::Unauthorized);
            }
            Ok(first.to_str().map_err(|_| Error::Unauthorized)?.into())
        };
        if one("x-lmm-protocol")? != "2" {
            return Err(Error::Invalid);
        }
        let authorization = one("authorization")?;
        let raw = authorization
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        let digest: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        if !(32..=256).contains(&raw.len()) || !bool::from(digest.ct_eq(&self.token)) {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}
fn response(result: Result<Receipt>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let (status, code) = match e {
                Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
                Error::Forbidden => (StatusCode::FORBIDDEN, "denied"),
                Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
                Error::Conflict => (StatusCode::CONFLICT, "conflict"),
                Error::Missing => (StatusCode::NOT_FOUND, "missing"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "unknown"),
            };
            (status, Json(serde_json::json!({"error":code}))).into_response()
        }
    }
}
async fn settle(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(r): Json<SettleRequest>,
) -> Response {
    response(
        async {
            s.check(&headers)?;
            let _permit = s.slots.try_acquire().map_err(|_| Error::Pending)?;
            s.store.settle_fixed_witness(&r, &s.role).await
        }
        .await,
    )
}
async fn release(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(r): Json<ReleaseRequest>,
) -> Response {
    response(
        async {
            s.check(&headers)?;
            let _permit = s.slots.try_acquire().map_err(|_| Error::Pending)?;
            s.store.release_fixed_witness(&r, &s.role).await
        }
        .await,
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
async fn capabilities(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(_): Json<Empty>,
) -> Response {
    let result:Result<serde_json::Value>=async {
        s.check(&headers)?;
        let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_meta.component_contract WHERE name='tool-market-credit' AND fingerprint=$1)").bind(Sha256::digest(super::credits::SCHEMA_V2.as_bytes()).to_vec()).fetch_one(&s.store.base.pool).await?;
        if !ready {return Err(Error::Pending);}
        Ok(serde_json::json!({"protocol_major":2,"purpose":"core_host_fixed_success","role":s.role,"features":["admit","settle","release"]}))
    }.await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(Error::Unauthorized) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"unauthorized"})),
        )
            .into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"unknown"})),
        )
            .into_response(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmitRequest {
    quote_id: String,
}
async fn admit(
    State(s): State<Service>,
    headers: HeaderMap,
    Json(r): Json<AdmitRequest>,
) -> Response {
    let result:Result<serde_json::Value>=async {
        s.check(&headers)?;
        let _permit=s.slots.try_acquire().map_err(|_|Error::Pending)?;
        if !digest(&r.quote_id) {return Err(Error::Invalid);}
        let mut tx=s.store.base.begin().await?;
        let (mode,role,_)=CreditStore::result_contract(&mut tx,&r.quote_id).await?;
        if mode!="core_host_fixed_success" || role!=s.role {return Err(Error::Forbidden);}
        let row=sqlx::query("SELECT snapshot,seller_user_id FROM core_tool_market.credit_quotes WHERE id=$1 AND approved AND NOT suspended FOR SHARE").bind(&r.quote_id).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let sqlx::types::Json(quote):sqlx::types::Json<super::credits::Quote>=row.try_get("snapshot")?;
        if !matches!(quote.price,super::credits::Price::Fixed{price_quota} if price_quota>0) || quote.expires<=ToolMarketStore::now(&mut tx).await? {return Err(Error::Forbidden);}
        CreditStore::check_version(&mut tx,row.try_get("seller_user_id")?,&quote.service_id,&quote.version_id).await?;
        tx.commit().await?;
        Ok(serde_json::json!({"quote_id":r.quote_id,"role":s.role,"proof_mode":mode,"ready":true}))
    }.await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(Error::Unauthorized) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"unauthorized"})),
        )
            .into_response(),
        Err(Error::Forbidden) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"denied"})),
        )
            .into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"unknown"})),
        )
            .into_response(),
    }
}
pub fn router(
    store: CreditStore,
    ordinary_token: &[u8],
    purpose_token: &[u8],
    role: &str,
) -> Result<Router> {
    if !(32..=256).contains(&purpose_token.len())
        || !purpose_token.iter().all(u8::is_ascii_graphic)
        || purpose_token == ordinary_token
        || !name(role)
    {
        return Err(Error::Invalid);
    }
    let s = Service {
        store,
        role: role.into(),
        token: Sha256::digest(purpose_token).into(),
        slots: Arc::new(Semaphore::new(2)),
    };
    Ok(Router::new()
        .route(
            "/internal/tool-market/v2/fixed-witness/admit",
            post(admit).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/internal/tool-market/v2/fixed-witness/capabilities",
            post(capabilities).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/internal/tool-market/v2/fixed-witness/settle",
            post(settle),
        )
        .route(
            "/internal/tool-market/v2/fixed-witness/release",
            post(release),
        )
        .layer(DefaultBodyLimit::max(2_850_000))
        .with_state(s))
}
