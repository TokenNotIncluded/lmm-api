//! Complete bounded CSV exports of the same self/admin sources as the log page.
//! Database rows are streamed into a capped document before returning headers:
//! invalid receipts or an exceeded limit cannot become a successful partial CSV.
use super::*;
use crate::history_read::{HistoryError, HistoryQuery, HistoryStore};
use axum::body::{Body, Bytes};
use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::{Context, Poll},
    time::Duration,
};
use tokio_stream::Stream;

const MAX_ROWS: usize = 10_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ACTIVE: usize = 4;
const EXPORT_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
enum ExportError {
    Unauthorized,
    Forbidden,
    Invalid,
    TooDense,
    Unavailable,
    Busy,
    Timeout,
}
impl From<sqlx::Error> for ExportError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<IdentityError> for ExportError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            _ => Self::Unavailable,
        }
    }
}
impl From<Error> for ExportError {
    fn from(error: Error) -> Self {
        match error {
            Error::Identity(error) => error.into(),
            Error::Invalid => Self::Invalid,
            Error::TooManyBuckets => Self::TooDense,
            _ => Self::Unavailable,
        }
    }
}
impl From<HistoryError> for ExportError {
    fn from(error: HistoryError) -> Self {
        match error {
            HistoryError::Unauthorized => Self::Unauthorized,
            HistoryError::Forbidden => Self::Forbidden,
            HistoryError::Invalid => Self::Invalid,
            HistoryError::TooDense => Self::TooDense,
            HistoryError::Unavailable => Self::Unavailable,
        }
    }
}

/// Shared by both export routes, including different browser sessions belonging
/// to the same authenticated user. No credential or client-supplied ID is a key.
struct ExportGate {
    active: Mutex<HashSet<i64>>,
    limit: usize,
}
impl ExportGate {
    fn new(limit: usize) -> Self {
        Self {
            active: Mutex::new(HashSet::new()),
            limit,
        }
    }
    fn acquire(self: &Arc<Self>, user: i64) -> Result<ExportLease, ExportError> {
        let mut active = self.active.lock().map_err(|_| ExportError::Unavailable)?;
        if active.len() >= self.limit || !active.insert(user) {
            return Err(ExportError::Busy);
        }
        Ok(ExportLease {
            gate: self.clone(),
            user,
        })
    }
}
struct ExportLease {
    gate: Arc<ExportGate>,
    user: i64,
}
impl Drop for ExportLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.gate.active.lock() {
            active.remove(&self.user);
        }
    }
}
fn acquire(user: i64) -> Result<ExportLease, ExportError> {
    static GATE: OnceLock<Arc<ExportGate>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(ExportGate::new(MAX_ACTIVE)))
        .acquire(user)
}

struct Csv {
    bytes: Vec<u8>,
    limit: usize,
}
impl Csv {
    fn new() -> Self {
        Self {
            bytes: b"\xef\xbb\xbf".to_vec(),
            limit: MAX_BYTES,
        }
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), ExportError> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > self.limit)
        {
            return Err(ExportError::TooDense);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    /// RFC 4180 quoting, including embedded quotes/newlines and empty unknowns.
    fn row(&mut self, cells: &[String]) -> Result<(), ExportError> {
        for (index, cell) in cells.iter().enumerate() {
            if index != 0 {
                self.append(b",")?;
            }
            self.append(b"\"")?;
            for part in cell.split_inclusive('"') {
                self.append(part.as_bytes())?;
                if part.ends_with('"') {
                    self.append(b"\"")?;
                }
            }
            self.append(b"\"")?;
        }
        self.append(b"\r\n")
    }
    fn header(&mut self, cells: &[&str]) -> Result<(), ExportError> {
        self.row(&cells.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }
}
/// Do not allow names/IDs to become formulas after spreadsheet whitespace or
/// BOM trimming. Numeric cells are emitted separately from verified integers.
fn text(value: &str) -> String {
    let trimmed =
        value.trim_start_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\u{feff}');
    if trimmed.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}
fn optional<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}
fn window(start: Option<i64>, end: Option<i64>) -> Result<(), ExportError> {
    match (start, end) {
        (Some(start), Some(end))
            if start >= 0 && end >= start && end <= 253402300799 && end - start <= WINDOW =>
        {
            Ok(())
        }
        _ => Err(ExportError::Invalid),
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str, ExportError> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(ExportError::Unauthorized);
    }
    let (scheme, secret) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .ok_or(ExportError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(ExportError::Unauthorized);
    }
    Ok(secret)
}

