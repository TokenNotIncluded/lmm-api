//! HumanGrant: scoped AI authorization with explicit human consent.

pub mod assets;
pub mod engine;
pub mod http;
pub mod signature;
pub mod store;
pub mod types;

pub use engine::HumanGrantEngine;
pub use http::router;
pub use store::{DDL, HumanGrantStore, HumanGrantTx};
pub use types::*;
