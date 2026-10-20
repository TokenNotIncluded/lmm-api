//! Fixed external OAuth providers. No browser input selects an issuer or endpoint.
use crate::identity::IdentityError;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, Url, redirect::Policy};
use serde::Deserialize;
use std::time::Duration;
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, IdentityError>;
const MAX_PROVIDER_BODY: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExternalProvider {
    Github,
    LinuxDo,
    Discord,
}

impl ExternalProvider {
    pub fn parse(slug: &str) -> Result<Self> {
        match slug {
            "github" => Ok(Self::Github),
            "linuxdo" => Ok(Self::LinuxDo),
            "discord" => Ok(Self::Discord),
            _ => Err(IdentityError::Invalid),
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::LinuxDo => "linuxdo",
            Self::Discord => "discord",
        }
    }

    pub fn issuer(self) -> &'static str {
        match self {
            Self::Github => "https://github.com",
            Self::LinuxDo => "https://connect.linux.do",
            Self::Discord => "https://discord.com",
        }
    }

    /// GitHub documents S256, and Discord publishes it in its official metadata.
    /// Linux DO's documented confidential-client flow does not promise PKCE.
    pub fn uses_pkce(self) -> bool {
        matches!(self, Self::Github | Self::Discord)
    }

    fn scope(self) -> &'static str {
        match self {
            Self::Github => "read:user",
            Self::LinuxDo => "user",
            Self::Discord => "identify",
        }
    }

    fn authorization_endpoint(self) -> &'static str {
        match self {
            Self::Github => "https://github.com/login/oauth/authorize",
            Self::LinuxDo => "https://connect.linux.do/oauth2/authorize",
            Self::Discord => "https://discord.com/oauth2/authorize",
        }
    }

    fn token_endpoint(self) -> &'static str {
        match self {
            Self::Github => "https://github.com/login/oauth/access_token",
            Self::LinuxDo => "https://connect.linux.do/oauth2/token",
            Self::Discord => "https://discord.com/api/oauth2/token",
        }
    }

    fn user_endpoint(self) -> &'static str {
        match self {
            Self::Github => "https://api.github.com/user",
            Self::LinuxDo => "https://connect.linux.do/api/user",
            Self::Discord => "https://discord.com/api/v10/users/@me",
        }
    }

    fn accepts_callback_path(self, path: &str) -> bool {
        match self {
            Self::Github => matches!(path, "/oauth/github" | "/api/oauth/github"),
            Self::LinuxDo => matches!(path, "/oauth/linuxdo" | "/api/oauth/linuxdo"),
            Self::Discord => path == "/oauth/discord",
        }
    }
}

/// Deliberately no Debug implementation: this holds a client secret.
pub struct ExternalOAuth {
    provider: ExternalProvider,
    client_id: String,
    client_secret: Zeroizing<String>,
    redirect_uri: String,
    client: Client,
    token_endpoint: Url,
    user_endpoint: Url,
}

impl ExternalOAuth {
    pub fn new(
        provider: ExternalProvider,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<Self> {
        if client_id.is_empty()
            || client_id.len() > 512
            || !client_id.bytes().all(|b| b.is_ascii_graphic() && b != b':')
            || client_secret.is_empty()
            || client_secret.len() > 4096
            || client_secret.chars().any(char::is_control)
            || redirect_uri.len() > 2048
        {
            return Err(IdentityError::Invalid);
        }
        let redirect = Url::parse(redirect_uri).map_err(|_| IdentityError::Invalid)?;
        if redirect.scheme() != "https"
            || redirect.domain().is_none()
            || !redirect.username().is_empty()
            || redirect.password().is_some()
            || redirect.query().is_some()
            || redirect.fragment().is_some()
            || !provider.accepts_callback_path(redirect.path())
            || redirect.as_str() != redirect_uri
        {
            return Err(IdentityError::Invalid);
        }
        Ok(Self {
            provider,
            client_id: client_id.into(),
            client_secret: Zeroizing::new(client_secret.into()),
            redirect_uri: redirect_uri.into(),
            client: provider_client(true)?,
            token_endpoint: Url::parse(provider.token_endpoint())
                .map_err(|_| IdentityError::Storage)?,
            user_endpoint: Url::parse(provider.user_endpoint())
                .map_err(|_| IdentityError::Storage)?,
        })
    }

    pub fn provider(&self) -> ExternalProvider {
        self.provider
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn authorization_url(&self, state: &str, pkce_challenge: &str) -> Result<String> {
        if !(32..=256).contains(&state.len()) || !unreserved(state) {
            return Err(IdentityError::Invalid);
        }
        validate_challenge(pkce_challenge)?;
        let mut url = Url::parse(self.provider.authorization_endpoint())
            .map_err(|_| IdentityError::Storage)?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", self.client_id()),
            ("redirect_uri", self.redirect_uri()),
            ("scope", self.provider.scope()),
            ("state", state),
        ]);
        if self.provider.uses_pkce() {
            url.query_pairs_mut().extend_pairs([
                ("code_challenge", pkce_challenge),
                ("code_challenge_method", "S256"),
            ]);
        }
        Ok(url.into())
    }

