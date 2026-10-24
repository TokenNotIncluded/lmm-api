//! Explicit installation-owner operation. Compiled pinned catalog only; no
//! runtime request can register a builtin or impersonate a publisher account.
use super::{
    credits::{self, Price},
    *,
};
const CATALOG: &str =
    include_str!("../../../extensions/internal/modules/toolmarket/builtin_catalog.json");
#[derive(Deserialize)]
struct Service {
    key: String,
    service_id: String,
    version_id: String,
    tools: Vec<Tool>,
}
#[derive(Deserialize)]
struct Tool {
    tool_id: String,
    name: String,
    input_schema: String,
    input_schema_digest: String,
    version_id: String,
    price_quota: i64,
}
#[derive(Serialize)]
pub struct InstallReceipt {
    pub tools: u32,
    pub catalog_digest: String,
}
fn expected_catalog() -> Result<Vec<(credits::Quote, String, String)>> {
    let services: Vec<Service> = serde_json::from_str(CATALOG).map_err(|_| Error::Invalid)?;
    let mut expected = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for service in services {
        if !matches!(
            service.key.as_str(),
            "wallet" | "drawing" | "open_source_bounties"
        ) {
            return Err(Error::Invalid);
        }
        for tool in service.tools {
            if tool.price_quota != 0
                || tool.version_id != service.version_id
                || !tool.name.starts_with(&format!("{}.", service.key))
                || format!("{:x}", Sha256::digest(tool.input_schema.as_bytes()))
                    != tool.input_schema_digest
            {
                return Err(Error::Invalid);
            }
            let price = Price::Fixed { price_quota: 0 };
            let pricing_digest = hash(&price)?;
            let quote = credits::Quote {
                id: hash(&[
                    "legacy-credit-quote-v2",
                    &tool.version_id,
                    &tool.tool_id,
                    &tool.input_schema_digest,
                    &pricing_digest,
                ])?,
                service_id: service.service_id.clone(),
                tool_id: tool.tool_id,
                version_id: tool.version_id,
                remote_digest: tool.input_schema_digest.clone(),
                pricing_digest,
                price,
                expires: i64::MAX,
            };
            if !ids.insert(quote.id.clone()) {
                return Err(Error::Invalid);
            }
            expected.push((quote, tool.name, tool.input_schema_digest));
        }
    }
    if expected.len() != 29 {
        return Err(Error::Invalid);
    }
    Ok(expected)
}
pub async fn check_builtin_registry(pool: &PgPool) -> Result<InstallReceipt> {
    let mut tx = ToolMarketStore::from_pool(pool.clone()).begin().await?;
    let out = check_builtin_registry_in_transaction(&mut tx).await?;
    tx.commit().await?;
    Ok(out)
}
pub async fn check_builtin_registry_in_transaction(
    c: &mut sqlx::PgConnection,
) -> Result<InstallReceipt> {
    let installed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_meta.component_contract WHERE name='tool-market-credit' AND fingerprint=$1)").bind(Sha256::digest(credits::SCHEMA_V2.as_bytes()).to_vec()).fetch_one(&mut *c).await?;
    if !installed {
        return Err(Error::Forbidden);
    }
    for (quote, kind, schema) in expected_catalog()? {
        let row=sqlx::query("SELECT q.snapshot,q.seller_user_id,q.seller_account_id,q.approved,q.suspended,q.visibility,q.ceiling_credits,b.kind,b.input_schema_digest FROM core_tool_market.credit_quotes q JOIN core_tool_market.credit_builtin_bindings b ON b.quote_id=q.id WHERE q.id=$1").bind(&quote.id).fetch_optional(&mut *c).await?.ok_or(Error::Missing)?;
        let Json(saved): Json<credits::Quote> = row.try_get("snapshot")?;
        if saved != quote
            || row.try_get::<Option<i64>, _>("seller_user_id")?.is_some()
            || row
                .try_get::<Option<i64>, _>("seller_account_id")?
                .is_some()
            || !row.try_get::<bool, _>("approved")?
            || row.try_get::<bool, _>("suspended")?
            || row.try_get::<String, _>("visibility")? != "public"
            || row.try_get::<i64, _>("ceiling_credits")? != 0
            || row.try_get::<String, _>("kind")? != kind
            || row.try_get::<String, _>("input_schema_digest")? != schema
        {
            return Err(Error::Conflict);
        }
    }
    let active:i64=sqlx::query_scalar("SELECT count(*) FROM core_tool_market.credit_builtin_bindings b JOIN core_tool_market.credit_quotes q ON q.id=b.quote_id WHERE q.approved").fetch_one(&mut *c).await?;
    if active != 29 {
        return Err(Error::Conflict);
    }
    Ok(InstallReceipt {
        tools: 29,
        catalog_digest: format!("{:x}", Sha256::digest(CATALOG.as_bytes())),
    })
}
pub async fn install_compiled_builtins(pool: &PgPool) -> Result<InstallReceipt> {
    let mut tx = ToolMarketStore::from_pool(pool.clone()).begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(741803120)")
        .execute(&mut *tx)
        .await?;
    let owner:bool=sqlx::query_scalar("SELECT count(*)=3 AND bool_and(pg_get_userbyid(c.relowner)=current_user) AND (current_user=session_user OR EXISTS(SELECT FROM pg_roles r WHERE r.rolname=current_user AND NOT r.rolcanlogin AND pg_has_role(session_user,r.rolname,'MEMBER'))) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname='core_tool_market' AND c.relname IN ('credit_quotes','credit_builtin_bindings')) OR (n.nspname='core_meta' AND c.relname='component_contract')").fetch_one(&mut *tx).await?;
    if !owner {
        return Err(Error::Forbidden);
    }
    let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_tool_market.credit_calls) OR EXISTS(SELECT FROM core_tool_market.credit_consents) OR EXISTS(SELECT FROM core_tool_market.credit_services) OR EXISTS(SELECT FROM core_tool_market.credit_quotes WHERE seller_user_id IS NOT NULL)").fetch_one(&mut *tx).await?;
    if occupied {
        return Err(Error::Conflict);
    }
    let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_meta.component_contract WHERE name='tool-market-credit' AND fingerprint=$1)").bind(Sha256::digest(credits::SCHEMA_V2.as_bytes()).to_vec()).fetch_one(&mut *tx).await?;
    if !ready {
        return Err(Error::Forbidden);
    }
    for (quote, kind, schema) in expected_catalog()? {
        sqlx::query("INSERT INTO core_tool_market.credit_quotes(id,fee_bps,policy_version,snapshot,approved,ceiling_credits) VALUES($1,0,1,$2,TRUE,0) ON CONFLICT(id) DO NOTHING").bind(&quote.id).bind(Json(&quote)).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core_tool_market.credit_builtin_bindings(quote_id,kind,input_schema_digest) VALUES($1,$2,$3) ON CONFLICT(quote_id) DO NOTHING").bind(&quote.id).bind(kind).bind(schema).execute(&mut *tx).await?;
    }
    let out = check_builtin_registry_in_transaction(&mut tx).await?;
    tx.commit().await?;
    Ok(out)
}

