//! Immutable installer-reviewed result contracts. No remote schema resolution.
use super::*;
use serde_json::Value;
fn bounded_schema(v: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > 32 || *nodes > 2048 {
        return Err(Error::Invalid);
    }
    match v {
        Value::Object(m) => {
            for (key, v) in m {
                if matches!(key.as_str(), "$ref" | "$dynamicRef" | "$recursiveRef")
                    && !v.as_str().is_some_and(|s| s.starts_with('#'))
                {
                    return Err(Error::Forbidden);
                }
                if key == "$id" {
                    return Err(Error::Forbidden);
                }
                bounded_schema(v, depth + 1, nodes)?;
            }
        }
        Value::Array(a) => {
            for v in a {
                bounded_schema(v, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(super) fn validate_schema(schema: &Value, result: &Value) -> Result<()> {
    if serde_json::to_vec(schema)
        .map_err(|_| Error::Invalid)?
        .len()
        > 32768
    {
        return Err(Error::Invalid);
    }
    bounded_schema(schema, 0, &mut 0)?;
    // Deliberately no compiled-schema cache; regex compilation and DFA state
    // each have fixed bounds. Fancy/backtracking regex features are rejected.
    let validator = jsonschema::options()
        .with_pattern_options(
            jsonschema::PatternOptions::regex()
                .size_limit(65536)
                .dfa_size_limit(65536),
        )
        .build(schema)
        .map_err(|_| Error::Invalid)?;
    if !validator.is_valid(result) {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(super) fn validate_mcp(result: &Value) -> Result<()> {
    let m = result.as_object().ok_or(Error::Invalid)?;
    if m.keys().any(|k| {
        !matches!(
            k.as_str(),
            "content"
                | "structuredContent"
                | "isError"
                | "_meta"
                | "success"
                | "status"
                | "inputRequests"
                | "requestState"
        )
    }) {
        return Err(Error::Invalid);
    }
    if m.get("isError").is_some_and(|v| !v.is_boolean())
        || m.get("success").is_some_and(|v| !v.is_boolean())
        || m.get("status").is_some_and(|v| !v.is_string())
    {
        return Err(Error::Invalid);
    }
    if let Some(content) = m.get("content") {
        let a = content.as_array().ok_or(Error::Invalid)?;
        if a.len() > 1024 {
            return Err(Error::Invalid);
        }
        for block in a {
            let b = block.as_object().ok_or(Error::Invalid)?;
            let kind = b
                .get("type")
                .and_then(Value::as_str)
                .ok_or(Error::Invalid)?;
            let allowed: &[&str] = match kind {
                "text" => {
                    if !b.get("text").is_some_and(Value::is_string) {
                        return Err(Error::Invalid);
                    }
                    &["type", "text", "annotations", "_meta"]
                }
                "image" | "audio" => {
                    if !b.get("data").is_some_and(Value::is_string)
                        || !b.get("mimeType").is_some_and(Value::is_string)
                    {
                        return Err(Error::Invalid);
                    }
                    &["type", "data", "mimeType", "annotations", "_meta"]
                }
                "resource_link" => {
                    if !b.get("uri").is_some_and(Value::is_string)
                        || !b.get("name").is_some_and(Value::is_string)
                    {
                        return Err(Error::Invalid);
                    }
                    &[
                        "type",
                        "uri",
                        "name",
                        "description",
                        "mimeType",
                        "size",
                        "title",
                        "icons",
                        "annotations",
                        "_meta",
                    ]
                }
                "resource" => {
                    let resource = b
                        .get("resource")
                        .and_then(Value::as_object)
                        .ok_or(Error::Invalid)?;
                    if !resource.get("uri").is_some_and(Value::is_string)
                        || (!resource.get("text").is_some_and(Value::is_string)
                            && !resource.get("blob").is_some_and(Value::is_string))
                    {
                        return Err(Error::Invalid);
                    }
                    &["type", "resource", "annotations", "_meta"]
                }
                _ => return Err(Error::Invalid),
            };
            if b.keys().any(|k| !allowed.contains(&k.as_str())) {
                return Err(Error::Invalid);
            }
        }
    }
    if m.get("structuredContent").is_some_and(|v| !v.is_object()) {
        return Err(Error::Invalid);
    }
    if m.get("inputRequests")
        .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        || m.get("requestState")
            .is_some_and(|v| v.as_str().is_none_or(|s| !s.is_empty()))
    {
        return Err(Error::Pending);
    }
    Ok(())
}
pub(super) fn terminal_failure(result: &Value) -> Result<bool> {
    validate_mcp(result)?;
    let mut failed = false;
    for value in [Some(result), result.get("structuredContent")]
        .into_iter()
        .flatten()
    {
        if value.get("isError").and_then(Value::as_bool) == Some(true)
            || value.get("success").and_then(Value::as_bool) == Some(false)
        {
            failed = true;
        }
        if let Some(status) = value.get("status") {
            let status = status.as_str().ok_or(Error::Invalid)?.to_ascii_lowercase();
            match status.as_str() {
                "failed" | "failure" | "error" | "cancelled" => failed = true,
                "success" | "succeeded" | "completed" | "done" | "ok" => {}
                _ => return Err(Error::Pending),
            }
        }
    }
    Ok(failed)
}
impl super::credit_access::CreditStore {
    pub(super) async fn result_contract(
        tx: &mut Transaction<'_, Postgres>,
        quote: &str,
    ) -> Result<(String, String, Value)> {
        let row=sqlx::query("SELECT proof_mode,witness_role,output_schema,output_schema_digest FROM core_tool_market.credit_result_contracts WHERE quote_id=$1").bind(quote).fetch_optional(&mut **tx).await?.ok_or(Error::Forbidden)?;
        let Json(schema): Json<Value> = row.try_get("output_schema")?;
        if hash(&schema)? != row.try_get::<String, _>("output_schema_digest")? {
            return Err(Error::Forbidden);
        }
        bounded_schema(&schema, 0, &mut 0)?;
        if serde_json::to_vec(&schema)
            .map_err(|_| Error::Invalid)?
            .len()
            > 32768
        {
            return Err(Error::Invalid);
        }
        jsonschema::options()
            .with_pattern_options(
                jsonschema::PatternOptions::regex()
                    .size_limit(65536)
                    .dfa_size_limit(65536),
            )
            .build(&schema)
            .map_err(|_| Error::Invalid)?;
        Ok((
            row.try_get("proof_mode")?,
            row.try_get("witness_role")?,
            schema,
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_mcp_blocks_and_bounded_local_schema() {
        for v in [
            serde_json::json!({"content":[1]}),
            serde_json::json!({"content":[{"type":"text","text":1}]}),
            serde_json::json!({"content":[{"type":"rogue"}]}),
        ] {
            assert!(validate_mcp(&v).is_err());
        }
        assert!(
            validate_mcp(&serde_json::json!({"content":[{"type":"text","text":"ok"}]})).is_ok()
        );
        assert!(
            validate_schema(
                &serde_json::json!({"$ref":"file:///etc/passwd"}),
                &Value::Null
            )
            .is_err()
        );
        let schema = serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
        assert!(validate_schema(&schema, &serde_json::json!({"ok":true})).is_ok());
        assert!(validate_schema(&schema, &serde_json::json!({"ok":true,"extra":1})).is_err());
    }
}
