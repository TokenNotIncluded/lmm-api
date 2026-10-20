//! Core protocol engine and state machine for HumanGrant 0.1 Draft.

use super::signature::fingerprint;
use super::store::{HumanGrantStore, HumanGrantTx};
use super::types::*;
use axum::http::HeaderMap;
use base64::prelude::*;
use rand::RngCore;
use subtle::ConstantTimeEq;

pub struct HumanGrantEngine {
    pub store: HumanGrantStore,
    pub origin: String,
    pub local: bool,
    pub mail: Option<std::sync::Arc<super::mail::SmtpMailer>>,
}

impl HumanGrantEngine {
    pub fn new(store: HumanGrantStore, origin: String, local: bool) -> Self {
        Self {
            store,
            origin,
            local,
            mail: None,
        }
    }

    pub fn random_bytes(len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        rand::thread_rng().fill_bytes(&mut buf);
        buf
    }

    pub fn random_id() -> String {
        BASE64_URL_SAFE_NO_PAD.encode(Self::random_bytes(32))
    }

    pub fn flow_cookie_name(&self) -> &'static str {
        if self.local && self.origin.starts_with("http:") {
            "lmm-hg-local"
        } else {
            "__Host-lmm-hg"
        }
    }

    pub fn parse_cookie_value<'a>(&self, headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        let mut found = None;
        for raw in headers.get_all(axum::http::header::COOKIE) {
            let Ok(s) = raw.to_str() else { continue };
            for part in s.split(';') {
                if let Some((k, v)) = part.trim().split_once('=')
                    && k == name
                {
                    if found.is_some() {
                        return None; // Duplicate cookie rejected
                    }
                    found = Some(v);
                }
            }
        }
        found
    }

    pub async fn get_settings(
        &self,
        tx: &mut HumanGrantTx,
        now: i64,
    ) -> Result<Settings, ProtocolError> {
        if let Some(s) = tx.get::<Settings>("settings", "site", now).await? {
            Ok(s)
        } else {
            let default_settings = Settings::default();
            tx.put("settings", "site", "", 0, &default_settings).await?;
            Ok(default_settings)
        }
    }

    pub async fn audit(
        &self,
        tx: &mut HumanGrantTx,
        action: &str,
        actor: &str,
        grantor: &str,
        target: &str,
        now: i64,
    ) -> Result<String, ProtocolError> {
        let id = Self::random_id();
        let payload = serde_json::json!({
            "id": id,
            "at": now,
            "action": action,
            "actor": actor,
            "grantor_id": grantor,
            "target_id": target,
        });
        tx.put("audit", &id, grantor, now + RETENTION_SECONDS, &payload)
            .await?;
        Ok(id)
    }

    pub async fn check_limits(
        &self,
        tx: &mut HumanGrantTx,
        limits: &[(&str, i64, i64)],
        now: i64,
    ) -> Result<bool, ProtocolError> {
        #[derive(serde::Serialize, serde::Deserialize, Default)]
        struct Counter {
            count: i64,
        }

        for (key, window, max_count) in limits {
            let bucket = now / window;
            let limit_id = fingerprint(format!("{key}:{bucket}").as_bytes());
            let current = tx
                .get::<Counter>("limit", &limit_id, now)
                .await?
                .unwrap_or_default();
            if current.count >= *max_count {
                return Ok(false);
            }
        }

        for (key, window, _) in limits {
            let bucket = now / window;
            let limit_id = fingerprint(format!("{key}:{bucket}").as_bytes());
            let mut current = tx
                .get::<Counter>("limit", &limit_id, now)
                .await?
                .unwrap_or_default();
            current.count += 1;
            let exp = (bucket + 1) * window + 1;
            tx.put("limit", &limit_id, "", exp, &current).await?;
        }

        Ok(true)
    }

    pub async fn grant_status(
        &self,
        tx: &mut HumanGrantTx,
        grant: &Grant,
        settings: &Settings,
        now: i64,
    ) -> Result<String, ProtocolError> {
        if grant.revoked_at != 0 || grant.epoch != settings.epoch || !settings.enabled {
            return Ok("revoked".to_string());
        }
        if grant.expires_at <= now {
            return Ok("expired".to_string());
        }
        if tx.current_human(&grant.human, now).await?.is_none() {
            return Ok("account_disabled".to_string());
        }
        if self.effective_permissions(grant).is_empty() {
            return Ok("permissions_removed".to_string());
        }
        if grant.token_hash.is_empty() {
            return Ok("pending_claim".to_string());
        }
        Ok("active".to_string())
    }

    pub fn effective_permissions(&self, grant: &Grant) -> Vec<Permission> {
        if grant.client_id != CLIENT_ID || grant.target_service != SERVICE_ID {
            return vec![];
        }
        let mut pairs = Vec::new();
        for p in &grant.permissions {
            if p.action == PROFILE_READ
                && p.resource == grant.human.id
                && grant.requested.contains(p)
            {
                pairs.push(p.clone());
            }
        }
        pairs
    }

    pub async fn build_state(
        &self,
        tx: &mut HumanGrantTx,
        req: &Request,
        settings: &Settings,
        now: i64,
    ) -> Result<State, ProtocolError> {
        let (status, status_text) = match req.phase.as_str() {
            "waiting_human" => ("waiting", "等待人类中"),
            "authenticating_human" => ("authenticating", "人类登录中"),
            "awaiting_consent" => ("awaiting_consent", "等待同意中"),
            "approved" => ("approved", "已批准"),
            "succeeded" => ("success", "登录成功"),
            "rejected" => ("rejected", "已拒绝"),
            "cancelled" => ("cancelled", "已取消"),
            "expired" => ("expired", "已过期"),
            _ => ("unknown", "未知"),
        };

        let email_retry = if req.last_email == 0 {
            0
        } else {
            (req.last_email + 60 - now).max(0)
        };

        let mut grant_id = None;
        let mut grant_expires = None;
        let mut grant_status = "none".to_string();

        if let Some(gid) = &req.grant_id
            && let Some(g) = tx.get::<Grant>("grant", gid, now).await?
        {
            grant_id = Some(g.grant_id.clone());
            grant_expires = Some(g.expires_at);
            grant_status = self.grant_status(tx, &g, settings, now).await?;
        }

        Ok(State {
            protocol_version: VERSION.to_string(),
            request_id: req.id.clone(),
            status: status.to_string(),
            status_text: status_text.to_string(),
            phase: req.phase.clone(),
            created_at: req.created,
            status_updated_at: req.updated,
            checked_at: now,
            expires_at: req.expires,
            poll_after_seconds: req.poll_after,
            human_authorization_url: format!("{}/human/{}", self.origin, req.link),
            requested_permissions: req.input.requested_permissions.clone(),
            email_request_status: req.email_status.clone(),
            email_retry_after_seconds: email_retry,
            target_service: req.input.target_service.clone(),
            client_id: req.input.client_id.clone(),
            forwarding_prompt: format!(
                "Open {}/human/{} to verify your account and choose the access to approve. This link is for the human, not for the AI to claim a credential.",
                self.origin, req.link
            ),
            grant_id,
            grant_status,
            grant_expires_at: grant_expires,
        })
    }

    pub fn state_to_markdown(state: &State) -> String {
        let mut out = String::from("# HumanGrant\n\n");
        let val = serde_json::to_value(state).unwrap_or_default();
        let fields = [
            "protocol_version",
            "request_id",
            "status",
            "status_text",
            "phase",
            "created_at",
            "status_updated_at",
            "checked_at",
            "expires_at",
            "poll_after_seconds",
            "human_authorization_url",
            "requested_permissions",
            "email_request_status",
            "email_retry_after_seconds",
            "target_service",
            "client_id",
            "forwarding_prompt",
            "grant_id",
            "grant_status",
            "grant_expires_at",
        ];
        for f in fields {
            if let Some(v) = val.get(f) {
                out.push_str(&format!("{f}: {v}\n"));
            }
        }
        out
    }

    /// A retained request is not necessarily a live consent request.
    pub async fn check_pending_request(
        &self,
        tx: &mut HumanGrantTx,
        request: &Request,
        now: i64,
    ) -> Result<(), ProtocolError> {
        let settings = self.get_settings(tx, now).await?;
        if !settings.enabled || request.epoch != settings.epoch {
            return Err(failure(403, "feature_disabled"));
        }
        if request.expires <= now {
            return Err(failure(410, "expired"));
        }
        if !matches!(
            request.phase.as_str(),
            "waiting_human" | "authenticating_human" | "awaiting_consent"
        ) {
            return Err(failure(409, "invalid_state"));
        }
        Ok(())
    }

    pub async fn check_flow(
        &self,
        tx: &mut HumanGrantTx,
        headers: &HeaderMap,
        context: &str,
        now: i64,
    ) -> Result<Flow, ProtocolError> {
        let cookie_name = self.flow_cookie_name();
        let key = self
            .parse_cookie_value(headers, cookie_name)
            .ok_or_else(|| failure(403, "csrf_failed"))?;
        let csrf = headers
            .get("x-hg-csrf")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| failure(403, "csrf_failed"))?;

        let flow_id = fingerprint(key.as_bytes());
        let flow = tx
            .get::<Flow>("flow", &flow_id, now)
            .await?
            .ok_or_else(|| failure(403, "csrf_failed"))?;

        if flow.context != context || flow.expires <= now {
            return Err(failure(403, "csrf_failed"));
        }

        if !bool::from(flow.csrf.as_bytes().ct_eq(csrf.as_bytes())) {
            return Err(failure(403, "csrf_failed"));
        }

        Ok(flow)
    }

    pub async fn new_flow(
        &self,
        tx: &mut HumanGrantTx,
        context: &str,
        human: Human,
        verified: bool,
        now: i64,
    ) -> Result<(Flow, String), ProtocolError> {
        let key = Self::random_id();
        let csrf = Self::random_id();
        let flow = Flow {
            csrf,
            context: context.to_string(),
            human: human.clone(),
            verified,
            expires: now + 600,
        };
        let flow_id = fingerprint(key.as_bytes());
        tx.put("flow", &flow_id, &human.id, flow.expires, &flow)
            .await?;
        Ok((flow, key))
    }
}