#[cfg(test)]
use crate::test_runtime as runtime_fixture;
#[cfg(test)]
mod tests {
    use super::*;
    #[sqlx::test(migrations = false)]
    async fn compiled_catalog_rejects_runtime_and_accepts_authorized_nologin_owner(pool: PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        let runtime = super::runtime_fixture::RuntimeRole::create(&pool).await;
        assert!(matches!(
            install_compiled_builtins(&runtime.pool).await,
            Err(Error::Forbidden)
        ));
        runtime.cleanup().await;
        let role = format!("market_installer_{:016x}", rand::random::<u64>());
        let admin: String = sqlx::query_scalar("SELECT current_user::TEXT")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            admin
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        sqlx::raw_sql(&format!("CREATE ROLE {role} NOLOGIN;GRANT {role} TO {admin};GRANT USAGE ON SCHEMA core_tool_market,core_meta TO {role};GRANT SELECT ON ALL TABLES IN SCHEMA core_tool_market TO {role};ALTER TABLE core_tool_market.credit_quotes OWNER TO {role};ALTER TABLE core_tool_market.credit_builtin_bindings OWNER TO {role};ALTER TABLE core_meta.component_contract OWNER TO {role};")).execute(&pool).await.unwrap();
        let options = pool
            .connect_options()
            .as_ref()
            .clone()
            .options([("role", role.as_str())]);
        let installer = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let pair: (String, String) = sqlx::query_as("SELECT current_user::TEXT,session_user::TEXT")
            .fetch_one(&installer)
            .await
            .unwrap();
        assert_eq!(pair.0, role);
        assert_ne!(pair.0, pair.1);
        let receipt = install_compiled_builtins(&installer).await.unwrap();
        assert_eq!(receipt.tools, 29);
        assert_eq!(check_builtin_registry(&installer).await.unwrap().tools, 29);
        let replay = install_compiled_builtins(&installer).await.unwrap();
        assert_eq!(replay.catalog_digest, receipt.catalog_digest);
        let facts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_tool_market.credit_builtin_bindings),(SELECT count(*) FROM core_tool_market.credit_quotes WHERE seller_user_id IS NOT NULL OR seller_account_id IS NOT NULL)").fetch_one(&pool).await.unwrap();
        assert_eq!(facts, (29, 0));
        installer.close().await;
        sqlx::raw_sql(&format!("ALTER TABLE core_tool_market.credit_quotes OWNER TO {admin};ALTER TABLE core_tool_market.credit_builtin_bindings OWNER TO {admin};ALTER TABLE core_meta.component_contract OWNER TO {admin};DROP OWNED BY {role};DROP ROLE {role};")).execute(&pool).await.unwrap();
    }
}