struct Document {
    csv: Csv,
    rows: usize,
    source: &'static str,
    filename: &'static str,
    lease: ExportLease,
}
// A bounded sender owns the full document and admission lease. Disconnects and
// a stalled download release both; queued chunks copy at most 128 KiB rather
// than retaining the entire backing allocation after the deadline.
struct CsvBody {
    receiver: tokio::sync::mpsc::Receiver<Bytes>,
    remaining: usize,
    ended: bool,
}
fn body(bytes: Vec<u8>, lease: ExportLease, deadline: Duration) -> CsvBody {
    let remaining = bytes.len();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let _lease = lease;
        let _ = tokio::time::timeout(deadline, async {
            for chunk in bytes.chunks(CHUNK_BYTES) {
                if sender.send(Bytes::copy_from_slice(chunk)).await.is_err() {
                    return;
                }
            }
        })
        .await;
    });
    CsvBody {
        receiver,
        remaining,
        ended: false,
    }
}
impl Stream for CsvBody {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(bytes)) => {
                self.remaining -= bytes.len();
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(None) => {
                self.ended = true;
                Poll::Ready((self.remaining != 0).then(|| {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "usage CSV transfer did not complete",
                    ))
                }))
            }
        }
    }
}
fn response(result: Result<Document, ExportError>) -> Response {
    match result {
        Ok(doc) => {
            let size = doc.csv.bytes.len();
            let mut response =
                Body::from_stream(body(doc.csv.bytes, doc.lease, EXPORT_TIMEOUT)).into_response();
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/csv; charset=utf-8"),
            );
            headers.insert(
                header::CONTENT_DISPOSITION,
                header::HeaderValue::from_str(&format!(
                    "attachment; filename=\"{}\"",
                    doc.filename
                ))
                .unwrap(),
            );
            headers.insert(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_str(&size.to_string()).unwrap(),
            );
            headers.insert(
                header::HeaderName::from_static("x-export-bytes"),
                header::HeaderValue::from_str(&size.to_string()).unwrap(),
            );
            headers.insert(
                header::HeaderName::from_static("x-export-rows"),
                header::HeaderValue::from_str(&doc.rows.to_string()).unwrap(),
            );
            headers.insert(
                header::HeaderName::from_static("x-export-source"),
                header::HeaderValue::from_static(doc.source),
            );
            headers.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                header::HeaderValue::from_static("nosniff"),
            );
            response
        }
        Err(error) => {
            let (status, code, message) = match error {
                ExportError::Unauthorized => (
                    StatusCode::UNAUTHORIZED,
                    "AUTH_UNAUTHORIZED",
                    "Authentication is required",
                ),
                ExportError::Forbidden => (
                    StatusCode::FORBIDDEN,
                    "USAGE_EXPORT_FORBIDDEN",
                    "This session cannot export the selected usage scope",
                ),
                ExportError::Invalid => (
                    StatusCode::BAD_REQUEST,
                    "USAGE_EXPORT_INVALID",
                    "Select an explicit time window of at most 31 days; pagination is not accepted",
                ),
                ExportError::TooDense => (
                    StatusCode::BAD_REQUEST,
                    "USAGE_EXPORT_TOO_DENSE",
                    "Use narrower filters: export is limited to 10000 records and 32 MiB",
                ),
                ExportError::Busy => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "USAGE_EXPORT_BUSY",
                    "Another usage export is active; retry after it finishes",
                ),
                ExportError::Timeout => (
                    StatusCode::GATEWAY_TIMEOUT,
                    "USAGE_EXPORT_TIMEOUT",
                    "Usage export exceeded its time limit",
                ),
                ExportError::Unavailable => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "USAGE_EXPORT_UNAVAILABLE",
                    "Complete validated usage is temporarily unavailable",
                ),
            };
            let mut response = (
                status,
                Json(json!({"success":false,"code":code,"message":message})),
            )
                .into_response();
            if error == ExportError::Busy {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, header::HeaderValue::from_static("2"));
            }
            response
        }
    }
}

pub(super) async fn self_export(
    State(store): State<UsageStore>,
    headers: HeaderMap,
    query: Result<Query<UsageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let secret = bearer(&headers)?;
        let Query(query) = query.map_err(|_| ExportError::Invalid)?;
        tokio::time::timeout(EXPORT_TIMEOUT, native(&store, secret, query))
            .await
            .map_err(|_| ExportError::Timeout)?
    }
    .await;
    response(result)
}

