//! A single offline schema operation in lmm-core, never a daemon or startup hook.
use lmm_core::{
    config,
    core_schema::{UpgradeError, UpgradeRequest, upgrade_database},
};
use std::{error::Error, io};

pub async fn run(request: UpgradeRequest) -> Result<(), Box<dyn Error>> {
    let url = config::database_url()?
        .ok_or_else(|| io::Error::other("upgrade-db requires an owner database URL"))?;
    match upgrade_database(&url, request).await {
        Ok(report) => {
            serde_json::to_writer(io::stdout().lock(), &report)?;
            println!();
            Ok(())
        }
        Err(error) => {
            let mut report = serde_json::json!({
                "version":1,"protocol":"core.upgrade-db.v1","status":"rejected","code":error.to_string()
            });
            if let UpgradeError::CommitUnknown {
                upgrade_id,
                backend_pid,
            } = &error
            {
                report["status"] = "unknown".into();
                report["upgrade_id"] = upgrade_id.clone().into();
                report["original_backend_pid"] = (*backend_pid).into();
                report["recovery"] = "reuse_exact_original_request_and_id; observe_original_backend_and_receipt_under_session_fence".into();
            }
            serde_json::to_writer(io::stdout().lock(), &report)?;
            println!();
            Err(error.into())
        }
    }
}
