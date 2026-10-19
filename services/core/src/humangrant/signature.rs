//! Ed25519 HTTP Signature profile verification for HumanGrant 0.1 Draft.

use super::types::{BODY_LIMIT, Proof, ProtocolError, failure};
use base64::prelude::*;
use ring::signature::{ED25519, UnparsedPublicKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub fn fingerprint(data: &[u8]) -> String {
    let sum = Sha256::digest(data);
    BASE64_URL_SAFE_NO_PAD.encode(sum)
}

pub fn origin(raw: &str, local_allowed: bool) -> Result<String, ProtocolError> {
    let u = reqwest::Url::parse(raw).map_err(|_| failure(400, "invalid_origin"))?;
    if u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || (u.path() != "" && u.path() != "/")
    {
        return Err(failure(400, "invalid_origin"));
    }
    if u.scheme() != "https" {
        if !local_allowed
            || u.scheme() != "http"
            || !matches!(
                u.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            )
        {
            return Err(failure(400, "invalid_origin"));
        }
    }
    let result = format!("{}://{}", u.scheme(), u.host_str().unwrap());
    let formatted = if let Some(port) = u.port() {
        format!("{}:{}", result, port)
    } else {
        result
    };
    if raw != formatted && raw != format!("{}/", formatted) {
        return Err(failure(400, "noncanonical_origin"));
    }
    Ok(formatted)
}

pub fn clean_path(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    if path.contains('%')
        || path.contains('\\')
        || path.contains('\r')
        || path.contains('\n')
        || path.contains("//")
        || path.contains("/./")
        || path.contains("/../")
    {
        return false;
    }
    true
}

fn decode_canonical_b64url(value: &str, expected_len: usize) -> Option<Vec<u8>> {
    let decoded = BASE64_URL_SAFE_NO_PAD.decode(value).ok()?;
    if decoded.len() != expected_len || BASE64_URL_SAFE_NO_PAD.encode(&decoded) != value {
        return None;
    }
    Some(decoded)
}

fn decode_canonical_b64std(value: &str, expected_len: usize) -> Option<Vec<u8>> {
    let decoded = BASE64_STANDARD.decode(value).ok()?;
    if decoded.len() != expected_len || BASE64_STANDARD.encode(&decoded) != value {
        return None;
    }
    Some(decoded)
}

/// Parses the signature input parameters:
/// `hg=("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization");created=<c>;expires=<e>;nonce="<nonce>";keyid="<keyid>";alg="ed25519";tag="humangrant-0.1"`
pub struct ParsedSignatureInput {
    pub created: i64,
    pub expires: i64,
    pub nonce: String,
    pub keyid: String,
    pub raw_params: String,
}

pub fn parse_signature_input(input: &str) -> Option<ParsedSignatureInput> {
    const PREFIX: &str = "hg=(\"@method\" \"@target-uri\" \"content-digest\" \"content-type\" \"x-hg-key\" \"authorization\");";
    if !input.starts_with(PREFIX) {
        return None;
    }
    let raw_params = input.strip_prefix("hg=")?;
    let params_str = input.strip_prefix(PREFIX)?;

    let mut created = None;
    let mut expires = None;
    let mut nonce = None;
    let mut keyid = None;
    let mut alg = None;
    let mut tag = None;

    for part in params_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        if let Some(val) = part.strip_prefix("created=") {
            if created.is_some() {
                return None; // Duplicate parameter rejected
            }
            created = Some(val.parse::<i64>().ok()?);
        } else if let Some(val) = part.strip_prefix("expires=") {
            if expires.is_some() {
                return None; // Duplicate parameter rejected
            }
            expires = Some(val.parse::<i64>().ok()?);
        } else if let Some(val) = part.strip_prefix("nonce=") {
            if nonce.is_some() {
                return None; // Duplicate parameter rejected
            }
            if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                let n = &val[1..val.len() - 1];
                if n.len() != 32 || decode_canonical_b64url(n, 24).is_none() {
                    return None;
                }
                nonce = Some(n.to_string());
            } else {
                return None;
            }
        } else if let Some(val) = part.strip_prefix("keyid=") {
            if keyid.is_some() {
                return None; // Duplicate parameter rejected
            }
            if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                keyid = Some(val[1..val.len() - 1].to_string());
            } else {
                return None;
            }
        } else if let Some(val) = part.strip_prefix("alg=") {
            if alg.is_some() {
                return None; // Duplicate parameter rejected
            }
            if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                alg = Some(val[1..val.len() - 1].to_string());
            } else {
                return None;
            }
        } else if let Some(val) = part.strip_prefix("tag=") {
            if tag.is_some() {
                return None; // Duplicate parameter rejected
            }
            if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                tag = Some(val[1..val.len() - 1].to_string());
            } else {
                return None;
            }
        } else {
            // Unknown parameter rejected
            return None;
        }
    }

    if alg.as_deref() != Some("ed25519") || tag.as_deref() != Some("humangrant-0.1") {
        return None;
    }

    let created = created?;
    let expires = expires?;
    let nonce = nonce?;
    let keyid = keyid?;

    Some(ParsedSignatureInput {
        created,
        expires,
        nonce,
        keyid,
        raw_params: raw_params.to_string(),
    })
}

