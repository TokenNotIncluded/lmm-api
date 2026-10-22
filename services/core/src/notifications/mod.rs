//! Core-owned notification facts, preferences and durable email attempts.
//! Explicit offline provisioning reuses core_events and core_console.options;
//! ordinary startup performs no schema or subscription mutation.
mod policy;
mod producer;
mod worker;
pub use policy::{Kind, Preferences, SettingsUpdate, SitePolicy};
pub use producer::{PolicyInstallation, install_policy};
pub(crate) use producer::{load_policy, observe_ledger, publish_routing_change};
pub use worker::{NotificationRuntime, WorkerConfig, spawn_worker};

pub(crate) fn storage() -> sqlx::Error {
    sqlx::Error::Protocol("notification evidence unavailable; transaction must roll back".into())
}

pub(crate) fn event_error(error: crate::events::EventError) -> sqlx::Error {
    // Database errors retain their original retry classification in callers.
    // A bounded full queue is an availability failure, never permission to
    // commit money without its notification fact.
    if matches!(
        error,
        crate::events::EventError::Busy | crate::events::EventError::Full
    ) {
        return sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "notification event store unavailable; retry the same operation",
        ));
    }
    let reason = match error {
        crate::events::EventError::Busy => {
            "notification event store busy; retry the same operation"
        }
        crate::events::EventError::Full => {
            "notification event capacity exhausted; retry the same operation"
        }
        _ => "notification event evidence unavailable; transaction must roll back",
    };
    sqlx::Error::Protocol(reason.into())
}
