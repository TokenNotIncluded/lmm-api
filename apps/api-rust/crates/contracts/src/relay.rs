//! Typed protocol boundary for chat-completions, Responses, and Messages.
//!
//! Legacy provenance:
//! - `relaykit/relayconvert/request_registry.go`
//! - `relaykit/relayconvert/response_registry.go`
//! - `relaykit/relayconvert/internal/oai_chat/*`
//! - `relaykit/relayconvert/internal/oai_responses/*`
//! - `relaykit/dto/{openai_request,openai_response,claude,gemini}.go`

#![allow(missing_docs)]

mod canonical;
mod capability;
mod convert;
mod ir;
mod ir_convert;
mod json;
mod registry;
mod wire;

pub use canonical::*;
pub use capability::*;
pub use convert::*;
pub use ir::*;
pub use ir_convert::*;
pub use json::*;
pub use registry::*;
pub use wire::*;

#[cfg(test)]
mod tests;
