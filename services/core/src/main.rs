use lmm_core::{
    config,
    identity::IdentityStore,
    lifecycle::{Admission, Lifecycle},
};
use std::{
    env,
    error::Error,
    ffi::OsString,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod admin;

const USAGE: &str = "Usage: lmm-core [--config /absolute/core.toml | --legacy-env] [serve | init-db | upgrade-db | create-first-owner | bootstrap-user | complete-setup | install-tool-market-builtins | install-usage-receipt-index | plan-admin-activation | apply-admin-activation | schema-manifest | check-db]\n\n\
Commands:\n\
  serve           Start the core service (the default without arguments).\n\
  schema-manifest Print the compiled schema contract as JSON; accepts no config options and performs no I/O except stdout.\n\
  check-db        Read-only schema/runtime authority JSON; requires explicit --config.\n\
  init-db         Initialize a new, empty core database.\n\
  upgrade-db      Explicit offline exact Core26 ledger revision; requires owner --config and exact-target stdin JSON.\n\
                  Legacy 24->25 trust-pricing requires the preserved 25-component installer.\n\
                  Additive 25->26 requires the preserved 26-component installer before this ledger revision.\n\
  create-first-owner  Create the first production owner offline from private stdin JSON; requires explicit --config and a fresh store.\n\
  bootstrap-user  Read a user_id and platform_level JSON object from stdin.\n\n\
  install-tool-market-builtins  Install the compiled tool catalog offline as installation owner.\n\n\
  complete-setup  Record setup completion offline after creating or importing root.\n\
  install-usage-receipt-index  Offline owner-only index installation; requires explicit --config.\n\
  plan-admin-activation  Read-only candidate binding for one imported admin; requires explicit --config (not --legacy-env).\n\
  apply-admin-activation Owner-only apply of one human-confirmed activation; requires explicit --config (not --legacy-env).\n\n\
Options:\n\
  --config PATH  Read one absolute TOML file (default /etc/lmm.conf.d/core.toml).\n\
  --legacy-env   Explicit temporary environment compatibility; conflicts with --config.\n\
  -h, --help      Show this help without reading database configuration.\n\
  -V, --version   Show the core version.\n\n\
bootstrap-user accepts at most 4096 bytes and writes a one-time credential to\n\
stdout. Redirect that output to a private file; do not put secrets in arguments.\n\
install-usage-receipt-index reads exactly one approved prior catalog SHA256 from\n\
stdin (64 hex digits, optional LF/CRLF). Use separate installer-owner configuration\n\
after independent frozen-source review; never runtime credentials or automatic detection.\n\
plan-admin-activation and apply-admin-activation read at most 64 KiB of deny_unknown_fields\n\
JSON from stdin and print secret-free diagnostics. They never invent a human approval.";

enum Command {
    Serve,
    InitDb,
    UpgradeDb,
    CreateFirstOwner,
    BootstrapUser,
    CompleteSetup,
    InstallToolMarketBuiltins,
    InstallUsageReceiptIndex,
    PlanAdminActivation,
    ApplyAdminActivation,
    SchemaManifest,
    CheckDb,
    Help,
    Version,
}

struct Arguments {
    command: Command,
    config: Option<PathBuf>,
    legacy_env: bool,
}

fn command(args: impl IntoIterator<Item = OsString>) -> io::Result<Arguments> {
    let mut args = args.into_iter();
    let mut operation = None;
    let mut config = None;
    let mut legacy_env = false;
    let mut help = false;
    let mut version = false;
    let bad = || io::Error::new(io::ErrorKind::InvalidInput, USAGE);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--config") if config.is_none() && !legacy_env => {
                let path = PathBuf::from(args.next().ok_or_else(bad)?);
                if !path.is_absolute() {
                    return Err(bad());
                }
                config = Some(path);
            }
            Some("--legacy-env") if !legacy_env && config.is_none() => legacy_env = true,
            Some("-h" | "--help") if !help && !version => help = true,
            Some("-V" | "--version") if !version && !help && operation.is_none() => version = true,
            Some(
                "serve"
                | "init-db"
                | "upgrade-db"
                | "create-first-owner"
                | "bootstrap-user"
                | "complete-setup"
                | "install-tool-market-builtins"
                | "install-usage-receipt-index"
                | "plan-admin-activation"
                | "apply-admin-activation"
                | "schema-manifest"
                | "check-db",
            ) if operation.is_none() && !version => {
                operation = Some(match arg.to_str() {
                    Some("init-db") => Command::InitDb,
                    Some("upgrade-db") => Command::UpgradeDb,
                    Some("create-first-owner") => Command::CreateFirstOwner,
                    Some("bootstrap-user") => Command::BootstrapUser,
                    Some("complete-setup") => Command::CompleteSetup,
                    Some("install-tool-market-builtins") => Command::InstallToolMarketBuiltins,
                    Some("install-usage-receipt-index") => Command::InstallUsageReceiptIndex,
                    Some("plan-admin-activation") => Command::PlanAdminActivation,
                    Some("apply-admin-activation") => Command::ApplyAdminActivation,
                    Some("schema-manifest") => Command::SchemaManifest,
                    Some("check-db") => Command::CheckDb,
                    _ => Command::Serve,
                });
            }
            _ => return Err(bad()),
        }
    }
    Ok(Arguments {
        command: if help {
            Command::Help
        } else if version {
            Command::Version
        } else {
            operation.unwrap_or(Command::Serve)
        },
        config,
        legacy_env,
    })
}

