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
    InstallToolMarketBuiltins,
    BootstrapUser(Bootstrap),
    CreateFirstOwner(first_owner::Request),
    InstallUsageReceiptIndex([u8; 32]),
    PlanAdminActivation(PlanAdminActivationRequest),
    ApplyAdminActivation(Box<ApplyAdminActivationRequest>),
}

impl Operation {
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
    let url = config::database_url()?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a core database URL is required",
        )
    })?;
    let pool = config::database_pool(&url).await?;
    let store = IdentityStore::from_pool(pool.clone());
    match operation {
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
