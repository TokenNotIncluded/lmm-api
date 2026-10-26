//! Disposable installer seed + real human consent/reserve/dispatch for joint tests.
//! Never compiled into runtime or used against an existing database.
use super::{
    credit_access::{ConsentRequest, CreditStore},
    credit_execution::{BuiltinContext, ReserveRequest},
    credits::{self, Price},
    *,
};
pub(crate) struct BuiltinFixture {
    pub secret: String,
    pub context: BuiltinContext,
    pub store: CreditStore,
}
pub(crate) async fn builtin(
    pool: &PgPool,
    tool: &str,
    version: &str,
    kind: &str,
    schema_digest: &str,
    input_digest: &str,
) -> BuiltinFixture {
    crate::core_schema::init(pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let secret = identity.bootstrap_user(7, 1).await.unwrap().secret;
    builtin_with_session(
        pool,
        &secret,
        tool,
        version,
        kind,
        schema_digest,
        input_digest,
    )
    .await
}

/// Reuse an actual browser Session without changing its user or creating a
/// parallel synthetic credential. The normal consent/reserve/dispatch owns all
/// subsequent authority and funds decisions.
pub(crate) async fn builtin_with_session(
    pool: &PgPool,
    secret: &str,
    tool: &str,
    version: &str,
    kind: &str,
    schema_digest: &str,
    input_digest: &str,
) -> BuiltinFixture {
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_tool_market.credit_calls') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap();
    if !exists {
        sqlx::raw_sql(credits::SCHEMA_V2)
            .execute(pool)
            .await
            .unwrap();
    }
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let actor = identity.authorize(secret).await.unwrap();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::BIGINT")
            .fetch_one(pool)
            .await
            .unwrap();
    let price = Price::Fixed { price_quota: 0 };
    let quote = credits::Quote {
        id: hash(&["builtin-fixture", tool, version]).unwrap(),
        service_id: kind.split('.').next().unwrap().into(),
        tool_id: tool.into(),
        version_id: version.into(),
        remote_digest: "a".repeat(64),
        pricing_digest: hash(&price).unwrap(),
        price,
        expires: i64::MAX,
    };
    sqlx::query("INSERT INTO core_tool_market.credit_quotes(id,fee_bps,policy_version,snapshot,approved,ceiling_credits) VALUES($1,0,1,$2,TRUE,0)").bind(&quote.id).bind(Json(&quote)).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_tool_market.credit_builtin_bindings(quote_id,kind,input_schema_digest) VALUES($1,$2,$3)").bind(&quote.id).bind(kind).bind(schema_digest).execute(pool).await.unwrap();
    let store = CreditStore::from_pool(pool.clone());
    let owner = Owner {
        user: actor.user_id.to_string(),
        account: actor.owner.id.to_string(),
    };
    let grant = store
        .create_consent(
            secret,
            &ConsentRequest {
                owner: owner.clone(),
                operation_id: "joint-builtin-human-consent".into(),
                client_id: "web-market".into(),
                tool_id: tool.into(),
                version_id: version.into(),
                max_price_quota: 0,
                max_total_quota: 0,
                max_calls: 1,
                expires_at: now + 3600,
            },
        )
        .await
        .unwrap();
    let call = store
        .reserve(
            secret,
            &ReserveRequest {
                owner,
                client_id: "web-market".into(),
                request_key: "joint-builtin-execution".into(),
                quote_id: quote.id,
                grant_id: grant.id.clone(),
                tool_id: tool.into(),
                version_id: version.into(),
                input_digest: input_digest.into(),
                resolve_by: now + 1800,
            },
        )
        .await
        .unwrap();
    let dispatched = store.dispatch(secret, &call.call_id).await.unwrap();
    assert!(dispatched.dispatch_won);
    BuiltinFixture {
        secret: secret.to_owned(),
        context: BuiltinContext {
            call_id: call.call_id,
            dispatch_id: dispatched.dispatch_id,
            tool_id: tool.into(),
            version_id: version.into(),
            grant_id: grant.id,
            input_digest: input_digest.into(),
        },
        store,
    }
}
