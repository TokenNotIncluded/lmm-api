use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use oxide_auth::code_grant::extensions::Pkce;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
pub const SCOPES: [&str; 3] = ["market:discover", "market:invoke", "market:manage"];
pub(crate) fn loopback_ip(u: &reqwest::Url) -> Option<std::net::IpAddr> {
    let ip = u
        .host_str()?
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .ok()?
        .to_canonical();
    ip.is_loopback().then_some(ip)
}
#[derive(Clone)]
pub struct MarketProfile {
    pub(crate) origin: String,
    pub(crate) issuer: String,
    pub(crate) resource: String,
    pub(crate) max_budget: i64,
    pub(crate) isolated: bool,
}
impl MarketProfile {
    pub fn new(origin: &str, max_budget: i64) -> Result<Self> {
        Self::build(origin, max_budget, false)
    }
    pub fn isolated_loopback_test(origin: &str, max_budget: i64) -> Result<Self> {
        Self::build(origin, max_budget, true)
    }
    fn build(origin: &str, max_budget: i64, isolated: bool) -> Result<Self> {
        let u = reqwest::Url::parse(origin).map_err(|_| Error::Invalid)?;
        let valid = if isolated {
            u.scheme() == "http" && loopback_ip(&u).is_some() && u.port().is_some_and(|p| p >= 1024)
        } else {
            u.scheme() == "https" && u.domain().is_some() && u.port().is_none()
        };
        if !valid
            || u.origin().ascii_serialization() != origin
            || origin.len() > 256
            || u.path() != "/"
            || u.query().is_some()
            || u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
            || !(1..=9_007_199_254_740_991).contains(&max_budget)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            origin: origin.into(),
            issuer: format!("{origin}/api/oauth2/market"),
            resource: format!("{origin}/mcp/market"),
            max_budget,
            isolated,
        })
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn resource(&self) -> &str {
        &self.resource
    }
    /// Host-reviewed resource endpoint, never taken from a request or Host header.
    pub fn with_resource(mut self, resource: &str) -> Result<Self> {
        let u = reqwest::Url::parse(resource).map_err(|_| Error::Invalid)?;
        let transport = if self.isolated {
            u.scheme() == "http"
                && loopback_ip(&u).is_some()
                && loopback_ip(&u)
                    == reqwest::Url::parse(&self.origin)
                        .ok()
                        .as_ref()
                        .and_then(loopback_ip)
                && u.port().is_some_and(|p| p >= 1024)
        } else {
            u.scheme() == "https" && u.domain().is_some() && u.port().is_none()
        };
        if !transport
            || resource.len() > 512
            || u.as_str() != resource
            || u.path() != "/mcp/market"
            || u.query().is_some()
            || u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
        {
            return Err(Error::Invalid);
        }
        self.resource = resource.into();
        Ok(self)
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
}
pub(crate) fn secret(prefix: &str) -> String {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    format!(
        "{prefix}{}",
        b.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
pub(crate) fn hash(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}
pub(crate) fn canonical(s: &str, prefix: &str) -> Result<()> {
    if s.len() == prefix.len() + 64
        && s.starts_with(prefix)
        && s[prefix.len()..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(Error::Unauthorized)
    }
}
pub(crate) fn scopes(raw: &str) -> Result<Vec<String>> {
    let mut s = raw.split(' ').map(str::to_owned).collect::<Vec<_>>();
    s.sort();
    if !s.iter().any(|s| s == "market:discover")
        || s.is_empty()
        || s.len() > 3
        || s.windows(2).any(|w| w[0] == w[1])
        || s.iter().any(|s| !SCOPES.contains(&s.as_str()))
    {
        return Err(Error::Invalid);
    }
    Ok(s)
}
pub(crate) fn redirect(raw: &str) -> Result<String> {
    let mut u = reqwest::Url::parse(raw).map_err(|_| Error::Invalid)?;
    if raw.len() > 512
        || u.as_str() != raw
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || raw.contains('%')
        || u.path().contains("//")
        || u.path().split('/').any(|p| p == "." || p == "..")
    {
        return Err(Error::Invalid);
    }
    if u.scheme() == "https" && u.domain().is_some() && !matches!(u.domain(), Some("localhost")) {
        return Ok(raw.into());
    }
    if u.scheme() == "http"
        && matches!(u.host_str(), Some("127.0.0.1" | "[::1]"))
        && u.port().is_some_and(|p| p > 0)
    {
        u.set_port(None).map_err(|_| Error::Invalid)?;
        return Ok(u.into());
    }
    if u.scheme() == "http" && u.host_str() == Some("localhost") {
        return Ok(raw.into());
    }
    Err(Error::Invalid)
}
pub(crate) fn challenge(raw: &str) -> Result<()> {
    if raw.len() != 43 || URL_SAFE_NO_PAD.decode(raw).is_err() {
        return Err(Error::Invalid);
    }
    Pkce::required()
        .challenge(Some("S256".into()), Some(raw.into()))
        .map(|_| ())
        .map_err(|_| Error::Invalid)
}
pub(crate) fn verify(challenge: &str, verifier: &str) -> Result<()> {
    if !(43..=128).contains(&verifier.len())
        || !verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
    {
        return Err(Error::Invalid);
    }
    let value = Pkce::required()
        .challenge(Some("S256".into()), Some(challenge.into()))
        .map_err(|_| Error::Invalid)?;
    Pkce::required()
        .verify(value, Some(verifier.into()))
        .map_err(|_| Error::Unauthorized)
}
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    #[serde(default)]
    pub client_name: String,
    #[serde(default)]
    pub application_type: String,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: String,
    #[serde(default)]
    pub grant_types: Vec<String>,
    #[serde(default)]
    pub response_types: Vec<String>,
    #[serde(default)]
    pub scope: String,
}
impl Registration {
    pub(crate) fn validate(&mut self) -> Result<Vec<String>> {
        if self.client_name.trim().is_empty() {
            self.client_name = "External MCP client".into()
        } else {
            self.client_name = format!("External MCP: {}", self.client_name.trim())
        }
        if self.token_endpoint_auth_method.is_empty() {
            self.token_endpoint_auth_method = "none".into()
        }
        if self.grant_types.is_empty() {
            self.grant_types = vec!["authorization_code".into(), "refresh_token".into()]
        }
        self.grant_types.sort();
        if self.response_types.is_empty() {
            self.response_types = vec!["code".into()]
        }
        if self.scope.is_empty() {
            self.scope = SCOPES.join(" ")
        }
        if self.client_name.len() > 110
            || self.client_name.chars().any(char::is_control)
            || self.token_endpoint_auth_method != "none"
            || self.response_types != ["code"]
            || !(self.grant_types == ["authorization_code"]
                || self.grant_types == ["authorization_code", "refresh_token"])
            || self.redirect_uris.is_empty()
            || self.redirect_uris.len() > 4
        {
            return Err(Error::Invalid);
        }
        let s = scopes(&self.scope)?;
        self.scope = s.join(" ");
        let mut r = self
            .redirect_uris
            .iter()
            .map(|r| redirect(r))
            .collect::<Result<Vec<_>>>()?;
        r.sort();
        r.dedup();
        let kind = if r[0].starts_with("https://") {
            "web"
        } else {
            "native"
        };
        if r.iter()
            .any(|r| r.starts_with("https://") != (kind == "web"))
            || (!self.application_type.is_empty() && self.application_type != kind)
        {
            return Err(Error::Invalid);
        }
        self.application_type = kind.into();
        Ok(r)
    }
}
#[derive(Serialize)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_id_issued_at: i64,
    #[serde(flatten)]
    pub metadata: Registration,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub resource: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
}
impl AuthorizationRequest {
    pub(crate) fn validate(&self, p: &MarketProfile) -> Result<Vec<String>> {
        if self.response_type != "code"
            || self.resource != p.resource
            || self.code_challenge_method != "S256"
            || self.state.is_empty()
            || self.state.len() > 256
            || self.state.chars().any(char::is_control)
        {
            return Err(Error::Invalid);
        }
        canonical(&self.client_id, "lmm-mcp-")?;
        challenge(&self.code_challenge)?;
        redirect(&self.redirect_uri)?;
        scopes(&self.scope)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenForm {
    pub grant_type: String,
    pub client_id: String,
    pub resource: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}
#[derive(Serialize)]
pub struct TokenWire {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub scope: String,
}
