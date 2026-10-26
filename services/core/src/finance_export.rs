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
            // spreadsheet formulas, including after leading whitespace, BOMs
            // or control characters. Preserve the original cell text.
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
    let trimmed =
        value.trim_start_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\u{feff}');
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

#[cfg(test)]
mod csv_formula_independent_tests {
    use super::*;
    use crate::history_read::{HistorySummary, projection::MoneyProjection};

    const TEXT_FIELDS: [&str; 5] = [
        "username",
        "token_name",
        "model_name",
        "group",
        "request_id",
    ];
    const PREFIXES: [&str; 14] = [
        "",
        " ",
        "\t",
        "\r",
        "\n",
        "\u{0001}",
        "\u{001f}",
        "\u{0085}",
        "\u{009f}",
        "\u{00a0}",
        "\u{2003}",
        "\u{2028}",
        "\u{feff}",
        " \u{feff}\u{009f}\u{2003}",
    ];

    fn snapshot(field: &str, text: &str) -> HistorySnapshot {
        let mut row = HistoryRow {
            source_log_id: 17,
            user_id: 7,
            token_id: Some(9),
            created_at: 1_700_000_000,
            r#type: 3,
            username: "normal-user".into(),
            token_name: "normal-token".into(),
            model_name: "normal-model".into(),
            quota: Some(-17),
            prompt_tokens: Some(11),
            completion_tokens: Some(13),
            group: "normal-group".into(),
            request_id: "normal-request".into(),
            channel: None,
            upstream_request_id: None,
            money: MoneyProjection::unknown("not_consumption"),
        };
        match field {
            "username" => row.username = text.into(),
            "token_name" => row.token_name = text.into(),
            "model_name" => row.model_name = text.into(),
            "group" => row.group = text.into(),
            "request_id" => row.request_id = text.into(),
            _ => unreachable!("test field is one of TEXT_FIELDS"),
        }
        HistorySnapshot {
            contract: crate::history_read::CONTRACT,
            snapshot_at: 1_700_000_001,
            items: vec![row],
            total: 1,
            page: 1,
            page_size: 20,
            summary: HistorySummary {
                raw_consumption_quota: "0".into(),
                invalid_quota_records: 0,
                unpriced_quota_records: 0,
                quota_projection_truncated: false,
                money: MoneyProjection::unknown("not_consumption"),
            },
            source: "legacy_log_only",
        }
    }

    fn csv(snapshot: HistorySnapshot) -> String {
        String::from_utf8(encode(snapshot, ExportFormat::Csv).unwrap().bytes).unwrap()
    }

    #[test]
    fn formula_markers_after_unicode_bom_and_control_prefixes_are_protected() {
        for prefix in PREFIXES {
            for marker in ['=', '+', '-', '@'] {
                let original = format!("{prefix}{marker}synthetic");
                assert_eq!(safe_text(&original), format!("'{original}"), "{original:?}");
            }
        }
    }

    #[test]
    fn all_csv_text_fields_are_protected_while_json_preserves_original_text() {
        for field in TEXT_FIELDS {
            for prefix in PREFIXES {
                for marker in ['=', '+', '-', '@'] {
                    let original = format!("{prefix}{marker}synthetic");
                    let document = csv(snapshot(field, &original));
                    assert!(
                        document.contains(&format!("\"'{original}\"")),
                        "{field} was not protected: {original:?}"
                    );
                    let json = encode(snapshot(field, &original), ExportFormat::Json).unwrap();
                    let value: serde_json::Value = serde_json::from_slice(&json.bytes).unwrap();
                    assert_eq!(value["usage"][0][field], original, "{field}");
                    assert_eq!(value["complete_financial_export"], false);
                }
            }
        }
    }

    #[test]
    fn ordinary_and_already_apostrophe_prefixed_text_remains_unchanged() {
        for original in [
            "正常中文",
            "ordinary",
            "  ordinary",
            "\u{00a0}ordinary",
            "\u{feff}ordinary",
            "'=synthetic",
            "'@synthetic",
            "\u{feff}'=synthetic",
            "\u{009f}'-synthetic",
        ] {
            assert_eq!(safe_text(original), original);
            for field in TEXT_FIELDS {
                let document = csv(snapshot(field, original));
                assert!(
                    document.contains(&format!("\"{original}\"")),
                    "{field}, {original:?}"
                );
                let json = encode(snapshot(field, original), ExportFormat::Json).unwrap();
                let value: serde_json::Value = serde_json::from_slice(&json.bytes).unwrap();
                assert_eq!(value["usage"][0][field], original);
            }
        }
    }

    #[test]
    fn text_guard_does_not_change_typed_numbers_or_money_availability() {
        let document = csv(snapshot("username", "\u{feff}=synthetic"));
        assert!(document.contains(",\"-17\",\"11\",\"13\","));
        assert!(!document.contains("\"'-17\""));
        assert!(document.ends_with(",\"\",\"false\",\"not_consumption\"\r\n"));
        let json = encode(
            snapshot("username", "\u{feff}=synthetic"),
            ExportFormat::Json,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json.bytes).unwrap();
        assert_eq!(value["usage"][0]["quota"], -17);
        assert_eq!(value["usage"][0]["prompt_tokens"], 11);
        assert_eq!(value["usage"][0]["completion_tokens"], 13);
        assert_eq!(value["usage"][0]["quota_money_available"], false);
        assert!(value["usage"][0].get("quota_usd").is_none());

        let mut known = snapshot("username", "ordinary");
        known.items[0].quota = Some(25);
        known.items[0].r#type = 2;
        known.items[0].money =
            crate::history_read::projection::log_money(Some(25), 2, Some(500_000));
        known.summary.raw_consumption_quota = "25".into();
        known.summary.money = known.items[0].money.clone();
        let document = csv(known);
        assert!(document.contains(",\"25\",\"11\",\"13\","));
        assert!(document.ends_with(",\"0.00005\",\"true\",\"captured_credits_per_usd\"\r\n"));
    }
}
