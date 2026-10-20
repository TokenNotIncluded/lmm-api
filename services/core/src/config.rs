#[path = "config_market_oauth.rs"]
mod market_oauth_config;
pub use market_oauth_config::{market_oauth_configured, market_oauth_issuer, market_oauth_profile};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fs, io,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};

fn invalid_config(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub const DEFAULT_CONFIG: &str = "/etc/lmm.conf.d/core.toml";

enum Source {
    LegacyEnv,
    File(BTreeMap<&'static str, OsString>),
}
static SOURCE: OnceLock<Source> = OnceLock::new();

/// Called once by the executable before reading any runtime settings. File mode
/// never reads or changes process environment, and never falls back to it.
pub fn initialize(path: Option<&Path>, legacy_env: bool) -> io::Result<()> {
    if legacy_env && path.is_some() {
        return Err(invalid_config(
            "--config and --legacy-env are mutually exclusive",
        ));
    }
    let source = if legacy_env {
        Source::LegacyEnv
    } else {
        if env::vars_os().any(|(key, _)| key.as_encoded_bytes().starts_with(b"LMM_CORE_")) {
            return Err(invalid_config(
                "TOML configuration cannot be mixed with LMM_CORE environment settings",
            ));
        }
        let document = configuration_file(path.unwrap_or(Path::new(DEFAULT_CONFIG)), 256 * 1024)?;
        let values = parse_core_file(&document)?;
        // Even an offline command must not accept references to broadly
        // readable secrets or unsafe sidecar paths in its selected config.
        for (name, path) in &values {
            if name.ends_with("_FILE") {
                if name.ends_with("_CONFIG_FILE") {
                    configuration_file(Path::new(path), 16 * 1024 * 1024)?;
                } else {
                    private_file(Path::new(path), 16 * 1024)?;
                }
            }
        }
        Source::File(values)
    };
    SOURCE
        .set(source)
        .map_err(|_| invalid_config("core configuration was already initialized"))
}

pub fn setting_os(name: &str) -> Option<OsString> {
    match SOURCE.get() {
        Some(Source::LegacyEnv) => env::var_os(name),
        Some(Source::File(values)) => values.get(name).cloned(),
        None => None,
    }
}

pub fn setting(name: &str) -> Result<String, env::VarError> {
    setting_os(name)
        .ok_or(env::VarError::NotPresent)?
        .into_string()
        .map_err(env::VarError::NotUnicode)
}

fn default_listen() -> String {
    "127.0.0.1:8080".to_owned()
}
fn one() -> u32 {
    1
}
fn two() -> u32 {
    2
}
fn sixteen() -> u32 {
    16
}
fn thirty() -> u32 {
    30
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ServerFile {
    listen: String,
    worker_threads: u32,
    max_inflight: u32,
    drain_timeout_seconds: u32,
}
impl Default for ServerFile {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            worker_threads: one(),
            max_inflight: sixteen(),
            drain_timeout_seconds: thirty(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseFile {
    url_file: PathBuf,
    #[serde(default = "two")]
    max_connections: u32,
    #[serde(default)]
    acquire_timeout_seconds: Option<u32>,
    #[serde(default)]
    check_timeout_seconds: Option<u32>,
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct IdentityFile {
    humangrant_enabled: bool,
    humangrant_smtp_file: Option<PathBuf>,
    registration_enabled: bool,
    password_slots: u32,
    api_key_encryption_key_file: Option<PathBuf>,
    mfa: Option<MfaFile>,
    oauth2: Option<OAuth2File>,
    web: Option<WebFile>,
    google: Option<GoogleFile>,
    github: Option<GoogleFile>,
    linuxdo: Option<GoogleFile>,
    discord: Option<GoogleFile>,
}
impl Default for IdentityFile {
    fn default() -> Self {
        Self {
            humangrant_enabled: false,
            humangrant_smtp_file: None,
            registration_enabled: false,
            password_slots: one(),
            api_key_encryption_key_file: None,
            mfa: None,
            oauth2: None,
            web: None,
            google: None,
            github: None,
            linuxdo: None,
            discord: None,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuth2File {
    issuer: String,
    #[serde(default)]
    isolated_loopback_test: bool,
    isolated_loopback_proxy: Option<bool>,
    isolated_secret_directory: Option<PathBuf>,
    pi_max_credits: Option<i64>,
    groups: Vec<String>,
    access_ttl_seconds: u32,
    family_ttl_seconds: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebFile {
    origin: String,
    refresh_key_file: PathBuf,
    #[serde(default)]
    local_http: bool,
    // Publication facts must be explicit whenever Web authentication is enabled.
    user_agreement_enabled: bool,
    privacy_policy_enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MfaFile {
    keyring_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MfaKeyringFile {
    version: u32,
    active_key_id: String,
    keys: Vec<MfaKeyFile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MfaKeyFile {
    id: String,
    key_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoogleFile {
    client_id: String,
    client_secret_file: PathBuf,
    redirect_uri: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcFile {
    socket: PathBuf,
    token_file: PathBuf,
    shared_group_id: Option<u32>,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ConsoleFile {
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolMarketFile {
    socket: PathBuf,
    token_file: PathBuf,
    shared_group_id: Option<u32>,
    receipt_adapters_config_file: Option<PathBuf>,
    fixed_witness_token_file: Option<PathBuf>,
    fixed_witness_role: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportProjectionFile {
    token_file: PathBuf,
    socket: Option<PathBuf>,
    shared_group_id: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelFile {
    config_file: PathBuf,
    #[serde(default)]
    local_http: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AbuseFile {
    credential_config_file: PathBuf,
    #[serde(default)]
    local_http: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinancialFile {
    config_file: PathBuf,
    token_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssistantFile {
    config_file: PathBuf,
    token_file: PathBuf,
    #[serde(default)]
    tools_enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IngressFile {
    ack_url: String,
    token_file: PathBuf,
    instance: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteControlFile {
    origin: String,
    token_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreFile {
    version: u32,
    #[serde(default)]
    server: ServerFile,
    database: Option<DatabaseFile>,
    #[serde(default)]
    identity: IdentityFile,
    rpc: Option<RpcFile>,
    #[serde(default)]
    console: ConsoleFile,
    model: Option<ModelFile>,
    abuse: Option<AbuseFile>,
    payments: Option<FinancialFile>,
    commerce: Option<FinancialFile>,
    assistant: Option<AssistantFile>,
    tool_market: Option<ToolMarketFile>,
    market_oauth: Option<market_oauth_config::MarketOAuthFile>,
    support_projection: Option<SupportProjectionFile>,
    wallet: Option<SupportProjectionFile>,
    bounties: Option<SupportProjectionFile>,
    images: Option<SupportProjectionFile>,
    ingress: Option<IngressFile>,
    remote_control: Option<RemoteControlFile>,
    edge_access: Option<crate::edge_access::TypedConfig>,
}

fn parse_document<T: serde::de::DeserializeOwned>(document: &[u8]) -> io::Result<T> {
    let document =
        std::str::from_utf8(document).map_err(|_| invalid_config("core TOML must be UTF-8"))?;
    // Parser errors can contain configuration values; never return them.
    toml::from_str(document).map_err(|_| invalid_config("invalid core TOML configuration"))
}

fn absolute_path(path: &Path) -> io::Result<()> {
    if path.to_str().is_none()
        || !path.is_absolute()
        || path
            .as_os_str()
            .as_encoded_bytes()
            .split(|b| *b == b'/')
            .any(|part| part == b"." || part == b"..")
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid_config(
            "core paths must be absolute without dot components",
        ));
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid_config("core paths must not contain symlinks"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid_config("core path unavailable")),
        }
    }
    Ok(())
}

fn parse_core_file(document: &[u8]) -> io::Result<BTreeMap<&'static str, OsString>> {
    let file: CoreFile = parse_document(document)?;
    if file.version != 1 {
        return Err(invalid_config("unsupported core configuration version"));
    }
    if file.database.is_none()
        && (file.identity.registration_enabled
            || file.identity.web.is_some()
            || file.identity.google.is_some()
            || file.identity.github.is_some()
            || file.identity.linuxdo.is_some()
            || file.identity.discord.is_some()
            || file.identity.api_key_encryption_key_file.is_some()
            || file.identity.mfa.is_some()
            || file.identity.oauth2.is_some()
            || file.console.enabled
            || file.model.is_some()
            || file.abuse.is_some()
            || file.payments.is_some()
            || file.commerce.is_some()
            || file.assistant.is_some()
            || file.tool_market.is_some()
            || file.market_oauth.is_some()
            || file.support_projection.is_some()
            || file.wallet.is_some()
            || file.bounties.is_some()
            || file.images.is_some()
            || file.remote_control.is_some()
            || file.edge_access.is_some())
    {
        return Err(invalid_config(
            "configured identity and business capabilities require a core database",
        ));
    }
    file.server
        .listen
        .parse::<std::net::SocketAddr>()
        .map_err(|_| invalid_config("invalid core listen address"))?;
    let mut values = BTreeMap::new();
    let mut put = |name, value: String| {
        values.insert(name, value.into());
    };
    let configured_listen = file.server.listen.clone();
    if file.edge_access.is_some() && !file.console.enabled {
        return Err(invalid_config(
            "edge access requires the console policy store",
        ));
    }
    if (file.identity.github.is_some()
        || file.identity.linuxdo.is_some()
        || file.identity.discord.is_some())
        && file.identity.web.is_none()
    {
        return Err(invalid_config(
            "external browser login requires Web authentication",
        ));
    }
    put("LMM_CORE_LISTEN", file.server.listen);
    for (name, number, maximum) in [
        ("LMM_CORE_WORKER_THREADS", file.server.worker_threads, 4),
        ("LMM_CORE_MAX_INFLIGHT", file.server.max_inflight, 64),
        (
            "LMM_CORE_DRAIN_TIMEOUT_SECONDS",
            file.server.drain_timeout_seconds,
            300,
        ),
        ("LMM_CORE_PASSWORD_SLOTS", file.identity.password_slots, 2),
    ] {
        if number == 0 || number > maximum {
            return Err(invalid_config("invalid bounded core resource limit"));
        }
        put(name, number.to_string());
    }
    put(
        "LMM_CORE_REGISTRATION_ENABLED",
        file.identity.registration_enabled.to_string(),
    );
    put("LMM_CORE_CONSOLE_ENABLED", file.console.enabled.to_string());
    let mut paths = Vec::new();
    if let Some(db) = file.database {
        if !(1..=8).contains(&db.max_connections) {
            return Err(invalid_config("invalid core database connection limit"));
        }
        let acquire_timeout_seconds = db.acquire_timeout_seconds.unwrap_or(3);
        if !(1..=30).contains(&acquire_timeout_seconds) {
            return Err(invalid_config("invalid core database acquire timeout"));
        }
        let check_timeout_seconds = db.check_timeout_seconds.unwrap_or(15);
        if !(1..=120).contains(&check_timeout_seconds) {
            return Err(invalid_config("invalid core database check timeout"));
        }
        put("LMM_CORE_DB_CONNECTIONS", db.max_connections.to_string());
        put(
            "LMM_CORE_DB_ACQUIRE_TIMEOUT_SECONDS",
            acquire_timeout_seconds.to_string(),
        );
        put(
            "LMM_CORE_DB_CHECK_TIMEOUT_SECONDS",
            check_timeout_seconds.to_string(),
        );
        paths.push(("LMM_CORE_DATABASE_URL_FILE", db.url_file));
    }
    if let Some(path) = file.identity.api_key_encryption_key_file {
        paths.push(("LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE", path));
    }
    if let Some(mfa) = file.identity.mfa {
        paths.push(("LMM_CORE_MFA_KEYRING_FILE", mfa.keyring_file));
    }
    if let Some(market) = file.market_oauth {
        let web = file
            .identity
            .web
            .as_ref()
            .ok_or_else(|| invalid_config("market OAuth requires Web authentication"))?;
        for (name, value) in
            market_oauth_config::parse(market, web, &configured_listen, file.tool_market.is_some())?
        {
            put(name, value);
        }
    }
    if let Some(oauth2) = file.identity.oauth2 {
        let web = file
            .identity
            .web
            .as_ref()
            .ok_or_else(|| invalid_config("OAuth2 requires configured Web authentication"))?;
        let profile = if oauth2.isolated_loopback_test {
            if !web.local_http
                || !file.model.as_ref().is_some_and(|model| model.local_http)
                || oauth2.isolated_secret_directory.is_none()
            {
                return Err(invalid_config(
                    "isolated OAuth2 requires explicit local Web/models and private secrets",
                ));
            }
            validate_isolated_listener(
                &oauth2.issuer,
                &configured_listen,
                oauth2.isolated_loopback_proxy.unwrap_or(false),
            )?;
            crate::identity::grants::LmmProfile::isolated_loopback_test(
                &oauth2.issuer,
                oauth2.groups.clone(),
            )
        } else {
            if web.local_http
                || oauth2.isolated_secret_directory.is_some()
                || oauth2.isolated_loopback_proxy.is_some()
            {
                return Err(invalid_config(
                    "production OAuth2 cannot use local test settings",
                ));
            }
            crate::identity::grants::LmmProfile::new(&oauth2.issuer, oauth2.groups.clone())
        };
        if web.origin != oauth2.issuer
            || !(1..=86_400).contains(&oauth2.access_ttl_seconds)
            || !(1..=31_536_000).contains(&oauth2.family_ttl_seconds)
            || profile.is_err()
        {
            return Err(invalid_config(
                "OAuth2 requires exact Web issuer, explicit transport profile and bounded lifetimes",
            ));
        }
        put(
            "LMM_CORE_OAUTH2_ISOLATED_LOOPBACK_TEST",
            oauth2.isolated_loopback_test.to_string(),
        );
        if let Some(proxy) = oauth2.isolated_loopback_proxy {
            put("LMM_CORE_OAUTH2_ISOLATED_LOOPBACK_PROXY", proxy.to_string());
        }
        if let Some(directory) = oauth2.isolated_secret_directory {
            paths.push(("LMM_CORE_OAUTH2_ISOLATED_SECRET_DIRECTORY", directory));
        }
        if let Some(maximum) = oauth2.pi_max_credits {
            if file.model.is_none() || !(1..=9_007_199_254_740_991).contains(&maximum) {
                return Err(invalid_config(
                    "Pi delegation requires configured actual model service and finite budget ceiling",
                ));
            }
            put("LMM_CORE_OAUTH2_PI_MAX_CREDITS", maximum.to_string());
        }
        put("LMM_CORE_OAUTH2_ISSUER", oauth2.issuer);
        put(
            "LMM_CORE_OAUTH2_GROUPS",
            serde_json::to_string(&oauth2.groups)
                .map_err(|_| invalid_config("invalid OAuth2 groups"))?,
        );
        put(
            "LMM_CORE_OAUTH2_ACCESS_TTL_SECONDS",
            oauth2.access_ttl_seconds.to_string(),
        );
        put(
            "LMM_CORE_OAUTH2_FAMILY_TTL_SECONDS",
            oauth2.family_ttl_seconds.to_string(),
        );
    }

    put(
        "LMM_CORE_HUMANGRANT_ENABLED",
        file.identity.humangrant_enabled.to_string(),
    );
    if file.identity.humangrant_enabled && file.identity.web.is_none() {
        return Err(invalid_config(
            "HumanGrant requires native Web authentication",
        ));
    }
    if let Some(path) = file.identity.humangrant_smtp_file {
        if !file.identity.humangrant_enabled {
            return Err(invalid_config(
                "HumanGrant SMTP requires HumanGrant enabled",
            ));
        }
        absolute_path(&path)?;
        put(
            "LMM_CORE_HUMANGRANT_SMTP_FILE",
            path.to_string_lossy().into_owned(),
        );
    }
    if let Some(web) = file.identity.web {
        put("LMM_CORE_WEB_ORIGIN", web.origin);
        put("LMM_CORE_WEB_LOCAL_HTTP", web.local_http.to_string());
        put(
            "LMM_CORE_WEB_USER_AGREEMENT_ENABLED",
            web.user_agreement_enabled.to_string(),
        );
        put(
            "LMM_CORE_WEB_PRIVACY_POLICY_ENABLED",
            web.privacy_policy_enabled.to_string(),
        );
        paths.push(("LMM_CORE_WEB_REFRESH_KEY_FILE", web.refresh_key_file));
    }
    if let Some(google) = file.identity.google {
        put("LMM_CORE_GOOGLE_CLIENT_ID", google.client_id);
        put("LMM_CORE_GOOGLE_REDIRECT_URI", google.redirect_uri);
        paths.push((
            "LMM_CORE_GOOGLE_CLIENT_SECRET_FILE",
            google.client_secret_file,
        ));
    }
    for (provider, id_key, uri_key, secret_key) in [
        (
            file.identity.github,
            "LMM_CORE_GITHUB_CLIENT_ID",
            "LMM_CORE_GITHUB_REDIRECT_URI",
            "LMM_CORE_GITHUB_CLIENT_SECRET_FILE",
        ),
        (
            file.identity.linuxdo,
            "LMM_CORE_LINUXDO_CLIENT_ID",
            "LMM_CORE_LINUXDO_REDIRECT_URI",
            "LMM_CORE_LINUXDO_CLIENT_SECRET_FILE",
        ),
        (
            file.identity.discord,
            "LMM_CORE_DISCORD_CLIENT_ID",
            "LMM_CORE_DISCORD_REDIRECT_URI",
            "LMM_CORE_DISCORD_CLIENT_SECRET_FILE",
        ),
    ] {
        if let Some(provider) = provider {
            put(id_key, provider.client_id);
            put(uri_key, provider.redirect_uri);
            paths.push((secret_key, provider.client_secret_file));
        }
    }
    if let Some(edge) = file.edge_access {
        paths.push(("LMM_CORE_EDGE_PROOF_FILE", edge.proof_file));
    }
    if let Some(rpc) = file.rpc {
        if let Some(gid) = rpc.shared_group_id {
            if gid == 0 {
                return Err(invalid_config("RPC shared group must not be root"));
            }
            put("LMM_CORE_RPC_SHARED_GROUP_ID", gid.to_string());
        }
        paths.extend([
            ("LMM_CORE_RPC_SOCKET", rpc.socket),
            ("LMM_CORE_RPC_TOKEN_FILE", rpc.token_file),
        ]);
    }
    if let Some(tool) = file.tool_market {
        if tool.fixed_witness_token_file.is_some() != tool.fixed_witness_role.is_some() {
            return Err(invalid_config(
                "fixed witness requires both purpose credential and role",
            ));
        }
        if let Some(path) = tool.receipt_adapters_config_file {
            paths.push(("LMM_CORE_TOOL_MARKET_RECEIPT_ADAPTERS_CONFIG_FILE", path));
        }
        if let Some(path) = tool.fixed_witness_token_file {
            paths.push(("LMM_CORE_TOOL_MARKET_FIXED_WITNESS_TOKEN_FILE", path));
        }
        if let Some(role) = tool.fixed_witness_role {
            put("LMM_CORE_TOOL_MARKET_FIXED_WITNESS_ROLE", role);
        }
        if let Some(gid) = tool.shared_group_id {
            if gid == 0 {
                return Err(invalid_config("tool market shared group must not be root"));
            }
            put("LMM_CORE_TOOL_MARKET_SHARED_GROUP_ID", gid.to_string());
        }
        paths.extend([
            ("LMM_CORE_TOOL_MARKET_SOCKET", tool.socket),
            ("LMM_CORE_TOOL_MARKET_TOKEN_FILE", tool.token_file),
        ]);
    }
    for (service, prefix) in [(file.images, "images"), (file.bounties, "bounties")] {
        if let Some(service) = service {
            let (token_name, socket_name, group_name) = match prefix {
                "images" => (
                    "LMM_CORE_IMAGES_TOKEN_FILE",
                    "LMM_CORE_IMAGES_SOCKET",
                    "LMM_CORE_IMAGES_SHARED_GROUP_ID",
                ),
                _ => (
                    "LMM_CORE_BOUNTIES_TOKEN_FILE",
                    "LMM_CORE_BOUNTIES_SOCKET",
                    "LMM_CORE_BOUNTIES_SHARED_GROUP_ID",
                ),
            };
            paths.push((token_name, service.token_file));
            if let Some(socket) = service.socket {
                paths.push((socket_name, socket));
            }
            if let Some(gid) = service.shared_group_id {
                if gid == 0 {
                    return Err(invalid_config("private service group must not be root"));
                }
                put(group_name, gid.to_string());
            }
        }
    }
    if let Some(wallet) = file.wallet {
        paths.push(("LMM_CORE_WALLET_TOKEN_FILE", wallet.token_file));
        if let Some(socket) = wallet.socket {
            paths.push(("LMM_CORE_WALLET_SOCKET", socket));
        }
        if let Some(gid) = wallet.shared_group_id {
            if gid == 0 {
                return Err(invalid_config("wallet private group must not be root"));
            }
            put("LMM_CORE_WALLET_SHARED_GROUP_ID", gid.to_string());
        }
    }
    if let Some(support) = file.support_projection {
        paths.push(("LMM_CORE_SUPPORT_PROJECTION_TOKEN_FILE", support.token_file));
        if let Some(socket) = support.socket {
            paths.push(("LMM_CORE_SUPPORT_PROJECTION_SOCKET", socket));
        }
        if let Some(gid) = support.shared_group_id {
            if gid == 0 {
                return Err(invalid_config("support projection group must not be root"));
            }
            put(
                "LMM_CORE_SUPPORT_PROJECTION_SHARED_GROUP_ID",
                gid.to_string(),
            );
        }
    }
    if let Some(model) = file.model {
        put("LMM_CORE_MODEL_LOCAL_HTTP", model.local_http.to_string());
        paths.push(("LMM_CORE_MODEL_CONFIG_FILE", model.config_file));
    }
    if let Some(abuse) = file.abuse {
        put("LMM_CORE_ABUSE_LOCAL_HTTP", abuse.local_http.to_string());
        paths.push((
            "LMM_CORE_ABUSE_CREDENTIAL_CONFIG_FILE",
            abuse.credential_config_file,
        ));
    }
    for (section, config_name, token_name) in [
        (
            file.payments,
            "LMM_CORE_PAYMENTS_CONFIG_FILE",
            "LMM_CORE_PAYMENTS_TOKEN_FILE",
        ),
        (
            file.commerce,
            "LMM_CORE_COMMERCE_CONFIG_FILE",
            "LMM_CORE_COMMERCE_TOKEN_FILE",
        ),
    ] {
        if let Some(section) = section {
            paths.extend([
                (config_name, section.config_file),
                (token_name, section.token_file),
            ]);
        }
    }
    if let Some(assistant) = file.assistant {
        put(
            "LMM_CORE_ASSISTANT_TOOLS_ENABLED",
            assistant.tools_enabled.to_string(),
        );
        paths.extend([
            ("LMM_CORE_ASSISTANT_CONFIG_FILE", assistant.config_file),
            ("LMM_CORE_ASSISTANT_TOKEN_FILE", assistant.token_file),
        ]);
    }
    if let Some(remote) = file.remote_control {
        put("LMM_CORE_REMOTE_CONTROL_ORIGIN", remote.origin);
        paths.push(("LMM_CORE_REMOTE_CONTROL_TOKEN_FILE", remote.token_file));
    }
    if let Some(ingress) = file.ingress {
        put("LMM_CORE_INGRESS_ACK_URL", ingress.ack_url);
        put("LMM_CORE_INSTANCE", ingress.instance);
        paths.push(("LMM_CORE_INGRESS_TOKEN_FILE", ingress.token_file));
    }
    for (name, path) in paths {
        absolute_path(&path)?;
        values.insert(name, path.into_os_string());
    }
    Ok(values)
}

pub fn remote_control_configured() -> bool {
    setting_os("LMM_CORE_REMOTE_CONTROL_ORIGIN").is_some()
        || setting_os("LMM_CORE_REMOTE_CONTROL_TOKEN_FILE").is_some()
}
#[cfg(unix)]
pub fn remote_control(
    store: crate::identity::IdentityStore,
    stopped: tokio::sync::watch::Receiver<bool>,
) -> io::Result<Option<crate::remote_control::RemoteProxy>> {
    match (
        setting("LMM_CORE_REMOTE_CONTROL_ORIGIN"),
        setting_os("LMM_CORE_REMOTE_CONTROL_TOKEN_FILE"),
    ) {
        (Err(env::VarError::NotPresent), None) => Ok(None),
        (Ok(origin), Some(path)) => {
            let token = service_token(Path::new(&path))?;
            crate::remote_control::RemoteProxy::new(store, &origin, &token, stopped).map(Some)
        }
        _ => Err(invalid_config(
            "configure both remote relay origin and service token file",
        )),
    }
}

pub fn listen_address() -> io::Result<std::net::SocketAddr> {
    match setting("LMM_CORE_LISTEN") {
        Ok(value) => value
            .parse()
            .map_err(|_| invalid_config("invalid core listen address")),
        Err(env::VarError::NotPresent) => default_listen()
            .parse()
            .map_err(|_| invalid_config("invalid default core listen address")),
        Err(_) => Err(invalid_config("invalid core listen address")),
    }
}
pub fn drain_timeout() -> io::Result<Duration> {
    Ok(Duration::from_secs(
        configured_limit("LMM_CORE_DRAIN_TIMEOUT_SECONDS", 30, 300)? as u64,
    ))
}
#[cfg(unix)]
pub fn service_token(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = private_file(path, 4096)?;
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| invalid_config("invalid service credential"))?
        .trim()
        .as_bytes();
    if !(32..=256).contains(&value.len()) || !value.iter().all(u8::is_ascii_graphic) {
        return Err(invalid_config("invalid service credential"));
    }
    Ok(value.to_vec())
}
pub fn rpc_shared_group() -> io::Result<Option<u32>> {
    match setting("LMM_CORE_RPC_SHARED_GROUP_ID") {
        Err(env::VarError::NotPresent) => Ok(None),
        Ok(value) => value
            .parse::<u32>()
            .ok()
            .filter(|gid| *gid != 0)
            .map(Some)
            .ok_or_else(|| invalid_config("invalid RPC shared group")),
        Err(_) => Err(invalid_config("invalid RPC shared group")),
    }
}

pub fn identity_auth(
    store: crate::identity::IdentityStore,
) -> io::Result<crate::identity::IdentityStore> {
    let mut store = if matches!(SOURCE.get(), Some(Source::LegacyEnv)) {
        store
            .with_auth_from_env()
            .map_err(|_| invalid_config("invalid identity authentication configuration"))?
    } else {
        let mut store =
            store.with_registration_enabled(bool_setting("LMM_CORE_REGISTRATION_ENABLED")?);
        if let (Ok(id), Some(path), Ok(uri)) = (
            setting("LMM_CORE_GOOGLE_CLIENT_ID"),
            setting_os("LMM_CORE_GOOGLE_CLIENT_SECRET_FILE"),
            setting("LMM_CORE_GOOGLE_REDIRECT_URI"),
        ) {
            let secret = String::from_utf8(private_file(Path::new(&path), 4096)?)
                .map_err(|_| invalid_config("invalid OAuth credential"))?;
            let google = crate::identity::GoogleOAuth::new(id, secret.trim().to_owned(), uri)
                .map_err(|_| invalid_config("invalid OAuth configuration"))?;
            store = store.with_google_oauth(google);
        }
        store
    };
    for (provider, id_key, uri_key, secret_key) in [
        (
            crate::identity::ExternalProvider::Github,
            "LMM_CORE_GITHUB_CLIENT_ID",
            "LMM_CORE_GITHUB_REDIRECT_URI",
            "LMM_CORE_GITHUB_CLIENT_SECRET_FILE",
        ),
        (
            crate::identity::ExternalProvider::LinuxDo,
            "LMM_CORE_LINUXDO_CLIENT_ID",
            "LMM_CORE_LINUXDO_REDIRECT_URI",
            "LMM_CORE_LINUXDO_CLIENT_SECRET_FILE",
        ),
        (
            crate::identity::ExternalProvider::Discord,
            "LMM_CORE_DISCORD_CLIENT_ID",
            "LMM_CORE_DISCORD_REDIRECT_URI",
            "LMM_CORE_DISCORD_CLIENT_SECRET_FILE",
        ),
    ] {
        let configured = [
            setting_os(id_key),
            setting_os(uri_key),
            setting_os(secret_key),
        ];
        if configured.iter().all(Option::is_none) {
            continue;
        }
        let id = setting(id_key)
            .map_err(|_| invalid_config("incomplete external OAuth configuration"))?;
        let uri = setting(uri_key)
            .map_err(|_| invalid_config("incomplete external OAuth configuration"))?;
        let path = setting_os(secret_key)
            .ok_or_else(|| invalid_config("incomplete external OAuth configuration"))?;
        let secret = String::from_utf8(private_file(Path::new(&path), 4096)?)
            .map_err(|_| invalid_config("invalid OAuth credential"))?;
        let provider = crate::identity::ExternalOAuth::new(provider, &id, secret.trim(), &uri)
            .map_err(|_| invalid_config("invalid external OAuth configuration"))?;
        store = store
            .with_external_oauth(provider)
            .map_err(|_| invalid_config("duplicate external OAuth configuration"))?;
    }
    if let Some(path) = setting_os("LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE") {
        store = store
            .with_api_key_encryption_key(&private_file(Path::new(&path), 32)?)
            .map_err(|_| invalid_config("invalid API key encryption key"))?;
    }
    if let Some(path) = setting_os("LMM_CORE_MFA_KEYRING_FILE") {
        let ring: MfaKeyringFile = parse_document(&private_file(Path::new(&path), 16 * 1024)?)?;
        if ring.version != 1 || ring.keys.is_empty() || ring.keys.len() > 8 {
            return Err(invalid_config("invalid MFA keyring configuration"));
        }
        // A later file error must also wipe keys already read successfully.
        let mut keys = Vec::new();
        for key in ring.keys {
            keys.push((
                key.id,
                zeroize::Zeroizing::new(private_file(&key.key_file, 32)?),
            ));
        }
        // The keyring constructor immediately guards every transferred key
        // before validating labels, lengths or duplicate IDs.
        let keys = keys
            .into_iter()
            .map(|(id, mut key)| (id, std::mem::take(&mut *key)))
            .collect();
        store = store
            .with_mfa_keys(ring.active_key_id, keys)
            .map_err(|_| invalid_config("invalid MFA keyring configuration"))?;
    }
    Ok(store)
}

pub fn ingress() -> io::Result<Option<crate::lifecycle::ingress::IngressAcks>> {
    match (
        setting("LMM_CORE_INGRESS_ACK_URL"),
        setting_os("LMM_CORE_INGRESS_TOKEN_FILE"),
        setting("LMM_CORE_INSTANCE"),
    ) {
        (Err(env::VarError::NotPresent), None, Err(env::VarError::NotPresent)) => Ok(None),
        (Ok(url), Some(path), Ok(instance)) => {
            let bytes = private_file(Path::new(&path), 4096)?;
            let token = std::str::from_utf8(&bytes)
                .map_err(|_| invalid_config("invalid ingress credential"))?
                .trim()
                .as_bytes();
            crate::lifecycle::ingress::IngressAcks::new(&url, token, &instance)
                .map(Some)
                .map_err(|_| invalid_config("invalid ingress configuration"))
        }
        _ => Err(invalid_config(
            "configure ingress endpoint, credential and instance together",
        )),
    }
}

pub fn identity_database_required() -> io::Result<bool> {
    Ok(bool_setting("LMM_CORE_REGISTRATION_ENABLED")?
        || setting_os("LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE").is_some()
        || setting_os("LMM_CORE_MFA_KEYRING_FILE").is_some()
        || setting_os("LMM_CORE_GOOGLE_CLIENT_ID").is_some()
        || setting_os("LMM_CORE_GOOGLE_CLIENT_SECRET").is_some()
        || setting_os("LMM_CORE_GOOGLE_CLIENT_SECRET_FILE").is_some()
        || setting_os("LMM_CORE_GOOGLE_REDIRECT_URI").is_some()
        || setting_os("LMM_CORE_GITHUB_CLIENT_ID").is_some()
        || setting_os("LMM_CORE_LINUXDO_CLIENT_ID").is_some()
        || setting_os("LMM_CORE_DISCORD_CLIENT_ID").is_some()
        || oauth2_configured()
        || market_oauth_configured())
}

pub fn edge_access_config() -> io::Result<Option<crate::edge_access::TypedConfig>> {
    let proof = setting_os("LMM_CORE_EDGE_PROOF_FILE").map(PathBuf::from);
    if proof.is_none() {
        return Ok(None);
    }
    let tokens: Vec<PathBuf> = match SOURCE.get() {
        Some(Source::File(values)) => values
            .iter()
            .filter(|(name, _)| edge_service_token_setting(name))
            .map(|(_, path)| PathBuf::from(path))
            .collect(),
        Some(Source::LegacyEnv) => env::vars_os()
            .filter(|(name, _)| name.to_str().is_some_and(edge_service_token_setting))
            .map(|(_, path)| PathBuf::from(path))
            .collect(),
        None => vec![],
    };
    checked_edge_access_config(proof, &tokens)
}

fn edge_service_token_setting(name: &str) -> bool {
    name.starts_with("LMM_CORE_") && name.ends_with("_TOKEN_FILE")
}

fn checked_edge_access_config(
    proof: Option<PathBuf>,
    tokens: &[PathBuf],
) -> io::Result<Option<crate::edge_access::TypedConfig>> {
    let Some(proof_file) = proof else {
        return Ok(None);
    };
    #[cfg(not(unix))]
    {
        let _ = (proof_file, tokens);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "edge access credentials require Unix private files",
        ))
    }
    #[cfg(unix)]
    {
        let proof = zeroize::Zeroizing::new(service_token(&proof_file)?);
        for path in tokens {
            if path == &proof_file {
                return Err(invalid_config(
                    "edge proof must be independent from service credentials",
                ));
            }
            let token = zeroize::Zeroizing::new(service_token(path)?);
            if proof.as_slice() == token.as_slice() {
                return Err(invalid_config(
                    "edge proof must be independent from service credentials",
                ));
            }
        }
        Ok(Some(crate::edge_access::TypedConfig { proof_file }))
    }
}

/// A bounded core pool. RPC identity and event delivery use separate pools.
pub async fn database_pool(url: &str) -> io::Result<sqlx::PgPool> {
    bounded_database_pool(url, database_connections()?).await
}

async fn bounded_database_pool(url: &str, maximum: u32) -> io::Result<sqlx::PgPool> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(maximum)
        .min_connections(0)
        .idle_timeout(Some(Duration::from_secs(30)))
        .acquire_timeout(Duration::from_secs(u64::from(
            database_acquire_timeout_seconds()?,
        )))
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET statement_timeout='5s'")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET idle_in_transaction_session_timeout='10s'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .map_err(|_| io::Error::other("core database connection unavailable"))
}

fn bool_setting(name: &'static str) -> io::Result<bool> {
    match setting(name).as_deref() {
        Ok("true" | "1") => Ok(true),
        Ok("false" | "0") | Err(env::VarError::NotPresent) => Ok(false),
        _ => Err(invalid_config("invalid boolean core setting")),
    }
}

pub fn console_enabled() -> io::Result<bool> {
    bool_setting("LMM_CORE_CONSOLE_ENABLED")
}

pub fn payments_configured() -> bool {
    setting_os("LMM_CORE_PAYMENTS_CONFIG_FILE").is_some()
        || setting_os("LMM_CORE_PAYMENTS_TOKEN_FILE").is_some()
}

pub fn commerce_configured() -> bool {
    setting_os("LMM_CORE_COMMERCE_CONFIG_FILE").is_some()
        || setting_os("LMM_CORE_COMMERCE_TOKEN_FILE").is_some()
}

pub fn tool_market_configured() -> bool {
    setting_os("LMM_CORE_TOOL_MARKET_SOCKET").is_some()
        || setting_os("LMM_CORE_TOOL_MARKET_TOKEN_FILE").is_some()
        || setting_os("LMM_CORE_TOOL_MARKET_SHARED_GROUP_ID").is_some()
}
pub fn wallet_configured() -> bool {
    setting_os("LMM_CORE_WALLET_TOKEN_FILE").is_some()
        || setting_os("LMM_CORE_WALLET_SOCKET").is_some()
        || setting_os("LMM_CORE_WALLET_SHARED_GROUP_ID").is_some()
}
pub fn bounties_configured() -> bool {
    setting_os("LMM_CORE_BOUNTIES_TOKEN_FILE").is_some()
        || setting_os("LMM_CORE_BOUNTIES_SOCKET").is_some()
        || setting_os("LMM_CORE_BOUNTIES_SHARED_GROUP_ID").is_some()
}
pub fn image_host_configured() -> bool {
    setting_os("LMM_CORE_IMAGES_TOKEN_FILE").is_some()
        || setting_os("LMM_CORE_IMAGES_SOCKET").is_some()
        || setting_os("LMM_CORE_IMAGES_SHARED_GROUP_ID").is_some()
}
pub fn bounties_browser_router(store: crate::bounties::BountyStore) -> io::Result<axum::Router> {
    let origin = setting("LMM_CORE_WEB_ORIGIN")
        .map_err(|_| invalid_config("bounty confirmation requires Web browser authentication"))?;
    if setting_os("LMM_CORE_WEB_REFRESH_KEY_FILE").is_none() {
        return Err(invalid_config(
            "bounty confirmation requires Web browser authentication",
        ));
    }
    crate::bounties::http::confirm_router_with_local_http(
        store,
        origin,
        bool_setting("LMM_CORE_WEB_LOCAL_HTTP")?,
    )
    .map_err(|_| invalid_config("invalid bounty confirmation origin"))
}
pub fn image_browser_router(gateway: &crate::model_http::ModelGateway) -> io::Result<axum::Router> {
    let images = gateway
        .image_service()
        .ok_or_else(|| invalid_config("image host requires configured image models"))?;
    let origin = setting("LMM_CORE_WEB_ORIGIN")
        .map_err(|_| invalid_config("image confirmation requires Web browser authentication"))?;
    if setting_os("LMM_CORE_WEB_REFRESH_KEY_FILE").is_none() {
        return Err(invalid_config(
            "image confirmation requires Web browser authentication",
        ));
    }
    crate::model_http::image_http::browser_router_with_local_http(
        images.clone(),
        origin,
        bool_setting("LMM_CORE_WEB_LOCAL_HTTP")?,
    )
    .map_err(|_| invalid_config("invalid image confirmation origin"))
}
/// Requires the configured browser cookie family. No native Session confirmation fallback.
pub fn wallet_browser_router(store: crate::wallet::Store) -> io::Result<axum::Router> {
    let origin = setting("LMM_CORE_WEB_ORIGIN")
        .map_err(|_| invalid_config("wallet confirmation requires Web browser authentication"))?;
    if setting_os("LMM_CORE_WEB_REFRESH_KEY_FILE").is_none() {
        return Err(invalid_config(
            "wallet confirmation requires Web browser authentication",
        ));
    }
    crate::wallet::http::browser_router_with_local_http(
        store,
        origin,
        bool_setting("LMM_CORE_WEB_LOCAL_HTTP")?,
    )
    .map_err(|_| invalid_config("invalid wallet browser confirmation origin"))
}
pub fn support_projection_configured() -> bool {
    setting_os("LMM_CORE_SUPPORT_PROJECTION_TOKEN_FILE").is_some()
        || setting_os("LMM_CORE_SUPPORT_PROJECTION_SOCKET").is_some()
        || setting_os("LMM_CORE_SUPPORT_PROJECTION_SHARED_GROUP_ID").is_some()
}
#[cfg(unix)]
pub async fn tool_market_server(
    url: &str,
    gateway: Option<&crate::model_http::ModelGateway>,
) -> io::Result<Option<crate::tool_market::server::ToolMarketServer>> {
    if !tool_market_configured()
        && !support_projection_configured()
        && !wallet_configured()
        && !bounties_configured()
        && !image_host_configured()
    {
        return Ok(None);
    }
    let tool = match (
        setting_os("LMM_CORE_TOOL_MARKET_SOCKET"),
        setting_os("LMM_CORE_TOOL_MARKET_TOKEN_FILE"),
    ) {
        (None, None) if !tool_market_configured() => None,
        (Some(socket), Some(token)) => Some((socket, service_token(Path::new(&token))?)),
        _ => {
            return Err(invalid_config(
                "tool market requires private socket and independent token file",
            ));
        }
    };
    let support = setting_os("LMM_CORE_SUPPORT_PROJECTION_TOKEN_FILE")
        .map(|p| service_token(Path::new(&p)))
        .transpose()?;
    if support_projection_configured() && support.is_none() {
        return Err(invalid_config(
            "support projection requires independent token file",
        ));
    }
    let wallet = setting_os("LMM_CORE_WALLET_TOKEN_FILE")
        .map(|p| service_token(Path::new(&p)))
        .transpose()?;
    if wallet_configured() && wallet.is_none() {
        return Err(invalid_config("wallet requires independent token file"));
    }
    let bounties = setting_os("LMM_CORE_BOUNTIES_TOKEN_FILE")
        .map(|p| service_token(Path::new(&p)))
        .transpose()?;
    if bounties_configured() && bounties.is_none() {
        return Err(invalid_config("bounties requires independent token file"));
    }
    let images = setting_os("LMM_CORE_IMAGES_TOKEN_FILE")
        .map(|p| service_token(Path::new(&p)))
        .transpose()?;
    if image_host_configured() && images.is_none() {
        return Err(invalid_config("images requires independent token file"));
    }
    let witness = setting_os("LMM_CORE_TOOL_MARKET_FIXED_WITNESS_TOKEN_FILE")
        .map(|p| service_token(Path::new(&p)))
        .transpose()?;
    let witness_role = setting("LMM_CORE_TOOL_MARKET_FIXED_WITNESS_ROLE").ok();
    if witness.is_some() != witness_role.is_some() || witness.is_some() && tool.is_none() {
        return Err(invalid_config(
            "fixed witness requires ordinary tool host and separate purpose credential and role",
        ));
    }
    let sockets = [
        tool.as_ref().map(|v| v.0.clone()),
        setting_os("LMM_CORE_SUPPORT_PROJECTION_SOCKET"),
        setting_os("LMM_CORE_WALLET_SOCKET"),
        setting_os("LMM_CORE_BOUNTIES_SOCKET"),
        setting_os("LMM_CORE_IMAGES_SOCKET"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let socket = sockets
        .first()
        .cloned()
        .ok_or_else(|| invalid_config("private services require an explicit private socket"))?;
    if sockets.iter().any(|v| v != &socket) {
        return Err(invalid_config(
            "private services must share their configured private socket",
        ));
    }
    absolute_path(Path::new(&socket))?;
    let mut credentials = Vec::new();
    if let Some((_, token)) = &tool {
        credentials.push(token.as_slice());
    }
    if let Some(token) = &support {
        credentials.push(token.as_slice());
    }
    if let Some(token) = &wallet {
        credentials.push(token.as_slice());
    }
    for token in [&bounties, &images, &witness].into_iter().flatten() {
        credentials.push(token.as_slice());
    }
    if credentials
        .iter()
        .enumerate()
        .any(|(i, a)| credentials.iter().skip(i + 1).any(|b| a == b))
    {
        return Err(invalid_config(
            "private tool and support credentials must be independent",
        ));
    }
    for token in &credentials {
        for name in [
            "LMM_CORE_RPC_TOKEN_FILE",
            "LMM_CORE_ASSISTANT_TOKEN_FILE",
            "LMM_CORE_PAYMENTS_TOKEN_FILE",
            "LMM_CORE_COMMERCE_TOKEN_FILE",
            "LMM_CORE_REMOTE_CONTROL_TOKEN_FILE",
            "LMM_CORE_INGRESS_TOKEN_FILE",
        ] {
            if let Some(path) = setting_os(name) {
                if service_token(Path::new(&path))? == *token {
                    return Err(invalid_config(
                        "private projection credential must be independent from other services",
                    ));
                }
            }
        }
        if gateway.is_some_and(|g| g.contains_credential(std::str::from_utf8(token).unwrap_or("")))
        {
            return Err(invalid_config(
                "private projection credential must be independent from model credentials",
            ));
        }
    }
    let group_setting = |name| -> io::Result<Option<u32>> {
        match setting(name) {
            Ok(raw) => Ok(Some(
                raw.parse::<u32>()
                    .ok()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| invalid_config("private projection group must not be root"))?,
            )),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(invalid_config("invalid private projection group")),
        }
    };
    let groups = [
        group_setting("LMM_CORE_TOOL_MARKET_SHARED_GROUP_ID")?,
        group_setting("LMM_CORE_SUPPORT_PROJECTION_SHARED_GROUP_ID")?,
        group_setting("LMM_CORE_WALLET_SHARED_GROUP_ID")?,
        group_setting("LMM_CORE_BOUNTIES_SHARED_GROUP_ID")?,
        group_setting("LMM_CORE_IMAGES_SHARED_GROUP_ID")?,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let group = groups.first().copied();
    if groups.iter().any(|v| Some(*v) != group) {
        return Err(invalid_config(
            "shared private services require the same group",
        ));
    }
    let pool = bounded_database_pool(url, 4).await?;
    let mut app = axum::Router::new();
    if let Some((_, token)) = &tool {
        app = app.merge(
            crate::builtins::http::router(crate::builtins::Store::from_pool(pool.clone()), token)
                .map_err(|_| invalid_config("invalid private builtin registry service"))?,
        );
        app = app.merge(
            crate::tool_market::http::router(
                crate::tool_market::ToolMarketStore::from_pool(pool.clone()),
                token,
            )
            .map_err(|_| invalid_config("invalid private tool market service"))?,
        );
        let mut credit = crate::tool_market::credit_access::CreditStore::from_pool(pool.clone());
        if let Some(profile) = market_oauth_profile()? {
            credit = credit.with_market_oauth_profile(Arc::new(profile));
        }
        if let Some(path) = setting_os("LMM_CORE_TOOL_MARKET_RECEIPT_ADAPTERS_CONFIG_FILE") {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Adapters {
                adapters: Vec<crate::tool_market::receipt_verifier::AdapterConfig>,
            }
            let document = configuration_file(Path::new(&path), 256 * 1024)?;
            let config: Adapters = parse_document(&document)?;
            let verifier = crate::tool_market::receipt_verifier::ReceiptVerifier::from_adapters(
                config.adapters,
            )
            .map_err(|_| invalid_config("invalid Core supplier receipt adapters"))?;
            let mut forbidden = credentials
                .iter()
                .map(|token| token.to_vec())
                .collect::<Vec<_>>();
            for name in [
                "LMM_CORE_RPC_TOKEN_FILE",
                "LMM_CORE_ASSISTANT_TOKEN_FILE",
                "LMM_CORE_PAYMENTS_TOKEN_FILE",
                "LMM_CORE_COMMERCE_TOKEN_FILE",
                "LMM_CORE_REMOTE_CONTROL_TOKEN_FILE",
                "LMM_CORE_INGRESS_TOKEN_FILE",
            ] {
                if let Some(path) = setting_os(name) {
                    forbidden.push(service_token(Path::new(&path))?);
                }
            }
            verifier
                .reject_credentials(&forbidden.iter().map(Vec::as_slice).collect::<Vec<_>>())
                .map_err(|_| {
                    invalid_config(
                        "supplier receipt keys must be independent from service credentials",
                    )
                })?;
            if let Some(gateway) = gateway {
                verifier.reject_model_credentials(gateway).map_err(|_| {
                    invalid_config(
                        "supplier receipt keys must be independent from model credentials",
                    )
                })?;
            }
            credit = credit.with_receipt_verifier(Arc::new(verifier));
        }
        app = app.merge(
            crate::tool_market::credit_http::router(credit.clone(), token)
                .map_err(|_| invalid_config("invalid private tool market credit service"))?,
        );
        if let (Some(purpose), Some(role)) = (&witness, &witness_role) {
            app = app.merge(
                crate::tool_market::fixed_witness_http::router(credit, token, purpose, role)
                    .map_err(|_| invalid_config("invalid fixed witness purpose service"))?,
            );
        }
    }
    if let Some(issuer) = market_oauth_issuer(pool.clone()).await? {
        let token = tool.as_ref().map(|v| v.1.as_slice()).ok_or_else(|| {
            invalid_config("market OAuth private service requires tool-market host credential")
        })?;
        app = app.merge(
            crate::market_oauth::http::private_router(issuer, token)
                .map_err(|_| invalid_config("invalid market OAuth private router"))?,
        );
    }
    if let Some(token) = &support {
        let projection = crate::support_projection::SupportProjection::new(pool.clone());
        projection
            .check_readonly()
            .await
            .map_err(|_| invalid_config("support projection read authority unavailable"))?;
        app = app.merge(
            crate::support_projection::http::router(projection, token)
                .map_err(|_| invalid_config("invalid private support projection service"))?,
        );
    }
    if let Some(token) = &wallet {
        let store = crate::wallet::Store::new(pool.clone());
        store
            .check_ready()
            .await
            .map_err(|_| invalid_config("wallet private service not ready"))?;
        app = app.merge(
            crate::wallet::http::internal_router(store, token.clone())
                .map_err(|_| invalid_config("invalid private wallet service"))?,
        );
    }
    if let Some(token) = &bounties {
        let store = crate::bounties::BountyStore::from_pool(pool.clone());
        store
            .check_ready()
            .await
            .map_err(|_| invalid_config("bounty private service not ready"))?;
        let token = std::str::from_utf8(token)
            .map_err(|_| invalid_config("invalid bounties host token"))?;
        app = app.merge(
            crate::bounties::http::internal_router(store, token)
                .map_err(|_| invalid_config("invalid private bounty service"))?,
        );
    }
    if let Some(token) = &images {
        let service = gateway
            .and_then(|g| g.image_service())
            .ok_or_else(|| invalid_config("image host requires configured image models"))?;
        app = app.merge(
            crate::model_http::image_internal::internal_router(service.clone(), token.clone())
                .map_err(|_| invalid_config("invalid private image service"))?,
        );
    }
    let binding_token = tool
        .as_ref()
        .map(|v| v.1.as_slice())
        .or(support.as_deref())
        .or(wallet.as_deref())
        .or(bounties.as_deref())
        .or(images.as_deref())
        .ok_or_else(|| invalid_config("missing private binding credential"))?;
    crate::tool_market::server::ToolMarketServer::bind_with_router(
        Path::new(&socket),
        binding_token,
        app,
        group,
    )
    .map(Some)
}

pub fn assistant_tools_enabled() -> io::Result<bool> {
    let enabled = bool_setting("LMM_CORE_ASSISTANT_TOOLS_ENABLED")?;
    if enabled && (!assistant_configured() || setting_os("LMM_CORE_WEB_ORIGIN").is_none()) {
        return Err(invalid_config(
            "assistant tools require configured assistant and Web identity",
        ));
    }
    Ok(enabled)
}
pub fn assistant_configured() -> bool {
    setting_os("LMM_CORE_ASSISTANT_CONFIG_FILE").is_some()
        || setting_os("LMM_CORE_ASSISTANT_TOKEN_FILE").is_some()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssistantConfig {
    version: u32,
    personal_text: crate::assistant_model::PersonalTextPolicy,
}
#[cfg(unix)]
pub async fn assistant_rpc(
    url: &str,
    gateway: Option<&crate::model_http::ModelGateway>,
    control_token: &[u8],
) -> io::Result<Option<crate::assistant_model::rpc::AssistantRpc>> {
    let Some((document, token)) = rpc_configuration(
        "LMM_CORE_ASSISTANT_CONFIG_FILE",
        "LMM_CORE_ASSISTANT_TOKEN_FILE",
    )?
    else {
        return Ok(None);
    };
    let gateway = gateway
        .ok_or_else(|| invalid_config("assistant requires the configured Core model gateway"))?;
    if token == control_token
        || gateway.contains_credential(
            std::str::from_utf8(&token)
                .map_err(|_| invalid_config("invalid assistant credential"))?,
        )
    {
        return Err(invalid_config(
            "assistant credential must be independent from identity and provider credentials",
        ));
    }
    for name in [
        "LMM_CORE_PAYMENTS_TOKEN_FILE",
        "LMM_CORE_COMMERCE_TOKEN_FILE",
        "LMM_CORE_REMOTE_CONTROL_TOKEN_FILE",
        "LMM_CORE_INGRESS_TOKEN_FILE",
        "LMM_CORE_SUPPORT_PROJECTION_TOKEN_FILE",
        "LMM_CORE_TOOL_MARKET_TOKEN_FILE",
        "LMM_CORE_WALLET_TOKEN_FILE",
    ] {
        if let Some(path) = setting_os(name) {
            if service_token(Path::new(&path))? == token {
                return Err(invalid_config(
                    "assistant credential must be independent from other service credentials",
                ));
            }
        }
    }
    let config: AssistantConfig = parse_document(&document)?;
    if config.version != 1 {
        return Err(invalid_config(
            "unsupported assistant configuration version",
        ));
    }
    let pool = bounded_database_pool(url, 4).await?;
    if config.personal_text.gift_tools {
        let enabled: bool = sqlx::query_scalar(
            "SELECT enabled AND exclusive_cap>1 FROM core_gift.policy WHERE singleton",
        )
        .fetch_one(&pool)
        .await
        .map_err(|_| invalid_config("gift policy unavailable"))?;
        if !enabled {
            return Err(invalid_config(
                "gift tools require an explicitly enabled Core gift policy",
            ));
        }
        if let Some(sponsor) = &config.personal_text.sponsor {
            let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.accounts WHERE id=$1 AND active) AND extract(epoch FROM clock_timestamp())::bigint < $2").bind(sponsor.treasury_account_id).bind(sponsor.expires_at_unix).fetch_one(&pool).await.map_err(|_|invalid_config("assistant sponsor unavailable"))?;
            if !live {
                return Err(invalid_config(
                    "assistant sponsor policy expired or treasury inactive",
                ));
            }
        }
    }
    let service = crate::assistant_model::AssistantModel::new(
        pool.clone(),
        gateway.clone(),
        config.personal_text,
    )
    .map_err(|_| invalid_config("invalid assistant policy or model capability"))?;
    let gift_enabled = service.gift_enabled();
    let mut rpc = crate::assistant_model::rpc::AssistantRpc::new(service, &token, true)
        .map_err(|_| invalid_config("invalid assistant service configuration"))?;
    if gift_enabled {
        rpc = rpc.with_gifts(crate::gift::GiftStore::new(pool.clone()));
    }
    if assistant_tools_enabled()? {
        // Read-only startup checks: no schema creation and no synthetic Session.
        sqlx::query("SELECT o.id,o.arguments_json,o.resource_schema_sha256,o.resource_price_sha256,t.session_id,t.host_policy,t.source_web_session_sid,r.model_request_id,c.id FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id JOIN core_assistant.agent_rounds r ON r.id=o.round_id LEFT JOIN core_assistant_tools.confirmations c ON c.operation_id=o.id WHERE FALSE")
            .execute(&pool).await.map_err(|_| invalid_config("assistant tools require installed durable agent schema and runtime read authority"))?;
        let facts =
            crate::assistant_tools_core::facts::BusinessFactsStore::new(pool.clone(), gateway);
        facts
            .check_readonly()
            .await
            .map_err(|_| invalid_config("assistant business facts read authority unavailable"))?;
        rpc = rpc.with_business_facts(facts).with_confirmations(
            crate::assistant_tools_core::confirmation::ConfirmationStore::new(pool.clone()),
        );
    }
    Ok(Some(rpc))
}

pub fn assistant_tools_browser_router(pool: sqlx::PgPool) -> io::Result<Option<axum::Router>> {
    if !assistant_tools_enabled()? {
        return Ok(None);
    }
    let origin = setting("LMM_CORE_WEB_ORIGIN")
        .map_err(|_| invalid_config("assistant confirmation requires Web origin"))?;
    let local = bool_setting("LMM_CORE_WEB_LOCAL_HTTP")?;
    let state =
        crate::assistant_tools_core::confirmation::BrowserConfirmation::new(pool, &origin, local)
            .map_err(|_| invalid_config("invalid assistant confirmation browser configuration"))?;
    Ok(Some(crate::assistant_tools_core::confirmation::router(
        state,
    )))
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct PaymentPermissions {
    prepare: bool,
    credit_evidence: bool,
    refund_user: bool,
    refund_evidence: bool,
    read_user: bool,
    reconcile: bool,
    proof_ingress: bool,
    proof_read: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestPaymentChannel {
    id: i64,
    merchant: String,
    webhook_secret_file: PathBuf,
    max_usd_cents: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentConfig {
    version: u32,
    // Only the explicitly named synthetic/test provider is supported here.
    provider: String,
    channels: Vec<TestPaymentChannel>,
    permissions: PaymentPermissions,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct CommerceConfig {
    version: u32,
    pay: bool,
    settle: bool,
    refund: bool,
    read: bool,
    complete_delivery: bool,
    reconcile: bool,
}

#[cfg(unix)]
fn rpc_configuration(
    config_name: &'static str,
    token_name: &'static str,
) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    match (setting_os(config_name), setting_os(token_name)) {
        (None, None) => Ok(None),
        (Some(config), Some(token)) => Ok(Some((
            configuration_file(Path::new(&config), 64 * 1024)?,
            service_token(Path::new(&token))?,
        ))),
        _ => Err(invalid_config(
            "configure both financial RPC document and independent credential file",
        )),
    }
}

#[cfg(unix)]
pub struct FinancialRpcs {
    pub payments: Option<crate::payments::rpc::PaymentsRpc>,
    pub commerce: Option<crate::commerce::rpc::CommerceRpc>,
}

#[cfg(unix)]
fn distinct_financial_credentials(
    control: &[u8],
    payment: Option<&[u8]>,
    commerce: Option<&[u8]>,
) -> io::Result<()> {
    if payment.is_some_and(|token| token == control)
        || commerce.is_some_and(|token| token == control)
        || matches!((payment, commerce), (Some(a), Some(b)) if a == b)
    {
        return Err(invalid_config(
            "financial RPC credentials must be distinct from each other and identity transport",
        ));
    }
    Ok(())
}

/// Each money service has its own credential and bounded writable pool. The
/// generic identity transport credential never gains spending authority.
#[cfg(unix)]
pub async fn financial_rpcs(url: &str, control_token: &[u8]) -> io::Result<FinancialRpcs> {
    let payments = rpc_configuration(
        "LMM_CORE_PAYMENTS_CONFIG_FILE",
        "LMM_CORE_PAYMENTS_TOKEN_FILE",
    )?;
    let commerce = rpc_configuration(
        "LMM_CORE_COMMERCE_CONFIG_FILE",
        "LMM_CORE_COMMERCE_TOKEN_FILE",
    )?;
    distinct_financial_credentials(
        control_token,
        payments.as_ref().map(|(_, token)| token.as_slice()),
        commerce.as_ref().map(|(_, token)| token.as_slice()),
    )?;
    let payments = if let Some((document, token)) = payments {
        let config: PaymentConfig = parse_document(&document)
            .map_err(|_| invalid_config("invalid test payment configuration"))?;
        if config.version != 1
            || config.provider != "stripe-test"
            || config.channels.is_empty()
            || config.channels.len() > 16
        {
            return Err(invalid_config(
                "payments require bounded explicitly configured Stripe test channels",
            ));
        }
        let mut channels = Vec::new();
        for channel in config.channels {
            let secret = String::from_utf8(private_file(&channel.webhook_secret_file, 4096)?)
                .map_err(|_| invalid_config("invalid private payment verification key"))?;
            channels.push(crate::payments::Channel::stripe_test(
                channel.id,
                channel.merchant,
                secret.trim().to_owned(),
                channel.max_usd_cents,
            ));
        }
        let p = config.permissions;
        let permissions = crate::payments::rpc::Permissions {
            prepare: p.prepare,
            credit_evidence: p.credit_evidence,
            refund_user: p.refund_user,
            refund_evidence: p.refund_evidence,
            read_user: p.read_user,
            reconcile: p.reconcile,
            proof_ingress: p.proof_ingress,
            proof_read: p.proof_read,
        };
        let store =
            crate::payments::PaymentsStore::new(bounded_database_pool(url, 2).await?, channels)
                .map_err(|_| invalid_config("invalid test payment channel configuration"))?;
        let rpc = crate::payments::rpc::PaymentsRpc::new(store, &token, permissions)
            .map_err(|_| invalid_config("invalid payment RPC configuration"))?;
        if rpc.features().is_empty() {
            return Err(invalid_config(
                "payment RPC requires explicit service permissions",
            ));
        }
        Some(rpc)
    } else {
        None
    };
    let commerce = if let Some((document, token)) = commerce {
        let config: CommerceConfig = parse_document(&document)
            .map_err(|_| invalid_config("invalid commerce RPC configuration"))?;
        if config.version != 1 {
            return Err(invalid_config("unsupported commerce configuration version"));
        }
        let store = crate::commerce::CommerceStore::connect(url)
            .await
            .map_err(|_| invalid_config("commerce database unavailable"))?;
        let rpc = crate::commerce::rpc::CommerceRpc::new(
            store,
            &token,
            crate::commerce::rpc::Permissions {
                pay: config.pay,
                settle: config.settle,
                refund: config.refund,
                read: config.read,
                complete_delivery: config.complete_delivery,
                reconcile: config.reconcile,
            },
        )
        .map_err(|_| invalid_config("invalid commerce RPC configuration"))?;
        if rpc.features().is_empty() {
            return Err(invalid_config(
                "commerce RPC requires explicit service permissions",
            ));
        }
        Some(rpc)
    } else {
        None
    };
    Ok(FinancialRpcs { payments, commerce })
}

/// Secret/configuration files must be private regular files. Errors deliberately
/// omit paths and contents, which may carry operator-provided credential data.
pub(crate) fn private_file(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    read_file(path, maximum, true)
}
fn configuration_file(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    read_file(path, maximum, false)
}
fn read_file(path: &Path, maximum: usize, private: bool) -> io::Result<Vec<u8>> {
    read_file_with_metadata(path, maximum, private).map(|(bytes, _)| bytes)
}
fn read_file_with_metadata(
    path: &Path,
    maximum: usize,
    private: bool,
) -> io::Result<(Vec<u8>, fs::Metadata)> {
    absolute_path(path)?;
    let before = fs::symlink_metadata(path)
        .map_err(|_| invalid_config("core configuration file unavailable"))?;
    if !before.is_file() {
        return Err(invalid_config("core configuration must be a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = before.permissions().mode() & 0o777;
        if (private && !matches!(mode, 0o400 | 0o600)) || (!private && mode & 0o022 != 0) {
            return Err(invalid_config("core configuration file must be private"));
        }
    }
    let file =
        fs::File::open(path).map_err(|_| invalid_config("core configuration file unavailable"))?;
    let opened = file
        .metadata()
        .map_err(|_| invalid_config("core configuration file unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err(invalid_config(
                "core configuration file changed while opening",
            ));
        }
        use std::os::unix::fs::PermissionsExt;
        let mode = opened.permissions().mode() & 0o777;
        if (private && !matches!(mode, 0o400 | 0o600)) || (!private && mode & 0o022 != 0) {
            return Err(invalid_config("core configuration file must be private"));
        }
    }
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid_config("core configuration file unavailable"))?;
    if bytes.len() > maximum {
        return Err(invalid_config("core configuration file is too large"));
    }
    Ok((bytes, opened))
}

pub fn oauth2_configured() -> bool {
    [
        "LMM_CORE_OAUTH2_ISSUER",
        "LMM_CORE_OAUTH2_PI_MAX_CREDITS",
        "LMM_CORE_OAUTH2_GROUPS",
        "LMM_CORE_OAUTH2_ACCESS_TTL_SECONDS",
        "LMM_CORE_OAUTH2_FAMILY_TTL_SECONDS",
    ]
    .iter()
    .any(|name| setting_os(name).is_some())
}
fn validate_isolated_listener(issuer: &str, listen: &str, proxy: bool) -> io::Result<()> {
    let origin =
        reqwest::Url::parse(issuer).map_err(|_| invalid_config("invalid isolated issuer"))?;
    let ip: std::net::IpAddr = origin
        .host_str()
        .unwrap_or("")
        .trim_matches(['[', ']'])
        .parse()
        .map_err(|_| invalid_config("isolated issuer must be numeric loopback"))?;
    let expected = std::net::SocketAddr::new(
        ip,
        origin
            .port()
            .ok_or_else(|| invalid_config("isolated issuer requires a dynamic port"))?,
    );
    let actual = listen
        .parse::<std::net::SocketAddr>()
        .map_err(|_| invalid_config("isolated listener must be numeric loopback"))?;
    if (!proxy && actual != expected)
        || (proxy
            && (actual.ip() != expected.ip()
                || !actual.ip().is_loopback()
                || !(49152..=65535).contains(&actual.port())
                || actual.port() == 50837))
    {
        return Err(invalid_config(
            "isolated issuer requires matching listener or explicit bounded loopback proxy",
        ));
    }
    Ok(())
}
fn isolated_secret(directory: &Path, path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let canonical =
        fs::canonicalize(path).map_err(|_| invalid_config("isolated secret is unavailable"))?;
    if canonical != path || !canonical.starts_with(directory) {
        return Err(invalid_config(
            "isolated secrets cannot share a normal credential registry",
        ));
    }
    let (bytes, metadata) = read_file_with_metadata(path, maximum, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.uid() != fs::metadata(directory)?.uid() {
            return Err(invalid_config(
                "isolated secrets must be independent private files",
            ));
        }
    }
    Ok(bytes)
}
fn validate_isolated_oauth_configuration(issuer: &str) -> io::Result<()> {
    if !bool_setting("LMM_CORE_MODEL_LOCAL_HTTP")? {
        return Err(invalid_config(
            "isolated OAuth2 requires synthetic local models",
        ));
    }
    validate_isolated_listener(
        issuer,
        &setting("LMM_CORE_LISTEN").map_err(|_| invalid_config("isolated listener is missing"))?,
        bool_setting("LMM_CORE_OAUTH2_ISOLATED_LOOPBACK_PROXY")?,
    )?;
    let directory = setting_os("LMM_CORE_OAUTH2_ISOLATED_SECRET_DIRECTORY")
        .map(PathBuf::from)
        .ok_or_else(|| invalid_config("isolated OAuth2 secret directory is missing"))?;
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || fs::canonicalize(&directory)? != directory {
        return Err(invalid_config(
            "isolated OAuth2 requires an independent private directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o700 {
            return Err(invalid_config("isolated secret directory must be private"));
        }
    }
    let path = |name| {
        setting_os(name)
            .map(PathBuf::from)
            .ok_or_else(|| invalid_config("isolated secret configuration is incomplete"))
    };
    isolated_secret(&directory, &path("LMM_CORE_WEB_REFRESH_KEY_FILE")?, 4096)?;
    let database = isolated_secret(&directory, &path("LMM_CORE_DATABASE_URL_FILE")?, 16_384)?;
    let database = reqwest::Url::parse(
        std::str::from_utf8(&database)
            .map_err(|_| invalid_config("invalid isolated database URL"))?
            .trim(),
    )
    .map_err(|_| invalid_config("invalid isolated database URL"))?;
    let name = database.path().strip_prefix('/').unwrap_or("");
    let reviewed_fixture_name = name.len() == 20
        && name.starts_with("a09_")
        && name.ends_with("_a10")
        && name.as_bytes()[4..16].iter().all(u8::is_ascii_hexdigit);
    if !matches!(database.scheme(), "postgres" | "postgresql")
        || !matches!(database.host_str(), Some("127.0.0.1" | "[::1]"))
        || !(name.starts_with("lmm_pi_isolated_") || reviewed_fixture_name)
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || database.fragment().is_some()
        || database
            .query_pairs()
            .any(|(key, value)| key != "sslmode" || value != "disable")
        || database
            .port()
            .is_none_or(|port| port < 1024 || matches!(port, 50837 | 47399 | 18080 | 18081))
    {
        return Err(invalid_config(
            "isolated OAuth2 requires a fresh named loopback test database",
        ));
    }
    let document = configuration_file(&path("LMM_CORE_MODEL_CONFIG_FILE")?, 4 * 1024 * 1024)?;
    let model: ModelConfig = parse_document(&document)
        .map_err(|_| invalid_config("invalid isolated model configuration"))?;
    if !model.external_models.is_empty()
        || model.routing.upstreams.is_empty()
        || model.credentials.is_empty()
        || model.routing.upstreams.iter().any(|upstream| {
            crate::routing::Endpoint::local_loopback(upstream.endpoint.clone()).is_err()
        })
    {
        return Err(invalid_config(
            "isolated OAuth2 providers must all be synthetic numeric loopback",
        ));
    }
    for credential in model.credentials {
        let secret = isolated_secret(&directory, &credential.secret_file, 4096)?;
        let secret = std::str::from_utf8(&secret)
            .map_err(|_| invalid_config("invalid synthetic credential"))?
            .trim();
        if secret.len() < 33
            || secret.len() > 128
            || !secret.starts_with("lmm-isolated-test-")
            || !secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(invalid_config(
                "isolated OAuth2 cannot load normal provider credentials",
            ));
        }
    }
    Ok(())
}

/// Explicit public-client profile. There is no default enabled issuer, group,
/// lifetime, local-HTTP relaxation or fallback to a forwarded request origin.
pub async fn oauth2_issuer(
    store: crate::identity::IdentityStore,
) -> io::Result<Option<crate::identity::grants::LmmIssuer>> {
    if !oauth2_configured() {
        return Ok(None);
    }
    let required = |name| {
        setting(name).map_err(|_| invalid_config("OAuth2 profile configuration is incomplete"))
    };
    let issuer = required("LMM_CORE_OAUTH2_ISSUER")?;
    let isolated = bool_setting("LMM_CORE_OAUTH2_ISOLATED_LOOPBACK_TEST")?;
    if required("LMM_CORE_WEB_ORIGIN")? != issuer
        || setting_os("LMM_CORE_WEB_REFRESH_KEY_FILE").is_none()
        || bool_setting("LMM_CORE_WEB_LOCAL_HTTP")? != isolated
    {
        return Err(invalid_config(
            "OAuth2 requires exact Web origin and explicit matching transport mode",
        ));
    }
    let groups: Vec<String> = serde_json::from_str(&required("LMM_CORE_OAUTH2_GROUPS")?)
        .map_err(|_| invalid_config("invalid OAuth2 group profile"))?;
    let mut profile = if isolated {
        validate_isolated_oauth_configuration(&issuer)?;
        crate::identity::grants::LmmProfile::isolated_loopback_test(&issuer, groups)
    } else {
        if setting_os("LMM_CORE_OAUTH2_ISOLATED_SECRET_DIRECTORY").is_some()
            || setting_os("LMM_CORE_OAUTH2_ISOLATED_LOOPBACK_PROXY").is_some()
        {
            return Err(invalid_config(
                "production OAuth2 cannot use an isolated secret registry",
            ));
        }
        crate::identity::grants::LmmProfile::new(&issuer, groups)
    }
    .map_err(|_| invalid_config("invalid OAuth2 issuer or group profile"))?;
    if let Some(value) = setting_os("LMM_CORE_OAUTH2_PI_MAX_CREDITS") {
        if setting_os("LMM_CORE_MODEL_CONFIG_FILE").is_none() {
            return Err(invalid_config(
                "Pi delegation requires configured model service",
            ));
        }
        let maximum = value
            .to_str()
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(|| invalid_config("invalid Pi budget ceiling"))?;
        profile = profile
            .with_pi_budget_limit(maximum)
            .map_err(|_| invalid_config("invalid Pi budget ceiling"))?;
    }
    let lifetime = |name, maximum| -> io::Result<i64> {
        required(name)?
            .parse::<i64>()
            .ok()
            .filter(|value| (1..=maximum).contains(value))
            .ok_or_else(|| invalid_config("invalid bounded OAuth2 lifetime"))
    };
    let access = lifetime("LMM_CORE_OAUTH2_ACCESS_TTL_SECONDS", 86_400)?;
    let family = lifetime("LMM_CORE_OAUTH2_FAMILY_TTL_SECONDS", 31_536_000)?;
    crate::identity::grants::LmmIssuer::open(store, profile, access, family)
        .await
        .map(Some)
        .map_err(|_| invalid_config("OAuth2 installed schema or profile is unavailable"))
}

pub struct WebAuthConfig {
    pub auth: crate::legacy_auth::LegacyAuthConfig,
    pub registration_enabled: bool,
    pub user_agreement_enabled: bool,
    pub privacy_policy_enabled: bool,
}

fn explicit_legal_policy(
    agreement: Option<&str>,
    privacy: Option<&str>,
) -> io::Result<(bool, bool)> {
    let parse = |value| match value {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(invalid_config(
            "Web legal publication flags must each be explicitly true or false",
        )),
    };
    Ok((parse(agreement)?, parse(privacy)?))
}

pub fn legacy_auth() -> io::Result<Option<WebAuthConfig>> {
    let origin = setting("LMM_CORE_WEB_ORIGIN");
    let key_file = setting_os("LMM_CORE_WEB_REFRESH_KEY_FILE");
    let local_http = bool_setting("LMM_CORE_WEB_LOCAL_HTTP")?;
    match (origin, key_file) {
        (Err(env::VarError::NotPresent), None) if !local_http => Ok(None),
        (Ok(origin), Some(path)) => {
            let agreement = setting("LMM_CORE_WEB_USER_AGREEMENT_ENABLED");
            let privacy = setting("LMM_CORE_WEB_PRIVACY_POLICY_ENABLED");
            let (user_agreement_enabled, privacy_policy_enabled) =
                explicit_legal_policy(agreement.as_deref().ok(), privacy.as_deref().ok())?;
            let key = private_file(Path::new(&path), 128)?;
            let config = crate::legacy_auth::LegacyAuthConfig::new(&origin, &key)
                .map_err(|_| invalid_config("invalid Web authentication configuration"))?;
            let config = if local_http {
                config
                    .with_insecure_local_http()
                    .map_err(|_| invalid_config("local Web HTTP origin is not allowed"))?
            } else {
                config
            };
            Ok(Some(WebAuthConfig {
                auth: config.with_legal_acceptance_required(
                    user_agreement_enabled || privacy_policy_enabled,
                ),
                registration_enabled: bool_setting("LMM_CORE_REGISTRATION_ENABLED")?,
                user_agreement_enabled,
                privacy_policy_enabled,
            }))
        }
        _ => Err(invalid_config(
            "configure both Web origin and refresh key file",
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelUpstream {
    id: u64,
    endpoint: String,
    protocol: crate::routing::Protocol,
    credential: crate::routing::CredentialRef,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelRouting {
    version: u64,
    groups: Vec<crate::routing::ModelGroup>,
    upstreams: Vec<ModelUpstream>,
    routes: Vec<crate::routing::ModelRoute>,
    aliases: Vec<crate::routing::ModelAlias>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCredential {
    reference: crate::routing::CredentialRef,
    secret_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyGroup {
    key_id: i64,
    group: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageProviderConfig {
    id: u64,
    vendor: String,
    upstream_model: String,
    endpoint: String,
    secret_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelConfig {
    version: u32,
    routing: ModelRouting,
    prices: crate::routing::PriceConfig,
    credentials: Vec<ModelCredential>,
    key_groups: Vec<KeyGroup>,
    input_token_ceiling: i64,
    lease_seconds: i64,
    timeout_seconds: u64,
    max_output_tokens: u32,
    #[serde(default)]
    local_streaming: bool,
    #[serde(default)]
    external_models: Vec<crate::model_http::ExternalModelContext>,
    #[serde(default)]
    image_prices: Option<crate::model_http::image_price::ImagePriceConfig>,
    #[serde(default)]
    image_providers: Vec<ImageProviderConfig>,
}

#[cfg(test)]
fn model_gateway(
    pool: sqlx::PgPool,
    document: &[u8],
    local_http: bool,
) -> io::Result<crate::model_http::ModelGateway> {
    model_gateway_with_origins(pool, document, local_http).map(|(gateway, _)| gateway)
}
/// Read the same typed sidecar used by the gateway; an env filename alone does
/// not imply image spending authority. This performs no database writes.
pub fn images_configured() -> io::Result<bool> {
    let Some(path) = setting_os("LMM_CORE_MODEL_CONFIG_FILE") else {
        return Ok(false);
    };
    let config: ModelConfig =
        parse_document(&configuration_file(Path::new(&path), 16 * 1024 * 1024)?)?;
    match (&config.image_prices, config.image_providers.is_empty()) {
        (None, true) => Ok(false),
        (Some(prices), false) => {
            prices
                .validate()
                .map_err(|_| invalid_config("invalid image tariffs"))?;
            Ok(true)
        }
        _ => Err(invalid_config(
            "image tariffs and providers must be configured together",
        )),
    }
}
fn model_gateway_with_origins(
    pool: sqlx::PgPool,
    document: &[u8],
    local_http: bool,
) -> io::Result<(crate::model_http::ModelGateway, Vec<CredentialOrigin>)> {
    let config: ModelConfig = parse_document(document)
        .map_err(|_| invalid_config("invalid model configuration document"))?;
    if local_http != config.external_models.is_empty() {
        return Err(invalid_config(
            "model provider mode requires explicit matching context contracts",
        ));
    }
    let provider_policy = crate::model_http::ProviderPolicy::new(
        config.external_models.clone(),
        config.input_token_ceiling,
    )
    .map_err(|_| invalid_config("invalid external model context contracts"))?;
    if config.version != 1
        || config.routing.routes.is_empty()
        || config.credentials.is_empty()
        || !(1..=240).contains(&config.timeout_seconds)
        || !(1..=65_536).contains(&config.max_output_tokens)
    {
        return Err(invalid_config(
            "model configuration has no usable grants or exceeds limits",
        ));
    }
    let upstreams = config
        .routing
        .upstreams
        .into_iter()
        .map(|upstream| {
            let endpoint = if local_http {
                // Local mode cannot accidentally contact an actual paid provider.
                crate::routing::Endpoint::local_loopback(upstream.endpoint)
            } else {
                crate::routing::Endpoint::try_from(upstream.endpoint)
            }
            .map_err(|_| invalid_config("invalid model upstream endpoint"))?;
            Ok(crate::routing::UpstreamConfig {
                id: upstream.id,
                endpoint,
                protocol: upstream.protocol,
                credential: upstream.credential,
                enabled: upstream.enabled,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    // Reject unusable weighted targets at startup, before reading credentials.
    // Dispatch repeats this validation against its frozen selected attempt.
    let upstream_by_id: BTreeMap<_, _> = upstreams.iter().map(|u| (u.id, u)).collect();
    for route in config.routing.routes.iter().filter(|route| route.enabled) {
        for target in route
            .targets
            .iter()
            .filter(|target| target.enabled && target.weight > 0)
        {
            let upstream = upstream_by_id
                .get(&target.upstream_id)
                .filter(|upstream| upstream.enabled)
                .ok_or_else(|| invalid_config("model target has no enabled upstream"))?;
            provider_policy
                .validate_binding(
                    upstream.id,
                    &target.upstream_model,
                    upstream.protocol,
                    upstream.endpoint.as_str(),
                    config.input_token_ceiling,
                    // Binding needs a usable positive output capacity. Each
                    // selected request checks its actual output ceiling.
                    1,
                )
                .map_err(|_| invalid_config("invalid model provider context binding"))?;
        }
    }
    let required: BTreeSet<_> = upstreams
        .iter()
        .filter(|u| u.enabled)
        .map(|u| (u.credential.id, u.credential.revision))
        .collect();
    let mut supplied = BTreeSet::new();
    let mut credentials = Vec::new();
    let mut credential_origins = Vec::new();
    for credential in config.credentials {
        let reference = credential.reference;
        if !supplied.insert((reference.id, reference.revision)) {
            return Err(invalid_config("duplicate model credential reference"));
        }
        let (secret, origin) =
            credential_file(&credential.secret_file, "invalid model upstream credential")?;
        credential_origins.push(origin);
        credentials.push(crate::model_http::UpstreamCredential { reference, secret });
    }
    if required.is_empty() || required != supplied {
        return Err(invalid_config(
            "model credential references do not match enabled upstreams",
        ));
    }
    let mut key_groups = BTreeMap::new();
    for grant in config.key_groups {
        if key_groups.insert(grant.key_id, grant.group).is_some() {
            return Err(invalid_config("duplicate model key grant"));
        }
    }
    let routing = crate::routing::RoutingConfig {
        version: config.routing.version,
        groups: config.routing.groups,
        upstreams,
        routes: config.routing.routes,
        aliases: config.routing.aliases,
    };
    let prices = crate::routing::PriceSnapshot::compile(config.prices)
        .map_err(|_| invalid_config("invalid model prices"))?;
    let routing = crate::routing::Router::new(routing, Arc::new(prices))
        .map_err(|_| invalid_config("invalid model routes"))?;
    let limits = crate::relay::Limits {
        max_in_flight: max_inflight()?,
        total_timeout: Duration::from_secs(config.timeout_seconds),
        header_timeout: Duration::from_secs(config.timeout_seconds.min(30)),
        read_timeout: Duration::from_secs(config.timeout_seconds.min(30)),
        hook_timeout: Duration::from_secs(5),
        max_output_tokens: config.max_output_tokens,
        ..crate::relay::Limits::default()
    };
    let images = match config.image_prices {
        None if config.image_providers.is_empty() => None,
        None => {
            return Err(invalid_config(
                "image providers require explicit exact tariffs",
            ));
        }
        Some(prices) => {
            for rule in &prices.rules {
                if routing.catalog(&rule.billing_group).is_err() {
                    return Err(invalid_config("image tariff has no enabled model group"));
                }
            }
            if config.image_providers.is_empty() || config.image_providers.len() > 128 {
                return Err(invalid_config(
                    "image tariffs require bounded explicit providers",
                ));
            }
            let mut specs = Vec::new();
            for provider in config.image_providers {
                let (secret, origin) =
                    credential_file(&provider.secret_file, "invalid image upstream credential")?;
                credential_origins.push(origin);
                specs.push(crate::model_http::image_execution::ProviderSpec {
                    upstream_id: provider.id,
                    vendor: provider.vendor,
                    upstream_model: provider.upstream_model,
                    base_url: provider.endpoint,
                    secret,
                    local_http,
                });
            }
            Some(
                crate::model_http::image_execution::ImageService::new(
                    pool.clone(),
                    crate::billing::charging::Charging::new(
                        pool.clone(),
                        Arc::new(crate::billing::transactional_ledger::TransactionalLedger),
                    ),
                    prices,
                    specs,
                    &limits,
                )
                .map_err(|_| invalid_config("unsafe or incomplete image provider configuration"))?,
            )
        }
    };
    let gateway = crate::model_http::ModelGateway::new_with_provider_policy(
        pool,
        Arc::new(routing),
        credentials,
        key_groups,
        limits,
        crate::model_http::ChargePolicy {
            input_token_ceiling: config.input_token_ceiling,
            lease_seconds: config.lease_seconds,
        },
        provider_policy,
    )
    .map_err(|_| invalid_config("unsafe or incomplete model gateway configuration"))?;
    let gateway = if let Some(images) = images {
        gateway.with_image_service(images)
    } else {
        gateway
    };
    Ok((
        if config.local_streaming {
            gateway.with_local_streaming()
        } else {
            gateway
        },
        credential_origins,
    ))
}

// Host-only file identities and in-memory values; never expose these via HTTP,
// Debug or serialization. File identities are taken from the opened private file.
struct CredentialOrigin {
    canonical_file: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
impl CredentialOrigin {
    fn same_file(&self, other: &Self) -> bool {
        if self.canonical_file == other.canonical_file {
            return true;
        }
        #[cfg(unix)]
        {
            self.device == other.device && self.inode == other.inode
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}
fn credential_file(path: &Path, invalid: &'static str) -> io::Result<(String, CredentialOrigin)> {
    let (bytes, opened) = read_file_with_metadata(path, 4096, true)?;
    let bytes = zeroize::Zeroizing::new(bytes);
    let secret = std::str::from_utf8(&bytes)
        .map_err(|_| invalid_config(invalid))?
        .trim();
    if secret.is_empty() || !secret.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid_config(invalid));
    }
    // Reject a replaced or newly linked path before accepting its provenance.
    absolute_path(path)?;
    let canonical_file = fs::canonicalize(path).map_err(|_| invalid_config(invalid))?;
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        let current = fs::symlink_metadata(path).map_err(|_| invalid_config(invalid))?;
        if !current.is_file() || current.dev() != opened.dev() || current.ino() != opened.ino() {
            return Err(invalid_config(invalid));
        }
        (opened.dev(), opened.ino())
    };
    #[cfg(not(unix))]
    let _ = opened;
    Ok((
        secret.to_owned(),
        CredentialOrigin {
            canonical_file,
            #[cfg(unix)]
            device,
            #[cfg(unix)]
            inode,
        },
    ))
}
#[derive(Default)]
struct AbuseCredentialRegistry {
    secrets: BTreeMap<String, String>,
    origins: Vec<CredentialOrigin>,
    allowed_origins: BTreeMap<String, BTreeSet<String>>,
    additional_roots: BTreeMap<String, reqwest::Certificate>,
}
fn reject_credential_reuse(
    isolated: &AbuseCredentialRegistry,
    normal: Option<&crate::model_http::ModelGateway>,
    normal_origins: &[CredentialOrigin],
) -> io::Result<()> {
    let same_file = isolated
        .origins
        .iter()
        .any(|origin| normal_origins.iter().any(|normal| origin.same_file(normal)));
    let same_value = normal.is_some_and(|gateway| {
        isolated
            .secrets
            .values()
            .any(|secret| gateway.contains_credential(secret))
    });
    if same_file || same_value {
        return Err(invalid_config(
            "isolated moderation credentials must be independent",
        ));
    }
    Ok(())
}

// Host-only sidecar. Administration and public policies carry aliases only.
// Intentionally no Debug/Serialize implementation for credential material.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AbuseCredentialFile {
    alias: String,
    secret_file: PathBuf,
    allowed_origins: Vec<String>,
    tls_ca_file: Option<PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AbuseCredentialsFile {
    version: u32,
    credentials: Vec<AbuseCredentialFile>,
}

#[cfg(test)]
fn abuse_credentials(document: &[u8]) -> io::Result<AbuseCredentialRegistry> {
    abuse_credentials_for_network(document, false)
}
fn abuse_credentials_for_network(
    document: &[u8],
    local_http: bool,
) -> io::Result<AbuseCredentialRegistry> {
    let config: AbuseCredentialsFile = parse_document(document)
        .map_err(|_| invalid_config("invalid isolated moderation credential configuration"))?;
    if config.version != 1 || config.credentials.len() > 32 {
        return Err(invalid_config(
            "invalid isolated moderation credential configuration",
        ));
    }
    let mut registry = AbuseCredentialRegistry::default();
    for entry in config.credentials {
        if entry.alias.is_empty()
            || entry.alias.len() > 64
            || !entry
                .alias
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || registry.secrets.contains_key(&entry.alias)
        {
            return Err(invalid_config(
                "invalid isolated moderation credential alias",
            ));
        }
        if entry.allowed_origins.is_empty() || entry.allowed_origins.len() > 16 {
            return Err(invalid_config("invalid isolated moderation origin policy"));
        }
        let mut allowed = BTreeSet::new();
        for raw in entry.allowed_origins {
            let origin = crate::relay::honeypot::validate_origin(&raw, local_http)
                .map_err(|_| invalid_config("invalid isolated moderation origin policy"))?;
            if !allowed.insert(origin) {
                return Err(invalid_config("duplicate isolated moderation origin"));
            }
        }
        let additional_root = entry
            .tls_ca_file
            .map(|path| {
                let bytes = private_file(&path, 64 * 1024)?;
                // With reqwest's rustls backend, from_pem only retains bytes;
                // a PEM with no certificate would also add zero roots at build.
                // Use its maintained PEM reader, require a certificate, then
                // let its actual TLS root store validate every DER certificate.
                let parsed = reqwest::Certificate::from_pem_bundle(&bytes)
                    .map_err(|_| invalid_config("invalid isolated moderation CA"))?;
                if parsed.is_empty() {
                    return Err(invalid_config("invalid isolated moderation CA"));
                }
                let certificate = reqwest::Certificate::from_pem(&bytes)
                    .map_err(|_| invalid_config("invalid isolated moderation CA"))?;
                reqwest::Client::builder()
                    .no_proxy()
                    .tls_built_in_root_certs(false)
                    .add_root_certificate(certificate.clone())
                    .build()
                    .map_err(|_| invalid_config("invalid isolated moderation CA"))?;
                Ok(certificate)
            })
            .transpose()?;
        let (secret, origin) = credential_file(
            &entry.secret_file,
            "invalid isolated moderation upstream credential",
        )?;
        registry
            .allowed_origins
            .insert(entry.alias.clone(), allowed);
        if let Some(root) = additional_root {
            registry.additional_roots.insert(entry.alias.clone(), root);
        }
        registry.secrets.insert(entry.alias, secret);
        registry.origins.push(origin);
    }
    Ok(registry)
}

/// Compose the installed abuse authority and a separate uncharged transport.
/// This never enables the ordinary model gateway: the host attaches it only to
/// the already explicitly configured gateway. With no sidecar, fixed text needs
/// no upstream credential and administrators see an empty alias registry.
pub fn models_and_abuse(
    pool: sqlx::PgPool,
) -> io::Result<(
    crate::abuse::AbuseStore,
    Option<crate::model_http::ModelGateway>,
)> {
    let (normal, normal_origins) = models_with_origins(pool.clone())?;
    let local_http = bool_setting("LMM_CORE_ABUSE_LOCAL_HTTP")?;
    let mut registry = match setting_os("LMM_CORE_ABUSE_CREDENTIAL_CONFIG_FILE") {
        Some(path) => abuse_credentials_for_network(
            &configuration_file(Path::new(&path), 64 * 1024)?,
            local_http,
        )?,
        None => AbuseCredentialRegistry::default(),
    };
    reject_credential_reuse(&registry, normal.as_ref(), &normal_origins)?;
    // Domain admission and transport receive the same host-normalized origin
    // registry. Neither can expand it from a request or a persisted policy.
    let store = crate::abuse::AbuseStore::new_with_network_policy(
        pool,
        registry.allowed_origins.clone(),
        local_http,
    )
    .map_err(|_| invalid_config("invalid isolated moderation network policy"))?;
    let mut credentials = BTreeMap::new();
    for (alias, secret) in registry.secrets {
        let allowed = registry
            .allowed_origins
            .get(&alias)
            .ok_or_else(|| invalid_config("invalid isolated moderation network policy"))?
            .iter()
            .cloned()
            .collect();
        let credential = crate::model_http::HoneypotCredential::new(
            secret,
            allowed,
            registry.additional_roots.remove(&alias),
        )
        .map_err(|_| invalid_config("invalid isolated moderation network policy"))?;
        credentials.insert(alias, credential);
    }
    let limits = crate::relay::Limits {
        max_in_flight: max_inflight()?,
        request_bytes: 1024 * 1024,
        total_timeout: Duration::from_secs(240),
        hook_timeout: Duration::from_secs(5),
        ..crate::relay::Limits::default()
    };
    let gateway = crate::model_http::HoneypotGateway::new_with_network_policy(
        store.clone(),
        credentials,
        local_http,
        limits,
    )
    .map_err(|_| invalid_config("invalid isolated moderation transport configuration"))?;
    Ok((store, normal.map(|normal| normal.with_honeypot(gateway))))
}

pub fn models(pool: sqlx::PgPool) -> io::Result<Option<crate::model_http::ModelGateway>> {
    models_with_origins(pool).map(|(gateway, _)| gateway)
}
fn models_with_origins(
    pool: sqlx::PgPool,
) -> io::Result<(
    Option<crate::model_http::ModelGateway>,
    Vec<CredentialOrigin>,
)> {
    let local_http = bool_setting("LMM_CORE_MODEL_LOCAL_HTTP")?;
    match setting_os("LMM_CORE_MODEL_CONFIG_FILE") {
        Some(path) => model_gateway_with_origins(
            pool,
            &configuration_file(Path::new(&path), 16 * 1024 * 1024)?,
            local_http,
        )
        .map(|(gateway, origins)| (Some(gateway), origins)),
        None if !local_http => Ok((None, Vec::new())),
        None => Err(invalid_config(
            "local model HTTP requires a model configuration file",
        )),
    }
}

pub fn database_url() -> io::Result<Option<String>> {
    let inline = setting_os("LMM_CORE_DATABASE_URL")
        .map(|value| {
            value.into_string().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "database URL is not valid UTF-8",
                )
            })
        })
        .transpose()?;
    let file = setting_os("LMM_CORE_DATABASE_URL_FILE").map(PathBuf::from);
    read_database_url(inline, file)
}
/// Resource limits for the independently deployed Rust process.
fn parse_limit(name: &str, value: Option<&str>, default: u32, maximum: u32) -> io::Result<u32> {
    let number = match value {
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid {name}")))?,
        None => default,
    };
    if number == 0 || number > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {name}: expected 1..={maximum}"),
        ));
    }
    Ok(number)
}

fn configured_limit(name: &str, default: u32, maximum: u32) -> io::Result<u32> {
    match setting(name) {
        Ok(value) => parse_limit(name, Some(&value), default, maximum),
        Err(env::VarError::NotPresent) => parse_limit(name, None, default, maximum),
        Err(env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {name}"),
        )),
    }
}

pub fn worker_threads() -> io::Result<usize> {
    Ok(configured_limit("LMM_CORE_WORKER_THREADS", 1, 4)? as usize)
}

pub fn database_connections() -> io::Result<u32> {
    configured_limit("LMM_CORE_DB_CONNECTIONS", 2, 8)
}

pub fn database_acquire_timeout_seconds() -> io::Result<u32> {
    configured_limit("LMM_CORE_DB_ACQUIRE_TIMEOUT_SECONDS", 3, 30)
}

/// Deadline for the one-shot read-only check-db observation only.
pub fn database_check_timeout_seconds() -> io::Result<u32> {
    configured_limit("LMM_CORE_DB_CHECK_TIMEOUT_SECONDS", 15, 120)
}

pub fn password_slots() -> io::Result<usize> {
    Ok(configured_limit("LMM_CORE_PASSWORD_SLOTS", 1, 2)? as usize)
}

pub fn max_inflight() -> io::Result<usize> {
    Ok(configured_limit("LMM_CORE_MAX_INFLIGHT", 16, 64)? as usize)
}

fn read_database_url(inline: Option<String>, file: Option<PathBuf>) -> io::Result<Option<String>> {
    let value = match (inline, file) {
        (Some(_), Some(_)) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "configure exactly one database URL source",
            ));
        }
        (Some(value), None) => value,
        (None, Some(path)) => String::from_utf8(private_file(&path, 16 * 1024)?)
            .map_err(|_| invalid_config("database URL is not valid UTF-8"))?,
        (None, None) => return Ok(None),
    };
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "database URL is empty",
        ));
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_proof_disabled_does_not_read_service_files() {
        assert!(
            checked_edge_access_config(None, &[PathBuf::from("/missing-service-token")])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn edge_proof_service_settings_include_every_core_token_purpose() {
        for name in [
            "RPC",
            "TOOL_MARKET",
            "WALLET",
            "BOUNTIES",
            "IMAGES",
            "SUPPORT_PROJECTION",
            "TOOL_MARKET_FIXED_WITNESS",
            "INGRESS",
            "REMOTE_CONTROL",
            "PAYMENTS",
            "COMMERCE",
            "ASSISTANT",
            "FUTURE_PURPOSE",
        ] {
            assert!(edge_service_token_setting(&format!(
                "LMM_CORE_{name}_TOKEN_FILE"
            )));
        }
        for name in [
            "OTHER_RPC_TOKEN_FILE",
            "LMM_CORE_EDGE_PROOF_FILE",
            "LMM_CORE_RPC_TOKEN_FILE_EXTRA",
        ] {
            assert!(!edge_service_token_setting(name));
        }
    }

    #[cfg(unix)]
    #[test]
    fn edge_proof_rejects_same_path_and_trimmed_bytes_in_distinct_private_files() {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let directory = std::env::temp_dir().join(format!(
            "lmm-edge-proof-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let proof = directory.join("edge");
        let reused = directory.join("service-reused");
        let independent = directory.join("service-independent");
        for (path, contents) in [
            (&proof, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"),
            (&reused, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            (&independent, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n"),
        ] {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            file.write_all(contents.as_bytes()).unwrap();
        }
        for path in [&proof, &reused] {
            let error = checked_edge_access_config(Some(proof.clone()), &[path.clone()])
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(
                error.to_string(),
                "edge proof must be independent from service credentials"
            );
        }
        let config = checked_edge_access_config(Some(proof.clone()), &[independent])
            .unwrap()
            .unwrap();
        assert_eq!(config.proof_file, proof);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deployment_edge_requires_database_console_and_private_absolute_path() {
        let edge = "\n[edge_access]\nproof_file='/nonexistent-lmm-test/edge-proof'\n";
        assert!(parse_core_file(format!("version=1{edge}").as_bytes()).is_err());
        let base = "version=1\n[database]\nurl_file='/nonexistent-lmm-test/db'\n";
        assert!(parse_core_file(format!("{base}{edge}").as_bytes()).is_err());
        let document = format!("{base}[console]\nenabled=true\n{edge}");
        let values = parse_core_file(document.as_bytes()).unwrap();
        assert_eq!(
            values.get("LMM_CORE_EDGE_PROOF_FILE"),
            Some(&OsString::from("/nonexistent-lmm-test/edge-proof"))
        );
        assert!(
            parse_core_file(
                document
                    .replace("'/nonexistent-lmm-test/edge-proof'", "'relative-proof'")
                    .as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn deployment_external_oauth_requires_web_and_separate_provider_settings() {
        let base = "version=1\n[database]\nurl_file='/nonexistent-lmm-test/db'\n";
        for (provider, key) in [
            ("github", "LMM_CORE_GITHUB_CLIENT_SECRET_FILE"),
            ("linuxdo", "LMM_CORE_LINUXDO_CLIENT_SECRET_FILE"),
            ("discord", "LMM_CORE_DISCORD_CLIENT_SECRET_FILE"),
        ] {
            let oauth = format!(
                "[identity.{provider}]\nclient_id='public-client'\nclient_secret_file='/nonexistent-lmm-test/{provider}'\nredirect_uri='https://api.example.test/oauth/{provider}'\n"
            );
            assert!(parse_core_file(format!("{base}{oauth}").as_bytes()).is_err());
            let web = "[identity.web]\norigin='https://api.example.test'\nrefresh_key_file='/nonexistent-lmm-test/refresh'\nuser_agreement_enabled=true\nprivacy_policy_enabled=true\n";
            let values = parse_core_file(format!("{base}{web}{oauth}").as_bytes()).unwrap();
            assert_eq!(
                values.get(key),
                Some(&OsString::from(format!("/nonexistent-lmm-test/{provider}")))
            );
            assert!(!values.contains_key("LMM_CORE_GOOGLE_CLIENT_SECRET_FILE"));
        }
    }
    #[test]
    fn builtin_private_hosts_require_database_and_separate_witness_configuration() {
        for (section, token) in [
            ("images", "LMM_CORE_IMAGES_TOKEN_FILE"),
            ("bounties", "LMM_CORE_BOUNTIES_TOKEN_FILE"),
        ] {
            let block = format!(
                "[{section}]\ntoken_file='/missing-token'\nsocket='/missing-private.sock'\n"
            );
            assert!(parse_core_file(format!("version=1\n{block}").as_bytes()).is_err());
            let document = format!("version=1\n[database]\nurl_file='/missing-db'\n{block}");
            assert!(
                parse_core_file(document.as_bytes())
                    .unwrap()
                    .contains_key(token)
            );
            assert!(parse_core_file(format!("{document}shared_group_id=0\n").as_bytes()).is_err());
        }
        let base = "version=1\n[database]\nurl_file='/missing-db'\n[tool_market]\nsocket='/missing-private.sock'\ntoken_file='/missing-token'\n";
        for incomplete in [
            "fixed_witness_role='reviewed-host'\n",
            "fixed_witness_token_file='/missing-purpose-token'\n",
        ] {
            assert!(parse_core_file(format!("{base}{incomplete}").as_bytes()).is_err());
        }
        let complete = format!(
            "{base}fixed_witness_role='reviewed-host'\nfixed_witness_token_file='/missing-purpose-token'\nreceipt_adapters_config_file='/missing-adapters'\n"
        );
        let settings = parse_core_file(complete.as_bytes()).unwrap();
        assert_eq!(
            settings["LMM_CORE_TOOL_MARKET_FIXED_WITNESS_ROLE"],
            "reviewed-host"
        );
        assert!(settings.contains_key("LMM_CORE_TOOL_MARKET_RECEIPT_ADAPTERS_CONFIG_FILE"));
    }
    #[test]
    fn assistant_and_tool_market_have_explicit_private_configuration() {
        for section in [
            "[assistant]\nconfig_file='/missing-assistant'\ntoken_file='/missing-token'\n",
            "[tool_market]\nsocket='/missing-tool.sock'\ntoken_file='/missing-token'\n",
        ] {
            assert!(parse_core_file(format!("version=1\n{section}").as_bytes()).is_err());
            let document = format!("version=1\n[database]\nurl_file='/missing-db'\n{section}");
            let settings = parse_core_file(document.as_bytes()).unwrap();
            assert!(
                settings.contains_key(if section.starts_with("[assistant]") {
                    "LMM_CORE_ASSISTANT_TOKEN_FILE"
                } else {
                    "LMM_CORE_TOOL_MARKET_TOKEN_FILE"
                })
            );
        }
        assert!(parse_core_file(b"version=1\n[database]\nurl_file='/missing-db'\n[tool_market]\nsocket='/missing.sock'\ntoken_file='/missing-token'\nshared_group_id=0\n").is_err());
        assert!(parse_core_file(b"version=1\n[database]\nurl_file='/missing-db'\n[assistant]\nconfig_file='/missing-policy'\ntoken='secret-literal'\n").is_err());
    }
    #[test]
    fn typed_toml_is_versioned_bounded_and_does_not_accept_secret_literals() {
        let minimal = parse_core_file(b"version=1\n").unwrap();
        assert_eq!(minimal["LMM_CORE_LISTEN"], "127.0.0.1:8080");
        assert_eq!(minimal["LMM_CORE_DRAIN_TIMEOUT_SECONDS"], "30");
        assert!(!minimal.contains_key("LMM_CORE_DATABASE_URL"));
        for document in [
            b"".as_slice(), b"version=2\n", b"version=1\nversion=1\n",
            b"version=1\nunknown='do-not-print-this'\n", b"version=1\n[database]\nurl='secret'\n",
            b"version=1\n[server]\nworker_threads=0\n", b"version=1\n[server]\ndrain_timeout_seconds=301\n",
            b"version=1\n[identity.web]\norigin='http://api.lmm.local'\nrefresh_key_file='/missing-key'\n",
            b"version=1\n[identity]\napi_key_encryption_key_file='relative-key'\n",
            b"version=1\n[identity]\nregistration_enabled=true\n",
            b"version=1\n[identity.google]\nclient_id='id'\nclient_secret_file='/missing-key'\nredirect_uri='https://example.test/core/v1/auth/oauth/google/callback'\n",
            b"version=1\n[rpc]\nsocket='/tmp/../core.sock'\ntoken_file='/token'\n",
            b"version=1\n[rpc]\nsocket='/tmp/./core.sock'\ntoken_file='/token'\n",
            b"version=1\n[rpc]\nsocket='/core.sock'\ntoken_file='/token'\nshared_group_id=0\n",
            b"version=1\n\xff",
        ] { let error = parse_core_file(document).unwrap_err().to_string(); assert!(!error.contains("do-not-print-this")); }
    }

    #[test]
    fn typed_toml_maps_all_supported_capabilities_without_environment_mutation() {
        let document = b"version=1\n[server]\nlisten='127.0.0.1:8000'\nworker_threads=2\nmax_inflight=8\ndrain_timeout_seconds=15\n[database]\nurl_file='/nonexistent-lmm-test/database'\nmax_connections=3\nacquire_timeout_seconds=7\ncheck_timeout_seconds=60\n[identity]\nregistration_enabled=true\npassword_slots=2\napi_key_encryption_key_file='/nonexistent-lmm-test/api-key'\n[identity.web]\norigin='https://api.example.test'\nrefresh_key_file='/nonexistent-lmm-test/refresh'\nuser_agreement_enabled=false\nprivacy_policy_enabled=true\n[identity.google]\nclient_id='public-client-id'\nclient_secret_file='/nonexistent-lmm-test/google'\nredirect_uri='https://api.example.test/core/v1/auth/oauth/google/callback'\n[rpc]\nsocket='/nonexistent-lmm-test/core.sock'\ntoken_file='/nonexistent-lmm-test/rpc'\nshared_group_id=65534\n[console]\nenabled=true\n[model]\nconfig_file='/nonexistent-lmm-test/model.toml'\nlocal_http=true\n[payments]\nconfig_file='/nonexistent-lmm-test/payments.toml'\ntoken_file='/nonexistent-lmm-test/payments-token'\n[commerce]\nconfig_file='/nonexistent-lmm-test/commerce.toml'\ntoken_file='/nonexistent-lmm-test/commerce-token'\n[ingress]\nack_url='http://127.0.0.1:3000/__ack'\ntoken_file='/nonexistent-lmm-test/ingress'\ninstance='core-test'\n";
        let parsed = parse_core_file(document).unwrap();
        assert_eq!(parsed.len(), 33);
        assert_eq!(parsed["LMM_CORE_HUMANGRANT_ENABLED"], "false");
        for (key, expected) in [
            ("LMM_CORE_WORKER_THREADS", "2"),
            ("LMM_CORE_DRAIN_TIMEOUT_SECONDS", "15"),
            ("LMM_CORE_DB_CONNECTIONS", "3"),
            ("LMM_CORE_DB_ACQUIRE_TIMEOUT_SECONDS", "7"),
            ("LMM_CORE_DB_CHECK_TIMEOUT_SECONDS", "60"),
            ("LMM_CORE_PASSWORD_SLOTS", "2"),
            (
                "LMM_CORE_GOOGLE_CLIENT_SECRET_FILE",
                "/nonexistent-lmm-test/google",
            ),
            (
                "LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE",
                "/nonexistent-lmm-test/api-key",
            ),
            ("LMM_CORE_RPC_SHARED_GROUP_ID", "65534"),
            ("LMM_CORE_INSTANCE", "core-test"),
        ] {
            assert_eq!(parsed[key], expected);
        }
        assert!(!parsed.contains_key("LMM_CORE_GOOGLE_CLIENT_SECRET"));
    }

    #[tokio::test]
    async fn deployment_google_legacy_frontend_callback_preserves_exact_registered_uri() {
        let parsed = parse_core_file(
            b"version=1\n[database]\nurl_file='/missing-db'\n[identity.web]\norigin='https://example.test'\nrefresh_key_file='/missing-refresh'\n[identity.google]\nclient_id='public-test-client'\nclient_secret_file='/missing-google'\nredirect_uri='https://example.test/oauth/google'\n",
        )
        .unwrap();
        assert_eq!(
            parsed["LMM_CORE_GOOGLE_REDIRECT_URI"],
            "https://example.test/oauth/google"
        );
        assert_eq!(
            parsed["LMM_CORE_GOOGLE_CLIENT_SECRET_FILE"],
            "/missing-google"
        );
        let google = crate::identity::GoogleOAuth::new(
            parsed["LMM_CORE_GOOGLE_CLIENT_ID"].clone(),
            "test-only-client-secret".into(),
            parsed["LMM_CORE_GOOGLE_REDIRECT_URI"].clone(),
        )
        .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .min_connections(0)
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let store = crate::identity::IdentityStore::from_pool(pool).with_google_oauth(google);
        let web =
            crate::legacy_auth::LegacyAuthConfig::new(&parsed["LMM_CORE_WEB_ORIGIN"], &[43; 32])
                .unwrap();
        assert_eq!(web.google_browser_callback(&store), Some("/oauth/google"));
        assert_eq!(web.google_client_id(&store), Some("public-test-client"));
        assert!(!store.google_native_enabled());
        let foreign =
            crate::legacy_auth::LegacyAuthConfig::new("https://foreign.test", &[43; 32]).unwrap();
        assert!(foreign.google_browser_callback(&store).is_none());
        assert!(foreign.google_client_id(&store).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_toml_allows_readable_templates_but_rejects_writable_and_symlink_paths() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = std::env::temp_dir().join(format!(
            "lmm-toml-path-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("core.toml");
        fs::write(&path, b"version=1\n").unwrap();
        for mode in [0o400, 0o600, 0o640, 0o644] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(configuration_file(&path, 256).is_ok());
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
        assert!(configuration_file(&path, 256).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let linked = directory.with_extension("link");
        symlink(&directory, &linked).unwrap();
        assert!(configuration_file(&linked.join("core.toml"), 256).is_err());
        fs::remove_file(linked).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn financial_configuration_cannot_reuse_identity_or_sibling_authority() {
        let control = b"control-credential-only";
        let payment = b"payment-credential-only";
        let commerce = b"commerce-credential-only";
        assert!(distinct_financial_credentials(control, None, None).is_ok());
        assert!(distinct_financial_credentials(control, Some(payment), Some(commerce)).is_ok());
        for (a, b) in [
            (Some(control.as_slice()), None),
            (None, Some(control.as_slice())),
            (Some(payment.as_slice()), Some(payment.as_slice())),
        ] {
            assert!(distinct_financial_credentials(control, a, b).is_err());
        }
        let permissions: PaymentPermissions = parse_document(b"").unwrap();
        assert!(
            !permissions.prepare
                && !permissions.credit_evidence
                && !permissions.refund_user
                && !permissions.refund_evidence
                && !permissions.read_user
                && !permissions.reconcile
                && !permissions.proof_ingress
                && !permissions.proof_read
        );
        assert!(parse_document::<CommerceConfig>(b"version=1\ntransfer=true\n").is_err());
        assert!(parse_document::<PaymentConfig>(b"version=1\nprovider='stripe-test'\nchannels=[]\n[permissions]\nrelease_refund=true\n").is_err());
    }
    #[test]
    fn isolated_oauth2_requires_explicit_profile_private_registry_and_exact_listener() {
        let document = "version=1\n[server]\nlisten='127.0.0.1:54321'\n[database]\nurl_file='/nonexistent-oauth/db'\n[identity.web]\norigin='http://127.0.0.1:54321'\nrefresh_key_file='/nonexistent-oauth/refresh'\nlocal_http=true\nuser_agreement_enabled=false\nprivacy_policy_enabled=false\n[identity.oauth2]\nissuer='http://127.0.0.1:54321'\nisolated_loopback_test=true\nisolated_secret_directory='/nonexistent-oauth/isolated'\ngroups=['default']\naccess_ttl_seconds=900\nfamily_ttl_seconds=86400\n[model]\nconfig_file='/nonexistent-oauth/model'\nlocal_http=true\n";
        let values = parse_core_file(document.as_bytes()).unwrap();
        assert_eq!(values["LMM_CORE_OAUTH2_ISOLATED_LOOPBACK_TEST"], "true");
        let proxy = document
            .replace(
                "isolated_loopback_test=true",
                "isolated_loopback_test=true\nisolated_loopback_proxy=true",
            )
            .replace("listen='127.0.0.1:54321'", "listen='127.0.0.1:54322'");
        assert!(parse_core_file(proxy.as_bytes()).is_ok());
        for bad in [
            proxy.replace("listen='127.0.0.1:54322'", "listen='0.0.0.0:54322'"),
            proxy.replace("listen='127.0.0.1:54322'", "listen='127.0.0.1:50837'"),
            proxy.replace("listen='127.0.0.1:54322'", "listen='127.0.0.1:18080'"),
            proxy.replace("listen='127.0.0.1:54322'", "listen='[::1]:54322'"),
        ] {
            assert!(parse_core_file(bad.as_bytes()).is_err());
        }

        for invalid in [
            document.replace(
                "isolated_loopback_test=true",
                "isolated_loopback_test=false",
            ),
            document.replace(
                "isolated_secret_directory='/nonexistent-oauth/isolated'\n",
                "",
            ),
            document.replace("listen='127.0.0.1:54321'", "listen='0.0.0.0:54321'"),
            document.replace("listen='127.0.0.1:54321'", "listen='127.0.0.1:54322'"),
            document.replace("http://127.0.0.1:54321", "http://localhost:54321"),
            document.replace("54321", "50837"),
            document.replace("local_http=true", "local_http=false"),
        ] {
            assert!(parse_core_file(invalid.as_bytes()).is_err());
        }
    }

    #[test]
    fn oauth2_requires_exact_https_web_origin_groups_and_explicit_lifetimes() {
        let common = "version=1\n[database]\nurl_file='/nonexistent-oauth/database'\n[identity.web]\norigin='https://issuer.example.test'\nrefresh_key_file='/nonexistent-oauth/refresh'\nuser_agreement_enabled=true\nprivacy_policy_enabled=true\n";
        let oauth = "[identity.oauth2]\nissuer='https://issuer.example.test'\ngroups=['default']\naccess_ttl_seconds=900\nfamily_ttl_seconds=86400\n";
        let values = parse_core_file(format!("{common}{oauth}").as_bytes()).unwrap();
        assert_eq!(
            values.get("LMM_CORE_OAUTH2_ISSUER").unwrap(),
            "https://issuer.example.test"
        );
        for bad in [
            oauth.replace(
                "issuer='https://issuer.example.test'",
                "issuer='https://issuer.example.test'\nisolated_loopback_proxy=false",
            ),
            oauth.replace("https://issuer.example.test", "https://other.example.test"),
            oauth.replace("https://issuer.example.test", "http://issuer.example.test"),
            oauth.replace("groups=['default']", "groups=[]"),
            oauth.replace("groups=['default']", "groups=['default','default']"),
            oauth.replace("access_ttl_seconds=900", "access_ttl_seconds=0"),
            oauth.replace("family_ttl_seconds=86400", "family_ttl_seconds=31536001"),
            oauth.replace("access_ttl_seconds=900\n", ""),
        ] {
            assert!(parse_core_file(format!("{common}{bad}").as_bytes()).is_err());
        }
        assert!(
            parse_core_file(
                format!("{common}{oauth}")
                    .replace(
                        "origin='https://issuer.example.test'",
                        "origin='https://issuer.example.test'\nlocal_http=true"
                    )
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_core_file(
                format!("version=1\n[database]\nurl_file='/nonexistent-oauth/database'\n{oauth}")
                    .as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn database_acquire_timeout_defaults_and_bounds_are_explicit() {
        const NAME: &str = "LMM_CORE_DB_ACQUIRE_TIMEOUT_SECONDS";
        let base = "version=1\n[database]\nurl_file='/nonexistent-lmm-test/database'\n";
        let default = parse_core_file(base.as_bytes()).unwrap();
        assert_eq!(default[NAME], "3");
        assert_eq!(parse_limit(NAME, None, 3, 30).unwrap(), 3);
        for seconds in [1, 3, 30] {
            let document = format!("{base}acquire_timeout_seconds={seconds}\n");
            assert_eq!(
                parse_core_file(document.as_bytes()).unwrap()[NAME],
                OsString::from(seconds.to_string())
            );
            assert_eq!(
                parse_limit(NAME, Some(&seconds.to_string()), 3, 30).unwrap(),
                seconds
            );
        }
        for value in ["0", "31", "-1", "1.5", "'3'", "4294967296"] {
            let document = format!("{base}acquire_timeout_seconds={value}\n");
            assert!(
                parse_core_file(document.as_bytes()).is_err(),
                "accepted {value}"
            );
            assert!(
                parse_limit(NAME, Some(value), 3, 30).is_err(),
                "accepted legacy {value}"
            );
        }
        assert!(!parse_core_file(b"version=1\n").unwrap().contains_key(NAME));
    }

    #[test]
    fn database_check_timeout_defaults_and_bounds_are_explicit() {
        const NAME: &str = "LMM_CORE_DB_CHECK_TIMEOUT_SECONDS";
        let base = "version=1\n[database]\nurl_file='/nonexistent-lmm-test/database'\n";
        let default = parse_core_file(base.as_bytes()).unwrap();
        assert_eq!(default[NAME], "15");
        assert_eq!(parse_limit(NAME, None, 15, 120).unwrap(), 15);
        for seconds in [1, 15, 120] {
            let document = format!("{base}check_timeout_seconds={seconds}\n");
            assert_eq!(
                parse_core_file(document.as_bytes()).unwrap()[NAME],
                OsString::from(seconds.to_string())
            );
            assert_eq!(
                parse_limit(NAME, Some(&seconds.to_string()), 15, 120).unwrap(),
                seconds
            );
        }
        for value in ["0", "121", "-1", "1.5", "'15'", "4294967296"] {
            let document = format!("{base}check_timeout_seconds={value}\n");
            assert!(
                parse_core_file(document.as_bytes()).is_err(),
                "accepted {value}"
            );
            assert!(
                parse_limit(NAME, Some(value), 15, 120).is_err(),
                "accepted {value}"
            );
        }
        assert!(!parse_core_file(b"version=1\n").unwrap().contains_key(NAME));
    }

    #[test]
    fn resource_limits_reject_invalid_or_unbounded_settings() {
        assert_eq!(parse_limit("TEST_LIMIT", None, 2, 8).unwrap(), 2);
        assert_eq!(parse_limit("TEST_LIMIT", Some("1"), 2, 8).unwrap(), 1);
        for invalid in ["", "0", "9", "-1", "1.5", "not-a-number"] {
            assert!(parse_limit("TEST_LIMIT", Some(invalid), 2, 8).is_err());
        }
    }

    #[test]
    fn assistant_tools_are_a_separate_explicit_opt_in() {
        let absent = parse_core_file(b"version=1\n").unwrap();
        assert!(!absent.contains_key("LMM_CORE_ASSISTANT_TOOLS_ENABLED"));
        let ordinary=parse_core_file(b"version=1\n[database]\nurl_file='/missing-db'\n[assistant]\nconfig_file='/tmp/assistant.toml'\ntoken_file='/tmp/assistant-token'\n").unwrap();
        assert_eq!(ordinary["LMM_CORE_ASSISTANT_TOOLS_ENABLED"], "false");
        let opted=parse_core_file(b"version=1\n[database]\nurl_file='/missing-db'\n[assistant]\nconfig_file='/tmp/assistant.toml'\ntoken_file='/tmp/assistant-token'\ntools_enabled=true\n").unwrap();
        assert_eq!(opted["LMM_CORE_ASSISTANT_TOOLS_ENABLED"], "true");
        for value in ["'true'", "1"] {
            let doc = format!(
                "version=1\n[database]\nurl_file='/missing-db'\n[assistant]\nconfig_file='/tmp/assistant.toml'\ntoken_file='/tmp/assistant-token'\ntools_enabled={value}\n"
            );
            assert!(parse_core_file(doc.as_bytes()).is_err());
        }
        assert!(parse_core_file(b"version=1\n[payments]\nconfig_file='/tmp/payments.toml'\ntoken_file='/tmp/payments-token'\ntools_enabled=true\n").is_err());
    }
    #[test]
    fn database_configuration_is_explicit_and_fails_closed() {
        assert_eq!(read_database_url(None, None).unwrap(), None);
        assert!(read_database_url(Some(" ".into()), None).is_err());
        assert!(read_database_url(Some("postgres://test".into()), Some("unused".into())).is_err());
        assert_eq!(
            read_database_url(Some("postgres://test\n".into()), None)
                .unwrap()
                .as_deref(),
            Some("postgres://test")
        );
    }

    #[test]
    fn web_legal_publication_must_be_explicit_and_unambiguous() {
        assert_eq!(
            explicit_legal_policy(Some("false"), Some("false")).unwrap(),
            (false, false)
        );
        assert_eq!(
            explicit_legal_policy(Some("true"), Some("false")).unwrap(),
            (true, false)
        );
        for (agreement, privacy) in [
            (None, Some("false")),
            (Some("true"), None),
            (Some("1"), Some("false")),
            (Some("false"), Some("FALSE")),
        ] {
            assert!(explicit_legal_policy(agreement, privacy).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_configuration_rejects_public_symlink_and_oversized_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = std::env::temp_dir().join(format!(
            "lmm-runtime-config-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("secret");
        fs::write(&path, b"test-value").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(private_file(&path, 128).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(private_file(&path, 128).unwrap(), b"test-value");
        assert!(private_file(&path, 4).is_err());
        let link = directory.join("link");
        symlink(&path, &link).unwrap();
        assert!(private_file(&link, 128).is_err());
        assert!(private_file(Path::new("relative"), 128).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn moderation_credentials_are_opt_in_and_require_database_configuration() {
        let parsed = parse_core_file(b"version=1\n").unwrap();
        assert!(!parsed.contains_key("LMM_CORE_ABUSE_CREDENTIAL_CONFIG_FILE"));
        assert!(!parsed.contains_key("LMM_CORE_MODEL_CONFIG_FILE"));
        let sidecar =
            b"version=1\n[abuse]\ncredential_config_file='/nonexistent-lmm-test/abuse.toml'\n";
        assert!(parse_core_file(sidecar).is_err());
        let configured = b"version=1\n[database]\nurl_file='/nonexistent-lmm-test/db'\n[abuse]\ncredential_config_file='/nonexistent-lmm-test/abuse.toml'\n";
        let parsed = parse_core_file(configured).unwrap();
        assert_eq!(
            parsed["LMM_CORE_ABUSE_CREDENTIAL_CONFIG_FILE"],
            "/nonexistent-lmm-test/abuse.toml"
        );
        assert!(!parsed.contains_key("LMM_CORE_MODEL_CONFIG_FILE"));
        assert!(
            abuse_credentials(b"version=1\ncredentials=[]\n")
                .unwrap()
                .secrets
                .is_empty()
        );
        assert!(abuse_credentials(b"version=2\ncredentials=[]\n").is_err());
        let error =
            abuse_credentials(b"version=1\ncredentials=[]\nsecret='never-return-this-marker'\n")
                .err()
                .unwrap();
        assert!(!error.to_string().contains("never-return-this-marker"));
        for alias in ["", "bad alias", "bad/alias", "..\n"] {
            let document = format!(
                "version=1\n[[credentials]]\nalias={alias:?}\nsecret_file='/nonexistent-lmm-test/secret'\nallowed_origins=['https://isolated.example']\n"
            );
            assert!(abuse_credentials(document.as_bytes()).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn isolated_moderation_credentials_enforce_private_files_and_unique_aliases() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = std::env::temp_dir().join(format!(
            "lmm-abuse-config-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&directory).unwrap();
        let secret = directory.join("upstream-secret");
        fs::write(&secret, b"synthetic-moderation-secret\n").unwrap();
        let row = format!(
            "[[credentials]]\nalias='isolated-1'\nsecret_file={:?}\nallowed_origins=['https://isolated.example']\n",
            secret.to_str().unwrap()
        );
        let document = format!("version=1\n{row}");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(abuse_credentials(document.as_bytes()).is_err());
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
        let registry = abuse_credentials(document.as_bytes()).unwrap();
        assert_eq!(
            registry.secrets.keys().collect::<Vec<_>>(),
            vec!["isolated-1"]
        );
        assert_eq!(
            registry.secrets["isolated-1"],
            "synthetic-moderation-secret"
        );
        assert!(abuse_credentials(format!("version=1\n{row}{row}").as_bytes()).is_err());
        let link = directory.join("linked");
        symlink(&secret, &link).unwrap();
        let linked = document.replace(&secret.display().to_string(), &link.display().to_string());
        assert!(abuse_credentials(linked.as_bytes()).is_err());
        fs::write(&secret, b"not-a-valid\ncredential").unwrap();
        assert!(abuse_credentials(document.as_bytes()).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_registry_rejects_normal_provider_file_hardlink_and_normalized_value() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "lmm-abuse-independent-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&directory).unwrap();
        let normal_file = directory.join("normal-provider");
        fs::write(&normal_file, b"normal-synthetic-key\n").unwrap();
        fs::set_permissions(&normal_file, fs::Permissions::from_mode(0o600)).unwrap();
        // Reuse the repository's real loopback ModelConfig shape. Construction
        // uses a lazy isolated pool and makes no model/database/network call.
        let document = format!(
            r#"version=1
input_token_ceiling=100
lease_seconds=20
timeout_seconds=3
max_output_tokens=20
local_streaming=true
credentials=[{{reference={{id=7,revision=1}},secret_file='{}'}}]
key_groups=[{{key_id=1,group='allowed'}}]
[routing]
version=1
groups=[{{name='allowed',enabled=true}}]
upstreams=[{{id=1,endpoint='{}',protocol='openai_chat',credential={{id=7,revision=1}},enabled=true}}]
routes=[{{id=1,model='local-model',group='allowed',enabled=true,speeds=['standard'],targets=[{{id=1,upstream_id=1,upstream_model='provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}}],failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}}}]
aliases=[]
[prices]
version=1
rules=[{{id=1,model='local-model',group='allowed',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}]
"#,
            normal_file.display(),
            "http://127.0.0.1:9/custom/v1"
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let (normal, origins) =
            model_gateway_with_origins(pool, document.as_bytes(), true).unwrap();
        assert!(normal.contains_credential("normal-synthetic-key"));
        let isolated = |path: &Path| {
            abuse_credentials(
                format!(
                    "version=1\n[[credentials]]\nalias='isolated'\nsecret_file={:?}\nallowed_origins=['https://isolated.example']\n",
                    path.to_str().unwrap()
                )
                .as_bytes(),
            )
            .unwrap()
        };
        let same_file = isolated(&normal_file);
        assert!(same_file.origins[0].same_file(&origins[0]));
        let hardlink = directory.join("separate-name");
        fs::hard_link(&normal_file, &hardlink).unwrap();
        let same_inode = isolated(&hardlink);
        assert_ne!(
            same_inode.origins[0].canonical_file,
            origins[0].canonical_file
        );
        assert!(same_inode.origins[0].same_file(&origins[0]));
        let same_value_file = directory.join("copied-value");
        fs::write(&same_value_file, b"  normal-synthetic-key  \n").unwrap();
        fs::set_permissions(&same_value_file, fs::Permissions::from_mode(0o600)).unwrap();
        let same_value = isolated(&same_value_file);
        assert!(!same_value.origins[0].same_file(&origins[0]));
        for registry in [same_file, same_inode, same_value] {
            let error = reject_credential_reuse(&registry, Some(&normal), &origins).unwrap_err();
            assert_eq!(
                error.to_string(),
                "isolated moderation credentials must be independent"
            );
        }
        let independent_file = directory.join("independent-value");
        fs::write(&independent_file, b"isolated-synthetic-key\n").unwrap();
        fs::set_permissions(&independent_file, fs::Permissions::from_mode(0o600)).unwrap();
        let independent = isolated(&independent_file);
        assert!(!normal.contains_credential(&independent.secrets["isolated"]));
        assert!(reject_credential_reuse(&independent, Some(&normal), &origins).is_ok());
        let empty = AbuseCredentialRegistry::default();
        assert!(reject_credential_reuse(&empty, Some(&normal), &origins).is_ok());
        assert!(reject_credential_reuse(&independent, None, &[]).is_ok());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn isolated_network_local_mode_is_separate_and_defaults_false() {
        let doc = b"version=1\n[database]\nurl_file='/nonexistent-lmm-test/db'\n[abuse]\ncredential_config_file='/nonexistent-lmm-test/abuse.toml'\n";
        let parsed = parse_core_file(doc).unwrap();
        assert_eq!(parsed["LMM_CORE_ABUSE_LOCAL_HTTP"], "false");
        assert!(!parsed.contains_key("LMM_CORE_MODEL_LOCAL_HTTP"));
        let local = [doc.as_slice(), b"local_http=true\n"].concat();
        assert_eq!(
            parse_core_file(&local).unwrap()["LMM_CORE_ABUSE_LOCAL_HTTP"],
            "true"
        );
        let invalid = [doc.as_slice(), b"insecure=true\n"].concat();
        assert!(parse_core_file(&invalid).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn isolated_origins_are_required_canonical_bounded_and_local_only_by_opt_in() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "lmm-origin-config-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&root).unwrap();
        let secret = root.join("secret");
        fs::write(&secret, b"isolated-origin-synthetic-key").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
        let prefix = format!(
            "version=1\n[[credentials]]\nalias='isolated'\nsecret_file={:?}\n",
            secret.to_str().unwrap()
        );
        assert!(abuse_credentials(prefix.as_bytes()).is_err());
        for origins in [
            "[]",
            "['https://allowed.example/path']",
            "['https://user@allowed.example']",
            "['https://allowed.example?secret=marker']",
            "['https://allowed.example#marker']",
            "['http://allowed.example']",
            "['https://127.0.0.1']",
            "['http://127.0.0.1:9443']",
            "['https://allowed.example','https://ALLOWED.example:443/']",
        ] {
            let doc = format!("{prefix}allowed_origins={origins}\n");
            let error = abuse_credentials(doc.as_bytes()).err().unwrap();
            assert!(!error.to_string().contains("marker"));
        }
        let too_many = (0..17)
            .map(|n| format!("'https://h{n}.example'"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            abuse_credentials(format!("{prefix}allowed_origins=[{too_many}]\n").as_bytes())
                .is_err()
        );
        let secure = format!(
            "{prefix}allowed_origins=['https://ALLOWED.example:443/','https://other.example:8443']\n"
        );
        let registry = abuse_credentials(secure.as_bytes()).unwrap();
        assert_eq!(
            registry.allowed_origins["isolated"],
            BTreeSet::from([
                "https://allowed.example".to_owned(),
                "https://other.example:8443".to_owned()
            ])
        );
        assert!(registry.additional_roots.is_empty());
        let local = format!("{prefix}allowed_origins=['http://127.0.0.1:9443']\n");
        assert!(abuse_credentials(local.as_bytes()).is_err());
        assert!(abuse_credentials_for_network(local.as_bytes(), true).is_ok());
        assert!(
            abuse_credentials_for_network(
                format!("{prefix}allowed_origins=['http://localhost:9443']\n").as_bytes(),
                true
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn isolated_ca_uses_private_file_parser_and_stays_alias_scoped() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        use std::process::{Command, Stdio};
        let root = std::env::temp_dir().join(format!(
            "lmm-origin-ca-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&root).unwrap();
        let secret = root.join("secret");
        fs::write(&secret, b"isolated-ca-synthetic-key").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
        let ca = root.join("ca.pem");
        let document = |path: &Path| {
            format!(
                "version=1\n[[credentials]]\nalias='with-ca'\nsecret_file={:?}\nallowed_origins=['https://allowed.example']\ntls_ca_file={:?}\n[[credentials]]\nalias='system-roots-only'\nsecret_file={:?}\nallowed_origins=['https://other.example']\n",
                secret.to_str().unwrap(),
                path.to_str().unwrap(),
                secret.to_str().unwrap()
            )
        };
        fs::write(&ca, b"not a certificate").unwrap();
        fs::set_permissions(&ca, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(abuse_credentials(document(&ca).as_bytes()).is_err());
        for invalid in [
            b"".as_slice(),
            b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----\n".as_slice(),
            b"-----BEGIN CERTIFICATE-----\nAA==\n-----END CERTIFICATE-----\n".as_slice(),
            b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n".as_slice(),
        ] {
            fs::write(&ca, invalid).unwrap();
            assert!(abuse_credentials(document(&ca).as_bytes()).is_err());
        }
        let status = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-keyout",
                "ca.key",
                "-out",
                "ca.pem",
                "-subj",
                "/CN=isolated-host-config-test",
            ])
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        fs::set_permissions(&ca, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(abuse_credentials(document(&ca).as_bytes()).is_err());
        fs::set_permissions(&ca, fs::Permissions::from_mode(0o600)).unwrap();
        let registry = abuse_credentials(document(&ca).as_bytes()).unwrap();
        assert_eq!(registry.additional_roots.len(), 1);
        assert!(registry.additional_roots.contains_key("with-ca"));
        assert!(!registry.additional_roots.contains_key("system-roots-only"));
        let valid = fs::read(&ca).unwrap();
        let mut bundle = valid.clone();
        bundle.extend_from_slice(&valid);
        fs::write(&ca, &bundle).unwrap();
        assert!(abuse_credentials(document(&ca).as_bytes()).is_ok());
        bundle.extend_from_slice(b"-----BEGIN CERTIFICATE-----\nAA==\n-----END CERTIFICATE-----\n");
        fs::write(&ca, bundle).unwrap();
        assert!(abuse_credentials(document(&ca).as_bytes()).is_err());
        fs::write(&ca, valid).unwrap();
        let link = root.join("link.pem");
        symlink(&ca, &link).unwrap();
        assert!(abuse_credentials(document(&link).as_bytes()).is_err());
        fs::write(&ca, vec![b'x'; 65_537]).unwrap();
        assert!(abuse_credentials(document(&ca).as_bytes()).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn model_provider_startup_requires_every_enabled_weighted_target_context() {
        use std::os::unix::fs::PermissionsExt;
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "model-config-fixture-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let secret = directory.join("synthetic-secret");
        fs::write(&secret, b"synthetic-provider-key\n").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
        let document = format!(
            r#"version=1
input_token_ceiling=100
lease_seconds=20
timeout_seconds=3
max_output_tokens=20
credentials=[{{reference={{id=7,revision=1}},secret_file={:?}}}]
key_groups=[{{key_id=1,group='allowed'}}]
external_models=[{{upstream_id=1,upstream_model='provider-model',protocol='openai_chat',endpoint='https://provider.example/v1',max_input_tokens=64,max_output_tokens=20,contract_revision='synthetic-contract-v1'}}]
[routing]
version=1
groups=[{{name='allowed',enabled=true}}]
upstreams=[{{id=1,endpoint='https://provider.example/v1',protocol='openai_chat',credential={{id=7,revision=1}},enabled=true}}]
routes=[{{id=1,model='model',group='allowed',enabled=true,speeds=['standard'],targets=[{{id=1,upstream_id=1,upstream_model='provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}}],failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}}}]
aliases=[]
[prices]
version=1
rules=[{{id=1,model='model',group='allowed',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}]
"#,
            secret.to_str().unwrap()
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1:1/synthetic")
            .unwrap();
        assert!(model_gateway(pool.clone(), document.as_bytes(), false).is_ok());
        // The installation's output maximum need not fit every model. Startup
        // binds both contracts; runtime checks the output actually requested.
        let low_output = document.replace(
            "max_output_tokens=20,contract_revision",
            "max_output_tokens=19,contract_revision",
        );
        let mixed = low_output
            .replacen(
                "external_models=[",
                "external_models=[{upstream_id=1,upstream_model='second-provider-model',protocol='openai_chat',endpoint='https://provider.example/v1',max_input_tokens=64,max_output_tokens=20,contract_revision='synthetic-contract-v1'},",
                1,
            )
            .replacen(
                "routes=[",
                concat!(
                    "routes=[{id=2,model='second-model',group='allowed',enabled=true,speeds=['standard'],",
                    "targets=[{id=2,upstream_id=1,upstream_model='second-provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}],",
                    "failover={max_attempts=1,retry_on=[],allow_replay_after_send=false}},",
                ),
                1,
            )
            .replacen(
                "rules=[",
                concat!(
                    "rules=[{id=2,model='second-model',group='allowed',speed='standard',",
                    "rates={input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0},",
                    "group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000},",
                ),
                1,
            );
        let mixed_gateway = model_gateway(pool.clone(), mixed.as_bytes(), false).unwrap();
        assert!(
            mixed_gateway
                .validate_assistant_policy("model", "allowed", 19)
                .is_ok()
        );
        assert!(
            mixed_gateway
                .validate_assistant_policy("model", "allowed", 20)
                .is_err()
        );
        assert!(
            mixed_gateway
                .validate_assistant_policy("second-model", "allowed", 20)
                .is_ok()
        );
        // Native API keys are created and revoked in the identity database.
        // A host need not predeclare their IDs in the synthetic access map.
        let dynamic_keys =
            document.replace("key_groups=[{key_id=1,group='allowed'}]", "key_groups=[]");
        let dynamic_gateway = model_gateway(pool.clone(), dynamic_keys.as_bytes(), false).unwrap();
        let _router = dynamic_gateway.router();
        // Empty static access never bypasses provider binding validation.
        assert!(
            model_gateway(
                pool.clone(),
                dynamic_keys
                    .replace("max_input_tokens=64", "max_input_tokens=101")
                    .as_bytes(),
                false,
            )
            .is_err()
        );
        // Explicit local mode can never consume an external provider contract.
        assert!(model_gateway(pool.clone(), document.as_bytes(), true).is_err());
        for invalid in [
            document.replace(
                "upstream_model='provider-model',protocol=",
                "upstream_model='other-model',protocol=",
            ),
            document.replace(
                "protocol='openai_chat',endpoint=",
                "protocol='openai_responses',endpoint=",
            ),
            document.replace(
                "endpoint='https://provider.example/v1',max_input_tokens",
                "endpoint='https://other.example/v1',max_input_tokens",
            ),
            document.replace("max_input_tokens=64", "max_input_tokens=101"),
            document.replace(
                "max_output_tokens=20,contract_revision",
                "max_output_tokens=0,contract_revision",
            ),
        ] {
            assert!(model_gateway(pool.clone(), invalid.as_bytes(), false).is_err());
        }
        // A second weighted fallback must also be bound, even if the first one
        // is fully usable. Reject before loading its deliberately absent secret.
        let second = "{id=2,upstream_id=2,upstream_model='missing-context',enabled=true,priority=1,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}";
        let unbound =
            document.replace("timeout_ms=5000}]", &format!("timeout_ms=5000}},{second}]"))
                .replace("enabled=true}]\nroutes=", "enabled=true},{id=2,endpoint='https://provider.example/v1',protocol='openai_chat',credential={id=7,revision=1},enabled=true}]\nroutes=");
        assert!(
            model_gateway(pool.clone(), unbound.as_bytes(), false)
                .err()
                .unwrap()
                .to_string()
                .contains("context binding")
        );
        let disabled = unbound.replace(
            "upstream_model='missing-context',enabled=true",
            "upstream_model='missing-context',enabled=false",
        );
        assert!(model_gateway(pool.clone(), disabled.as_bytes(), false).is_ok());
        let zero_weight = unbound.replace("priority=1,weight=1", "priority=1,weight=0");
        assert!(model_gateway(pool, zero_weight.as_bytes(), false).is_ok());
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn model_configuration_rejects_malformed_documents_and_empty_grants() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1:1/synthetic")
            .unwrap();
        assert!(
            model_gateway(pool.clone(), b"do-not-log-this-secret", false)
                .err()
                .unwrap()
                .to_string()
                .contains("invalid model configuration document")
        );
        let empty = b"version=1\ncredentials=[]\nkey_groups=[]\ninput_token_ceiling=1024\nlease_seconds=90\ntimeout_seconds=60\nmax_output_tokens=4096\n[routing]\nversion=1\ngroups=[]\nupstreams=[]\nroutes=[]\naliases=[]\n[prices]\nversion=1\nrules=[]\n";
        assert!(model_gateway(pool.clone(), empty, true).is_err());
        assert!(
            !model_gateway(pool, b"do-not-log-this-secret", true)
                .err()
                .unwrap()
                .to_string()
                .contains("do-not-log-this-secret")
        );
    }
}

/// Explicit optional HumanGrant assembly with the exact configured Web origin.
pub async fn humangrant(
    pool: sqlx::PgPool,
) -> io::Result<Option<(axum::Router, Arc<crate::humangrant::HumanGrantEngine>)>> {
    let enabled = match setting("LMM_CORE_HUMANGRANT_ENABLED").as_deref() {
        Ok("true") => true,
        Ok("false") | Err(_) => false,
        _ => return Err(invalid_config("invalid HumanGrant enabled flag")),
    };
    if !enabled {
        if setting_os("LMM_CORE_HUMANGRANT_SMTP_FILE").is_some() {
            return Err(invalid_config(
                "HumanGrant SMTP requires HumanGrant enabled",
            ));
        }
        return Ok(None);
    }
    let origin = setting("LMM_CORE_WEB_ORIGIN")
        .map_err(|_| invalid_config("HumanGrant requires Web authentication"))?;
    let local = setting("LMM_CORE_WEB_LOCAL_HTTP").as_deref() == Ok("true");
    let smtp: Option<crate::humangrant::mail::SmtpConfig> =
        setting_os("LMM_CORE_HUMANGRANT_SMTP_FILE")
            .map(|path| parse_document(&private_file(Path::new(&path), 16 * 1024)?))
            .transpose()?;
    crate::humangrant::runtime::open_with_mail(pool, &origin, local, smtp.as_ref())
        .await
        .map(Some)
}
