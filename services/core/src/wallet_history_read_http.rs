// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
//! Personal legacy top-up history only; payment configuration is owned separately.

use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};

use crate::{
    identity::{CredentialKind, IdentityError, IdentityStore, Principal},
    wallet_topup_snapshot::{HistoryQuery, HistoryReadError, WalletTopupHistory},
};

#[derive(Clone)]
pub struct WalletHistoryReadHttp {
    identity: IdentityStore,
    history: Option<WalletTopupHistory>,
}

impl WalletHistoryReadHttp {
    pub fn new(identity: IdentityStore) -> Self {
        Self {
            identity,
            history: None,
        }
    }

    pub fn with_history(mut self, history: WalletTopupHistory) -> Self {
        self.history = Some(history);
        self
    }
}

/// Merge once, before attaching the application's state. Does not register topup/info.
pub fn router(state: WalletHistoryReadHttp) -> Router {
    Router::new()
        .route("/api/user/topup/self", get(topup_self))
        .with_state(state)
}

fn response(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    response(
        status,
        json!({"success":false,"code":code,"message":message}),
    )
}

// @context-fold begin wallet-history-session-read
async fn authorize(
    state: &WalletHistoryReadHttp,
    headers: &HeaderMap,
) -> Result<Principal, Response> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let secret = values
        .next()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty() && !value.bytes().any(|byte| byte.is_ascii_whitespace()));
    let Some(secret) = secret.filter(|_| values.next().is_none()) else {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "A valid session is required.",
        ));
    };
    match state.identity.authorize(secret).await {
        Ok(actor) if actor.credential_kind == CredentialKind::Session => Ok(actor),
        Ok(_) | Err(IdentityError::Forbidden) => Err(error(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "A console session is required.",
        )),
        Err(IdentityError::Storage) => Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_UNAVAILABLE",
            "Session verification is unavailable.",
        )),
        Err(_) => Err(error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "A valid session is required.",
        )),
    }
}
// @context-fold end wallet-history-session-read

async fn topup_self(
    State(state): State<WalletHistoryReadHttp>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let principal = match authorize(&state, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let Ok(Query(query)) = query else {
        return error(
            StatusCode::BAD_REQUEST,
            "INVALID_QUERY",
            "invalid history pagination, search or sort",
        );
    };
    let Some(history) = state.history else {
        return history_unavailable();
    };
    match history
        .read(principal.user_id, principal.owner_account_id, &query)
        .await
    {
        Ok(data) => response(
            StatusCode::OK,
            json!({"success":true,"message":"","data":data}),
        ),
        Err(HistoryReadError::Invalid) => error(
            StatusCode::BAD_REQUEST,
            "INVALID_QUERY",
            "invalid history pagination, search or sort",
        ),
        Err(HistoryReadError::ScopeUnavailable) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "TOPUP_HISTORY_SCOPE_UNAVAILABLE",
            "this account is not covered by the imported legacy order snapshot",
        ),
        Err(HistoryReadError::Unavailable) => history_unavailable(),
    }
}
fn history_unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "TOPUP_HISTORY_SOURCE_UNAVAILABLE",
        "an audited legacy top_ups snapshot must be imported before history can be shown",
    )
}
