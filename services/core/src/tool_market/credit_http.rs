//! Protocol 2 closed legacy CREDIT port, never public HTTP. Separate user
//! Session and module host token are checked for every user mutation.
use super::{
    credit_access::*,
    credit_config::*,
    credit_execution::{self, *},
    credit_lifecycle::*,
    credit_reports::*,
    credits, *,
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
    token: [u8; 32],
    slots: Arc<Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Validate {
    account: String,
}
fn single<'a>(h: &'a HeaderMap, key: &str) -> Result<&'a str> {
    let mut v = h.get_all(key).iter();
    let first = v.next().ok_or(Error::Unauthorized)?;
    if v.next().is_some() {
        return Err(Error::Unauthorized);
    }
    first.to_str().map_err(|_| Error::Unauthorized)
}
impl Service {
    fn check(&self, h: &HeaderMap) -> Result<()> {
        let raw = single(h, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        let digest: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        if !(32..=256).contains(&raw.len()) || !bool::from(digest.ct_eq(&self.token)) {
            return Err(Error::Unauthorized);
        }
        if single(h, "x-lmm-protocol")? != "2" {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn response<T: Serialize>(result: Result<T>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let (status, code) = match e {
                Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
                Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
                Error::Forbidden => (StatusCode::FORBIDDEN, "denied"),
                Error::Missing => (StatusCode::NOT_FOUND, "missing"),
                Error::Conflict => (StatusCode::CONFLICT, "conflict"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "unknown"),
            };
            (status, Json(serde_json::json!({"error":code}))).into_response()
        }
    }
}
macro_rules! user_endpoint {
    ($name:ident,$input:ty,$method:ident) => {
        async fn $name(State(s): State<Service>, h: HeaderMap, Json(v): Json<$input>) -> Response {
            response(
                async {
                    s.check(&h)?;
                    let _permit = s.slots.try_acquire().map_err(|_| Error::Pending)?;
                    s.store
                        .$method(single(&h, "x-lmm-user-credential")?, &v)
                        .await
                }
                .await,
            )
        }
    };
}
macro_rules! host_endpoint {
    ($name:ident,$input:ty,$method:ident) => {
        async fn $name(State(s): State<Service>, h: HeaderMap, Json(v): Json<$input>) -> Response {
            response(
                async {
                    s.check(&h)?;
                    let _permit = s.slots.try_acquire().map_err(|_| Error::Pending)?;
                    s.store.$method(&v).await
                }
                .await,
            )
        }
    };
}
impl CreditStore {
    async fn meta_oauth_clients(
        &self,
        secret: &str,
        r: &super::credit_meta::OAuthClientsRequest,
    ) -> Result<super::credit_meta::OAuthClients> {
        self.meta_store().oauth_clients(secret, r).await
    }
    fn meta_store(&self) -> super::credit_meta::MetaStore {
        super::credit_meta::MetaStore::new(self.clone(), self.market_profile.as_deref().cloned())
    }
    async fn consent_list_request(
        &self,
        secret: &str,
        r: &ListRequest,
    ) -> Result<ListReceipt<ConsentReceipt>> {
        if secret.starts_with("lmm_market_") || secret.starts_with("lmm_at_") {
            self.meta_store().list_consents(secret, r).await
        } else {
            self.list_consents(secret, r).await
        }
    }
    async fn meta_read(&self, secret: &str, _: &Empty) -> Result<super::credit_meta::MetaView> {
        self.meta_store().read(secret).await
    }
    async fn meta_owner_read(
        &self,
        secret: &str,
        r: &super::credit_meta::Target,
    ) -> Result<super::credit_meta::OwnerView> {
        self.meta_store().owner_read(secret, r).await
    }
    async fn meta_owner_set(
        &self,
        secret: &str,
        r: &super::credit_meta::SetDelegation,
    ) -> Result<super::credit_meta::OwnerView> {
        self.meta_store().owner_set(secret, r).await
    }
    async fn meta_authorize(
        &self,
        secret: &str,
        r: &super::credit_meta::Authorize,
    ) -> Result<ConsentReceipt> {
        self.meta_store().authorize(secret, r).await
    }
    async fn meta_client_budget(
        &self,
        secret: &str,
        r: &super::credit_meta::ClientBudget,
    ) -> Result<BudgetReceipt> {
        self.meta_store().tighten_client(secret, r).await
    }
    async fn meta_tool_budget(
        &self,
        secret: &str,
        r: &super::credit_meta::ToolBudget,
    ) -> Result<ConsentReceipt> {
        self.meta_store().tighten_tool(secret, r).await
    }
    async fn meta_usage(
        &self,
        secret: &str,
        r: &super::credit_meta::Page,
    ) -> Result<super::credit_meta::UsageView> {
        self.meta_store().usage(secret, r).await
    }
    async fn meta_calls(
        &self,
        secret: &str,
        r: &super::credit_meta::Page,
    ) -> Result<super::credit_meta::ClientCalls> {
        self.meta_store().calls(secret, r).await
    }

    async fn consent_revoke(&self, s: &str, r: &IdRequest) -> Result<RevocationReceipt> {
        self.revoke(s, r, false).await
    }
    async fn token_revoke(&self, s: &str, r: &IdRequest) -> Result<RevocationReceipt> {
        self.revoke(s, r, true).await
    }
    async fn dispatch_request(
        &self,
        s: &str,
        r: &CallRequest,
    ) -> Result<credit_execution::Receipt> {
        self.dispatch(s, &r.call_id).await
    }
    async fn query_request(
        &self,
        s: &str,
        r: &CallRequest,
    ) -> Result<Option<credit_execution::Receipt>> {
        self.query(s, &r.call_id).await
    }
}
user_endpoint!(
    meta_oauth_clients,
    super::credit_meta::OAuthClientsRequest,
    meta_oauth_clients
);
user_endpoint!(meta_read, Empty, meta_read);
user_endpoint!(meta_owner_read, super::credit_meta::Target, meta_owner_read);
user_endpoint!(
    meta_owner_set,
    super::credit_meta::SetDelegation,
    meta_owner_set
);
user_endpoint!(
    meta_authorize,
    super::credit_meta::Authorize,
    meta_authorize
);
user_endpoint!(
    meta_client_budget,
    super::credit_meta::ClientBudget,
    meta_client_budget
);
user_endpoint!(
    meta_tool_budget,
    super::credit_meta::ToolBudget,
    meta_tool_budget
);
user_endpoint!(meta_usage, super::credit_meta::Page, meta_usage);
user_endpoint!(meta_calls, super::credit_meta::Page, meta_calls);
user_endpoint!(report_create, ReportRequest, create_report);
user_endpoint!(report_review, ReviewReportRequest, review_report);
user_endpoint!(report_list, ReportListRequest, list_reports);
user_endpoint!(income_list, ListRequest, list_income);
user_endpoint!(service_lifecycle, LifecycleRequest, lifecycle);
user_endpoint!(quote, credits::Quote, register_quote);
user_endpoint!(approve, ApproveRequest, approve_quote);
user_endpoint!(consent_create, ConsentRequest, create_consent);
user_endpoint!(consent_revoke, IdRequest, consent_revoke);
user_endpoint!(budget_set, BudgetRequest, set_budget);
user_endpoint!(token_create, TokenRequest, create_token);
user_endpoint!(token_revoke, IdRequest, token_revoke);
user_endpoint!(disconnect, ClientRequest, disconnect);
user_endpoint!(consent_list, ListRequest, consent_list_request);
user_endpoint!(budget_list, ListRequest, list_budgets);
user_endpoint!(token_list, ListRequest, list_tokens);
user_endpoint!(reserve, ReserveRequest, reserve);
user_endpoint!(dispatch, CallRequest, dispatch_request);
user_endpoint!(query, CallRequest, query_request);
user_endpoint!(config_update, ConfigRequest, update_config);
host_endpoint!(settle, SettleRequest, settle);
host_endpoint!(release, ReleaseRequest, release);
host_endpoint!(unknown, DispatchRequest, unknown);
async fn token_validate(
    State(s): State<Service>,
    h: HeaderMap,
    Json(v): Json<Validate>,
) -> Response {
    response(
        async {
            s.check(&h)?;
            let _permit = s.slots.try_acquire().map_err(|_| Error::Pending)?;
            s.store
                .validate_token(single(&h, "x-lmm-user-credential")?, &v.account)
                .await
        }
        .await,
    )
}
async fn config_read(State(s): State<Service>, h: HeaderMap, Json(_): Json<Empty>) -> Response {
    response(
        async {
            s.check(&h)?;
            let _permit = s.slots.try_acquire().map_err(|_| Error::Pending)?;
            s.store.config().await
        }
        .await,
    )
}
async fn capabilities(State(s): State<Service>, h: HeaderMap, Json(_): Json<Empty>) -> Response {
    response(async {
 s.check(&h)?;let _permit=s.slots.try_acquire().map_err(|_|Error::Pending)?;
 let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_meta.component_contract WHERE name='tool-market-credit' AND fingerprint=$1)").bind(Sha256::digest(credits::SCHEMA_V2.as_bytes()).to_vec()).fetch_one(&s.store.base.pool).await?;
 if !ready{return Err(Error::Pending)}
 let mut features=vec!["quotes","quotes.approve","access.consent-create","access.consent-revoke","access.budget-set","access.token-create","access.token-revoke","access.token-validate","access.client-disconnect","access.consent-list","access.budget-list","access.token-list","calls.reserve","calls.dispatch","calls.settle","calls.unknown","calls.release","calls.query","config.read","config.update","services.lifecycle","reports.create","reports.review","reports.list","income.list","meta.read","meta.owner-read","meta.owner-set","meta.authorize","meta.client-budget","meta.tool-budget","meta.usage","meta.calls","meta.oauth-clients"];
 features.extend(["credits","access","config","lifecycle","reports","actor","meta","actors.personal"]);
 if s.store.market_profile.is_some(){features.extend(["oauth.enabled","actors.oauth"]);}
 Ok(serde_json::json!({"protocol_major":2,"unit":"credit_500k_usd","features":features}))
}.await)
}
pub fn router(store: CreditStore, token: &[u8]) -> Result<Router> {
    if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
        return Err(Error::Invalid);
    }
    let state = Service {
        store,
        token: Sha256::digest(token).into(),
        slots: Arc::new(Semaphore::new(4)),
    };
    Ok(Router::new()
        .route(
            "/internal/tool-market/v2/meta/oauth-clients",
            post(meta_oauth_clients),
        )
        .route("/internal/tool-market/v2/meta/read", post(meta_read))
        .route(
            "/internal/tool-market/v2/meta/owner-read",
            post(meta_owner_read),
        )
        .route(
            "/internal/tool-market/v2/meta/owner-set",
            post(meta_owner_set),
        )
        .route(
            "/internal/tool-market/v2/meta/authorize",
            post(meta_authorize),
        )
        .route(
            "/internal/tool-market/v2/meta/client-budget",
            post(meta_client_budget),
        )
        .route(
            "/internal/tool-market/v2/meta/tool-budget",
            post(meta_tool_budget),
        )
        .route("/internal/tool-market/v2/meta/usage", post(meta_usage))
        .route("/internal/tool-market/v2/meta/calls", post(meta_calls))
        .route(
            "/internal/tool-market/v2/reports/create",
            post(report_create),
        )
        .route(
            "/internal/tool-market/v2/reports/review",
            post(report_review),
        )
        .route("/internal/tool-market/v2/reports/list", post(report_list))
        .route("/internal/tool-market/v2/income/list", post(income_list))
        .route("/internal/tool-market/v2/capabilities", post(capabilities))
        .route(
            "/internal/tool-market/v2/services/lifecycle",
            post(service_lifecycle),
        )
        .route("/internal/tool-market/v2/quotes", post(quote))
        .route("/internal/tool-market/v2/quotes/approve", post(approve))
        .route(
            "/internal/tool-market/v2/access/consent-create",
            post(consent_create),
        )
        .route(
            "/internal/tool-market/v2/access/consent-revoke",
            post(consent_revoke),
        )
        .route(
            "/internal/tool-market/v2/access/budget-set",
            post(budget_set),
        )
        .route(
            "/internal/tool-market/v2/access/token-create",
            post(token_create),
        )
        .route(
            "/internal/tool-market/v2/access/token-revoke",
            post(token_revoke),
        )
        .route(
            "/internal/tool-market/v2/access/token-validate",
            post(token_validate),
        )
        .route(
            "/internal/tool-market/v2/access/client-disconnect",
            post(disconnect),
        )
        .route(
            "/internal/tool-market/v2/access/consent-list",
            post(consent_list),
        )
        .route(
            "/internal/tool-market/v2/access/budget-list",
            post(budget_list),
        )
        .route(
            "/internal/tool-market/v2/access/token-list",
            post(token_list),
        )
        .route("/internal/tool-market/v2/calls/reserve", post(reserve))
        .route("/internal/tool-market/v2/calls/dispatch", post(dispatch))
        .route(
            "/internal/tool-market/v2/calls/settle",
            post(settle).layer(DefaultBodyLimit::max(2_850_000)),
        )
        .route("/internal/tool-market/v2/calls/unknown", post(unknown))
        .route(
            "/internal/tool-market/v2/calls/release",
            post(release).layer(DefaultBodyLimit::max(2_850_000)),
        )
        .route("/internal/tool-market/v2/calls/query", post(query))
        .route("/internal/tool-market/v2/config/read", post(config_read))
        .route(
            "/internal/tool-market/v2/config/update",
            post(config_update),
        )
        .layer(DefaultBodyLimit::max(32768))
        .with_state(state))
}
