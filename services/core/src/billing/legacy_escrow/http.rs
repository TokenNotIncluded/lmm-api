//! The historical escrow transport shares only the protected bounty UDS and its
//! independent host token. Every operation still authenticates the original
//! participant/owner Session in Core; this is not an import or generic money API.
use super::{CloseCommand, Error, LegacyEscrowStore, PayoutCommand, QueryCommand, SubmitCommand};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Clone)]
struct Internal {
    store: LegacyEscrowStore,
    token: [u8; 32],
}

fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, Error> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or(Error::Forbidden)?;
    if values.next().is_some() {
        return Err(Error::Forbidden);
    }
    value.to_str().map_err(|_| Error::Forbidden)
}

fn secure(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    response
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::Missing => (StatusCode::NOT_FOUND, "missing"),
            Self::Retry => (StatusCode::SERVICE_UNAVAILABLE, "retry"),
            Self::Storage => (StatusCode::SERVICE_UNAVAILABLE, "storage"),
            Self::UnknownOutcome => (StatusCode::SERVICE_UNAVAILABLE, "unknown"),
        };
        secure((status, Json(serde_json::json!({"error": code}))).into_response())
    }
}

pub fn internal_router(store: LegacyEscrowStore, token: &str) -> Result<Router, Error> {
    if !(32..=256).contains(&token.len()) || token.contains(['\r', '\n', '\0']) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route(
            "/internal/bounties/v1/legacy-escrow/{action}",
            post(internal),
        )
        .layer(DefaultBodyLimit::max(16384))
        .with_state(Internal {
            store,
            token: Sha256::digest(token.as_bytes()).into(),
        }))
}

async fn internal(
    State(state): State<Internal>,
    Path(action): Path<String>,
    request: Request,
) -> Result<Response, Error> {
    let headers = request.headers();
    let bearer = single(headers, "authorization")?
        .strip_prefix("Bearer ")
        .ok_or(Error::Forbidden)?;
    let digest: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
    if !(32..=256).contains(&bearer.len())
        || !bool::from(digest.ct_eq(&state.token))
        || single(headers, "x-lmm-protocol")? != "1"
    {
        return Err(Error::Forbidden);
    }
    let session = single(headers, "x-lmm-user-credential")?;
    if session.is_empty() || session.len() > 4096 || session.contains(['\r', '\n', '\0']) {
        return Err(Error::Forbidden);
    }
    let session = session.to_owned();
    // Deserialize directly into the closed command type. An intermediate JSON
    // Value would silently keep the last duplicate operation/evidence field.
    let receipt = match action.as_str() {
        "submit" => {
            let command: SubmitCommand = body(request).await?;
            serde_json::to_value(state.store.submit(&session, &command).await?)
        }
        "payout" => {
            let command: PayoutCommand = body(request).await?;
            serde_json::to_value(state.store.payout(&session, &command).await?)
        }
        "close" => {
            let command: CloseCommand = body(request).await?;
            serde_json::to_value(state.store.close(&session, &command).await?)
        }
        "query" => {
            let command: QueryCommand = body(request).await?;
            serde_json::to_value(state.store.query(&session, &command).await?)
        }
        _ => return Err(Error::Missing),
    }
    .map_err(|_| Error::Storage)?;
    Ok(secure(Json(receipt).into_response()))
}

async fn body<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, Error> {
    let Json(value) = Json::<T>::from_request(request, &())
        .await
        .map_err(|_| Error::Invalid)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    fn router() -> Router {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        internal_router(LegacyEscrowStore::from_pool(pool), &"s".repeat(32)).unwrap()
    }
    fn request(action: &str, body: &str) -> Request {
        Request::post(format!("/internal/bounties/v1/legacy-escrow/{action}"))
            .header("authorization", format!("Bearer {}", "s".repeat(32)))
            .header("x-lmm-protocol", "1")
            .header("x-lmm-user-credential", "synthetic-session")
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    }
    #[tokio::test]
    async fn duplicate_identity_headers_and_wrong_host_are_rejected_before_storage() {
        for header in ["authorization", "x-lmm-protocol", "x-lmm-user-credential"] {
            let mut req = request(
                "close",
                r#"{"migration_id":"test","source_project_id":1,"operation_key":"close"}"#,
            );
            let value = req.headers()[header].clone();
            req.headers_mut().append(header, value);
            let response = router().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        for header in ["authorization", "x-lmm-protocol", "x-lmm-user-credential"] {
            let mut req = request("close", "{}");
            req.headers_mut().remove(header);
            assert_eq!(
                router().oneshot(req).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        let mut req = request("close", "{}");
        req.headers_mut()
            .insert("authorization", "Bearer wrong-host-token".parse().unwrap());
        assert_eq!(
            router().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn transport_never_accepts_import_or_caller_selected_money() {
        for (action, body, status) in [
            ("import", "{}", StatusCode::NOT_FOUND),
            (
                "payout",
                r#"{"migration_id":"test","source_project_id":1,"source_challenge_id":2,"operation_key":"pay","amount_units":100}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "payout",
                r#"{"migration_id":"test","source_project_id":1,"source_challenge_id":2,"operation_key":"pay","recipient_account_id":3}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "query",
                r#"{"migration_id":"test","source_project_id":1,"operation_key":"pay","amount_units":100}"#,
                StatusCode::BAD_REQUEST,
            ),
            ("submit", "{", StatusCode::BAD_REQUEST),
            (
                "close",
                r#"{"migration_id":"test","source_project_id":1,"operation_key":"one","operation_key":"two"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "submit",
                r#"{"migration_id":"test","source_project_id":1,"source_challenge_id":2,"operation_key":"submit","pull_request_url":"https://github.com/a/b/pull/1","pull_request_url":"https://github.com/a/b/pull/2"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "query",
                r#"{"migration_id":"test","source_project_id":1,"operation_key":"query","expected_command":null,"expected_command":null}"#,
                StatusCode::BAD_REQUEST,
            ),
        ] {
            assert_eq!(
                router()
                    .oneshot(request(action, body))
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
        assert_eq!(
            router()
                .oneshot(request("submit", &"x".repeat(16385)))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[test]
    fn unknown_outcome_has_stable_non_success_code() {
        let response = Error::UnknownOutcome.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
}
