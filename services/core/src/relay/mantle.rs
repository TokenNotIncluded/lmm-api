//! AWS Mantle Messages uses Anthropic JSON/SSE and a distinct AWS authentication
//! contract. Signing is performed by AWS's Apache-2.0 `aws-sigv4` crate.
//!
//! Endpoint/header evidence:
//! https://docs.aws.amazon.com/bedrock/latest/userguide/inference-messages-api.html
//! Service signing name (`bedrock-mantle`):
//! https://docs.aws.amazon.com/bedrock/latest/userguide/count-tokens.html
//! Comparison reference: QuantumNous/new-api (AGPL-3.0),
//! d032204733db31208f10bac118b6e683aab3bf29. No Go signer is copied here.
use super::RelayError;
use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{
        PayloadChecksumKind, SignableBody, SignableRequest, SigningParams, SigningSettings, sign,
    },
    sign::v4,
};
use axum::http::{HeaderMap, Request, header};
use reqwest::Url;
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{fmt, time::SystemTime};
use subtle::{Choice, ConstantTimeEq};
use zeroize::Zeroizing;

#[cfg(test)]
#[path = "mantle_tests.rs"]
mod tests;

const SIGNING_SERVICE: &str = "bedrock-mantle";
const MESSAGES_PATH: &str = "/anthropic/v1/messages";

fn secret<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}
fn optional_secret<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Zeroizing<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(|value| value.map(Zeroizing::new))
}

// This secret document is never Serialize/Debug, nor part of channel JSON.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialDocument {
    #[serde(deserialize_with = "secret")]
    access_key_id: Zeroizing<String>,
    #[serde(deserialize_with = "secret")]
    secret_access_key: Zeroizing<String>,
    #[serde(default, deserialize_with = "optional_secret")]
    session_token: Option<Zeroizing<String>>,
}

impl CredentialDocument {
    fn parse(secret_json: &str) -> Result<Self, RelayError> {
        if secret_json.len() > 16 * 1024 {
            return Err(RelayError::Invalid("invalid Mantle credential document"));
        }
        let document: Self = serde_json::from_str(secret_json)
            .map_err(|_| RelayError::Invalid("invalid Mantle credential document"))?;
        if !(16..=128).contains(&document.access_key_id.len())
            || !document
                .access_key_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
            || !(20..=256).contains(&document.secret_access_key.len())
            || !document
                .secret_access_key
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
            || document.session_token.as_ref().is_some_and(|token| {
                token.is_empty()
                    || token.len() > 8192
                    || !token.bytes().all(|byte| byte.is_ascii_graphic())
            })
        {
            return Err(RelayError::Invalid("invalid Mantle credential fields"));
        }
        Ok(document)
    }
}

fn endpoint(value: &str, region: &str) -> Result<Url, RelayError> {
    let region_parts: Vec<_> = region.split('-').collect();
    if region.len() > 32
        || region_parts.len() < 3
        || region_parts[0].len() != 2
        || !region_parts[0]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase())
        || !region_parts[1..region_parts.len() - 1]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_lowercase()))
        || region_parts.last().is_none_or(|part| {
            part.is_empty() || part.len() > 2 || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
        || value.len() > 2048
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'\\')
    {
        return Err(RelayError::Invalid("invalid Mantle region or endpoint"));
    }
    let mut url = Url::parse(value).map_err(|_| RelayError::Invalid("invalid Mantle endpoint"))?;
    let host = format!("bedrock-mantle.{region}.api.aws");
    if url.scheme() != "https"
        || url.host_str() != Some(host.as_str())
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(
            url.path(),
            "/" | "/anthropic"
                | "/anthropic/"
                | "/anthropic/v1"
                | "/anthropic/v1/"
                | "/anthropic/v1/messages"
                | "/anthropic/v1/messages/"
        )
    {
        return Err(RelayError::Invalid(
            "Mantle endpoint does not match its region and Messages operation",
        ));
    }
    url.set_path(MESSAGES_PATH);
    Ok(url)
}

/// Validate a host-only region/endpoint contract without loading a credential.
pub fn validate_mantle_endpoint(endpoint_value: &str, region: &str) -> Result<(), RelayError> {
    endpoint(endpoint_value, region).map(|_| ())
}

/// Host-only construction check for credential separation across references.
/// These fingerprints are transient comparison keys, never persisted or logged.
pub(crate) fn mantle_credential_fingerprints(
    secret_json: &str,
) -> Result<Vec<[u8; 32]>, RelayError> {
    let document = CredentialDocument::parse(secret_json)?;
    Ok([&document.access_key_id, &document.secret_access_key]
        .into_iter()
        .chain(document.session_token.iter())
        .map(|value| Sha256::digest(value.as_bytes()).into())
        .collect())
}

