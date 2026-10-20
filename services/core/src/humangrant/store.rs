//! PostgreSQL persistence and transactional isolation for HumanGrant.

use super::types::{Account, Human, ProtocolError, failure};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use subtle::ConstantTimeEq;

pub const DDL: &str = include_str!("../../schema/humangrant.sql");

#[derive(Clone)]
pub struct HumanGrantStore {
    pool: PgPool,
}

impl HumanGrantStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn init_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(DDL).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn begin(&self) -> Result<HumanGrantTx, ProtocolError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            eprintln!("store begin transaction error: {:?}", e);
            failure(500, "internal_error")
        })?;
        // The single gate makes settings changes, consent, revocation and rate
        // counters linearizable across all Core processes sharing this store.
        let gate: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM core_humangrant.human_grant_gate WHERE id = 1 FOR UPDATE",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| failure(500, "internal_error"))?;
        if gate != Some(1) {
            return Err(failure(503, "not_ready"));
        }
        Ok(HumanGrantTx { tx })
    }
}

pub struct HumanGrantTx {
    pub tx: Transaction<'static, Postgres>,
}

impl HumanGrantTx {
    pub async fn commit(self) -> Result<(), ProtocolError> {
        self.tx.commit().await.map_err(|e| {
            eprintln!("store commit transaction error: {:?}", e);
            failure(500, "internal_error")
        })
    }

    pub async fn rollback(self) -> Result<(), ProtocolError> {
        self.tx.rollback().await.map_err(|e| {
            eprintln!("store rollback transaction error: {:?}", e);
            failure(500, "internal_error")
        })
    }