fn check_db_requested(args: &[OsString]) -> bool {
    let mut config_value = false;
    for arg in args {
        if config_value {
            config_value = false;
            continue;
        }
        if arg == "--config" {
            config_value = true;
            continue;
        }
        if arg == "check-db" {
            return true;
        }
    }
    false
}

fn main() -> Result<(), Box<dyn Error>> {
    let raw: Vec<_> = env::args_os().skip(1).collect();
    let checking = check_db_requested(&raw);
    let arguments = match command(raw) {
        Ok(arguments) => arguments,
        Err(_) if checking => std::process::exit(admin::invalid_check_db_arguments()),
        Err(error) => return Err(error.into()),
    };
    let operation = match arguments.command {
        Command::Help => {
            println!("{USAGE}");
            return Ok(());
        }
        Command::Version => {
            println!("lmm-core {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Command::SchemaManifest => {
            if arguments.config.is_some() || arguments.legacy_env {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE).into());
            }
            use std::io::Write;
            let mut output = io::stdout().lock();
            serde_json::to_writer(&mut output, &lmm_core::core_schema::schema_manifest())?;
            writeln!(output)?;
            return Ok(());
        }
        Command::CheckDb => std::process::exit(admin::check_database(
            arguments.config.as_deref(),
            arguments.legacy_env,
        )),
        Command::Serve => None,
        Command::InitDb => Some(admin::Operation::InitDb),
        Command::UpgradeDb => {
            if arguments.config.is_none() || arguments.legacy_env {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "upgrade-db requires explicit offline installer-owner --config",
                )
                .into());
            }
            Some(admin::Operation::upgrade_database(io::stdin().lock())?)
        }
        Command::CreateFirstOwner => {
            if arguments.config.is_none() || arguments.legacy_env {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "create-first-owner requires explicit installer-owner --config",
                )
                .into());
            }
            Some(admin::Operation::create_first_owner(io::stdin().lock())?)
        }
        Command::BootstrapUser => Some(admin::Operation::bootstrap(io::stdin().lock())?),
        Command::CompleteSetup => Some(admin::Operation::CompleteSetup),
        Command::InstallToolMarketBuiltins => {
            if arguments.config.is_none() || arguments.legacy_env {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "builtin catalog installation requires explicit installer-owner --config",
                )
                .into());
            }
            Some(admin::Operation::InstallToolMarketBuiltins)
        }
        Command::InstallUsageReceiptIndex => {
            let operation = admin::Operation::usage_receipt_index(io::stdin().lock())?;
            if arguments.config.is_none() || arguments.legacy_env {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage receipt index installation requires explicit installer-owner --config",
                )
                .into());
            }
            Some(operation)
        }
        Command::PlanAdminActivation => {
            if arguments.config.is_none() || arguments.legacy_env {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "plan-admin-activation requires explicit --config and cannot use --legacy-env",
                )
                .into());
            }
            Some(admin::Operation::plan_admin_activation(io::stdin().lock())?)
        }
        Command::ApplyAdminActivation => {
            if arguments.config.is_none() || arguments.legacy_env {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "apply-admin-activation requires explicit --config and cannot use --legacy-env",
                )
                .into());
            }
            Some(admin::Operation::apply_admin_activation(
                io::stdin().lock(),
            )?)
        }
    };
    config::initialize(arguments.config.as_deref(), arguments.legacy_env)?;
    // One worker and a bounded blocking pool are sufficient for the 1-core
    // profile. Password hashing remains on a separate bounded blocking worker.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config::worker_threads()?)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    match operation {
        Some(operation) => runtime.block_on(admin::run(operation)),
        None => runtime.block_on(run_core()),
    }
}

