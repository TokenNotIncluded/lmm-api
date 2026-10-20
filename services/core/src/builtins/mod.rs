//! Narrow Core authorization for code-owned marketplace builtins. This is not
//! a money API: wallet, bounty and model domains perform their own Session,
//! confirmation, immutable-basis and ledger checks before side effects.
use crate::tool_market::{
    Error, Result,
    credit_access::CreditStore,
    credit_execution::{BuiltinContext, BuiltinKind},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
pub mod http;

const ACTIONS: &[(&str, &str)] = &[
    ("drawing", "generate"),
    ("drawing", "list_capabilities"),
    ("wallet", "balance"),
    ("wallet", "topup_link"),
    ("wallet", "transfer.cancel"),
    ("wallet", "transfer.create"),
    ("wallet", "transfers.list"),
    ("open_source_bounties", "accept"),
    ("open_source_bounties", "approve"),
    ("open_source_bounties", "cancel"),
    ("open_source_bounties", "close"),
    ("open_source_bounties", "create_draft"),
    ("open_source_bounties", "delete_draft"),
    ("open_source_bounties", "get"),
    ("open_source_bounties", "list"),
    ("open_source_bounties", "list_accepted"),
    ("open_source_bounties", "list_disputes"),
    ("open_source_bounties", "list_owned"),
    ("open_source_bounties", "open_dispute"),
    ("open_source_bounties", "pause"),
    ("open_source_bounties", "publish"),
    ("open_source_bounties", "rate_owner"),
    ("open_source_bounties", "reject"),
    ("open_source_bounties", "resolve_dispute"),
    ("open_source_bounties", "resume"),
    ("open_source_bounties", "submit"),
    ("open_source_bounties", "tip"),
    ("open_source_bounties", "update_draft"),
    ("open_source_bounties", "withdraw"),
];
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub context: BuiltinContext,
    pub service_key: String,
    pub tool_name: String,
    pub input_schema_digest: String,
    pub arguments: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub user: String,
    pub account: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub owner: Owner,
    pub call_id: String,
    pub dispatch_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub input_digest: String,
    pub input_schema_digest: String,
}
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}
/// Match the existing Go encoding/json fingerprint, including HTML and JS line
/// separators. This is a payload fingerprint, never a signature or credential.
pub fn input_digest(arguments: &Value) -> Result<String> {
    if !arguments.is_object() {
        return Err(Error::Invalid);
    }
    let encoded = serde_json::to_string(arguments).map_err(|_| Error::Invalid)?;
    if encoded.len() > 65536 {
        return Err(Error::Invalid);
    }
    let encoded = encoded
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(format!("{:x}", Sha256::digest(encoded.as_bytes())))
}
fn kind(service: &str, tool: &str) -> Result<BuiltinKind> {
    let &(service, short) = ACTIONS
        .iter()
        .find(|(key, short)| *key == service && format!("{key}.{short}") == tool)
        .ok_or(Error::Forbidden)?;
    BuiltinKind::code_owned(service, short)
}
impl Store {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn check_ready(&self) -> Result<crate::tool_market::builtin_install::InstallReceipt> {
        crate::tool_market::builtin_install::check_builtin_registry(&self.pool).await
    }
    pub async fn authorize(&self, session: &str, request: &Invocation) -> Result<Authorization> {
        let expected = kind(&request.service_key, &request.tool_name)?;
        if input_digest(&request.arguments)? != request.context.input_digest {
            return Err(Error::Conflict);
        }
        if request.input_schema_digest.len() != 64
            || !request
                .input_schema_digest
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        for statement in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL lock_timeout='250ms'",
            "SET LOCAL statement_timeout='1500ms'",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        let actor =
            CreditStore::validate_builtin_context(&mut tx, session, &request.context, expected)
                .await?;
        if actor.input_schema_digest != request.input_schema_digest {
            return Err(Error::Conflict);
        }
        let result = Authorization {
            owner: Owner {
                user: actor.user_id.to_string(),
                account: actor.user_id.to_string(),
            },
            call_id: actor.call_id,
            dispatch_id: actor.dispatch_id,
            tool_id: actor.tool_id,
            version_id: actor.version_id,
            input_digest: request.context.input_digest.clone(),
            input_schema_digest: actor.input_schema_digest,
        };
        tx.commit().await?;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_code_owned_actions_and_existing_go_fingerprint_are_accepted() {
        assert!(kind("wallet", "wallet.balance").is_ok());
        assert!(kind("wallet", "wallet.admin_transfer").is_err());
        assert!(kind("drawing", "wallet.balance").is_err());
        assert!(kind("open_source_bounties", "open_source_bounties.approve").is_ok());
        let encoded = "{\"a\":\"\\u003c\\u0026\\u003e\\u2028\\u2029\",\"n\":9007199254740991}";
        assert_eq!(
            input_digest(&json!({"n":9007199254740991_i64,"a":"<&>\u{2028}\u{2029}"})).unwrap(),
            format!("{:x}", Sha256::digest(encoded.as_bytes()))
        );
        assert!(input_digest(&json!([])).is_err());
    }
    #[sqlx::test]
    async fn real_compiled_dispatch_rejects_substitution_and_revocation(pool: PgPool) {
        let args = json!({});
        let input = input_digest(&args).unwrap();
        let schema = "9".repeat(64);
        let f = crate::tool_market::credit_test_fixture::builtin(
            &pool,
            "wallet-balance-tool",
            "wallet-version",
            "wallet.balance",
            &schema,
            &input,
        )
        .await;
        let store = Store::from_pool(pool.clone());
        let request = Invocation {
            context: f.context.clone(),
            service_key: "wallet".into(),
            tool_name: "wallet.balance".into(),
            input_schema_digest: schema.clone(),
            arguments: args,
        };
        let original = store.authorize(&f.secret, &request).await.unwrap();
        assert_eq!(original.owner.user, "7");
        assert_eq!(original.call_id, request.context.call_id);
        let mut changed = request.clone();
        changed.context.dispatch_id = "unrelated-dispatch".into();
        assert!(matches!(
            store.authorize(&f.secret, &changed).await,
            Err(Error::Forbidden)
        ));
        changed = request.clone();
        changed.input_schema_digest = "8".repeat(64);
        assert!(matches!(
            store.authorize(&f.secret, &changed).await,
            Err(Error::Conflict)
        ));
        changed = request.clone();
        changed.tool_name = "wallet.transfer.create".into();
        assert!(matches!(
            store.authorize(&f.secret, &changed).await,
            Err(Error::Forbidden)
        ));
        changed = request.clone();
        changed.arguments = json!({"amount":500});
        assert!(matches!(
            store.authorize(&f.secret, &changed).await,
            Err(Error::Conflict)
        ));
        sqlx::query("UPDATE core_tool_market.credit_consents SET revoked_at=1 WHERE id=$1")
            .bind(&request.context.grant_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            store.authorize(&f.secret, &request).await,
            Err(Error::Forbidden)
        ));
    }
}
