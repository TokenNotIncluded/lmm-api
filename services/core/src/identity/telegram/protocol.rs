//! Ordinary Telegram Login Widget assertions, not Mini App init data.
//! https://core.telegram.org/widgets/login-legacy#checking-authorization
//! Cryptographic verification uses the existing MIT OR Apache-2.0 RustCrypto
//! hmac 0.12.1 / sha2 0.10.9 implementations and their constant-time verifier.
use crate::identity::IdentityError;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, IdentityError>;
const ASSERTION_TTL: i64 = 300;
const FUTURE_SKEW: i64 = 30;

/// Contains derived bot key material. Deliberately not Debug or serializable.
pub struct TelegramLogin {
    bot_name: String,
    signing_key: Zeroizing<[u8; 32]>,
}

/// Query values retain their exact decoded text for the Telegram signature.
/// The HTTP adapter removes its own state before deserializing this object.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramAuthorization {
    id: String,
    auth_date: String,
    hash: String,
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
    photo_url: Option<String>,
    lang: Option<String>,
}

/// Constructed only by successful signature, age and canonical identity checks.
/// Profile data, browser booleans and deserialization cannot create this proof.
pub(crate) struct VerifiedTelegramIdentity {
    subject: String,
    auth_date: i64,
    replay_digest: [u8; 32],
}

impl VerifiedTelegramIdentity {
    pub(super) fn subject(&self) -> &str {
        &self.subject
    }

    pub(super) fn auth_date(&self) -> i64 {
        self.auth_date
    }

    /// Login and binding must share the same consumed-assertion namespace.
    pub(super) fn replay_digest(&self) -> [u8; 32] {
        self.replay_digest
    }
}

fn positive_decimal(value: &str) -> Result<i64> {
    if value.is_empty()
        || value.len() > 19
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value.starts_with('0')
    {
        return Err(IdentityError::Unauthorized);
    }
    let number = value
        .parse::<i64>()
        .map_err(|_| IdentityError::Unauthorized)?;
    if number <= 0 {
        return Err(IdentityError::Unauthorized);
    }
    Ok(number)
}

fn bounded_profile(value: &Option<String>, maximum: usize) -> bool {
    value
        .as_ref()
        .is_none_or(|value| value.len() <= maximum && !value.chars().any(char::is_control))
}

fn signature_bytes(hash: &str) -> Result<[u8; 32]> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(IdentityError::Unauthorized);
    }
    let mut signature = [0_u8; 32];
    for (index, byte) in signature.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16)
            .map_err(|_| IdentityError::Unauthorized)?;
    }
    Ok(signature)
}

impl TelegramLogin {
    pub fn new(bot_name: &str, bot_token: &str) -> Result<Self> {
        let bot_name = bot_name.strip_prefix('@').unwrap_or(bot_name);
        if !(5..=32).contains(&bot_name.len())
            || !bot_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || bot_token.is_empty()
            || bot_token.len() > 4096
            || !bot_token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(IdentityError::Invalid);
        }
        Ok(Self {
            bot_name: bot_name.to_owned(),
            signing_key: Zeroizing::new(Sha256::digest(bot_token.as_bytes()).into()),
        })
    }

    pub fn bot_name(&self) -> &str {
        &self.bot_name
    }

    pub(crate) fn verify(
        &self,
        input: &TelegramAuthorization,
        now: i64,
    ) -> Result<VerifiedTelegramIdentity> {
        positive_decimal(&input.id)?;
        let auth_date = positive_decimal(&input.auth_date)?;
        if now <= 0
            || auth_date > now.saturating_add(FUTURE_SKEW)
            || auth_date <= now.saturating_sub(ASSERTION_TTL)
            || !bounded_profile(&input.first_name, 256)
            || !bounded_profile(&input.last_name, 256)
            || !bounded_profile(&input.username, 64)
            || !bounded_profile(&input.photo_url, 2048)
            || !bounded_profile(&input.lang, 64)
        {
            return Err(IdentityError::Unauthorized);
        }
        let signature = signature_bytes(&input.hash)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&*self.signing_key)
            .map_err(|_| IdentityError::Storage)?;
        // Alphabetical field order, including present empty values. Never
        // trim, normalize Unicode, URL-encode again, or include application state.
        let fields = [
            ("auth_date", Some(input.auth_date.as_str())),
            ("first_name", input.first_name.as_deref()),
            ("id", Some(input.id.as_str())),
            ("lang", input.lang.as_deref()),
            ("last_name", input.last_name.as_deref()),
            ("photo_url", input.photo_url.as_deref()),
            ("username", input.username.as_deref()),
        ];
        let mut first = true;
        for (name, value) in fields {
            if let Some(value) = value {
                if !first {
                    mac.update(b"\n");
                }
                first = false;
                mac.update(name.as_bytes());
                mac.update(b"=");
                mac.update(value.as_bytes());
            }
        }
        mac.verify_slice(&signature)
            .map_err(|_| IdentityError::Unauthorized)?;
        Ok(VerifiedTelegramIdentity {
            subject: input.id.clone(),
            auth_date,
            replay_digest: Sha256::digest(signature).into(),
        })
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
