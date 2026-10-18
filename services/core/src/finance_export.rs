//! Bounded, allowlisted usage export from the same authenticated historical
//! snapshot. This is the usage family; other financial families are not claimed.
use crate::history_read::{HistoryError, HistoryQuery, HistoryRow, HistorySnapshot, HistoryStore};
use serde::Serialize;
use std::io::{self, Write};

pub const MAX_EXPORT_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    Json,
    Csv,
}
pub struct ExportDocument {
    pub content_type: &'static str,
    pub filename: &'static str,
    pub bytes: Vec<u8>,
}

/// Core authenticates an administrator in the read transaction. A caller cannot
/// supply preselected user rows or a supposedly authenticated boolean.
pub async fn export(
    store: &HistoryStore,
    secret: &str,
    query: &HistoryQuery,
    format: ExportFormat,
) -> Result<ExportDocument, HistoryError> {
    let snapshot = store.export(secret, query).await?;
    encode(snapshot, format)
}

#[derive(Serialize)]
struct JsonExport<'a> {
    schema_version: &'static str,
    source: &'static str,
    snapshot_at: i64,
    total: i64,
    complete_financial_export: bool,
    included_families: [&'static str; 1],
    summary: &'a crate::history_read::HistorySummary,
    usage: &'a [HistoryRow],
}
struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_EXPORT_BYTES)
        {
            return Err(io::Error::other("export size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// No opaque `other`, raw request/provider payload, IP, credentials or source
/// archive/mapping metadata is a member of HistoryRow's public serialization.
pub(crate) fn encode(
    snapshot: HistorySnapshot,
    format: ExportFormat,
) -> Result<ExportDocument, HistoryError> {
    if snapshot.items.len() as i64 != snapshot.total || snapshot.summary.quota_projection_truncated
    {
        return Err(HistoryError::TooDense);
    }
    let mut out = BoundedBytes(Vec::new());
    let (content_type, filename) = match format {
        ExportFormat::Json => {
            serde_json::to_writer(
                &mut out,
                &JsonExport {
                    schema_version: "core.finance-usage-export.v1",
                    source: snapshot.source,
                    snapshot_at: snapshot.snapshot_at,
                    total: snapshot.total,
                    complete_financial_export: false,
                    included_families: ["historical_usage"],
                    summary: &snapshot.summary,
                    usage: &snapshot.items,
                },
            )
            .map_err(|_| HistoryError::TooDense)?;
            (
                "application/json; charset=utf-8",
                "lmm-historical-usage.json",
            )
        }
        ExportFormat::Csv => {
            // RFC 4180 escaping, UTF-8 and CRLF. Text cells cannot become
            // spreadsheet formulas, including after leading ASCII whitespace.
            write_csv(
                &mut out,
                &[
                    "source_log_id",
                    "user_id",
                    "created_at",
                    "type",
                    "username",
                    "token_name",
                    "model_name",
                    "quota",
                    "prompt_tokens",
                    "completion_tokens",
                    "group",
                    "request_id",
                    "quota_usd",
                    "quota_money_available",
                    "quota_money_basis",
                ],
            )?;
            for r in snapshot.items {
                let cells = [
                    r.source_log_id.to_string(),
                    r.user_id.to_string(),
                    r.created_at.to_string(),
                    r.r#type.to_string(),
                    safe_text(&r.username),
                    safe_text(&r.token_name),
                    safe_text(&r.model_name),
                    optional(r.quota),
                    optional(r.prompt_tokens),
                    optional(r.completion_tokens),
                    safe_text(&r.group),
                    safe_text(&r.request_id),
                    r.money.quota_usd.unwrap_or_default(),
                    r.money.quota_money_available.to_string(),
                    r.money.quota_money_basis.to_owned(),
                ];
                let refs: Vec<&str> = cells.iter().map(String::as_str).collect();
                write_csv(&mut out, &refs)?;
            }
            ("text/csv; charset=utf-8", "lmm-historical-usage.csv")
        }
    };
    Ok(ExportDocument {
        content_type,
        filename,
        bytes: out.0,
    })
}
fn optional(value: Option<i64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}
fn safe_text(value: &str) -> String {
    let trimmed = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if trimmed.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}
fn write_csv(out: &mut BoundedBytes, cells: &[&str]) -> Result<(), HistoryError> {
    for (i, cell) in cells.iter().enumerate() {
        if i != 0 {
            out.write_all(b",").map_err(|_| HistoryError::TooDense)?;
        }
        out.write_all(b"\"").map_err(|_| HistoryError::TooDense)?;
        for part in cell.split_inclusive('"') {
            out.write_all(part.as_bytes())
                .map_err(|_| HistoryError::TooDense)?;
            if part.ends_with('"') {
                out.write_all(b"\"").map_err(|_| HistoryError::TooDense)?;
            }
        }
        out.write_all(b"\"").map_err(|_| HistoryError::TooDense)?;
    }
    out.write_all(b"\r\n").map_err(|_| HistoryError::TooDense)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_delimiters_and_newlines_and_blocks_formulas() {
        for value in ["=1+1", "+SUM(A1)", " -1+2", "\t@SUM(A1)", "\r=cmd"] {
            assert!(safe_text(value).starts_with('\''));
        }
        assert_eq!(safe_text("正常中文"), "正常中文");
        let mut out = BoundedBytes(Vec::new());
        write_csv(&mut out, &["a,b", "x\"y", "line\nnext", ""]).unwrap();
        assert_eq!(
            String::from_utf8(out.0).unwrap(),
            "\"a,b\",\"x\"\"y\",\"line\nnext\",\"\"\r\n"
        );
    }
    #[test]
    fn response_limit_is_a_hard_limit() {
        let mut out = BoundedBytes(vec![0; MAX_EXPORT_BYTES - 1]);
        assert!(out.write_all(b"ab").is_err());
        assert_eq!(out.0.len(), MAX_EXPORT_BYTES - 1);
    }
}
