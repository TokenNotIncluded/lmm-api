use super::Error;
use crate::{
    accounts::AccountKind,
    identity::{self, CredentialKind, Principal},
};
use sqlx::{PgConnection, Row};

/// No Deserialize or public fields. This is original live browser authority,
/// never a role/user/payer claim accepted from a tool call or Go result.
pub struct VerifiedToolContext {
    credential: i64,
    user: i64,
    account: i64,
    version: i64,
}
pub struct Facts {
    pub user: i64,
    pub account: i64,
    pub role: String,
    pub developer: bool,
    pub auth_version: i64,
    pub browser_session: Option<String>,
}
impl VerifiedToolContext {
    /// Browser handler only: the caller has just validated this Human with the
    /// native HttpOnly refresh cookie inside the same locked transaction.
    pub(super) async fn from_authenticated_browser(
        c: &mut PgConnection,
        human: &crate::humangrant::Human,
    ) -> Result<Self, Error> {
        let user = human.id.parse::<i64>().map_err(|_| Error::Unauthorized)?;
        let row=sqlx::query("SELECT s.current_credential_id,u.personal_account_id FROM core_identity.web_sessions s JOIN core_identity.users u ON u.id=s.user_id WHERE s.sid=$1 AND s.user_id=$2 AND s.user_version=$3 AND u.auth_version=$3 AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp()")
            .bind(&human.session_id).bind(user).bind(human.auth_version).fetch_optional(&mut *c).await?.ok_or(Error::Unauthorized)?;
        let context = Self {
            credential: row.try_get("current_credential_id")?,
            user,
            account: row.try_get("personal_account_id")?,
            version: human.auth_version,
        };
        let facts = context.revalidate(c).await?;
        if facts.browser_session.as_deref() != Some(human.session_id.as_str()) {
            return Err(Error::Unauthorized);
        }
        Ok(context)
    }
    /// Only the Core worker, after binding the actual tool operation to its
    /// original delegation, may call this with its freshly authenticated actor.
    pub(crate) async fn verify(c: &mut PgConnection, actor: &Principal) -> Result<Self, Error> {
        if actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(Error::Forbidden);
        }
        let version:i64=sqlx::query_scalar("SELECT user_version FROM core_identity.credentials WHERE id=$1 AND user_id=$2 AND owner_account_id=$3")
            .bind(actor.credential_id).bind(actor.user_id).bind(actor.owner_account_id).fetch_optional(&mut *c).await?.ok_or(Error::Unauthorized)?;
        let context = Self {
            credential: actor.credential_id,
            user: actor.user_id,
            account: actor.owner_account_id,
            version,
        };
        context.revalidate(c).await?;
        Ok(context)
    }
    pub fn user_id(&self) -> i64 {
        self.user
    }
    pub fn account_id(&self) -> i64 {
        self.account
    }
    pub fn credential_id(&self) -> i64 {
        self.credential
    }
    pub async fn revalidate(&self, c: &mut PgConnection) -> Result<Facts, Error> {
        let row=sqlx::query("SELECT u.platform_role,u.auth_version,s.sid AS browser_session,(u.platform_role IN ('admin','superadmin') OR COALESCE(d.granted,FALSE)) AS developer FROM core_identity.credentials k JOIN core_identity.users u ON u.id=k.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id LEFT JOIN core_identity.web_session_credentials m ON m.credential_id=k.id LEFT JOIN core_identity.web_sessions s ON s.sid=m.sid LEFT JOIN core_identity.developer_access d ON d.user_id=u.id WHERE k.id=$1 AND k.user_id=$2 AND k.owner_account_id=$3 AND u.personal_account_id=$3 AND k.kind='session' AND k.revoked_at IS NULL AND k.expires_at>clock_timestamp() AND k.user_version=u.auth_version AND k.user_version=$4 AND u.active AND a.active AND a.kind='personal' AND (m.sid IS NULL OR (s.current_credential_id=k.id AND s.user_id=u.id AND s.user_version=u.auth_version AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp() AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days'))) FOR SHARE OF k,u,a")
            .bind(self.credential).bind(self.user).bind(self.account).bind(self.version).fetch_optional(c).await?.ok_or(Error::Unauthorized)?;
        Ok(Facts {
            user: self.user,
            account: self.account,
            role: row.try_get("platform_role")?,
            developer: row.try_get("developer")?,
            auth_version: row.try_get("auth_version")?,
            browser_session: row.try_get("browser_session")?,
        })
    }
}
pub(crate) async fn from_session(
    c: &mut PgConnection,
    secret: &str,
) -> Result<VerifiedToolContext, Error> {
    let actor = identity::authorize_in_transaction(c, secret).await?;
    VerifiedToolContext::verify(c, &actor).await
}