async fn native(
    store: &UsageStore,
    secret: &str,
    query: UsageQuery,
) -> Result<Document, ExportError> {
    if query.p.is_some() || query.page_size.is_some() || query.default_time.is_some() {
        return Err(ExportError::Invalid);
    }
    window(query.start_timestamp, query.end_timestamp)?;
    let mut tx = store.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='10s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *tx)
        .await?;
    // Exactly the native log page's current credential and owner gate.
    let actor = identity::authorize_in_transaction(&mut tx, secret).await?;
    if actor.credential_kind != CredentialKind::Session {
        return Err(ExportError::Forbidden);
    }
    safe(actor.user_id)?;
    let lease = acquire(actor.user_id)?;
    let owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=$1")
            .bind(actor.user_id)
            .fetch_one(&mut *tx)
            .await?;
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&mut *tx)
            .await?;
    let filter = query.validate(now)?;
    let mut csv = Csv::new();
    csv.header(&[
        "request_id",
        "user_id",
        "created_at_unix",
        "type",
        "model_name",
        "group",
        "token_id",
        "token_name",
        "state",
        "pricing_dimension",
        "prompt_tokens",
        "completion_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "cache_write_5m_tokens",
        "cache_write_1h_tokens",
        "cache_write_12h_tokens",
        "search_provider",
        "search_calls",
        "search_queries",
        "search_grounded_requests",
        "search_x_posts_fetched",
        "search_x_users_fetched",
        "embedding_provider",
        "embedding_text_tokens",
        "embedding_image_tokens",
        "embedding_total_input_tokens",
        "output_image_count",
        "reserved_credits",
        "gross_settled_credits",
        "refunded_credits",
        "net_consumption_credits",
        "net_consumption_usd",
        "money_available",
        "billing_source",
        "payer_kind",
        "platform_sponsored",
    ])?;
    let sql = format!(
        "SELECT * FROM ({SELECT_REQUESTS}) export_requests ORDER BY authorized_at DESC,id DESC LIMIT {}",
        MAX_ROWS + 1
    );
    let mut count = 0;
    {
        let mut rows = sqlx::query(&sql)
            .bind(actor.user_id)
            .bind(owner)
            .bind(filter.start)
            .bind(filter.end)
            .bind(query.r#type.unwrap_or(0))
            .bind(query.model_name.unwrap_or_default())
            .bind(query.token_name.unwrap_or_default())
            .bind(query.group.unwrap_or_default())
            .bind(query.request_id.unwrap_or_default())
            .fetch(&mut *tx);
        while let Some(row) = rows.next().await {
            count += 1;
            if count > MAX_ROWS {
                return Err(ExportError::TooDense);
            }
            native_row(&mut csv, &ValidatedRequest::from_row(row?)?)?;
        }
    }
    tx.commit().await?;
    Ok(Document {
        csv,
        rows: count,
        source: "native_personal_usage",
        filename: "lmm-personal-usage.csv",
        lease,
    })
}

fn native_row(csv: &mut Csv, request: &ValidatedRequest) -> Result<(), ExportError> {
    let settled = request.settled();
    let image = request.price.image.is_some();
    let tokens = settled && !image;
    let created = request.usage.cache_creation.filter(|_| tokens);
    let [search_provider, calls, queries, grounded, posts, users] =
        search_cells(request.usage.search.filter(|_| tokens))?;
    let embedding = request.usage.embedding.filter(|_| tokens);
    let embedding_provider = embedding
        .map(|value| {
            serde_json::to_value(value.provider)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or(ExportError::Unavailable)
        })
        .transpose()?
        .unwrap_or_default();
    csv.row(&[
        text(&request.id),
        request.user.to_string(),
        request.at.to_string(),
        (if settled && request.outcome.as_deref() == Some("completed") {
            "2"
        } else {
            "5"
        })
        .to_owned(),
        text(&request.price.model),
        text(&request.price.group),
        optional(request.key),
        text(&request.token_name),
        text(&request.state),
        (if image { "image" } else { "text" }).to_owned(),
        optional(tokens.then_some(request.usage.input)),
        optional(tokens.then_some(request.usage.output)),
        optional(tokens.then_some(request.usage.cached)),
        optional(created.map(|v| v.total)),
        optional(created.and_then(|v| v.five_minute)),
        optional(created.and_then(|v| v.one_hour)),
        optional(created.and_then(|v| v.twelve_hour)),
        search_provider,
        calls,
        queries,
        grounded,
        posts,
        users,
        embedding_provider,
        optional(embedding.and_then(|value| value.text_tokens)),
        optional(embedding.and_then(|value| value.image_tokens)),
        optional(embedding.and_then(|value| value.total_input_tokens)),
        optional(
            request
                .usage
                .image
                .filter(|_| settled)
                .map(|v| v.output_images),
        ),
        (if request.held() { request.reserved } else { 0 }).to_string(),
        optional(settled.then_some(request.amounts.gross)),
        optional(settled.then_some(request.amounts.refunded)),
        optional(settled.then_some(request.amounts.net())),
        settled
            .then(|| usd(request.amounts.net()))
            .unwrap_or_default(),
        settled.to_string(),
        (if request.subscription() {
            "subscription"
        } else {
            "wallet"
        })
        .to_owned(),
        text(&request.payer_kind),
        request.sponsored.to_string(),
    ])
}

