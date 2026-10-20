//! HumanGrant Protocol 0.1 Draft types and data transfer objects.

use serde::{Deserialize, Serialize};

pub const VERSION: &str = "HumanGrant Protocol 0.1 Draft";
pub const BODY_LIMIT: usize = 16 * 1024; // 16 KiB
pub const SERVICE_ID: &str = "lmm-api";
pub const CLIENT_ID: &str = "lmm-agent";
pub const PROFILE_READ: &str = "account.profile.read";
pub const BROWSER_PREFIX: &str = "/api/user/auth/humangrant";
pub const RETENTION_SECONDS: i64 = 30 * 86400; // 30 days

fn default_session_ttl() -> i64 {
    3600
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    pub action: String,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRequest {
    pub client_id: String,
    pub target_service: String,
    pub purpose: String,
    pub requested_permissions: Vec<Permission>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_human_id: Option<String>,
    #[serde(default = "default_session_ttl")]
    pub session_ttl_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub permissions: Vec<Permission>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailRequest {
    pub email: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub epoch: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            epoch: 1,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Human {
    pub id: String,
    pub session_id: String,
    pub session_version: i64,
    pub auth_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub display_name: String,
    #[serde(skip)]
    pub root: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub actor: String,
    pub public_key: String,
    pub credential: String,
    pub nonce: String,
    pub created: i64,
    pub expires: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub link: String,
    pub actor: String,
    pub public_key: String,
    pub input: CreateRequest,
    pub epoch: i64,
    pub created: i64,
    pub updated: i64,
    pub expires: i64,
    pub phase: String,
    pub poll_after: i64,
    pub last_poll: i64,
    pub email_status: String,
    pub last_email: i64,
    pub grant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub grant_id: String,
    pub request_id: String,
    pub human: Human,
    pub actor_id: String,
    pub public_key: String,
    pub client_id: String,
    pub target_service: String,
    pub requested: Vec<Permission>,
    pub permissions: Vec<Permission>,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked_at: i64,
    pub token_hash: String,
    pub epoch: i64,
    pub audit_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub csrf: String,
    pub context: String,
    pub human: Human,
    pub verified: bool,
    pub expires: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub protocol_version: String,
    pub request_id: String,
    pub status: String,
    pub status_text: String,
    pub phase: String,
    pub created_at: i64,
    pub status_updated_at: i64,
    pub checked_at: i64,
    pub expires_at: i64,
    pub poll_after_seconds: i64,
    pub human_authorization_url: String,
    pub requested_permissions: Vec<Permission>,
    pub email_request_status: String,
    pub email_retry_after_seconds: i64,
    pub target_service: String,
    pub client_id: String,
    pub forwarding_prompt: String,
    pub grant_id: Option<String>,
    pub grant_status: String,
    pub grant_expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub error: String,
    pub message: String,
    pub retry_after_seconds: i64,
    #[serde(skip)]
    pub status: u16,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for ProtocolError {}

pub fn failure(status: u16, code: &str) -> ProtocolError {
    ProtocolError {
        error: code.to_string(),
        message: "The request could not be completed.".to_string(),
        retry_after_seconds: 0,
        status,
    }
}

pub fn limited(seconds: i64) -> ProtocolError {
    let mut err = failure(429, "slow_down");
    err.retry_after_seconds = seconds;
    err
}