async fn run_core() -> Result<(), Box<dyn Error>> {
    let address = config::listen_address()?;
    let drain_timeout = config::drain_timeout()?;
    let database_url = config::database_url()?;
    let lifecycle = Lifecycle::with_capacity(config::max_inflight()?);
    let ingress = config::ingress()?;
    let legacy_config = config::legacy_auth()?;
    let console_enabled = config::console_enabled()?;
    let mut capabilities = lmm_core::http::Capabilities::default();
    let mut public_status = lmm_core::http::PublicStatus::default();
    let mut database_pool = None;
    let mut remote_store = None;
    let mut model_gateway = None;
    let mut humangrant_engine = None;
    let mut app = lmm_core::http::router();
    if let Some(url) = database_url.as_deref() {
        let pool = config::database_pool(url).await?;
        lmm_core::runtime_database::check(
            &pool,
            lmm_core::runtime_database::RequiredCapabilities {
                models: config::setting_os("LMM_CORE_MODEL_CONFIG_FILE").is_some(),
                images: config::images_configured()?,
                events: config::setting_os("LMM_CORE_RPC_SOCKET").is_some(),
                payments: config::payments_configured(),
                commerce: config::commerce_configured(),
                assistant: config::assistant_configured(),
                assistant_tools: config::assistant_tools_enabled()?,
                tool_market: config::tool_market_configured(),
                console: console_enabled,
                web_identity: legacy_config.is_some(),
                oauth2: config::oauth2_configured(),
                market_oauth: config::market_oauth_configured(),
                wallet: config::wallet_configured(),
                bounties: config::bounties_configured(),
            },
        )
        .await?;
        // Installation belongs solely to the explicit init-db command. Even a
        // valid identity-only installation cannot stand in for the whole core.
        lmm_core::core_schema::check(&pool).await?;
        let (abuse, configured_models) = config::models_and_abuse(pool.clone())?;
        let mut store = config::identity_auth(IdentityStore::from_pool(pool.clone()))?
            .with_password_slots(config::password_slots()?)?
            .with_financial_profile_enabled(legacy_config.is_some())
            .with_commerce_scope_enabled(config::commerce_configured());
        capabilities.identity = true;
        // Production never silently opts out of owner-controlled user pricing.
        // Library fixtures may omit the builder; serving this binary may not.
        model_gateway = configured_models.map(|gateway| gateway.with_trust_pricing());
        if let Some(gateway) = &model_gateway {
            store = store.with_dashboard_routing(gateway.dashboard_routing());
            // Trusted recovery keeps indeterminate upstream holds reserved.
            // A failed recovery prevents listener binding/readiness.
            gateway.recover_expired(64).await?;
            gateway.reconcile_completed_streams(64).await?;
            capabilities.models = true;
            capabilities.models_streaming = gateway.streaming_enabled();
        }
        // A Web deployment must not offer a second registration path that
        // bypasses the Web legal-consent contract.
        let native_store = if legacy_config.is_some() {
            store.clone().with_registration_enabled(false)
        } else {
            store.clone()
        };
        let mut routes = lmm_core::identity_http::router(native_store)
            .merge(lmm_core::abuse::admin::router(abuse))
            .merge(lmm_core::identity::admin_users::router(
                lmm_core::identity::admin_users::AdminUsersStore::new(store.clone()),
            ));
        capabilities.admin_users_read = true;
        if let Some((humangrant, engine)) = config::humangrant(pool.clone()).await? {
            if legacy_config.is_none() {
                return Err("HumanGrant requires native Web authentication".into());
            }
            routes = routes.merge(humangrant);
            humangrant_engine = Some(engine);
            capabilities.humangrant = true;
        }
        capabilities.abuse_admin = true;
        if console_enabled {
            routes = routes.merge(lmm_core::console_compat::router(
                lmm_core::console_compat::ConsoleCompat::new(pool.clone(), store.clone()),
            ));
            capabilities.console = true;
        }
        remote_store = Some(store.clone());
        routes = routes.merge(lmm_core::native_key_read::router(pool.clone()));
        if let Some(issuer) = config::market_oauth_issuer(pool.clone()).await? {
            routes = routes.merge(lmm_core::market_oauth::http::router(issuer));
        }
        let oauth2 = config::oauth2_issuer(store.clone()).await?;
        let oauth2 = oauth2.map(|issuer| {
            if let Some(gateway) = &model_gateway {
                issuer.with_catalog_surface(gateway.oauth_catalog_surface())
            } else {
                issuer
            }
        });
        if let Some(issuer) = &oauth2 {
            if issuer.profile().pi_budget_limit().is_some() && model_gateway.is_none() {
                return Err("Pi delegation requires actual model gateway".into());
            }
            if let Some(gateway) = &mut model_gateway {
                *gateway = gateway.clone().with_oauth2_issuer(issuer.clone());
            }
            routes = routes.merge(lmm_core::oauth2_http::router(issuer.clone()));
        }
        if let Some(mut config) = legacy_config {
            if let Some(issuer) = oauth2 {
                config.auth = config.auth.with_oauth2_issuer(issuer)?;
            }
            public_status = lmm_core::http::PublicStatus {
                password_login_enabled: true,
                two_factor_enabled: store.mfa_enabled(),
                password_register_enabled: config.registration_enabled,
                user_agreement_enabled: config.user_agreement_enabled,
                privacy_policy_enabled: config.privacy_policy_enabled,
                google_client_id: config.auth.google_client_id(&store).map(str::to_owned),
                google_browser_callback: config
                    .auth
                    .google_browser_callback(&store)
                    .map(str::to_owned),
                github_client_id: config
                    .auth
                    .external_client_id(&store, lmm_core::identity::ExternalProvider::Github)
                    .map(str::to_owned),
                linuxdo_client_id: config
                    .auth
                    .external_client_id(&store, lmm_core::identity::ExternalProvider::LinuxDo)
                    .map(str::to_owned),
                discord_client_id: config
                    .auth
                    .external_client_id(&store, lmm_core::identity::ExternalProvider::Discord)
                    .map(str::to_owned),
                telegram_bot_name: store.telegram_bot_name().map(str::to_owned),
            };
            routes = routes.merge(lmm_core::legacy_auth::router(store, config.auth));
            capabilities.legacy_web_auth = true;
        }
        if let Some(gateway) = &model_gateway {
            routes = routes.merge(gateway.router());
            routes = routes.merge(
                lmm_core::pricing_http::PricingHttp::from_gateway(pool.clone(), gateway).router(),
            );
            if config::image_host_configured() {
                routes = routes.merge(config::image_browser_router(gateway)?);
            }
        } else {
            routes = routes
                .merge(lmm_core::pricing_http::PricingHttp::unavailable(pool.clone()).router());
        }
        if config::wallet_configured() {
            let wallet = lmm_core::wallet::Store::new(pool.clone());
            wallet
                .check_ready()
                .await
                .map_err(|_| "wallet schema or runtime authority unavailable")?;
            routes = routes.merge(config::wallet_browser_router(wallet)?);
        }
        if config::bounties_configured() {
            let store = lmm_core::bounties::BountyStore::from_pool(pool.clone());
            store
                .check_ready()
                .await
                .map_err(|_| "bounty schema or runtime authority unavailable")?;
            routes = routes.merge(config::bounties_browser_router(store)?);
        }
        if let Some(confirmations) = config::assistant_tools_browser_router(pool.clone())? {
            routes = routes.merge(confirmations);
        }
        // The final readiness router is assembled after the optional RPC checks.
        app = routes;
        database_pool = Some(pool);
    } else if config::assistant_configured()
        || config::tool_market_configured()
        || config::support_projection_configured()
        || config::wallet_configured()
        || config::bounties_configured()
        || config::image_host_configured()
        || config::remote_control_configured()
        || legacy_config.is_some()
        || config::identity_database_required()?
        || console_enabled
        || config::payments_configured()
        || config::commerce_configured()
        || config::setting_os("LMM_CORE_MODEL_CONFIG_FILE").is_some()
        || config::setting_os("LMM_CORE_MODEL_LOCAL_HTTP").is_some()
        || config::setting_os("LMM_CORE_ABUSE_CREDENTIAL_CONFIG_FILE").is_some()
    {
        return Err("Web authentication and model configuration require a core database".into());
    }
    let (stop, stopped) = tokio::sync::watch::channel(false);
    #[cfg(unix)]
    let mut remote_proxy = None;
    #[cfg(unix)]
    if let Some(store) = remote_store
        && let Some(remote) = config::remote_control(store, stopped.clone())?
    {
        app = app.merge(remote.clone().router());
        remote_proxy = Some(remote);
    }
    #[cfg(not(unix))]
    if config::remote_control_configured() {
        return Err("remote relay requires Unix Core RPC on this release".into());
    }
    // RPC remains usable while accepted public requests and completions drain.
    #[cfg(unix)]
    let (rpc_stop, rpc_stopped) = tokio::sync::watch::channel(false);
    #[cfg(unix)]
    let mut assistant_drain = None;
    #[cfg(unix)]
    let rpc = match (
        config::setting_os("LMM_CORE_RPC_SOCKET"),
        config::setting_os("LMM_CORE_RPC_TOKEN_FILE"),
    ) {
        (None, None) => None,
        (Some(socket), Some(token_file)) => {
            let token = config::service_token(std::path::Path::new(&token_file))?;
            let store = match database_url.as_deref() {
                Some(url) => Some(lmm_core::internal_rpc::readonly_store(url).await?),
                None => None,
            };
            let mut server = lmm_core::internal_rpc::RpcServer::bind_with_group(
                std::path::Path::new(&socket),
                &token,
                store,
                config::rpc_shared_group()?,
            )?;
            if let Some(url) = database_url.as_deref() {
                let events = lmm_core::events::EventStore::connect(url).await?;
                server = server.with_events(events, "extensions")?;
                capabilities.events = true;
                if let Some(assistant) =
                    config::assistant_rpc(url, model_gateway.as_ref(), &token).await?
                {
                    assistant_drain = Some(assistant.clone());
                    server = server.with_assistant(assistant);
                }
                let financial = config::financial_rpcs(url, &token).await?;
                if let Some(payments) = financial.payments {
                    server = server.with_payments(payments);
                    capabilities.payments = true;
                }
                if let Some(commerce) = financial.commerce {
                    server = server.with_commerce(commerce);
                    capabilities.commerce = true;
                }
            }
            Some(server)
        }
        _ => return Err("configure both RPC socket and service credential file".into()),
    };
    #[cfg(unix)]
    if rpc.is_none()
        && (config::payments_configured()
            || config::commerce_configured()
            || config::assistant_configured())
    {
        return Err("financial RPC configuration requires the private RPC listener".into());
    }
    #[cfg(not(unix))]
    if config::setting_os("LMM_CORE_RPC_SOCKET").is_some()
        || config::setting_os("LMM_CORE_RPC_TOKEN_FILE").is_some()
        || config::payments_configured()
        || config::commerce_configured()
        || config::assistant_configured()
        || config::tool_market_configured()
        || config::support_projection_configured()
        || config::wallet_configured()
        || config::bounties_configured()
        || config::image_host_configured()
    {
        return Err("internal RPC requires a Unix socket on this release".into());
    }
    #[cfg(unix)]
    let tool_market_server = match database_url.as_deref() {
        Some(url) => config::tool_market_server(url, model_gateway.as_ref()).await?,
        None if config::tool_market_configured()
            || config::support_projection_configured()
            || config::wallet_configured()
            || config::bounties_configured()
            || config::image_host_configured() =>
        {
            return Err("tool market requires the Core database".into());
        }
        None => None,
    };
    let rpc_running = Arc::new(AtomicBool::new(false));
    let models_healthy = Arc::new(AtomicBool::new(true));
    if let Some(pool) = &database_pool {
        let rpc_status = capabilities.events.then(|| rpc_running.clone());
        app = lmm_core::http::configured_router_with_runtime_status(
            pool.clone(),
            capabilities,
            rpc_status,
            capabilities.models.then(|| models_healthy.clone()),
            public_status,
        )
        .merge(app);
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    let recovery_task = model_gateway.clone().map(|gateway| {
        let mut stopped = stopped.clone();
        let healthy = models_healthy.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {},
                    _ = stopped.changed() => break,
                }
                if *stopped.borrow() {
                    break;
                }
                // Finish accepted reconciliation transactions even if shutdown
                // arrives. Partial provider evidence never releases a hold.
                let recovered = gateway.recover_expired(64).await;
                let reconciled = gateway.reconcile_completed_streams(64).await;
                let available = recovered.is_ok() && reconciled.is_ok();
                healthy.store(available, Ordering::Release);
                if !available {
                    eprintln!("synthetic model recovery unavailable; model readiness withheld");
                }
            }
        })
    });
    #[cfg(unix)]
    let rpc_task = rpc.map(|server| {
        let stopped = rpc_stopped;
        let rpc_running = rpc_running.clone();
        rpc_running.store(true, Ordering::Release);
        tokio::spawn(async move {
            if server.serve(stopped).await.is_err() {
                eprintln!("internal RPC stopped; public core remains independent");
            }
            rpc_running.store(false, Ordering::Release);
        })
    });
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    if let Some(edge) = config::edge_access_config()? {
        let pool = database_pool
            .clone()
            .ok_or("edge access requires a database")?;
        app = lmm_core::edge_access::EdgeAccess::new(pool, edge)
            .await?
            .protect(app);
    }
    app = app.layer(axum::middleware::from_fn(
        lmm_core::http::request_body_deadline,
    ));
    app = app.layer(axum::middleware::from_fn_with_state(
        Admission::new(lifecycle.clone(), ingress),
        lmm_core::lifecycle::admit,
    ));
    #[cfg(unix)]
    if let Some(remote) = remote_proxy {
        app = app.layer(axum::middleware::from_fn_with_state(
            remote,
            lmm_core::remote_control::preadmit,
        ));
    }
    // All enabled components and listener binds have succeeded. The health body
    // lists capabilities; model readiness has its own explicit endpoint.
    assert!(lifecycle.mark_ready());
    let humangrant_mail_task = humangrant_engine
        .filter(|engine| engine.mail.is_some())
        .map(|engine| lmm_core::humangrant::mail::spawn_worker(engine, stopped.clone()));
    #[cfg(unix)]
    let tool_market_task = tool_market_server.map(|server| {
        let stopped = stopped.clone();
        tokio::spawn(async move { server.serve(stopped).await })
    });

    let signal_lifecycle = lifecycle.clone();
    let signal_stop = stop.clone();
    let signal_gateway = model_gateway.clone();
    #[cfg(unix)]
    let signal_assistant = assistant_drain.clone();
    let signal = tokio::spawn(async move {
        #[cfg(unix)]
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        signal_lifecycle.begin_drain();
        #[cfg(unix)]
        if let Some(assistant) = signal_assistant {
            assistant.close_admission();
        }
        let _ = signal_stop.send(true);
        if let Some(gateway) = signal_gateway {
            // Stop admission and cancel an overlong upstream after the grace
            // period. Never abort an accepted financial finalizer.
            gateway.drain(drain_timeout).await;
        }
    });
    eprintln!(
        "lmm-core {} listening on {}; identity={}, legacy_web_auth={}, console={}, synthetic_models={}, streaming={}, events={}, test_payments_rpc={}, commerce_rpc={}",
        env!("CARGO_PKG_VERSION"),
        address,
        capabilities.identity,
        capabilities.legacy_web_auth,
        capabilities.console,
        capabilities.models,
        capabilities.models_streaming,
        capabilities.events,
        capabilities.payments,
        capabilities.commerce,
    );
    let mut public_stop = stopped;
    // Deliberately no total timeout on a future model/SSE stream here.
    let result = lmm_core::public_server::serve(listener, app, async move {
        if !*public_stop.borrow() {
            let _ = public_stop.changed().await;
        }
    })
    .await;
    lifecycle.begin_drain();
    let _ = stop.send(true);
    if let Some(gateway) = &model_gateway {
        while gateway.drain(Duration::from_secs(5)).await.remaining != 0 {
            eprintln!("waiting for durable model finalizers");
        }
    }
    #[cfg(unix)]
    if let Some(assistant) = assistant_drain {
        while !assistant.drain(Duration::from_secs(5)).await {
            eprintln!("waiting for durable assistant finalizers");
        }
    }
    // HTTP EOF/cancellation is not proof that asynchronous completion is done.
    // Completion tasks must own a WorkLease and write their durable outcome.
    lifecycle.wait_idle().await;
    if let Some(task) = recovery_task {
        let _ = task.await;
    }
    if let Some(task) = humangrant_mail_task {
        task.await?;
    }
    #[cfg(unix)]
    {
        let _ = rpc_stop.send(true);
        if let Some(task) = rpc_task {
            task.await?;
        }
    }
    #[cfg(unix)]
    if let Some(task) = tool_market_task {
        task.await??;
    }
    if let Some(pool) = database_pool {
        pool.close().await;
    }
    signal.abort();
    result?;
    Ok(())
}