    pub async fn get<T: serde::de::DeserializeOwned>(
        &mut self,
        kind: &str,
        id: &str,
        now: i64,
    ) -> Result<Option<T>, ProtocolError> {
        let row = sqlx::query(
            "SELECT payload, expires_at FROM core_humangrant.human_grant_records WHERE kind = $1 AND id = $2",
        )
        .bind(kind)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store get error: {:?}", e);
            failure(500, "internal_error")
        })?;

        let Some(row) = row else {
            return Ok(None);
        };

        let expires_at: Option<i64> = row
            .try_get("expires_at")
            .map_err(|_| failure(500, "internal_error"))?;
        if let Some(exp) = expires_at
            && exp > 0
            && exp <= now
        {
            return Ok(None);
        }

        let payload: String = row
            .try_get("payload")
            .map_err(|_| failure(500, "internal_error"))?;
        let parsed = serde_json::from_str(&payload).map_err(|_| failure(500, "internal_error"))?;
        Ok(Some(parsed))
    }

    pub async fn get_for_update<T: serde::de::DeserializeOwned>(
        &mut self,
        kind: &str,
        id: &str,
        now: i64,
    ) -> Result<Option<T>, ProtocolError> {
        let row = sqlx::query(
            "SELECT payload, expires_at FROM core_humangrant.human_grant_records WHERE kind = $1 AND id = $2 FOR UPDATE",
        )
        .bind(kind)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store get_for_update error: {:?}", e);
            failure(500, "internal_error")
        })?;

        let Some(row) = row else {
            return Ok(None);
        };

        let expires_at: Option<i64> = row
            .try_get("expires_at")
            .map_err(|_| failure(500, "internal_error"))?;
        if let Some(exp) = expires_at
            && exp > 0
            && exp <= now
        {
            return Ok(None);
        }

        let payload: String = row
            .try_get("payload")
            .map_err(|_| failure(500, "internal_error"))?;
        let parsed = serde_json::from_str(&payload).map_err(|_| failure(500, "internal_error"))?;
        Ok(Some(parsed))
    }

    pub async fn atomic_reserve_nonce(
        &mut self,
        actor: &str,
        nonce: &str,
        expires_at: i64,
    ) -> Result<(), ProtocolError> {
        let nonce_id = format!("{actor}:{nonce}");
        let res = sqlx::query(
            r#"
            INSERT INTO core_humangrant.human_grant_records (kind, id, owner, expires_at, payload)
            VALUES ('nonce', $1, $2, $3, '{"used":1}')
            "#,
        )
        .bind(&nonce_id)
        .bind(actor)
        .bind(expires_at)
        .execute(&mut *self.tx)
        .await;

        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                Err(failure(401, "signature_replay"))
            }
            Err(e) => {
                eprintln!("store atomic_reserve_nonce error: {:?}", e);
                Err(failure(500, "internal_error"))
            }
        }
    }

    pub async fn put<T: serde::Serialize>(
        &mut self,
        kind: &str,
        id: &str,
        owner: &str,
        expires_at: i64,
        value: &T,
    ) -> Result<(), ProtocolError> {
        let payload = serde_json::to_string(value).map_err(|_| failure(500, "internal_error"))?;
        sqlx::query(
            r#"
            INSERT INTO core_humangrant.human_grant_records (kind, id, owner, expires_at, payload)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (kind, id) DO UPDATE SET
                owner = EXCLUDED.owner,
                expires_at = EXCLUDED.expires_at,
                payload = EXCLUDED.payload
            "#,
        )
        .bind(kind)
        .bind(id)
        .bind(owner)
        .bind(expires_at)
        .bind(payload)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store put error: {:?}", e);
            failure(500, "internal_error")
        })?;

        Ok(())
    }

    pub async fn delete(&mut self, kind: &str, id: &str) -> Result<(), ProtocolError> {
        sqlx::query("DELETE FROM core_humangrant.human_grant_records WHERE kind = $1 AND id = $2")
            .bind(kind)
            .bind(id)
            .execute(&mut *self.tx)
            .await
            .map_err(|e| {
                eprintln!("store delete error: {:?}", e);
                failure(500, "internal_error")
            })?;

        Ok(())
    }

    pub async fn list<T: serde::de::DeserializeOwned>(
        &mut self,
        kind: &str,
        owner: &str,
        limit: i64,
        now: i64,
    ) -> Result<Vec<T>, ProtocolError> {
        if !(1..=200).contains(&limit) {
            return Err(failure(400, "invalid_request"));
        }

        let rows = sqlx::query(
            r#"
            SELECT payload FROM core_humangrant.human_grant_records
            WHERE kind = $1 AND owner = $2 AND (expires_at = 0 OR expires_at > $3)
            ORDER BY id
            LIMIT $4
            "#,
        )
        .bind(kind)
        .bind(owner)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store list error: {:?}", e);
            failure(500, "internal_error")
        })?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let payload: String = row
                .try_get("payload")
                .map_err(|_| failure(500, "internal_error"))?;
            let item: T =
                serde_json::from_str(&payload).map_err(|_| failure(500, "internal_error"))?;
            results.push(item);
        }

        Ok(results)
    }

    pub async fn cleanup_expired(&mut self, now: i64) -> Result<(), ProtocolError> {
        let expired = sqlx::query(
            r#"
            SELECT kind, id FROM core_humangrant.human_grant_records
            WHERE expires_at > 0 AND expires_at <= $1
            ORDER BY expires_at
            LIMIT 64
            "#,
        )
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store cleanup_expired error: {:?}", e);
            failure(500, "internal_error")
        })?;

        for row in expired {
            let kind: String = row
                .try_get("kind")
                .map_err(|_| failure(500, "internal_error"))?;
            let id: String = row
                .try_get("id")
                .map_err(|_| failure(500, "internal_error"))?;
            let _ = sqlx::query(
                "DELETE FROM core_humangrant.human_grant_records WHERE kind = $1 AND id = $2",
            )
            .bind(kind)
            .bind(id)
            .execute(&mut *self.tx)
            .await;
        }

        Ok(())
    }

    /// Authenticate human from `new_api_refresh` cookie formatted as `{sid}.{secret}`.
    pub async fn authenticate_human(
        &mut self,
        refresh_cookie: Option<&str>,
        now: i64,
    ) -> Result<Option<(Human, Account)>, ProtocolError> {
        let Some(cookie) = refresh_cookie else {
            return Ok(None);
        };

        let Some((sid, secret)) = cookie.split_once('.') else {
            return Ok(None);
        };

        if sid.len() != 64
            || !sid
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !secret.starts_with("lmmr_")
            || secret.len() != 69
        {
            return Ok(None);
        }

        let secret_hash = Sha256::digest(secret.as_bytes()).to_vec();

        let row = sqlx::query(
            r#"
            SELECT s.sid, s.user_id, s.user_version,
                   EXTRACT(EPOCH FROM s.revoked_at)::bigint AS revoked_at,
                   EXTRACT(EPOCH FROM s.expires_at)::bigint AS expires_at,
                   EXTRACT(EPOCH FROM s.created_at)::bigint AS created_at,
                   s.weekly_logout,
                   s.refresh_digest, s.previous_refresh_digest,
                   COALESCE(s.previous_valid_until >= clock_timestamp(), FALSE) AS previous_live,
                   u.active AS user_active, u.auth_version, u.platform_role,
                   a.active AS account_active,
                   EXTRACT(EPOCH FROM c.revoked_at)::bigint AS cred_revoked_at,
                   c.user_version AS cred_user_version,
                   EXTRACT(EPOCH FROM c.expires_at)::bigint AS cred_expires_at,
                   p.username, p.display_name
            FROM core_identity.web_sessions s
            JOIN core_identity.users u ON u.id = s.user_id
            JOIN core_identity.user_profiles p ON p.user_id = u.id
            JOIN core_identity.accounts a ON a.id = u.personal_account_id
            JOIN core_identity.credentials c ON c.id = s.current_credential_id
            WHERE s.sid = $1
            FOR SHARE OF s, u, a, c
            "#,
        )
        .bind(sid)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store authenticate_human error: {:?}", e);
            failure(500, "internal_error")
        })?;

        let Some(row) = row else {
            return Ok(None);
        };

        let user_active: bool = row
            .try_get("user_active")
            .map_err(|_| failure(500, "internal_error"))?;
        let account_active: bool = row
            .try_get("account_active")
            .map_err(|_| failure(500, "internal_error"))?;
        let user_id: i64 = row
            .try_get("user_id")
            .map_err(|_| failure(500, "internal_error"))?;
        let user_version: i64 = row
            .try_get("user_version")
            .map_err(|_| failure(500, "internal_error"))?;
        let auth_version: i64 = row
            .try_get("auth_version")
            .map_err(|_| failure(500, "internal_error"))?;
        let revoked_at: Option<i64> = row
            .try_get("revoked_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let expires_at: i64 = row
            .try_get("expires_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let created_at: i64 = row
            .try_get("created_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let weekly_logout: bool = row
            .try_get("weekly_logout")
            .map_err(|_| failure(500, "internal_error"))?;

        let cred_revoked_at: Option<i64> = row
            .try_get("cred_revoked_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let cred_user_version: i64 = row
            .try_get("cred_user_version")
            .map_err(|_| failure(500, "internal_error"))?;
        let cred_expires_at: i64 = row
            .try_get("cred_expires_at")
            .map_err(|_| failure(500, "internal_error"))?;

        let refresh_digest: Vec<u8> = row
            .try_get("refresh_digest")
            .map_err(|_| failure(500, "internal_error"))?;
        let previous_refresh_digest: Option<Vec<u8>> = row
            .try_get("previous_refresh_digest")
            .map_err(|_| failure(500, "internal_error"))?;
        let previous_live: bool = row
            .try_get("previous_live")
            .map_err(|_| failure(500, "internal_error"))?;
        let platform_role: String = row
            .try_get("platform_role")
            .map_err(|_| failure(500, "internal_error"))?;
        let username: String = row
            .try_get("username")
            .map_err(|_| failure(500, "internal_error"))?;
        let display_name: String = row
            .try_get("display_name")
            .map_err(|_| failure(500, "internal_error"))?;

        if !user_active || !account_active {
            return Ok(None);
        }
        if revoked_at.is_some() || expires_at <= now {
            return Ok(None);
        }
        if weekly_logout && created_at <= now - 7 * 86400 {
            return Ok(None);
        }
        if auth_version != user_version {
            return Ok(None);
        }
        if cred_revoked_at.is_some() || cred_user_version != auth_version || cred_expires_at <= now
        {
            return Ok(None);
        }

        let is_current: bool = refresh_digest.ct_eq(&secret_hash).into();
        let is_prev = previous_live
            && previous_refresh_digest
                .as_ref()
                .is_some_and(|prev| prev.ct_eq(&secret_hash).into());

        if !is_current && !is_prev {
            return Ok(None);
        }

        // Only platform_role == "superadmin" (RoleRootUser = 100) is root.
        // RoleAdminUser (platform_role == "admin", role = 10) is NOT root!
        let is_root = platform_role == "superadmin";

        Ok(Some((
            Human {
                id: user_id.to_string(),
                session_id: sid.to_string(),
                session_version: user_version,
                auth_version,
            },
            Account {
                id: user_id.to_string(),
                username,
                display_name,
                root: is_root,
            },
        )))
    }

    /// Re-checks active human status and profile from database.
    pub async fn current_human(
        &mut self,
        human: &Human,
        now: i64,
    ) -> Result<Option<Account>, ProtocolError> {
        let user_id = human
            .id
            .parse::<i64>()
            .map_err(|_| failure(400, "invalid_request"))?;
        if user_id <= 0 {
            return Ok(None);
        }

        let row = sqlx::query(
            r#"
            SELECT s.sid,
                   EXTRACT(EPOCH FROM s.revoked_at)::bigint AS revoked_at,
                   EXTRACT(EPOCH FROM s.expires_at)::bigint AS expires_at,
                   EXTRACT(EPOCH FROM s.created_at)::bigint AS created_at,
                   s.weekly_logout, s.user_version,
                   u.active AS user_active, u.auth_version, u.platform_role,
                   a.active AS account_active,
                   EXTRACT(EPOCH FROM c.revoked_at)::bigint AS cred_revoked_at,
                   c.user_version AS cred_user_version,
                   EXTRACT(EPOCH FROM c.expires_at)::bigint AS cred_expires_at,
                   p.username, p.display_name
            FROM core_identity.web_sessions s
            JOIN core_identity.users u ON u.id = s.user_id
            JOIN core_identity.user_profiles p ON p.user_id = u.id
            JOIN core_identity.accounts a ON a.id = u.personal_account_id
            JOIN core_identity.credentials c ON c.id = s.current_credential_id
            WHERE s.sid = $1 AND u.id = $2
            FOR SHARE OF s, u, a, c
            "#,
        )
        .bind(&human.session_id)
        .bind(user_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(|e| {
            eprintln!("store current_human error: {:?}", e);
            failure(500, "internal_error")
        })?;

        let Some(row) = row else {
            return Ok(None);
        };

        let user_active: bool = row
            .try_get("user_active")
            .map_err(|_| failure(500, "internal_error"))?;
        let account_active: bool = row
            .try_get("account_active")
            .map_err(|_| failure(500, "internal_error"))?;
        let user_version: i64 = row
            .try_get("user_version")
            .map_err(|_| failure(500, "internal_error"))?;
        let auth_version: i64 = row
            .try_get("auth_version")
            .map_err(|_| failure(500, "internal_error"))?;
        let revoked_at: Option<i64> = row
            .try_get("revoked_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let expires_at: i64 = row
            .try_get("expires_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let created_at: i64 = row
            .try_get("created_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let weekly_logout: bool = row
            .try_get("weekly_logout")
            .map_err(|_| failure(500, "internal_error"))?;

        let cred_revoked_at: Option<i64> = row
            .try_get("cred_revoked_at")
            .map_err(|_| failure(500, "internal_error"))?;
        let cred_user_version: i64 = row
            .try_get("cred_user_version")
            .map_err(|_| failure(500, "internal_error"))?;
        let cred_expires_at: i64 = row
            .try_get("cred_expires_at")
            .map_err(|_| failure(500, "internal_error"))?;

        let platform_role: String = row
            .try_get("platform_role")
            .map_err(|_| failure(500, "internal_error"))?;
        let username: String = row
            .try_get("username")
            .map_err(|_| failure(500, "internal_error"))?;
        let display_name: String = row
            .try_get("display_name")
            .map_err(|_| failure(500, "internal_error"))?;

        if !user_active || !account_active {
            return Ok(None);
        }
        if revoked_at.is_some() || expires_at <= now {
            return Ok(None);
        }
        if weekly_logout && created_at <= now - 7 * 86400 {
            return Ok(None);
        }
        if auth_version != human.auth_version
            || user_version != human.session_version
            || auth_version != user_version
        {
            return Ok(None);
        }
        if cred_revoked_at.is_some() || cred_user_version != auth_version || cred_expires_at <= now
        {
            return Ok(None);
        }

        let is_root = platform_role == "superadmin";

        Ok(Some(Account {
            id: human.id.clone(),
            username,
            display_name,
            root: is_root,
        }))
    }
}
