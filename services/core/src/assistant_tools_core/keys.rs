use super::{Error, context::VerifiedToolContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListMyApiKeys {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub exact_name: Option<String>,
    pub token_id: Option<i64>,
}
#[derive(Serialize)]
struct Key {
    id: i64,
    name: String,
    status: i32,
    group: String,
    created_time: i64,
    accessed_time: i64,
    expired_time: i64,
}
pub async fn list_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    input: &ListMyApiKeys,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    // Main permits listing one's own key metadata at L0. Never reveal a key,
    // masked key, quota, network restriction or encrypted recovery material.
    let page = input.page.unwrap_or(1);
    let size = input.page_size.unwrap_or(20);
    let name = input.exact_name.as_deref().unwrap_or("");
    if !(1..=10000).contains(&page)
        || !(1..=50).contains(&size)
        || name.chars().count() > 50
        || input
            .token_id
            .is_some_and(|id| !(1..=9_007_199_254_740_991).contains(&id))
        || input.token_id.is_some() && (!name.is_empty() || page != 1)
    {
        return Err(Error::Invalid);
    }
    let rows=sqlx::query("SELECT k.id,s.name,s.group_name,CASE WHEN k.revoked_at IS NOT NULL OR NOT s.enabled THEN 2 WHEN k.expires_at<=clock_timestamp() THEN 3 ELSE 1 END AS status,extract(epoch FROM k.created_at)::bigint AS created,COALESCE(extract(epoch FROM s.accessed_at)::bigint,0) AS accessed,extract(epoch FROM k.expires_at)::bigint AS expires FROM core_identity.credentials k JOIN core_identity.api_key_settings s ON s.credential_id=k.id WHERE k.kind='api_key' AND k.revoked_at IS NULL AND k.user_id=$1 AND k.owner_account_id=$2 AND ($3='' OR s.name=$3) AND ($4::bigint IS NULL OR k.id=$4) ORDER BY k.id DESC LIMIT $5 OFFSET $6")
        .bind(actor.user).bind(actor.account).bind(name).bind(input.token_id).bind(size).bind((page-1)*size).fetch_all(&mut *c).await?;
    let total:i64=sqlx::query_scalar("SELECT count(*) FROM core_identity.credentials k JOIN core_identity.api_key_settings s ON s.credential_id=k.id WHERE k.kind='api_key' AND k.revoked_at IS NULL AND k.user_id=$1 AND k.owner_account_id=$2 AND ($3='' OR s.name=$3) AND ($4::bigint IS NULL OR k.id=$4)")
        .bind(actor.user).bind(actor.account).bind(name).bind(input.token_id).fetch_one(&mut *c).await?;
    let mut keys = Vec::with_capacity(rows.len());
    for r in rows {
        let id: i64 = r.try_get("id")?;
        let (_, remaining, unlimited) =
            crate::identity::assistant_key_quota(c, id, actor.user).await?;
        let status: i32 = r.try_get("status")?;
        keys.push(Key {
            id,
            name: r.try_get("name")?,
            status: if status == 1 && !unlimited && remaining <= 0 {
                4
            } else {
                status
            },
            group: r.try_get("group_name")?,
            created_time: r.try_get("created")?,
            accessed_time: r.try_get("accessed")?,
            expired_time: match r.try_get::<i64, _>("expires")? {
                253_402_300_799 => -1,
                value => value,
            },
        })
    }
    Ok(
        json!({"ok":true,"scope":"self","keys":keys,"total":total,"page":page,"page_size":size,"has_more":page*size<total,"credentials_returned":false}),
    )
}