/// Check credential isolation against each structured secret, without exposing
/// which field matched. Equal-size digests use constant-time comparison and all
/// present fields are checked; a matched prefix never short-circuits the scan.
pub fn mantle_credential_reuses(secret_json: &str, candidate: &str) -> Result<bool, RelayError> {
    let document = CredentialDocument::parse(secret_json)?;
    let candidate_digest: Zeroizing<[u8; 32]> =
        Zeroizing::new(Sha256::digest(candidate.as_bytes()).into());
    let mut matched = Choice::from(0);
    for value in [&document.access_key_id, &document.secret_access_key]
        .into_iter()
        .chain(document.session_token.iter())
    {
        let digest: Zeroizing<[u8; 32]> = Zeroizing::new(Sha256::digest(value.as_bytes()).into());
        matched |= digest.as_slice().ct_eq(candidate_digest.as_slice());
    }
    Ok(bool::from(matched))
}

/// Validate the selected administrator-owned Mantle binding without making any
/// network request. Used by both admission and catalog visibility checks.
pub fn validate_mantle_binding(
    endpoint_value: &str,
    region: &str,
    secret_json: &str,
) -> Result<(), RelayError> {
    MantleSigner::new(endpoint_value, region, secret_json).map(|_| ())
}

#[derive(Clone)]
pub(crate) struct MantleSigner {
    region: String,
    endpoint: Url,
    credentials: Credentials,
}
impl fmt::Debug for MantleSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MantleSigner")
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}
impl MantleSigner {
    pub(crate) fn new(
        endpoint_value: &str,
        region: &str,
        secret_json: &str,
    ) -> Result<Self, RelayError> {
        let endpoint = endpoint(endpoint_value, region)?;
        let document = CredentialDocument::parse(secret_json)?;
        let credentials = Credentials::new(
            document.access_key_id.to_string(),
            document.secret_access_key.to_string(),
            document
                .session_token
                .as_ref()
                .map(|value| value.to_string()),
            None,
            "lmm-core-private-credential-file",
        );
        Ok(Self {
            region: region.to_owned(),
            endpoint,
            credentials,
        })
    }
    pub(crate) fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// The final serialized bytes are signed and subsequently sent unchanged.
    /// No caller headers, API-key fallback or mutable signing service is accepted.
    pub(crate) fn headers(
        &self,
        selected_endpoint: &Url,
        body: &[u8],
        stream: bool,
        time: SystemTime,
    ) -> Result<HeaderMap, RelayError> {
        if selected_endpoint != &self.endpoint {
            return Err(RelayError::Invalid(
                "Mantle destination changed after selection",
            ));
        }
        let mut request = Request::builder()
            .method("POST")
            .uri(self.endpoint.as_str())
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT_ENCODING, "identity")
            .header(
                header::ACCEPT,
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .header("anthropic-version", "2023-06-01")
            .body(body)
            .map_err(|_| RelayError::Invalid("invalid Mantle request"))?;
        let identity = self.credentials.clone().into();
        // AWS exposes a non-exhaustive settings type; initialize through its
        // public default and set only the supported payload-hash field.
        let mut settings = SigningSettings::default();
        settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
        let params: SigningParams<'_> = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name(SIGNING_SERVICE)
            .time(time)
            .settings(settings)
            .build()
            .map_err(|_| RelayError::Invalid("invalid Mantle signing parameters"))?
            .into();
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| {
                value
                    .to_str()
                    .map(|value| (name.as_str(), value))
                    .map_err(|_| RelayError::Invalid("invalid Mantle signed header"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let signable = SignableRequest::new(
            "POST",
            self.endpoint.as_str(),
            headers.into_iter(),
            SignableBody::Bytes(body),
        )
        .map_err(|_| RelayError::Invalid("invalid Mantle request signature"))?;
        // AWS's signer can trace its canonical request, including an STS token.
        // Suppress only this synchronous call; restore the caller's subscriber
        // before returning, including on an error. No async work crosses scope.
        let signed = tracing::dispatcher::with_default(&tracing::Dispatch::none(), || {
            sign(signable, &params)
        });
        let (instructions, _) = signed
            .map_err(|_| RelayError::Invalid("Mantle signing failed"))?
            .into_parts();
        if !instructions.params().is_empty() {
            return Err(RelayError::Invalid("Mantle query signing is disabled"));
        }
        instructions.apply_to_request_http1x(&mut request);
        for name in [header::AUTHORIZATION.as_str(), "x-amz-security-token"] {
            if let Some(value) = request.headers_mut().get_mut(name) {
                value.set_sensitive(true);
            }
        }
        Ok(request.into_parts().0.headers)
    }
}
