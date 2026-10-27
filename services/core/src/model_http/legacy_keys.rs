//! Explicit aliases for owner-installed legacy keys. This registry changes only
//! the wire spelling; live identity, policy and spending checks remain mandatory.
use super::{ModelGateway, native, one_header, public_error};
use crate::relay::{ClientProtocol, RelayError};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, BTreeSet};

const MAX_KEYS: usize = 10_000;
const CREDENTIAL_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "x-auth-session",
    "mj-api-secret",
    "sec-websocket-protocol",
];

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Target {
    database: String,
    database_oid: i64,
    instance_nonce: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyBinding {
    source_key_id: i64,
    target_credential_id: i64,
    target_user_id: i64,
    target_account_id: i64,
    credential_user_version: i64,
    source_secret_sha256: String,
    sk_alias_sha256: String,
    #[serde(default)]
    revoked_rekey_required: bool,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    scope: String,
    source_archive_sha256: String,
    credential_receipt_sha256: String,
    target: Target,
    keys: Vec<KeyBinding>,
}

// Existing owner-produced mapping contract consumed by legacy_key_quota_opening.py.
// It contains no apply/status assertion and is not financial readiness evidence.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialReceipt {
    scope: String,
    source_archive_sha256: String,
    records: Vec<ReceiptRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptRecord {
    source_key_id: i64,
    target_credential_id: i64,
    target_user_id: i64,
    target_account_id: i64,
    source_secret_sha256: String,
}

#[derive(Clone, Copy)]
enum Spelling {
    Raw,
    SkPrefix,
}

#[derive(Clone)]
pub(super) struct Aliases {
    manifest: Manifest,
    wire: BTreeMap<[u8; 32], (usize, Spelling)>,
    verified: bool,
}

struct CredentialRow {
    id: i64,
    digest: Vec<u8>,
    kind: String,
    user_id: i64,
    user_version: i64,
    owner_account_id: i64,
    personal_account_id: i64,
    owner_kind: String,
    revoked: bool,
    settings_present: bool,
}

fn invalid() -> RelayError {
    RelayError::Invalid("invalid legacy key alias installation")
}

fn hash(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn digest(value: &str) -> Result<[u8; 32], RelayError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid());
    }
    let mut result = [0; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte: u8| {
            if byte <= b'9' {
                byte - b'0'
            } else {
                byte - b'a' + 10
            }
        };
        result[index] = digit(pair[0]) * 16 + digit(pair[1]);
    }
    if result == [0; 32] {
        return Err(invalid());
    }
    Ok(result)
}

impl Aliases {
    pub(super) fn parse(document: &[u8], receipt: &[u8]) -> Result<Self, RelayError> {
        let manifest: Manifest = serde_json::from_slice(document).map_err(|_| invalid())?;
        let receipt_value: CredentialReceipt =
            serde_json::from_slice(receipt).map_err(|_| invalid())?;
        if manifest.version != 1
            || manifest.scope != "legacy-api-key-wire-aliases-v1"
            || receipt_value.scope != "a02-credential-mapping-receipt"
            || receipt_value.source_archive_sha256 != manifest.source_archive_sha256
            || digest(&manifest.credential_receipt_sha256)? != hash(receipt)
            || manifest.keys.is_empty()
            || manifest.keys.len() > MAX_KEYS
            || manifest.keys.len() != receipt_value.records.len()
            || manifest.target.database.is_empty()
            || manifest.target.database.len() > 63
            || manifest.target.database.chars().any(char::is_control)
            || manifest.target.database_oid <= 0
        {
            return Err(invalid());
        }
        digest(&manifest.source_archive_sha256)?;
        digest(&manifest.target.instance_nonce)?;
        let mut receipts = BTreeMap::new();
        for record in receipt_value.records {
            if receipts.insert(record.source_key_id, record).is_some() {
                return Err(invalid());
            }
        }
        let mut ids = BTreeSet::new();
        let mut wire = BTreeMap::new();
        for (index, binding) in manifest.keys.iter().enumerate() {
            let source = receipts
                .remove(&binding.source_key_id)
                .ok_or_else(invalid)?;
            if [
                binding.source_key_id,
                binding.target_credential_id,
                binding.target_user_id,
                binding.target_account_id,
                binding.credential_user_version,
            ]
            .iter()
            .any(|id| *id <= 0)
                || !ids.insert(binding.target_credential_id)
                || source.target_credential_id != binding.target_credential_id
                || source.target_user_id != binding.target_user_id
                || source.target_account_id != binding.target_account_id
                || source.source_secret_sha256 != binding.source_secret_sha256
            {
                return Err(invalid());
            }
            for (value, spelling) in [
                (&binding.source_secret_sha256, Spelling::Raw),
                (&binding.sk_alias_sha256, Spelling::SkPrefix),
            ] {
                if wire.insert(digest(value)?, (index, spelling)).is_some() {
                    return Err(invalid());
                }
            }
        }
        Ok(Self {
            manifest,
            wire,
            verified: false,
        })
    }