/// Verify exact signature bytes and canonical headers according to HumanGrant 0.1 profile.
pub fn verify_signature(
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    configured_origin: &str,
    now: i64,
) -> Result<Proof, ProtocolError> {
    let fail = || failure(401, "invalid_signature");

    if !clean_path(path) || body.len() > BODY_LIMIT || (method == "GET" && !body.is_empty()) {
        return Err(fail());
    }

    let content_type = headers.get("content-type").ok_or_else(fail)?;
    let content_digest = headers.get("content-digest").ok_or_else(fail)?;
    let x_hg_key = headers.get("x-hg-key").ok_or_else(fail)?;
    let authorization = headers.get("authorization").ok_or_else(fail)?;
    let signature_input = headers.get("signature-input").ok_or_else(fail)?;
    let signature = headers.get("signature").ok_or_else(fail)?;

    if content_type != "application/json" {
        return Err(fail());
    }

    let digest_val = Sha256::digest(body);
    let expected_digest = format!("sha-256=:{}:", BASE64_STANDARD.encode(digest_val));
    if content_digest != &expected_digest {
        return Err(fail());
    }

    let pubkey_bytes = decode_canonical_b64url(x_hg_key, 32).ok_or_else(fail)?;

    if !authorization.starts_with("HumanGrant ") {
        return Err(fail());
    }
    let credential = authorization["HumanGrant ".len()..].trim();
    if credential != "anonymous" {
        if decode_canonical_b64url(credential, 32).is_none() {
            return Err(fail());
        }
    }

    let parsed = parse_signature_input(signature_input).ok_or_else(fail)?;

    if parsed.expires <= parsed.created
        || parsed.expires - parsed.created > 60
        || parsed.created > now + 30
        || parsed.expires <= now
    {
        return Err(fail());
    }

    if decode_canonical_b64url(&parsed.nonce, 24).is_none() {
        return Err(fail());
    }

    let expected_actor = format!("ed25519:{}", fingerprint(&pubkey_bytes));
    if parsed.keyid != expected_actor {
        return Err(fail());
    }

    if !signature.starts_with("hg=:") || !signature.ends_with(':') {
        return Err(fail());
    }
    let sig_raw = &signature["hg=:".len()..signature.len() - 1];
    let sig_bytes = decode_canonical_b64std(sig_raw, 64).ok_or_else(fail)?;

    let base = format!(
        "\"@method\": {}\n\"@target-uri\": {}{}\n\"content-digest\": {}\n\"content-type\": {}\n\"x-hg-key\": {}\n\"authorization\": {}\n\"@signature-params\": {}",
        method,
        configured_origin,
        path,
        content_digest,
        content_type,
        x_hg_key,
        authorization,
        parsed.raw_params
    );

    let peer_public_key = UnparsedPublicKey::new(&ED25519, &pubkey_bytes);
    peer_public_key
        .verify(base.as_bytes(), &sig_bytes)
        .map_err(|_| fail())?;

    Ok(Proof {
        actor: expected_actor,
        public_key: x_hg_key.clone(),
        credential: credential.to_string(),
        nonce: parsed.nonce,
        created: parsed.created,
        expires: parsed.expires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_signature_vector() {
        let method = "POST";
        let origin_str = "https://auth.example.test";
        let path = "/v1/requests";
        let created = 1700000000;
        let body = r#"{"client_id":"example-cli","target_service":"notes","purpose":"Read a note","requested_permissions":[{"action":"notes.read","resource":"alice-note"}],"expected_human_id":"alice"}"#.as_bytes();

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert(
            "content-digest".to_string(),
            "sha-256=:jN6odj5Tli9ZfoLdtLxtE/sUNtMKAKM2V7xq8/qri5Y=:".to_string(),
        );
        headers.insert(
            "x-hg-key".to_string(),
            "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo".to_string(),
        );
        headers.insert(
            "authorization".to_string(),
            "HumanGrant anonymous".to_string(),
        );
        headers.insert(
            "signature-input".to_string(),
            r#"hg=("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization");created=1700000000;expires=1700000060;nonce="AAECAwQFBgcICQoLDA0ODxAREhMUFRYX";keyid="ed25519:If4x36FUomFia_hUBG_SJxt77UtqvkWqWId-9H-XIbk";alg="ed25519";tag="humangrant-0.1""#.to_string(),
        );
        headers.insert(
            "signature".to_string(),
            "hg=:jmjuzgV58zQY0A8N+T9sss24Jwa8Kah5bvs2VSdWA43TScypSfoPkzlERC6XLBOvx8+D+ZqQ0WOVKwS2/Gd3AA==:".to_string(),
        );

        let proof = verify_signature(method, path, &headers, body, origin_str, created + 10)
            .expect("signature must verify against public test vector");

        assert_eq!(
            proof.actor,
            "ed25519:If4x36FUomFia_hUBG_SJxt77UtqvkWqWId-9H-XIbk"
        );
        assert_eq!(proof.credential, "anonymous");
        assert_eq!(proof.nonce, "AAECAwQFBgcICQoLDA0ODxAREhMUFRYX");
        assert_eq!(proof.created, 1700000000);
        assert_eq!(proof.expires, 1700000060);
    }
}
