//! Only public content already consumed from core_console.options is writable.
//! Host TOML, credentials, model policy and financial options are never setters.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Single {
    key: String,
    value: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bulk {
    values: BTreeMap<String, String>,
    #[serde(default)]
    expected_values: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Announcement {
    id: i64,
    content: String,
    #[serde(rename = "publishDate")]
    publish_date: String,
    #[serde(default)]
    extra: String,
    #[serde(default, rename = "mandatory")]
    _mandatory: bool,
    #[serde(default, rename = "ackRevision")]
    ack_revision: String,
    #[serde(default, rename = "type")]
    _kind: Option<AnnouncementKind>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AnnouncementKind {
    Default,
    Ongoing,
    Success,
    Warning,
    Error,
}

enum Operation {
    Single,
    Bulk,
    Validate,
}

enum Error {
    Auth(IdentityError),
    Storage,
    Invalid(&'static str),
    Unsupported,
    Conflict,
}

impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}

fn canonical(key: &str) -> Result<&str, Error> {
    match key {
        "Notice" => Ok("Notice"),
        "Announcements" | "console_setting.announcements" => Ok("console_setting.announcements"),
        _ => Err(Error::Unsupported),
    }
}

fn normalize(values: BTreeMap<String, String>) -> Result<BTreeMap<String, String>, Error> {
    let mut normalized = BTreeMap::new();
    for (key, value) in values {
        let key = canonical(&key)?.to_owned();
        if normalized.get(&key).is_some_and(|old| old != &value) {
            return Err(Error::Invalid(
                "Announcement aliases contain conflicting values",
            ));
        }
        normalized.insert(key, value);
    }
    Ok(normalized)
}

fn validate_values(values: &BTreeMap<String, String>) -> Result<(), Error> {
    // @context-fold begin validate-public-option-content
    if values.is_empty() {
        return Err(Error::Invalid("At least one option is required"));
    }
    for (key, value) in values {
        if value.len() > 256 * 1024 {
            return Err(Error::Invalid("Option content exceeds 256 KiB"));
        }
        if key != "console_setting.announcements" {
            continue;
        }
        let items: Vec<Announcement> = serde_json::from_str(value).map_err(|_| {
            Error::Invalid("Announcements must be a JSON array of announcement objects")
        })?;
        let mut ids = BTreeSet::new();
        for item in items {
            if item.id <= 0
                || !ids.insert(item.id)
                || item.content.trim().is_empty()
                || item.content.chars().count() > 500
                || item.extra.chars().count() > 100
                || item.ack_revision.len() > 32
                || !item
                    .ack_revision
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                || chrono::DateTime::parse_from_rfc3339(&item.publish_date).is_err()
            {
                return Err(Error::Invalid(
                    "Invalid announcement ID, content or publication date",
                ));
            }
        }
    }
    Ok(())
    // @context-fold end validate-public-option-content
}

pub(super) async fn write(
    State(state): State<OptionsHttp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle(state, headers, body, Operation::Single).await
}

pub(super) async fn bulk(
    State(state): State<OptionsHttp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle(state, headers, body, Operation::Bulk).await
}

pub(super) async fn validate(
    State(state): State<OptionsHttp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle(state, headers, body, Operation::Validate).await
}

async fn handle(state: OptionsHttp, headers: HeaderMap, body: Bytes, op: Operation) -> Response {
    match apply(&state, &headers, &body, op).await {
        Ok(body) => response(StatusCode::OK, body),
        Err(Error::Auth(error)) => auth_error(error),
        Err(error) => {
            let (status, code, message) = match error {
                Error::Storage => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "SYSTEM_OPTIONS_STORAGE_UNAVAILABLE",
                    "Settings could not be saved; no changes were committed",
                ),
                Error::Unsupported => (
                    StatusCode::CONFLICT,
                    "SYSTEM_OPTION_NOT_WRITABLE",
                    "Only Notice and announcement content are writable; host configuration, secrets and unsupported options were not saved",
                ),
                Error::Conflict => (
                    StatusCode::CONFLICT,
                    "SYSTEM_OPTIONS_CONFLICT",
                    "Settings changed since they were loaded; refresh before saving",
                ),
                Error::Invalid(message) => {
                    (StatusCode::BAD_REQUEST, "SYSTEM_OPTIONS_INVALID", message)
                }
                Error::Auth(_) => unreachable!(),
            };
            response(
                status,
                json!({"success":false,"code":code,"message":message}),
            )
        }
    }
}

async fn apply(
    state: &OptionsHttp,
    headers: &HeaderMap,
    body: &[u8],
    op: Operation,
) -> Result<serde_json::Value, Error> {
    let secret = bearer(headers).map_err(Error::Auth)?;
    let mut tx = state.pool.begin().await?;
    // Keep current session/user authority locked until settings AND audit commit.
    let actor = crate::identity::authorize_in_transaction(&mut tx, secret)
        .await
        .map_err(Error::Auth)?;
    if actor.credential_kind != CredentialKind::Session || actor.platform_level < 6 {
        return Err(Error::Auth(IdentityError::Forbidden));
    }
    let request = match op {
        Operation::Single => {
            let single: Single = serde_json::from_slice(body)
                .map_err(|_| Error::Invalid("Expected key and string value"))?;
            canonical(&single.key)?;
            let value = single.value.as_str().ok_or(Error::Invalid(
                "Public content options require string values",
            ))?;
            Bulk {
                values: BTreeMap::from([(single.key, value.to_owned())]),
                expected_values: None,
            }
        }
        Operation::Bulk | Operation::Validate => serde_json::from_slice(body).map_err(|_| {
            Error::Invalid("Expected values and optional expected_values string maps")
        })?,
    };
    let values = normalize(request.values)?;
    validate_values(&values)?;
    let expected = request.expected_values.map(normalize).transpose()?;
    if expected
        .as_ref()
        .is_some_and(|map| map.keys().any(|key| !values.contains_key(key)))
    {
        return Err(Error::Invalid(
            "Expected values must refer to options being saved",
        ));
    }
    // Also serialize absent-row inserts and announcement aliases before CAS.
    sqlx::query("LOCK TABLE core_console.options IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let stored: BTreeMap<String, (String, i64)> = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT key,value,revision FROM core_console.options
             WHERE key IN ('Notice','Announcements','console_setting.announcements')",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(key, value, revision)| (key, (value, revision)))
    .collect();
    if let Some(expected) = expected {
        for (key, value) in expected {
            let current = stored.get(&key).or_else(|| {
                (key == "console_setting.announcements")
                    .then(|| stored.get("Announcements"))
                    .flatten()
            });
            let default = if key == "Notice" { "" } else { "[]" };
            if current.map_or(default, |(value, _)| value.as_str()) != value {
                return Err(Error::Conflict);
            }
        }
    }
    if matches!(op, Operation::Validate) {
        tx.rollback().await?;
        return Ok(json!({"success":true,"message":"Validated; no changes saved","saved":false}));
    }
    let mut changes = Vec::new();
    for (key, value) in values {
        let keys: &[&str] = if key == "Notice" {
            &["Notice"]
        } else {
            &["Announcements", "console_setting.announcements"]
        };
        for key in keys {
            if stored.get(*key).is_some_and(|(old, _)| old == &value) {
                continue;
            }
            let revision: i64 = sqlx::query_scalar(
                "INSERT INTO core_console.options(key,value) VALUES ($1,$2)
                 ON CONFLICT (key) DO UPDATE SET value=EXCLUDED.value,
                 revision=core_console.options.revision+1,updated_at=statement_timestamp()
                 RETURNING revision",
            )
            .bind(key)
            .bind(&value)
            .fetch_one(&mut *tx)
            .await?;
            changes.push(json!({
                "key":key,"before":stored.get(*key),"after":[value,revision]
            }));
        }
    }
    if !changes.is_empty() {
        sqlx::query(
            "INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target)
             VALUES ($1,$2,'console.options.update',$3)",
        )
        .bind(actor.user_id)
        .bind(actor.credential_id)
        .bind(json!({"changes":changes}))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(json!({"success":true,"message":"Settings saved","saved":true}))
}

#[cfg(test)]
mod tests;