    pub(super) async fn verify(&mut self, pool: &PgPool) -> Result<(), RelayError> {
        self.verified = false;
        let mut tx = pool
            .begin()
            .await
            .map_err(|_| RelayError::BillingUnavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(|_| RelayError::BillingUnavailable)?;
        sqlx::query("SET LOCAL search_path=pg_catalog")
            .execute(&mut *tx)
            .await
            .map_err(|_| RelayError::BillingUnavailable)?;
        let row = sqlx::query("SELECT current_database() AS database, (SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()) AS database_oid, current_setting('lmm.migration_instance_nonce',true) AS instance_nonce")
            .fetch_one(&mut *tx).await.map_err(|_| RelayError::BillingUnavailable)?;
        let current = Target {
            database: row
                .try_get("database")
                .map_err(|_| RelayError::BillingUnavailable)?,
            database_oid: row
                .try_get("database_oid")
                .map_err(|_| RelayError::BillingUnavailable)?,
            instance_nonce: row
                .try_get::<Option<String>, _>("instance_nonce")
                .map_err(|_| RelayError::BillingUnavailable)?
                .ok_or_else(invalid)?,
        };
        let ids: Vec<i64> = self
            .manifest
            .keys
            .iter()
            .map(|key| key.target_credential_id)
            .collect();
        let digests: Vec<Vec<u8>> = self.wire.keys().map(|value| value.to_vec()).collect();
        // Include every matching digest, even if owned by an unlisted, revoked or
        // non-API credential. An exact credential can never become a fallback.
        let found = sqlx::query("SELECT c.id,c.digest,c.kind,c.user_id,c.user_version,c.owner_account_id,u.personal_account_id,a.kind AS owner_kind,c.revoked_at IS NOT NULL AS revoked,EXISTS(SELECT 1 FROM core_identity.api_key_settings s WHERE s.credential_id=c.id) AS settings_present FROM core_identity.credentials c JOIN core_identity.users u ON u.id=c.user_id JOIN core_identity.accounts a ON a.id=c.owner_account_id WHERE c.id=ANY($1::bigint[]) OR c.digest=ANY($2::bytea[])")
            .bind(ids).bind(digests).fetch_all(&mut *tx).await.map_err(|_| RelayError::BillingUnavailable)?;
        let rows = found
            .into_iter()
            .map(|row| {
                Ok(CredentialRow {
                    id: row.try_get("id")?,
                    digest: row.try_get("digest")?,
                    kind: row.try_get("kind")?,
                    user_id: row.try_get("user_id")?,
                    user_version: row.try_get("user_version")?,
                    owner_account_id: row.try_get("owner_account_id")?,
                    personal_account_id: row.try_get("personal_account_id")?,
                    owner_kind: row.try_get("owner_kind")?,
                    revoked: row.try_get("revoked")?,
                    settings_present: row.try_get("settings_present")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(|_| RelayError::BillingUnavailable)?;
        self.verify_rows(&current, &rows)?;
        tx.commit()
            .await
            .map_err(|_| RelayError::BillingUnavailable)?;
        self.verified = true;
        Ok(())
    }

    fn verify_rows(&self, target: &Target, rows: &[CredentialRow]) -> Result<(), RelayError> {
        if target != &self.manifest.target {
            return Err(invalid());
        }
        let by_id: BTreeMap<_, _> = rows.iter().map(|row| (row.id, row)).collect();
        if by_id.len() != rows.len() {
            return Err(invalid());
        }
        for key in &self.manifest.keys {
            let row = by_id.get(&key.target_credential_id).ok_or_else(invalid)?;
            let expected = digest(&key.source_secret_sha256)?;
            if row.digest.as_slice() != expected.as_slice()
                || row.kind != "api_key"
                || row.user_id != key.target_user_id
                || row.user_version != key.credential_user_version
                || row.owner_account_id != key.target_account_id
                || row.personal_account_id != key.target_account_id
                || row.owner_kind != "personal"
                || (key.revoked_rekey_required && !row.revoked)
                || !row.settings_present
            {
                return Err(invalid());
            }
        }
        for row in rows {
            let observed: [u8; 32] = row.digest.as_slice().try_into().map_err(|_| invalid())?;
            if let Some((index, spelling)) = self.wire.get(&observed)
                && (self.manifest.keys[*index].target_credential_id != row.id
                    || matches!(spelling, Spelling::SkPrefix))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn normalize(
        &self,
        path: &str,
        query: Option<&str>,
        headers: &mut HeaderMap,
    ) -> Result<(), RelayError> {
        if !self.verified {
            return Err(RelayError::BillingUnavailable);
        }
        let mut present = 0;
        for name in CREDENTIAL_HEADERS {
            if one_header(headers, name)?.is_some() {
                present += 1;
            }
        }
        if present > 1 {
            return Err(RelayError::Invalid("ambiguous model credentials"));
        }
        let alternate = if path == "/v1/messages" {
            Some(("x-api-key", ClientProtocol::Anthropic))
        } else if path.starts_with("/v1beta/models/") {
            Some(("x-goog-api-key", ClientProtocol::Gemini))
        } else if path == "/v1/models" && headers.contains_key("x-api-key") {
            Some(("x-api-key", ClientProtocol::Anthropic))
        } else if path == "/v1/models" && headers.contains_key("x-goog-api-key") {
            Some(("x-goog-api-key", ClientProtocol::Gemini))
        } else {
            None
        };
        let carrier = if headers.contains_key(header::AUTHORIZATION) {
            "authorization"
        } else if let Some((name, _)) = alternate {
            name
        } else {
            "authorization"
        };
        if let Some(value) = one_header(headers, carrier)? {
            let wire = if carrier == "authorization"
                && value
                    .get(..7)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Bearer "))
            {
                value[7..].trim()
            } else {
                value
            };
            if let Some((index, spelling)) = self.wire.get(&hash(wire.as_bytes())) {
                let canonical = match spelling {
                    Spelling::Raw => wire,
                    Spelling::SkPrefix => wire.strip_prefix("sk-").ok_or_else(invalid)?,
                };
                if self.manifest.keys[*index].revoked_rekey_required {
                    return Err(RelayError::RevokedLegacyKey);
                }
                if !ordinary(canonical)
                    || hash(canonical.as_bytes())
                        != digest(&self.manifest.keys[*index].source_secret_sha256)?
                    || headers.contains_key("x-lmm-group")
                    || query.is_some_and(|query| {
                        !(path.starts_with("/v1beta/models/") && query == "alt=sse")
                    })
                {
                    return Err(RelayError::Invalid(
                        "legacy credential cannot select another authority",
                    ));
                }
                let value = if carrier == "authorization" {
                    format!("Bearer {canonical}")
                } else {
                    canonical.to_owned()
                };
                let mut value = HeaderValue::from_str(&value).map_err(|_| invalid())?;
                value.set_sensitive(true);
                headers.insert(carrier, value);
            }
        }
        if let Some((_, client)) = alternate {
            *headers = native::key_headers(std::mem::take(headers), client)?;
        } else if CREDENTIAL_HEADERS[1..]
            .iter()
            .any(|name| headers.contains_key(*name))
        {
            return Err(RelayError::Invalid("unsupported model credential header"));
        }
        Ok(())
    }
}

fn ordinary(secret: &str) -> bool {
    let lower = secret.to_ascii_lowercase();
    (32..=256).contains(&secret.len())
        && secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !lower.contains("lmm_")
        && !lower.contains("oauth_managed_")
        && ![
            "lmmk_", "lmms_", "lmmr_", "lmmi_", "lmmf_", "lmmt_", "lmmo_",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

pub(super) async fn normalize(
    State(gateway): State<ModelGateway>,
    request: Request,
    next: Next,
) -> Response {
    let Some(aliases) = &gateway.legacy_keys else {
        return next.run(request).await;
    };
    let (mut parts, body) = request.into_parts();
    if let Err(error) = aliases.normalize(parts.uri.path(), parts.uri.query(), &mut parts.headers) {
        return if parts.uri.path() == "/v1/messages" {
            native::error_response(ClientProtocol::Anthropic, error)
        } else if parts.uri.path().starts_with("/v1beta/models/") {
            native::error_response(ClientProtocol::Gemini, error)
        } else {
            public_error(error)
        };
    }
    next.run(Request::from_parts(parts, body)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KEY: &str = "LegacySyntheticPrivateKey0123456789012345";
    fn encoded(value: &[u8]) -> String {
        format!("{:x}", Sha256::digest(value))
    }
    fn fixture_with_key(key: &str) -> (serde_json::Value, Vec<u8>) {
        let receipt = serde_json::to_vec(&json!({"scope":"a02-credential-mapping-receipt",
            "source_archive_sha256":"a".repeat(64),"records":[{"source_key_id":11,
            "target_credential_id":201,"target_user_id":71,"target_account_id":1,
            "source_secret_sha256":encoded(key.as_bytes())}]}))
        .unwrap();
        (
            json!({"version":1,"scope":"legacy-api-key-wire-aliases-v1",
            "source_archive_sha256":"a".repeat(64),
            "credential_receipt_sha256":encoded(&receipt),
            "target":{"database":"synthetic","database_oid":123,"instance_nonce":"c".repeat(64)},
            "keys":[{"source_key_id":11,"target_credential_id":201,"target_user_id":71,
                "target_account_id":1,"credential_user_version":3,"source_secret_sha256":encoded(key.as_bytes()),
                "sk_alias_sha256":encoded(format!("sk-{key}").as_bytes())}]}),
            receipt,
        )
    }
    fn fixture() -> (serde_json::Value, Vec<u8>) {
        fixture_with_key(KEY)
    }
    fn aliases() -> Aliases {
        let (manifest, receipt) = fixture();
        let mut aliases =
            Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), &receipt).unwrap();
        aliases.verified = true;
        aliases
    }
    fn row() -> CredentialRow {
        CredentialRow {
            id: 201,
            digest: hash(KEY.as_bytes()).to_vec(),
            kind: "api_key".into(),
            user_id: 71,
            user_version: 3,
            owner_account_id: 1,
            personal_account_id: 1,
            owner_kind: "personal".into(),
            revoked: false,
            settings_present: true,
        }
    }

    #[test]
    fn legacy_wire_registered_spellings_share_one_sensitive_canonical_header() {
        for value in [
            KEY.to_owned(),
            format!("sk-{KEY}"),
            format!("Bearer {KEY}"),
            format!("bEaReR  sk-{KEY} "),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            aliases()
                .normalize("/v1/chat/completions", None, &mut headers)
                .unwrap();
            assert_eq!(
                headers[header::AUTHORIZATION],
                format!("Bearer {KEY}"),
                "registered spelling"
            );
            assert!(headers[header::AUTHORIZATION].is_sensitive());
        }
    }

    #[test]
    fn legacy_wire_unregistered_native_and_suffix_values_are_never_rewritten() {
        for value in [
            format!("Bearer lmmk_{}", "a".repeat(64)),
            format!("Bearer sk-lmmk_{}", "a".repeat(64)),
            format!("Bearer sk-{KEY}-17"),
            format!("Bearer sk-sk-{KEY}"),
            "Bearer lmm_at_untrusted".into(),
            format!("sk-{KEY}-17"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            aliases()
                .normalize("/v1/chat/completions", None, &mut headers)
                .unwrap();
            assert_eq!(headers[header::AUTHORIZATION], value);
        }
    }

    #[test]
    fn legacy_wire_preserves_native_carriers_catalog_and_gemini_stream_query() {
        for (path, name, query) in [
            ("/v1/messages", "x-api-key", None),
            (
                "/v1beta/models/model:generateContent",
                "x-goog-api-key",
                None,
            ),
            (
                "/v1beta/models/model:streamGenerateContent",
                "x-goog-api-key",
                Some("alt=sse"),
            ),
            ("/v1/models", "x-api-key", None),
            (
                "/v1beta/models/model%3AstreamGenerateContent",
                "x-goog-api-key",
                Some("alt=sse"),
            ),
            ("/v1/models", "x-goog-api-key", None),
            ("/v1/images/generations", "authorization", None),
            ("/v1/systemone", "authorization", None),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, format!("sk-{KEY}").parse().unwrap());
            aliases().normalize(path, query, &mut headers).unwrap();
            assert_eq!(
                headers[header::AUTHORIZATION],
                format!("Bearer {KEY}"),
                "path={path}"
            );
            assert!(headers[header::AUTHORIZATION].is_sensitive());
        }
    }

    #[test]
    fn legacy_wire_rejects_mixed_duplicate_oauth_query_and_wrong_protocol_carriers() {
        for alternate in &CREDENTIAL_HEADERS[1..] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                format!("Bearer {KEY}").parse().unwrap(),
            );
            headers.insert(*alternate, HeaderValue::from_static("lmm_at_other"));
            assert!(
                aliases()
                    .normalize("/v1/chat/completions", None, &mut headers)
                    .is_err()
            );
        }
        let mut duplicate = HeaderMap::new();
        duplicate.append(header::AUTHORIZATION, KEY.parse().unwrap());
        duplicate.append(header::AUTHORIZATION, KEY.parse().unwrap());
        assert!(
            aliases()
                .normalize("/v1/models", None, &mut duplicate)
                .is_err()
        );
        let mut group = HeaderMap::new();
        group.insert(header::AUTHORIZATION, KEY.parse().unwrap());
        group.insert("x-lmm-group", HeaderValue::from_static(""));
        assert!(aliases().normalize("/v1/models", None, &mut group).is_err());
        for (path, name, query) in [
            ("/v1/messages", "x-goog-api-key", None),
            ("/v1/chat/completions", "x-api-key", None),
            ("/v1/images/generations", "x-api-key", None),
            ("/v1/systemone", "x-api-key", None),
            (
                "/v1beta/models/m:streamGenerateContent",
                "x-goog-api-key",
                Some("alt=sse&key=secret"),
            ),
            ("/v1/models", "authorization", Some("access_token=secret")),
            ("/v1/models", "authorization", Some("%6bey=secret")),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, KEY.parse().unwrap());
            assert!(
                aliases().normalize(path, query, &mut headers).is_err(),
                "path={path}"
            );
        }
    }

    #[test]
    fn legacy_wire_requires_actual_target_binding_and_rejects_any_alias_digest_collision() {
        let aliases = aliases();
        let target = aliases.manifest.target.clone();
        assert!(aliases.verify_rows(&target, &[row()]).is_ok());
        let mut wrong = target.clone();
        wrong.instance_nonce = "d".repeat(64);
        assert!(aliases.verify_rows(&wrong, &[row()]).is_err());
        assert!(aliases.verify_rows(&target, &[]).is_err());
        for changed in ["user", "account", "version", "kind", "settings", "digest"] {
            let mut bad = row();
            match changed {
                "user" => bad.user_id += 1,
                "account" => bad.owner_account_id += 1,
                "version" => bad.user_version += 1,
                "kind" => bad.kind = "session".into(),
                "settings" => bad.settings_present = false,
                _ => bad.digest = vec![0; 32],
            };
            assert!(
                aliases.verify_rows(&target, &[bad]).is_err(),
                "field={changed}"
            );
        }
        let mut collision = row();
        collision.id = 999;
        collision.digest = hash(format!("sk-{KEY}").as_bytes()).to_vec();
        assert!(
            aliases.verify_rows(&target, &[row(), collision]).is_err(),
            "all matching rows, including revoked credentials, must be supplied by the SQL query"
        );
    }

    #[test]
    fn legacy_wire_rejects_changed_receipts_blocked_proposals_and_unverified_mode() {
        let (mut manifest, receipt) = fixture();
        assert!(Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), b"{}").is_err());
        let proposal = json!({"scope":"legacy-api-key-proposal","status":"blocked"});
        let proposal = serde_json::to_vec(&proposal).unwrap();
        manifest["credential_receipt_sha256"] = json!(encoded(&proposal));
        assert!(Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), &proposal).is_err());
        let (manifest, _) = fixture();
        let aliases = Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), &receipt).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, KEY.parse().unwrap());
        assert!(matches!(
            aliases.normalize("/v1/models", None, &mut headers),
            Err(RelayError::BillingUnavailable)
        ));
    }

    #[test]
    fn legacy_wire_never_reinterprets_listed_native_oauth_managed_or_channel_secrets() {
        for key in [
            format!("lmm_at_{}", "a".repeat(64)),
            format!("lmm_rt_{}", "a".repeat(64)),
            format!("lmmk_{}", "a".repeat(64)),
            format!("lmms_{}", "a".repeat(64)),
            format!("oauth_managed_{}", "a".repeat(64)),
            format!("{KEY}-17"),
        ] {
            let (manifest, receipt) = fixture_with_key(&key);
            let mut aliases =
                Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), &receipt).unwrap();
            aliases.verified = true;
            for value in [key.clone(), format!("sk-{key}")] {
                let mut headers = HeaderMap::new();
                headers.insert(header::AUTHORIZATION, value.parse().unwrap());
                assert!(
                    aliases.normalize("/v1/models", None, &mut headers).is_err(),
                    "reserved spelling"
                );
            }
        }
    }

    #[test]
    fn legacy_wire_revoked_rekey_required_returns_explicit_recreate_prompt() {
        let managed = format!("oauth_managed_{}", "a".repeat(32));
        for key in [KEY.to_owned(), managed] {
            let (mut manifest, receipt) = fixture_with_key(&key);
            manifest["keys"][0]["revoked_rekey_required"] = json!(true);
            let mut aliases =
                Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), &receipt).unwrap();
            let target = aliases.manifest.target.clone();
            let mut unrevoked = row();
            unrevoked.digest = hash(key.as_bytes()).to_vec();
            assert!(
                aliases.verify_rows(&target, &[unrevoked]).is_err(),
                "revoked_rekey_required binding must reject unrevoked database row"
            );
            let mut revoked_row = row();
            revoked_row.digest = hash(key.as_bytes()).to_vec();
            revoked_row.revoked = true;
            assert!(aliases.verify_rows(&target, &[revoked_row]).is_ok());
            aliases.verified = true;
            for value in [
                key.clone(),
                format!("sk-{key}"),
                format!("Bearer {key}"),
                format!("Bearer sk-{key}"),
            ] {
                let mut headers = HeaderMap::new();
                headers.insert(header::AUTHORIZATION, value.parse().unwrap());
                let err = aliases
                    .normalize("/v1/chat/completions", None, &mut headers)
                    .unwrap_err();
                assert_eq!(err, RelayError::RevokedLegacyKey);
                assert_eq!(err.status(), 401);
                assert!(err.to_string().contains("recreate") || err.to_string().contains("重新创建"));
            }
        }
    }

    async fn pg_fixture(
        pool: PgPool,
    ) -> (
        PgPool,
        crate::identity::IdentityStore,
        crate::identity::IssuedCredential,
        Aliases,
    ) {
        crate::core_schema::init(&pool).await.unwrap();
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        let session = identity.bootstrap_user(71, 1).await.unwrap();
        let actor = identity.authorize(&session.secret).await.unwrap();
        let version: i64 =
            sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=71")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query("INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at) OVERRIDING SYSTEM VALUE VALUES(201,$1,'api_key',71,$2,$3,clock_timestamp()+interval '1 hour')")
            .bind(hash(KEY.as_bytes()).to_vec()).bind(version).bind(actor.owner_account_id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES(201,'Synthetic legacy key','allowed')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES(201,0,$1)").bind(actor.owner_account_id).execute(&pool).await.unwrap();
        let (mut manifest, mut receipt) = fixture();
        let mut records: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
        records["records"][0]["target_account_id"] = json!(actor.owner_account_id);
        receipt = serde_json::to_vec(&records).unwrap();
        manifest["credential_receipt_sha256"] = json!(encoded(&receipt));
        manifest["keys"][0]["target_account_id"] = json!(actor.owner_account_id);
        manifest["keys"][0]["credential_user_version"] = json!(version);
        let target=sqlx::query("SELECT current_database() AS database,(SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()) AS database_oid").fetch_one(&pool).await.unwrap();
        manifest["target"]["database"] = json!(target.get::<String, _>("database"));
        manifest["target"]["database_oid"] = json!(target.get::<i64, _>("database_oid"));
        let options = pool.connect_options().as_ref().clone().options([(
            "lmm.migration_instance_nonce",
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        )]);
        let observed = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let aliases = Aliases::parse(&serde_json::to_vec(&manifest).unwrap(), &receipt).unwrap();
        (observed, identity, session, aliases)
    }

    #[sqlx::test]
    async fn legacy_wire_pg_revocation_still_uses_live_native_authority(pool: PgPool) {
        let (observed, identity, session, mut aliases) = pg_fixture(pool).await;
        aliases.verify(&observed).await.unwrap();
        assert_eq!(identity.authorize(KEY).await.unwrap().credential_id, 201);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer sk-{KEY}").parse().unwrap(),
        );
        aliases
            .normalize("/v1/chat/completions", None, &mut headers)
            .unwrap();
        let canonical = headers[header::AUTHORIZATION]
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap();
        assert_eq!(
            identity.authorize(canonical).await.unwrap().credential_id,
            201
        );
        identity.revoke(&session.secret, 201).await.unwrap();
        aliases.verify(&observed).await.unwrap();
        assert!(matches!(
            identity.authorize(canonical).await,
            Err(crate::identity::IdentityError::Unauthorized)
        ));
        observed.close().await;
    }

    #[sqlx::test]
    async fn legacy_wire_pg_user_version_and_revoked_external_collision_fail_closed(pool: PgPool) {
        let (observed, identity, _, mut aliases) = pg_fixture(pool.clone()).await;
        aliases.verify(&observed).await.unwrap();
        sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=71")
            .execute(&pool)
            .await
            .unwrap();
        aliases.verify(&observed).await.unwrap();
        assert!(matches!(
            identity.authorize(KEY).await,
            Err(crate::identity::IdentityError::Unauthorized)
        ));
        sqlx::query("INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at,revoked_at) OVERRIDING SYSTEM VALUE SELECT 999,$1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour',clock_timestamp() FROM core_identity.users WHERE id=71")
            .bind(hash(format!("sk-{KEY}").as_bytes()).to_vec()).execute(&pool).await.unwrap();
        assert!(
            aliases.verify(&observed).await.is_err(),
            "a revoked credential outside the manifest still occupies the wire digest"
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, KEY.parse().unwrap());
        assert!(matches!(
            aliases.normalize("/v1/models", None, &mut headers),
            Err(RelayError::BillingUnavailable)
        ));
        observed.close().await;
    }

    fn model_gateway(pool: PgPool) -> ModelGateway {
        use std::{sync::Arc, time::Duration};
        let config=serde_json::from_value(json!({"version":1,
            "groups":[{"name":"allowed","enabled":true}],
            "upstreams":[{"id":1,"endpoint":"https://legacy-wire.invalid/v1","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"model","group":"allowed","enabled":true,"speeds":["standard"],
                "targets":[{"id":1,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":1000}],
                "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]})).unwrap();
        let prices=crate::routing::PriceSnapshot::compile(serde_json::from_value(json!({"version":1,"rules":[{
            "id":1,"model":"model","group":"allowed","speed":"standard",
            "rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap()).unwrap();
        let limits = crate::relay::Limits {
            total_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: 20,
            ..Default::default()
        };
        ModelGateway::new(
            pool.clone(),
            Arc::new(crate::routing::Router::new(config, Arc::new(prices)).unwrap()),
            vec![super::super::UpstreamCredential {
                reference: crate::routing::CredentialRef { id: 7, revision: 1 },
                secret: "synthetic-unreachable-provider-key".into(),
            }],
            BTreeMap::from([(201, "allowed".into())]),
            limits.clone(),
            super::super::ChargePolicy {
                input_token_ceiling: 100,
                lease_seconds: 20,
            },
        )
        .unwrap()
        .with_honeypot(
            super::super::HoneypotGateway::new(
                crate::abuse::AbuseStore::new(pool, BTreeSet::new()),
                BTreeMap::new(),
                limits,
            )
            .unwrap(),
        )
    }

    #[sqlx::test]
    async fn legacy_wire_pg_router_canonicalizes_before_honeypot_and_native_handlers(pool: PgPool) {
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use std::sync::Arc;
        use tower::ServiceExt;
        let (observed, _, _, mut aliases) = pg_fixture(pool.clone()).await;
        let mut gateway = model_gateway(pool.clone());
        let response = gateway
            .router()
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .header(header::AUTHORIZATION, format!("Bearer sk-{KEY}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "unconfigured mode retains the original exact-secret behavior"
        );
        aliases.verify(&observed).await.unwrap();
        gateway.legacy_keys = Some(Arc::new(aliases));
        let router = gateway.router();
        let ungranted = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .header(header::AUTHORIZATION, format!("sk-{KEY}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ungranted.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(ungranted.into_body(), 65_536).await.unwrap())
                .unwrap();
        assert_eq!(
            body["data"],
            json!([]),
            "a legacy alias must not manufacture a missing routing grant"
        );
        sqlx::query("INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES(71,TRUE,'synthetic-legacy-wire-pg') ON CONFLICT(user_id) DO UPDATE SET granted=EXCLUDED.granted,source_reference=EXCLUDED.source_reference")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(71,'allowed',TRUE,'synthetic-legacy-wire-pg')")
            .execute(&pool).await.unwrap();
        for carrier in ["authorization", "x-api-key", "x-goog-api-key"] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/models")
                        .header(carrier, format!("sk-{KEY}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "catalog carrier={carrier}"
            );
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap())
                    .unwrap();
            assert_eq!(body["data"].as_array().unwrap().len(), 1);
            assert_eq!(
                body["data"][0]["id"], "model",
                "catalog carrier={carrier} must expose the explicitly granted fixture model"
            );
        }
        for path in [
            "/v1/chat/completions",
            "/v1/responses",
            "/v1/messages",
            "/v1beta/models/model:generateContent",
            "/v1/systemone",
            "/typesafe/v1/systemone",
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header::AUTHORIZATION, format!("sk-{KEY}"))
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "path={path} must reject the missing request fields after canonical authentication"
            );
        }
        observed.close().await;
    }
}
