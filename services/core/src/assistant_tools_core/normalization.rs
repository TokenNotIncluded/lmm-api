//! Closed per-tool normalization. Source arguments and normalized drafts have
//! separate digests; arbitrary extension payloads are never countersigned.
use super::{Error, context::VerifiedToolContext, redaction};
use crate::assistant_model::agent::FrozenProposal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
fn decode<T: for<'a> Deserialize<'a>>(v: &Value) -> Result<T, Error> {
    serde_json::from_value(v.clone()).map_err(|_| Error::Invalid)
}
fn count(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.chars().count())
}
fn sanitized(text: &str) -> String {
    redaction::content(text).trim().to_owned()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateSource {
    kind: String,
    title: String,
    body: String,
    visibility: Option<String>,
}
#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CreateDraft {
    kind: String,
    title: String,
    body: String,
    visibility: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateSource {
    issue_id: i64,
    revision: i64,
    status: String,
    visibility: String,
    #[serde(default)]
    note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateDraft {
    issue_id: i64,
    revision: i64,
    status: String,
    visibility: String,
    note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffSource {
    message: String,
    #[serde(default)]
    action: String,
    target_user_id: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffDraft {
    message: String,
    core_conversation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GreetingSource {
    language: String,
    template: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GreetingDraft {
    arguments: Value,
    expected_revision: i64,
}
fn valid_greeting(source: &GreetingSource) -> bool {
    if !matches!(
        source.language.as_str(),
        "en" | "zh" | "zh-TW" | "fr" | "ja" | "ru" | "vi"
    ) || source.template.chars().count() > 512
        || source.template.contains('\0')
    {
        return false;
    }
    static VARIABLE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VARIABLE
        .get_or_init(|| {
            regex::Regex::new(r"\$\$|\$[A-Za-z_][A-Za-z0-9_]*")
                .expect("constant greeting variable regex")
        })
        .find_iter(&source.template)
        .all(|m| {
            matches!(
                m.as_str(),
                "$$" | "$name" | "$time" | "$date" | "$weekday" | "$level" | "$site" | "$balance"
            )
        })
}
pub(crate) async fn validate(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
    original: &Value,
    draft: &Value,
) -> Result<(), Error> {
    match proof.tool_name() {
        "create_site_issue" => {
            let source: CreateSource = decode(original)?;
            let actual: CreateDraft = decode(draft)?;
            if !matches!(
                source.kind.as_str(),
                "bug" | "security" | "experience" | "feature"
            ) {
                return Err(Error::Invalid);
            }
            let visibility = if source.kind == "security" {
                "admin".to_owned()
            } else {
                source.visibility.unwrap_or_else(|| "admin".into())
            };
            let expected = CreateDraft {
                kind: source.kind,
                title: sanitized(&source.title),
                body: sanitized(&source.body),
                visibility,
            };
            if !matches!(expected.visibility.as_str(), "user" | "admin")
                || !count(&expected.title, 2, 128)
                || !count(&expected.body, 5, 4000)
                || actual != expected
            {
                return Err(Error::Conflict);
            }
        }
        "update_site_issue" => {
            let source: UpdateSource = decode(original)?;
            let actual: UpdateDraft = decode(draft)?;
            if source.issue_id <= 0
                || source.issue_id > 9_007_199_254_740_991
                || source.revision < 1
                || source.revision > 9_007_199_254_740_991
                || !matches!(
                    source.status.as_str(),
                    "open" | "triaged" | "in_progress" | "resolved" | "declined"
                )
                || !matches!(source.visibility.as_str(), "user" | "admin")
                || !matches!(actual.visibility.as_str(), "user" | "admin")
                || actual.issue_id != source.issue_id
                || actual.revision != source.revision
                || actual.status != source.status
                || (actual.visibility != source.visibility && actual.visibility != "admin")
                || actual.note != sanitized(&source.note)
                || !count(&actual.note, 0, 1500)
            {
                return Err(Error::Conflict);
            }
        }
        "request_human_support" => {
            let source: HandoffSource = decode(original)?;
            let actual: HandoffDraft = decode(draft)?;
            if !matches!(source.action.as_str(), "" | "support")
                || source
                    .target_user_id
                    .is_some_and(|id| id != context.user_id())
                || actual.message != sanitized(&source.message)
                || !count(&actual.message, 5, 2000)
            {
                return Err(Error::Conflict);
            }
            let conversation:String=sqlx::query_scalar("SELECT conversation_id FROM core_assistant.agent_turns WHERE id=$1 AND owner_user_id=$2 AND owner_account_id=$3").bind(proof.turn_id()).bind(context.user_id()).bind(context.account_id()).fetch_one(c).await?;
            if actual.core_conversation_id != conversation {
                return Err(Error::Forbidden);
            }
        }
        "set_overview_greeting" => {
            let source: GreetingSource = decode(original)?;
            let actual: GreetingDraft = decode(draft)?;
            if actual.arguments != *original
                || !(0..=9_007_199_254_740_991).contains(&actual.expected_revision)
                || !valid_greeting(&source)
            {
                return Err(Error::Conflict);
            }
        }
        // Policy edits require their own exact typed domain validator; until that
        // exists even root cannot countersign a changed normalized payload.
        "prepare_admin_site_policy_change" => return Err(Error::Unavailable),
        _ => return Err(Error::Forbidden),
    }
    Ok(())
}
