//! Explicit market issuer startup configuration. No database or token issuance here.
use super::*;
use crate::market_oauth::{MarketIssuer, MarketProfile};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MarketOAuthFile {
    pub origin: String,
    pub resource: Option<String>,
    pub max_budget_credits: i64,
    #[serde(default)]
    pub isolated_loopback_test: bool,
    pub isolated_secret_directory: Option<PathBuf>,
}
pub(super) fn parse(
    file: MarketOAuthFile,
    web: &WebFile,
    listen: &str,
    has_private_market: bool,
) -> io::Result<Vec<(&'static str, String)>> {
    if !has_private_market
        || file.origin != web.origin
        || web.local_http != file.isolated_loopback_test
    {
        return Err(invalid_config(
            "market OAuth requires exact Web origin, explicit transport and private market service",
        ));
    }
    let mut values = vec![
        ("LMM_CORE_MARKET_OAUTH_ORIGIN", file.origin.clone()),
        (
            "LMM_CORE_MARKET_OAUTH_MAX_BUDGET_CREDITS",
            file.max_budget_credits.to_string(),
        ),
        (
            "LMM_CORE_MARKET_OAUTH_ISOLATED_LOOPBACK_TEST",
            file.isolated_loopback_test.to_string(),
        ),
    ];
    if let Some(resource) = &file.resource {
        values.push(("LMM_CORE_MARKET_OAUTH_RESOURCE", resource.clone()));
    }
    if file.isolated_loopback_test {
        validate_market_listener(&file.origin, listen)?;
        let directory = file.isolated_secret_directory.ok_or_else(|| {
            invalid_config("isolated market OAuth requires a private secret registry")
        })?;
        absolute_path(&directory)?;
        values.push((
            "LMM_CORE_MARKET_OAUTH_ISOLATED_SECRET_DIRECTORY",
            directory
                .to_str()
                .ok_or_else(|| invalid_config("invalid isolated market secret path"))?
                .to_owned(),
        ));
        MarketProfile::isolated_loopback_test(&file.origin, file.max_budget_credits)
    } else {
        if file.isolated_secret_directory.is_some() {
            return Err(invalid_config(
                "production market OAuth cannot use test settings",
            ));
        }
        MarketProfile::new(&file.origin, file.max_budget_credits)
    }
    .and_then(|p| match file.resource.as_deref() {
        Some(r) => p.with_resource(r),
        None => Ok(p),
    })
    .map_err(|_| invalid_config("invalid market OAuth profile"))?;
    Ok(values)
}
pub fn market_oauth_configured() -> bool {
    [
        "LMM_CORE_MARKET_OAUTH_ORIGIN",
        "LMM_CORE_MARKET_OAUTH_RESOURCE",
        "LMM_CORE_MARKET_OAUTH_MAX_BUDGET_CREDITS",
        "LMM_CORE_MARKET_OAUTH_ISOLATED_LOOPBACK_TEST",
        "LMM_CORE_MARKET_OAUTH_ISOLATED_SECRET_DIRECTORY",
    ]
    .iter()
    .any(|k| setting_os(k).is_some())
}
pub fn market_oauth_profile() -> io::Result<Option<MarketProfile>> {
    if !market_oauth_configured() {
        return Ok(None);
    }
    let required =
        |name| setting(name).map_err(|_| invalid_config("incomplete market OAuth profile"));
    let origin = required("LMM_CORE_MARKET_OAUTH_ORIGIN")?;
    let max = required("LMM_CORE_MARKET_OAUTH_MAX_BUDGET_CREDITS")?
        .parse::<i64>()
        .map_err(|_| invalid_config("invalid market OAuth budget ceiling"))?;
    let isolated = bool_setting("LMM_CORE_MARKET_OAUTH_ISOLATED_LOOPBACK_TEST")?;
    if required("LMM_CORE_WEB_ORIGIN")? != origin
        || setting_os("LMM_CORE_WEB_REFRESH_KEY_FILE").is_none()
        || !tool_market_configured()
        || bool_setting("LMM_CORE_WEB_LOCAL_HTTP")? != isolated
    {
        return Err(invalid_config(
            "market OAuth requires exact Web origin and private market service",
        ));
    }
    let profile = if isolated {
        validate_isolated(&origin)?;
        MarketProfile::isolated_loopback_test(&origin, max)
    } else {
        if setting_os("LMM_CORE_MARKET_OAUTH_ISOLATED_SECRET_DIRECTORY").is_some() {
            return Err(invalid_config(
                "production market OAuth cannot use test settings",
            ));
        }
        MarketProfile::new(&origin, max)
    }
    .and_then(|p| match setting_os("LMM_CORE_MARKET_OAUTH_RESOURCE") {
        Some(r) => r
            .to_str()
            .ok_or(crate::market_oauth::Error::Invalid)
            .and_then(|r| p.with_resource(r)),
        None => Ok(p),
    })
    .map_err(|_| invalid_config("invalid market OAuth profile"))?;
    Ok(Some(profile))
}
pub async fn market_oauth_issuer(pool: sqlx::PgPool) -> io::Result<Option<MarketIssuer>> {
    match market_oauth_profile()? {
        None => Ok(None),
        Some(profile) => MarketIssuer::open(pool, profile)
            .await
            .map(Some)
            .map_err(|_| invalid_config("market OAuth installed schema unavailable")),
    }
}
fn validate_market_listener(origin: &str, listen: &str) -> io::Result<()> {
    MarketProfile::isolated_loopback_test(origin, 1)
        .map_err(|_| invalid_config("invalid isolated market origin"))?;
    let u = reqwest::Url::parse(origin)
        .map_err(|_| invalid_config("invalid isolated market origin"))?;
    let ip = u
        .host_str()
        .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
        .ok_or_else(|| invalid_config("isolated market origin requires numeric loopback"))?
        .to_canonical();
    let addr = listen
        .parse::<std::net::SocketAddr>()
        .map_err(|_| invalid_config("invalid isolated market listener"))?;
    let zone = match addr {
        std::net::SocketAddr::V6(a) => a.scope_id() != 0,
        _ => false,
    };
    if zone
        || !addr.ip().to_canonical().is_loopback()
        || addr.ip().to_canonical() != ip
        || Some(addr.port()) != u.port()
        || addr.port() < 1024
    {
        return Err(invalid_config(
            "isolated market requires exact numeric loopback listener",
        ));
    }
    Ok(())
}
fn validate_isolated(origin: &str) -> io::Result<()> {
    validate_market_listener(
        origin,
        &setting("LMM_CORE_LISTEN")
            .map_err(|_| invalid_config("isolated market listener missing"))?,
    )?;
    let dir = setting_os("LMM_CORE_MARKET_OAUTH_ISOLATED_SECRET_DIRECTORY")
        .map(PathBuf::from)
        .ok_or_else(|| invalid_config("isolated market secret directory missing"))?;
    absolute_path(&dir)?;
    let metadata = fs::symlink_metadata(&dir)?;
    if !metadata.is_dir() || fs::canonicalize(&dir)? != dir {
        return Err(invalid_config(
            "isolated market secret directory must be independent",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o700 {
            return Err(invalid_config(
                "isolated market secret directory must be private",
            ));
        }
    }
    let path = |key| {
        setting_os(key)
            .map(PathBuf::from)
            .ok_or_else(|| invalid_config("incomplete isolated market credentials"))
    };
    isolated_secret(&dir, &path("LMM_CORE_WEB_REFRESH_KEY_FILE")?, 4096)?;
    isolated_secret(&dir, &path("LMM_CORE_TOOL_MARKET_TOKEN_FILE")?, 4096)?;
    let db = isolated_secret(&dir, &path("LMM_CORE_DATABASE_URL_FILE")?, 16384)?;
    let db = reqwest::Url::parse(
        std::str::from_utf8(&db)
            .map_err(|_| invalid_config("invalid isolated market database"))?
            .trim(),
    )
    .map_err(|_| invalid_config("invalid isolated market database"))?;
    let name = db.path().trim_start_matches('/');
    let fixture = name.len() == 20
        && name.starts_with("a09_")
        && name.ends_with("_a10")
        && name.as_bytes()[4..16].iter().all(u8::is_ascii_hexdigit);
    if !matches!(db.scheme(), "postgres" | "postgresql")
        || db.host_str() != Some("127.0.0.1")
        || !(name.starts_with("lmm_market_isolated_") || fixture)
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || db.fragment().is_some()
        || db
            .query_pairs()
            .any(|(k, v)| k != "sslmode" || v != "disable")
        || db
            .port()
            .is_none_or(|p| p < 1024 || matches!(p, 50837 | 47399 | 18080 | 18081 | 5432))
    {
        return Err(invalid_config(
            "isolated market OAuth requires a fresh named loopback database",
        ));
    }
    if let Some(model) = setting_os("LMM_CORE_MODEL_CONFIG_FILE") {
        if !bool_setting("LMM_CORE_MODEL_LOCAL_HTTP")? {
            return Err(invalid_config(
                "isolated market OAuth cannot coexist with normal provider transport",
            ));
        }
        let model: ModelConfig =
            parse_document(&configuration_file(Path::new(&model), 4 * 1024 * 1024)?)?;
        if !model.external_models.is_empty()
            || model
                .routing
                .upstreams
                .iter()
                .any(|u| crate::routing::Endpoint::local_loopback(u.endpoint.clone()).is_err())
        {
            return Err(invalid_config(
                "isolated market models must be synthetic loopback",
            ));
        }
        for credential in model.credentials {
            let secret = isolated_secret(&dir, &credential.secret_file, 4096)?;
            let secret = std::str::from_utf8(&secret)
                .map_err(|_| invalid_config("invalid synthetic model credential"))?
                .trim();
            if !(33..=128).contains(&secret.len())
                || !secret.starts_with("lmm-isolated-test-")
                || !secret
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(invalid_config(
                    "isolated market cannot load normal provider credentials",
                ));
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn web(origin: &str, local: bool) -> WebFile {
        WebFile {
            origin: origin.into(),
            refresh_key_file: "/nonexistent-market-secrets/web".into(),
            local_http: local,
            user_agreement_enabled: false,
            privacy_policy_enabled: false,
        }
    }
    fn file(origin: &str, isolated: bool) -> MarketOAuthFile {
        MarketOAuthFile {
            origin: origin.into(),
            resource: None,
            max_budget_credits: 1000,
            isolated_loopback_test: isolated,
            isolated_secret_directory: isolated
                .then(|| PathBuf::from("/nonexistent-market-secrets")),
        }
    }
    #[test]
    fn explicit_profile_preserves_https_and_resource_boundaries() {
        let p = parse(
            file("https://issuer.example.test", false),
            &web("https://issuer.example.test", false),
            "127.0.0.1:54321",
            true,
        )
        .unwrap();
        assert!(p.iter().any(
            |(k, v)| *k == "LMM_CORE_MARKET_OAUTH_ORIGIN" && v == "https://issuer.example.test"
        ));
        assert!(
            parse(
                file("https://issuer.example.test", false),
                &web("https://other.example.test", false),
                "127.0.0.1:54321",
                true
            )
            .is_err()
        );
        assert!(
            parse(
                file("https://issuer.example.test", false),
                &web("https://issuer.example.test", false),
                "127.0.0.1:54321",
                false
            )
            .is_err()
        );
        assert!(
            parse(
                file("http://127.0.0.1:54321", false),
                &web("http://127.0.0.1:54321", false),
                "127.0.0.1:54321",
                true
            )
            .is_err()
        );
    }
    #[test]
    fn isolated_mode_is_explicit_and_exact_listener() {
        assert!(
            parse(
                file("http://127.0.0.1:54321", true),
                &web("http://127.0.0.1:54321", true),
                "127.0.0.1:54321",
                true
            )
            .is_ok()
        );
        assert!(
            parse(
                file("http://127.0.0.1:54321", true),
                &web("http://127.0.0.1:54321", true),
                "0.0.0.0:54321",
                true
            )
            .is_err()
        );
        let mut f = file("http://127.0.0.1:54321", true);
        f.isolated_secret_directory = None;
        assert!(
            parse(
                f,
                &web("http://127.0.0.1:54321", true),
                "127.0.0.1:54321",
                true
            )
            .is_err()
        );
    }
}
