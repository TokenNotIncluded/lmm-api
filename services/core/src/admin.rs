//! Explicit administrative operations in the core executable. Service startup
//! never invokes these operations or creates database objects.
use lmm_core::{
    config,
    identity::{
        IdentityStore,
        admin_approval::{
            self, ApplyAdminActivationRequest, ApprovalError, PlanAdminActivationRequest,
        },
    },
};
use serde::Deserialize;

mod check_db;
mod first_owner;
mod schema_upgrade;
pub use check_db::{invalid_arguments as invalid_check_db_arguments, run as check_database};
use std::{
    error::Error,
    io::{self, Read},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    user_id: i64,
    platform_level: i16,
}

pub enum Operation {
    InitDb,
    CompleteSetup,
    ConfigureNotifications(lmm_core::notifications::PolicyInstallation),
    ConfigureRoutingBaseline(Box<lmm_core::routing::store::ReleaseDocument>),
    InstallToolMarketBuiltins,
    BootstrapUser(Bootstrap),
    CreateFirstOwner(first_owner::Request),
    InstallUsageReceiptIndex([u8; 32]),
    UpgradeDatabase(Box<lmm_core::core_schema::UpgradeRequest>),
    PlanAdminActivation(PlanAdminActivationRequest),
    ApplyAdminActivation(Box<ApplyAdminActivationRequest>),
}

impl Operation {
    pub fn configure_routing_baseline(input: impl Read) -> io::Result<Self> {
        let mut bytes = Vec::new();
        input.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        let document = lmm_core::routing::store::ReleaseDocument::decode(&bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid routing baseline JSON")
        })?;
        document.clone().into_router().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid routing baseline configuration",
            )
        })?;
        Ok(Self::ConfigureRoutingBaseline(Box::new(document)))
    }

    pub fn configure_notifications(input: impl Read) -> io::Result<Self> {
        let mut bytes = Vec::new();
        input.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "notification policy exceeds 4096 bytes",
            ));
        }
        let policy: lmm_core::notifications::PolicyInstallation = serde_json::from_slice(&bytes)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid notification policy JSON",
                )
            })?;
        if !policy.policy.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid notification policy",
            ));
        }
        Ok(Self::ConfigureNotifications(policy))
    }

    pub fn upgrade_database(input: impl Read) -> io::Result<Self> {
        lmm_core::core_schema::UpgradeRequest::read(input)
            .map(|request| Self::UpgradeDatabase(Box::new(request)))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))
    }
    pub fn create_first_owner(input: impl Read) -> io::Result<Self> {
        first_owner::read(input).map(Self::CreateFirstOwner)
    }
    pub fn usage_receipt_index(input: impl Read) -> io::Result<Self> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "invalid prior catalog SHA256");
        let mut bytes = Vec::new();
        input.take(67).read_to_end(&mut bytes)?;
        let hex = match bytes.as_slice() {
            [hex @ .., b'\r', b'\n'] if hex.len() == 64 => hex,
            [hex @ .., b'\n'] if hex.len() == 64 => hex,
            hex if hex.len() == 64 => hex,
            _ => return Err(bad()),
        };
        let mut digest = [0; 32];
        for (byte, pair) in digest.iter_mut().zip(hex.as_chunks::<2>().0) {
            let digit = |v: u8| match v {
                b'0'..=b'9' => Ok(v - b'0'),
                b'a'..=b'f' => Ok(v - b'a' + 10),
                b'A'..=b'F' => Ok(v - b'A' + 10),
                _ => Err(bad()),
            };
            *byte = digit(pair[0])? * 16 + digit(pair[1])?;
        }
        Ok(Self::InstallUsageReceiptIndex(digest))
    }

    pub fn bootstrap(input: impl Read) -> io::Result<Self> {
        let mut bytes = Vec::new();
        input.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "bootstrap input is too large",
            ));
        }
        // serde errors can include caller-provided field names or values.
        // Validate before connecting and report only fixed, secret-free text.
        let request: Bootstrap = serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid bootstrap JSON"))?;
        Ok(Self::BootstrapUser(request))
    }

    pub fn plan_admin_activation(input: impl Read) -> io::Result<Self> {
        let request = admin_approval::read_bounded_json(input).map_err(approval_io)?;
        Ok(Self::PlanAdminActivation(request))
    }

    pub fn apply_admin_activation(input: impl Read) -> io::Result<Self> {
        let request = admin_approval::read_bounded_json(input).map_err(approval_io)?;
        Ok(Self::ApplyAdminActivation(Box::new(request)))
    }
}

