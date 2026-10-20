//! Operator-installed tool definitions. There is no registration RPC.
use super::super::Error;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) const FREE_PRICE_VERSION: &str = "builtin-free-v1";
const FREE_PRICE: &[u8] =
    br#"{"kind":"free","unit":"credit_500k_usd","version":"builtin-free-v1"}"#;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolDomain {
    RegisteredCore,
    Extension,
}
impl ToolDomain {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RegisteredCore => "registered_core",
            Self::Extension => "extension",
        }
    }
}
#[derive(Clone)]
pub(crate) struct InstalledTool {
    pub name: String,
    pub description: String,
    pub version: String,
    pub schema_sha256: [u8; 32],
    pub price_sha256: [u8; 32],
    pub domain: ToolDomain,
    definition: Value,
    validator: Arc<jsonschema::Validator>,
}
impl InstalledTool {
    pub(crate) fn validate_arguments(&self, bytes: &[u8]) -> Result<Value, Error> {
        if bytes.len() > 65_536 {
            return Err(Error::Invalid);
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if !value.is_object() || !self.validator.is_valid(&value) {
            return Err(Error::Invalid);
        }
        Ok(value)
    }
    pub(crate) fn definition(&self) -> Value {
        self.definition.clone()
    }
}
#[derive(Clone)]
pub(crate) struct AgentRegistry {
    digest: [u8; 32],
    tools: BTreeMap<String, InstalledTool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    tools: Vec<Definition>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    name: String,
    description: String,
    version: String,
    schema_json: String,
    schema_sha256: String,
    price_version: String,
    price_sha256: String,
    domain: ToolDomain,
}
pub(super) fn hex32(s: &str) -> Result<[u8; 32], Error> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::Invalid);
    }
    let mut out = [0u8; 32];
    for (i, value) in out.iter_mut().enumerate() {
        *value = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| Error::Invalid)?;
    }
    Ok(out)
}
fn no_external_refs(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().all(|(key, v)| {
            (!(key == "$ref" || key == "$dynamicRef")
                || v.as_str().is_some_and(|s| s.starts_with('#')))
                && no_external_refs(v)
        }),
        Value::Array(values) => values.iter().all(no_external_refs),
        _ => true,
    }
}
impl AgentRegistry {
    /// Both artifact and digest come from reviewed private Core host configuration.
    /// The candidate extraction artifact intentionally lacks mandatory bindings,
    /// so it cannot silently install itself as a registry.
    pub(crate) fn from_reviewed(bytes: &[u8], expected_sha256: &str) -> Result<Self, Error> {
        if bytes.len() > 2_097_152 {
            return Err(Error::Invalid);
        }
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if digest != hex32(expected_sha256)? {
            return Err(Error::Invalid);
        }
        let artifact: Artifact = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if artifact.tools.is_empty() || artifact.tools.len() > 72 {
            return Err(Error::Invalid);
        }
        let free_price: [u8; 32] = Sha256::digest(FREE_PRICE).into();
        let mut tools = BTreeMap::new();
        for row in artifact.tools {
            if row.name.is_empty()
                || row.name.len() > 80
                || !row
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || row.description.len() > 8192
                || !super::super::label(&row.version, 128)
                || row.schema_json.len() > 65_536
                || row.price_version != FREE_PRICE_VERSION
                || hex32(&row.price_sha256)? != free_price
            {
                return Err(Error::Invalid);
            }
            let schema_sha256: [u8; 32] = Sha256::digest(row.schema_json.as_bytes()).into();
            if schema_sha256 != hex32(&row.schema_sha256)? {
                return Err(Error::Invalid);
            }
            let parameters: Value =
                serde_json::from_str(&row.schema_json).map_err(|_| Error::Invalid)?;
            if !parameters.is_object() || !no_external_refs(&parameters) {
                return Err(Error::Invalid);
            }
            let validator =
                Arc::new(jsonschema::validator_for(&parameters).map_err(|_| Error::Invalid)?);
            let definition = json!({"type":"function","function":{"name":row.name,"description":row.description,"parameters":parameters}});
            let entry = InstalledTool {
                name: row.name.clone(),
                description: row.description,
                version: row.version,
                schema_sha256,
                price_sha256: free_price,
                domain: row.domain,
                definition,
                validator,
            };
            if tools.insert(row.name, entry).is_some() {
                return Err(Error::Invalid);
            }
        }
        Ok(Self { digest, tools })
    }
    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub(crate) fn get(&self, name: &str) -> Result<&InstalledTool, Error> {
        self.tools.get(name).ok_or(Error::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const INSTALLED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/assistant-tools-v1.json"
    ));
    const INSTALLED_SHA: &str = "16828e2ace013589cd2792ec032abaa665447dba3039736552d8f3886f4b3494";
    #[test]
    fn reviewed_installed_subset_has_exact_schemas_and_no_uninstalled_authority() {
        let registry = AgentRegistry::from_reviewed(INSTALLED, INSTALLED_SHA).unwrap();
        assert_eq!(registry.tools.len(), 19);
        assert_eq!(
            registry
                .tools
                .values()
                .filter(|t| t.domain == ToolDomain::RegisteredCore)
                .count(),
            7
        );
        assert_eq!(
            registry
                .tools
                .values()
                .filter(|t| t.domain == ToolDomain::Extension)
                .count(),
            12
        );
        assert!(registry.get("prepare_new_user_gift").is_err());
        assert!(registry.get("call_market_tool").is_err());
        assert!(registry.get("set_user_role").is_err());
        assert!(
            registry
                .get("get_account_access")
                .unwrap()
                .validate_arguments(br#"{}"#)
                .is_ok()
        );
        assert!(
            registry
                .get("get_account_access")
                .unwrap()
                .validate_arguments(br#"{"payer":7}"#)
                .is_err()
        );
        assert!(
            registry
                .get("calculate_math")
                .unwrap()
                .validate_arguments(br#"{"expression":"1+1"}"#)
                .is_ok()
        );
        assert!(
            registry
                .get("calculate_math")
                .unwrap()
                .validate_arguments(br#"{"expression":2}"#)
                .is_err()
        );
    }
    #[test]
    fn altered_installed_bytes_or_schema_cannot_silently_acquire_authority() {
        let mut changed = INSTALLED.to_vec();
        changed.push(b' ');
        assert!(AgentRegistry::from_reviewed(&changed, INSTALLED_SHA).is_err());
        let mut changed: Value = serde_json::from_slice(INSTALLED).unwrap();
        changed["tools"][0]["schema_json"] = Value::String("{}".into());
        let bytes = serde_json::to_vec(&changed).unwrap();
        let sha = format!("{:x}", Sha256::digest(&bytes));
        assert!(AgentRegistry::from_reviewed(&bytes, &sha).is_err());
        changed["tools"][0]["schema_sha256"] =
            Value::String(format!("{:x}", Sha256::digest(b"{}")));
        changed["tools"][0]["schema_json"] =
            Value::String(r#"{"$ref":"https://untrusted.invalid/schema"}"#.into());
        let schema = changed["tools"][0]["schema_json"]
            .as_str()
            .unwrap()
            .as_bytes();
        changed["tools"][0]["schema_sha256"] =
            Value::String(format!("{:x}", Sha256::digest(schema)));
        let bytes = serde_json::to_vec(&changed).unwrap();
        let sha = format!("{:x}", Sha256::digest(&bytes));
        assert!(AgentRegistry::from_reviewed(&bytes, &sha).is_err());
    }
}