#[cfg(test)]
mod check_db_parser_tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn check_db_is_explicit_and_accepts_both_config_positions() {
        for input in [
            args(&["--config", "/private/runtime.toml", "check-db"]),
            args(&["check-db", "--config", "/private/runtime.toml"]),
        ] {
            let v = command(input).unwrap();
            assert!(matches!(v.command, Command::CheckDb));
            assert_eq!(v.config, Some(PathBuf::from("/private/runtime.toml")));
            assert!(!v.legacy_env);
        }
        let v = command(args(&["check-db"])).unwrap();
        assert!(matches!(v.command, Command::CheckDb));
        assert!(v.config.is_none());
    }
    #[test]
    fn upgrade_db_parser_is_explicit_and_rejects_multiple_operations() {
        for input in [
            args(&["upgrade-db", "--config", "/private/owner.toml"]),
            args(&["--config", "/private/owner.toml", "upgrade-db"]),
        ] {
            let value = command(input).unwrap();
            assert!(matches!(value.command, Command::UpgradeDb));
            assert_eq!(value.config, Some(PathBuf::from("/private/owner.toml")));
        }
        for input in [
            args(&["serve", "upgrade-db"]),
            args(&["upgrade-db", "init-db"]),
            args(&["upgrade-db", "--config", "relative.toml"]),
        ] {
            assert!(command(input).is_err());
        }
    }
    #[test]
    fn check_db_never_implicitly_selects_serve_or_accepts_extra_commands() {
        for input in [
            args(&["check-db", "serve"]),
            args(&["serve", "check-db"]),
            args(&["check-db", "--config", "relative.toml"]),
            args(&["check-db", "--unknown"]),
        ] {
            assert!(command(input).is_err());
        }
        assert!(matches!(
            command(args(&[])).unwrap().command,
            Command::Serve
        ));
        assert!(!check_db_requested(&args(&[
            "--config", "check-db", "serve"
        ])));
    }
}