fn approval_io(error: ApprovalError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        match error {
            ApprovalError::InputTooLarge => "activation input exceeds 64 KiB",
            ApprovalError::InvalidJson => "invalid activation JSON",
            other => return io::Error::other(other.to_string()),
        },
    )
}

pub async fn run(operation: Operation) -> Result<(), Box<dyn Error>> {
    // This branch connects directly, before creating any pool/IdentityStore.
    if let Operation::UpgradeDatabase(request) = &operation {
        return schema_upgrade::run((**request).clone()).await;
    }
    let url = config::database_url()?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a core database URL is required",
        )
    })?;
    let pool = config::database_pool(&url).await?;
    let store = IdentityStore::from_pool(pool.clone());
    match operation {
        Operation::UpgradeDatabase(_) => unreachable!("dedicated schema upgrade returned above"),
        Operation::InstallUsageReceiptIndex(prior) => {
            lmm_core::core_schema::install_usage_receipt_index(&pool, prior).await?;
            println!("Usage receipt index installed; schema receipts verified atomically.");
        }
        Operation::InstallToolMarketBuiltins => {
            lmm_core::core_schema::check(&pool).await?;
            let receipt =
                lmm_core::tool_market::builtin_install::install_compiled_builtins(&pool).await?;
            serde_json::to_writer(io::stdout().lock(), &receipt)?;
            println!();
        }
        Operation::InitDb => {
            lmm_core::core_schema::init(&pool).await?;
            lmm_core::core_schema::check(&pool).await?;
            println!("Fresh core database initialized. No existing data was changed.");
        }
        Operation::BootstrapUser(request) => {
            lmm_core::core_schema::check(&pool).await?;
            let issued = store
                .bootstrap_user(request.user_id, request.platform_level)
                .await?;
            // One-time secret output. Operators must redirect to a private file.
            serde_json::to_writer(io::stdout().lock(), &issued)?;
            println!();
        }
        Operation::CreateFirstOwner(request) => {
            let receipt = first_owner::run(&pool, request).await?;
            serde_json::to_writer(io::stdout().lock(), &receipt)?;
            println!();
        }
        Operation::ConfigureNotifications(input) => {
            lmm_core::core_schema::check(&pool).await?;
            let revision = lmm_core::notifications::install_policy(&pool, &input).await?;
            serde_json::to_writer(
                io::stdout().lock(),
                &serde_json::json!({"revision":revision,"mail_sent":false}),
            )?;
            println!();
        }
        Operation::ConfigureRoutingBaseline(document) => {
            let receipt = lmm_core::routing_admin::install_owner_baseline(&pool, *document).await?;
            serde_json::to_writer(io::stdout().lock(), &receipt)?;
            println!();
        }
        Operation::CompleteSetup => {
            lmm_core::core_schema::check(&pool).await?;
            // This explicit offline command cannot replace an imported receipt
            // or create an administrator. Runtime roles have no INSERT grant.
            lmm_core::console_compat::record_initialization(&pool, env!("CARGO_PKG_VERSION"))
                .await?;
            println!(
                "Core setup completion recorded. Existing initialization facts were preserved."
            );
        }
        Operation::PlanAdminActivation(request) => {
            lmm_core::core_schema::check(&pool).await?;
            let mut connection = pool.acquire().await?;
            let response = admin_approval::plan_admin_activation(&mut connection, &request).await?;
            serde_json::to_writer(io::stdout().lock(), &response)?;
            println!();
        }
        Operation::ApplyAdminActivation(request) => {
            lmm_core::core_schema::check(&pool).await?;
            let mut connection = pool.acquire().await?;
            let response =
                admin_approval::apply_admin_activation(&mut connection, &request).await?;
            serde_json::to_writer(io::stdout().lock(), &response)?;
            println!();
        }
    }
    Ok(())
}
