use super::{
    Error,
    context::{Facts, VerifiedToolContext},
};
use crate::identity::permissions::{self, PlatformRole};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserTarget {
    pub user_id: Option<i64>,
    pub identifier: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSummaryArgs {
    pub user_id: Option<i64>,
    pub identifier: Option<String>,
    pub days: Option<i64>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnUsageArgs {
    pub days: Option<i64>,
}
fn role(raw: &str) -> Result<PlatformRole, Error> {
    match raw {
        "user" => Ok(PlatformRole::User),
        "admin" => Ok(PlatformRole::Admin),
        "superadmin" => Ok(PlatformRole::Root),
        _ => Err(Error::Unavailable),
    }
}
pub(crate) async fn target(
    c: &mut PgConnection,
    actor: &Facts,
    input: &UserTarget,
) -> Result<i64, Error> {
    if input
        .user_id
        .is_some_and(|id| !(1..=9_007_199_254_740_991).contains(&id))
        || input
            .identifier
            .as_ref()
            .is_some_and(|s| s.chars().count() > 200 || s.chars().any(char::is_control))
    {
        return Err(Error::Invalid);
    }
    let identifier = input.identifier.as_deref().unwrap_or("").trim();
    let rows=sqlx::query("SELECT u.id,u.personal_account_id,u.platform_role FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE ($1::bigint IS NULL OR u.id=$1) AND ($2='' OR p.username=$2 OR (p.email<>'' AND lower(p.email)=lower($2)) OR u.id::text=$2) ORDER BY u.id LIMIT 2")
        .bind(input.user_id.or(if identifier.is_empty(){Some(actor.user)}else{None})).bind(identifier).fetch_all(&mut *c).await?;
    if rows.len() != 1 {
        return Err(if rows.is_empty() {
            Error::NotFound
        } else {
            Error::Conflict
        });
    }
    let r = &rows[0];
    let user = r.try_get("id")?;
    if user != actor.user {
        let actor_role = role(&actor.role)?;
        let target_role = role(r.try_get::<String, _>("platform_role")?.as_str())?;
        if !permissions::can_manage_role(actor_role, target_role) {
            return Err(Error::Forbidden);
        }
        // Imported authority must have its complete verified policy; do not turn
        // an administrator-looking role or a hash-only snapshot into access.
        if permissions::has_privileged_import_facts(c, actor.user)
            .await
            .map_err(|_| Error::Unavailable)?
        {
            let loaded = permissions::load_complete_policy(c, actor.user)
                .await
                .map_err(|_| Error::Forbidden)?;
            let decision = permissions::decide(
                &permissions::Actor {
                    user_id: actor.user,
                    personal_account_id: actor.account,
                    role: actor_role,
                    credential: permissions::Credential::Session,
                    active: true,
                    account_active: true,
                    auth_version: actor.auth_version,
                    credential_auth_version: actor.auth_version,
                    developer: if actor.developer {
                        permissions::DeveloperState::Granted
                    } else {
                        permissions::DeveloperState::Denied
                    },
                },
                &loaded.as_policy(),
                permissions::Action::UserRead,
                permissions::Scope::User {
                    user_id: user,
                    account_id: r.try_get("personal_account_id")?,
                    role: target_role,
                },
                None,
            );
            if !decision.allowed() {
                return Err(Error::Forbidden);
            }
        }
    }
    Ok(user)
}
pub async fn overview_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    input: &UserTarget,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    let user = target(c, &actor, input).await?;
    let r=sqlx::query("SELECT u.id,u.platform_role,u.active,p.username,p.display_name,p.email,extract(epoch FROM a.created_at)::bigint AS created,(SELECT max(extract(epoch FROM s.created_at)::bigint) FROM core_identity.web_sessions s WHERE s.user_id=u.id) AS last_login,(SELECT count(*) FROM core_identity.oauth_identities i WHERE i.user_id=u.id) AS oauth_count FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1").bind(user).fetch_one(&mut *c).await?;
    let mut value = json!({"ok":true,"scope":if user==actor.user{"self"}else{"administrator_target"},"user":{"id":user,"username":r.try_get::<String,_>("username")?,"display_name":r.try_get::<String,_>("display_name")?,"email":r.try_get::<String,_>("email")?,"role":match r.try_get::<String,_>("platform_role")?.as_str(){"user"=>1,"admin"=>10,"superadmin"=>100,_=>return Err(Error::Unavailable)},"status":if r.try_get::<bool,_>("active")?{1}else{2},"created_at":r.try_get::<i64,_>("created")?,"created_at_source":"core personal account creation","last_login_at":r.try_get::<Option<i64>,_>("last_login")?,"custom_oauth_count":r.try_get::<i64,_>("oauth_count")?,"secrets_and_subject_ids":"omitted"},"financial_projection_available":false});
    match crate::financial_profile::read_wallet(c, user).await {
        Ok(wallet) => {
            value["wallet"] = serde_json::to_value(wallet).map_err(|_| Error::Unavailable)?;
            value["financial_projection_available"] = json!(true)
        }
        Err(crate::financial_profile::ProfileError::Unavailable) => {}
        Err(_) => return Err(Error::Unavailable),
    }
    match crate::financial_profile::read(c, user).await {
        Ok(p) => {
            value["financial_profile"] = serde_json::to_value(p).map_err(|_| Error::Unavailable)?
        }
        Err(crate::financial_profile::ProfileError::Unavailable) => {}
        Err(_) => return Err(Error::Unavailable),
    }
    Ok(value)
}
pub async fn access_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    let keys:i64=sqlx::query_scalar("SELECT count(*) FROM core_identity.credentials k JOIN core_identity.api_key_settings s ON s.credential_id=k.id WHERE k.user_id=$1 AND k.owner_account_id=$2 AND k.kind='api_key' AND k.revoked_at IS NULL AND k.expires_at>clock_timestamp() AND s.enabled")
        .bind(actor.user).bind(actor.account).fetch_one(&mut *c).await?;
    let calls:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE actor_user_id=$1 AND owner_account_id=$2 AND assistant_grant_id IS NULL AND state='settled' AND outcome='completed'").bind(actor.user).bind(actor.account).fetch_one(&mut *c).await?;
    let mut overview = overview_in_transaction(c, context, &UserTarget::default()).await?;
    overview["access"] = json!({"developer_access_granted":actor.developer,"level":if actor.developer{"L1"}else{"L0"},"source":"core live identity"});
    overview["progress"] = json!({"active_personal_api_keys":keys,"completed_native_model_requests":calls,"client_installation_proof_available":false,"client_configuration_proof_available":false});
    Ok(overview)
}
pub async fn gift_status_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    let receipt = crate::gift::status_in_transaction(c, actor.user)
        .await
        .map_err(|_| Error::Unavailable)?;
    Ok(json!({"ok":true,"scope":"self","gift":receipt,"source":"core gift ledger"}))
}