fn search_cells(usage: Option<crate::routing::SearchUsage>) -> Result<[String; 6], ExportError> {
    let Some(usage) = usage else {
        return Ok(std::array::from_fn(|_| String::new()));
    };
    let provider = serde_json::to_value(usage.provider).map_err(|_| ExportError::Unavailable)?;
    Ok([
        text(provider.as_str().ok_or(ExportError::Unavailable)?),
        optional(usage.calls),
        optional(usage.queries),
        optional(usage.grounded_requests),
        optional(usage.x_posts_fetched),
        optional(usage.x_users_fetched),
    ])
}

pub(super) async fn admin_export(
    State(store): State<UsageStore>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let secret = bearer(&headers)?;
        let Query(query) = query.map_err(|_| ExportError::Invalid)?;
        tokio::time::timeout(EXPORT_TIMEOUT, historical(&store, secret, query))
            .await
            .map_err(|_| ExportError::Timeout)?
    }
    .await;
    response(result)
}
async fn historical(
    store: &UsageStore,
    secret: &str,
    query: HistoryQuery,
) -> Result<Document, ExportError> {
    if query.p.is_some() || query.page_size.is_some() {
        return Err(ExportError::Invalid);
    }
    window(query.start_timestamp, query.end_timestamp)?;
    let mut tx = store.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *tx)
        .await?;
    let actor = identity::authorize_admin_read_snapshot(&mut tx, secret)
        .await
        .map_err(|error| match error {
            identity::admin_users::AdminUsersError::Unauthorized => ExportError::Unauthorized,
            identity::admin_users::AdminUsersError::Forbidden => ExportError::Forbidden,
            _ => ExportError::Unavailable,
        })?;
    let lease = acquire(actor.user_id)?;
    tx.commit().await?;
    // This existing bounded read reauthenticates in its own read-only snapshot,
    // preserves administrator filters and requires installed complete history.
    let snapshot = HistoryStore::new(store.pool.clone())
        .export(secret, &query)
        .await?;
    if snapshot.items.len() > MAX_ROWS || snapshot.items.len() as i64 != snapshot.total {
        return Err(ExportError::TooDense);
    }
    let mut csv = Csv::new();
    csv.header(&[
        "source_log_id",
        "user_id",
        "created_at_unix",
        "type",
        "username",
        "token_id",
        "token_name",
        "model_name",
        "quota_raw",
        "prompt_tokens",
        "completion_tokens",
        "group",
        "request_id",
        "channel_id",
        "upstream_request_id",
        "quota_usd",
        "money_available",
        "money_basis",
    ])?;
    for row in snapshot.items {
        csv.row(&[
            row.source_log_id.to_string(),
            row.user_id.to_string(),
            row.created_at.to_string(),
            row.r#type.to_string(),
            text(&row.username),
            optional(row.token_id),
            text(&row.token_name),
            text(&row.model_name),
            optional(row.quota),
            optional(row.prompt_tokens),
            optional(row.completion_tokens),
            text(&row.group),
            text(&row.request_id),
            optional(row.channel),
            text(row.upstream_request_id.as_deref().unwrap_or("")),
            row.money.quota_usd.unwrap_or_default(),
            row.money.quota_money_available.to_string(),
            row.money.quota_money_basis.to_owned(),
        ])?;
    }
    Ok(Document {
        csv,
        rows: snapshot.total as usize,
        source: "legacy_log_only",
        filename: "lmm-historical-usage.csv",
        lease,
    })
}

#[cfg(test)]
#[path = "csv_export_tests.rs"]
mod tests;