    pub(super) async fn exchange(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<VerifiedExternalIdentity> {
        if code.is_empty()
            || code.len() > 4096
            || !code.bytes().all(|b| b.is_ascii_graphic())
            || !(43..=128).contains(&verifier.len())
            || !unreserved(verifier)
        {
            return Err(IdentityError::Invalid);
        }
        // This bounds the combined token and user requests, including body reads.
        tokio::time::timeout(Duration::from_secs(10), self.exchange_inner(code, verifier))
            .await
            .map_err(|_| IdentityError::Storage)?
    }

    async fn exchange_inner(&self, code: &str, verifier: &str) -> Result<VerifiedExternalIdentity> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.redirect_uri()),
        ];
        if self.provider.uses_pkce() {
            form.push(("code_verifier", verifier));
        }
        let mut request = self
            .client
            .post(self.token_endpoint.clone())
            .header(reqwest::header::ACCEPT, "application/json");
        if self.provider == ExternalProvider::Github {
            form.push(("client_id", self.client_id()));
            form.push(("client_secret", self.client_secret.as_str()));
        } else {
            request = request.basic_auth(self.client_id(), Some(self.client_secret.as_str()));
        }
        let token: TokenReply = bounded_json(
            request
                .form(&form)
                .send()
                .await
                .map_err(|_| IdentityError::Storage)?,
        )
        .await?;
        let access_token = Zeroizing::new(token.access_token);
        if !token.token_type.eq_ignore_ascii_case("bearer")
            || access_token.is_empty()
            || access_token.len() > 16_384
            || !access_token.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
            })
        {
            return Err(IdentityError::Unauthorized);
        }
        let reply = self
            .client
            .get(self.user_endpoint.clone())
            .bearer_auth(access_token.as_str())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| IdentityError::Storage)?;
        let subject = match self.provider {
            ExternalProvider::Github | ExternalProvider::LinuxDo => {
                let reply: NumericUserReply = bounded_json(reply).await?;
                positive_numeric_subject(reply.id)?
            }
            ExternalProvider::Discord => {
                let reply: DiscordUserReply = bounded_json(reply).await?;
                discord_subject(reply.id)?
            }
        };
        Ok(VerifiedExternalIdentity {
            provider: self.provider,
            subject,
        })
    }

    /// No runtime endpoint override exists. Tests can only use numeric loopback.
    #[cfg(test)]
    pub(super) fn with_test_endpoints(mut self, token: Url, user: Url) -> Result<Self> {
        for url in [&token, &user] {
            if url.scheme() != "http"
                || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
                || url.port().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(IdentityError::Invalid);
            }
        }
        self.client = provider_client(false)?;
        self.token_endpoint = token;
        self.user_endpoint = user;
        Ok(self)
    }
}

fn provider_client(https_only: bool) -> Result<Client> {
    Client::builder()
        .https_only(https_only)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .user_agent("lmm-api-external-login/0.1")
        .build()
        .map_err(|_| IdentityError::Storage)
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    token_type: String,
}

#[derive(Deserialize)]
struct NumericUserReply {
    id: i64,
}

#[derive(Deserialize)]
struct DiscordUserReply {
    id: String,
}

/// Only a successful fixed-provider token exchange and authenticated user reply
/// can construct this proof. Emails, names and provider roles are not authority.
pub(crate) struct VerifiedExternalIdentity {
    provider: ExternalProvider,
    subject: String,
}

impl VerifiedExternalIdentity {
    pub(super) fn provider(&self) -> ExternalProvider {
        self.provider
    }

    pub(super) fn issuer(&self) -> &'static str {
        self.provider.issuer()
    }

    pub(super) fn subject(&self) -> &str {
        &self.subject
    }
}

fn unreserved(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

fn validate_challenge(challenge: &str) -> Result<()> {
    if challenge.len() != 43 {
        return Err(IdentityError::Invalid);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(challenge)
        .map_err(|_| IdentityError::Invalid)?;
    if bytes.len() != 32 || URL_SAFE_NO_PAD.encode(&bytes) != challenge {
        return Err(IdentityError::Invalid);
    }
    Ok(())
}

fn positive_numeric_subject(id: i64) -> Result<String> {
    if id <= 0 {
        return Err(IdentityError::Unauthorized);
    }
    Ok(id.to_string())
}

fn discord_subject(id: String) -> Result<String> {
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(IdentityError::Unauthorized);
    }
    let value = id.parse::<u64>().map_err(|_| IdentityError::Unauthorized)?;
    if value == 0 || value.to_string() != id {
        return Err(IdentityError::Unauthorized);
    }
    Ok(id)
}

async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T> {
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_PROVIDER_BODY as u64)
    {
        return Err(IdentityError::Unauthorized);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| IdentityError::Storage)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_PROVIDER_BODY {
            return Err(IdentityError::Unauthorized);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| IdentityError::Unauthorized)
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
