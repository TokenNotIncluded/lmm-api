//! Core assistant business reads. Model/Go input never supplies authority.
//! The worker authenticates its original Session and delegates only a typed call.
pub mod account;
pub mod confirmation;
pub mod context;
pub mod facts;
pub mod keys;
pub mod models;
mod normalization;
pub mod redaction;
pub mod registration;
pub mod support;

use serde::Serialize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<crate::identity::IdentityError> for Error {
    fn from(e: crate::identity::IdentityError) -> Self {
        use crate::identity::IdentityError as I;
        match e {
            I::Invalid => Self::Invalid,
            I::Unauthorized => Self::Unauthorized,
            I::Forbidden => Self::Forbidden,
            I::Conflict => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}

use context::VerifiedToolContext;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgConnection;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}
/// Only these implemented tools are dispatchable; discovery must not advertise
/// the other main catalogue names until their real domain adapters exist.
pub enum TypedReadArgs {
    AccountAccess,
    AvailableModels,
    ModelPricing(models::ModelPricingArgs),
    UserOverview(account::UserTarget),
    UserUsage(account::UsageSummaryArgs),
    OwnUsage(account::OwnUsageArgs),
    ListMyApiKeys(keys::ListMyApiKeys),
    NewUserGiftStatus,
    RegistrationRisk,
    TechnicalSupportEligibility,
}
impl TypedReadArgs {
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 16_384 {
            return Err(Error::Invalid);
        }
        fn empty(bytes: &[u8]) -> Result<(), Error> {
            serde_json::from_slice::<EmptyArgs>(bytes)
                .map(|_| ())
                .map_err(|_| Error::Invalid)
        }
        Ok(match name {
            "get_available_models" => {
                empty(bytes)?;
                Self::AvailableModels
            }
            "get_model_pricing" => {
                Self::ModelPricing(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_account_access" => {
                empty(bytes)?;
                Self::AccountAccess
            }
            "get_user_overview" => {
                Self::UserOverview(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_user_usage_summary" => {
                Self::UserUsage(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_usage_summary" => {
                Self::OwnUsage(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "list_my_api_keys" => {
                Self::ListMyApiKeys(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_new_user_gift_status" => {
                empty(bytes)?;
                Self::NewUserGiftStatus
            }
            "get_registration_risk" => {
                empty(bytes)?;
                Self::RegistrationRisk
            }
            _ => return Err(Error::Invalid),
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::AccountAccess => "get_account_access",
            Self::AvailableModels => "get_available_models",
            Self::ModelPricing(_) => "get_model_pricing",
            Self::UserOverview(_) => "get_user_overview",
            Self::UserUsage(_) => "get_user_usage_summary",
            Self::OwnUsage(_) => "get_usage_summary",
            Self::ListMyApiKeys(_) => "list_my_api_keys",
            Self::NewUserGiftStatus => "get_new_user_gift_status",
            Self::RegistrationRisk => "get_registration_risk",
            Self::TechnicalSupportEligibility => "core_technical_support_eligibility",
        }
    }
}
#[derive(Serialize)]
pub struct TypedReadResult {
    pub tool_name: &'static str,
    pub output: Value,
}
pub async fn read_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    args: &TypedReadArgs,
) -> Result<TypedReadResult, Error> {
    let output = match args {
        TypedReadArgs::AvailableModels | TypedReadArgs::ModelPricing(_) => {
            return Err(Error::Unavailable);
        }
        TypedReadArgs::AccountAccess => account::access_in_transaction(c, context).await?,
        TypedReadArgs::UserOverview(input) => {
            account::overview_in_transaction(c, context, input).await?
        }
        TypedReadArgs::UserUsage(input) => {
            crate::usage::assistant_summary_in_transaction(c, context, input, false).await?
        }
        TypedReadArgs::OwnUsage(input) => {
            crate::usage::assistant_summary_in_transaction(
                c,
                context,
                &account::UsageSummaryArgs {
                    user_id: None,
                    identifier: None,
                    days: input.days,
                },
                true,
            )
            .await?
        }
        TypedReadArgs::ListMyApiKeys(input) => keys::list_in_transaction(c, context, input).await?,
        TypedReadArgs::NewUserGiftStatus => account::gift_status_in_transaction(c, context).await?,
        TypedReadArgs::RegistrationRisk => registration::risk_in_transaction(c, context).await?,
        TypedReadArgs::TechnicalSupportEligibility => {
            support::eligibility_in_transaction(c, context).await?
        }
    };
    Ok(TypedReadResult {
        tool_name: args.name(),
        output,
    })
}
#[cfg(test)]
mod tests;

/// Core's own implemented adapters. Host discovery intersects these immutable
/// definitions with its explicit approved scope; Go metadata adds no authority.
pub fn implemented_manifest() -> Result<Value, Error> {
    serde_json::from_str(include_str!("manifest.json")).map_err(|_| Error::Unavailable)
}

/// The real worker injects its serving gateway; the generic DB-only entry cannot
/// fabricate a model catalogue when this dependency is absent.
pub async fn read_with_gateway_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    args: &TypedReadArgs,
    gateway: &crate::model_http::ModelGateway,
) -> Result<TypedReadResult, Error> {
    let surface = models::ModelsProjection::from_gateway(gateway);
    let output = match args {
        TypedReadArgs::AvailableModels => surface.available_tool_in_transaction(c, context).await?,
        TypedReadArgs::ModelPricing(input) => {
            surface.pricing_in_transaction(c, context, input).await?
        }
        _ => return read_in_transaction(c, context, args).await,
    };
    Ok(TypedReadResult {
        tool_name: args.name(),
        output,
    })
}
