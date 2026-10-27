//! Read-only catalog for an installation without configured Web checkout.
//! Internal payment RPC availability does not establish a usable Web provider.
use crate::identity::{CredentialKind, IdentityError, IdentityStore};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;

/// Mount when no Web checkout catalog is configured. Never enables a provider.
pub fn unconfigured_router(identity: IdentityStore) -> Router {
    Router::new()
        .route("/api/user/topup/info", get(info))
        .with_state(identity)
        .layer(middleware::from_fn(no_store))
}

async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn bearer(headers: &HeaderMap) -> Result<&str, IdentityError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or(IdentityError::Unauthorized)?;
    if values.next().is_some() {
        return Err(IdentityError::Unauthorized);
    }
    value
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|secret| !secret.is_empty())
        .ok_or(IdentityError::Unauthorized)
}

async fn info(State(identity): State<IdentityStore>, headers: HeaderMap) -> Response {
    let secret = match bearer(&headers) {
        Ok(secret) => secret,
        Err(error) => return error_response(error),
    };
    match identity.authorize(secret).await {
        Ok(actor)
            if actor.credential_kind == CredentialKind::Session
                && actor.owner.kind == crate::accounts::AccountKind::Personal
                && actor.owner.id == actor.user_id =>
        {
            // @context-fold begin unconfigured-web-checkout-catalog
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "",
                    "data": {
                        "payment_available": false,
                        "payment_unavailable_reason": "web_checkout_not_configured",
                        "enable_online_topup": false,
                        "enable_stripe_topup": false,
                        "enable_creem_topup": false,
                        "enable_waffo_topup": false,
                        "enable_waffo_pancake_topup": false,
                        "enable_subscriptions": false,
                        "enable_redemption_code": false,
                        "pay_methods": [],
                        "creem_products": [],
                        "waffo_pay_methods": [],
                        "amount_options": [],
                        "discount": {},
                        "amount_unit": "CREDIT",
                        "credit_metadata_available": false,
                        // Disabled compatibility bounds, never an available quote.
                        "min_topup": 0,
                        "stripe_min_topup": 0,
                        "waffo_min_topup": 0,
                        "waffo_pancake_min_topup": 0
                    }
                })),
            )
                .into_response()
            // @context-fold end unconfigured-web-checkout-catalog
        }
        Ok(_) => error_response(IdentityError::Forbidden),
        Err(error) => error_response(error),
    }
}

fn error_response(error: IdentityError) -> Response {
    let (status, code) = match error {
        IdentityError::Unauthorized => (StatusCode::UNAUTHORIZED, "AUTH_UNAUTHORIZED"),
        IdentityError::Forbidden => (StatusCode::FORBIDDEN, "AUTH_FORBIDDEN"),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "TOPUP_IDENTITY_UNAVAILABLE",
        ),
    };
    (
        status,
        Json(json!({"success": false, "message": code, "code": code})),
    )
        .into_response()
}
