//! Runtime-owned protocol capability catalog.
//!
//! The contracts crate describes route claims, but it cannot know which HTTP
//! handlers are actually mounted. This module is the application-owned
//! inventory for the four native same-protocol passthrough paths and is
//! checked against the contracts registry before the listener starts.

use std::collections::BTreeSet;

use lmm_contracts::relay::{
    Direction, Protocol, Registry, RegistryValidationError, RuntimeCatalog, RuntimeRoute,
    SupportMatrix, ValidatedRegistry,
};

const RUNTIME_CATALOG_VERSION: &str = "api-rust-native-relay-v1";

/// Converter ID registered by the OpenAI Chat native passthrough handler.
pub const OPENAI_CHAT_RAW_CONVERTER: &str = "raw-openai-chat-v1";
/// Converter ID registered by the OpenAI Responses native passthrough handler.
pub const OPENAI_RESPONSES_RAW_CONVERTER: &str = "raw-openai-responses-v1";
/// Converter ID registered by the Claude native passthrough handler.
pub const CLAUDE_RAW_CONVERTER: &str = "raw-claude-messages-v1";
/// Converter ID registered by the Gemini native passthrough handler.
pub const GEMINI_RAW_CONVERTER: &str = "raw-gemini-generate-content-v1";

const RAW_RUNTIME_ROWS: [(Protocol, &str, &str); 4] = [
    (
        Protocol::OpenAi,
        OPENAI_CHAT_RAW_CONVERTER,
        "raw-openai-chat-v1-runtime",
    ),
    (
        Protocol::OpenAiResponses,
        OPENAI_RESPONSES_RAW_CONVERTER,
        "raw-openai-responses-v1-runtime",
    ),
    (
        Protocol::Claude,
        CLAUDE_RAW_CONVERTER,
        "raw-claude-messages-v1-runtime",
    ),
    (
        Protocol::Gemini,
        GEMINI_RAW_CONVERTER,
        "raw-gemini-generate-content-v1-runtime",
    ),
];

/// Builds the catalog from the native passthrough handlers actually mounted
/// by the current Rust listener.
#[must_use]
pub fn current_runtime_catalog() -> RuntimeCatalog {
    let converters = RAW_RUNTIME_ROWS
        .iter()
        .map(|(_, converter, _)| (*converter).to_owned())
        .collect::<BTreeSet<_>>();
    let finalizers = RAW_RUNTIME_ROWS
        .iter()
        .map(|(_, converter, _)| format!("{converter}-finalizer"))
        .collect::<BTreeSet<_>>();
    let adaptors = RAW_RUNTIME_ROWS
        .iter()
        .map(|(_, _, adaptor)| (*adaptor).to_owned())
        .collect::<BTreeSet<_>>();
    let routes = RAW_RUNTIME_ROWS
        .iter()
        .map(|(protocol, converter, adaptor)| {
            let mut route = RuntimeRoute::new(*protocol, *protocol);
            route.request_converter_id = Some((*converter).to_owned());
            route.response_converter_id = Some((*converter).to_owned());
            route.stream_converter_id = Some((*converter).to_owned());
            route.stream_finalizer_id = Some(format!("{converter}-finalizer"));
            route.runtime_adaptors.insert((*adaptor).to_owned());
            route
        })
        .collect::<Vec<_>>();
    RuntimeCatalog::new(
        RUNTIME_CATALOG_VERSION,
        converters,
        finalizers,
        adaptors,
        routes,
    )
}

/// Validates the current contracts registry against the mounted runtime.
pub fn validated_current_registry() -> Result<ValidatedRegistry, RegistryValidationError> {
    Registry::current().validate_against_catalog(&current_runtime_catalog())
}

/// Performs the startup fail-closed runtime drift check.
pub fn validate_protocol_runtime() -> Result<(), RegistryValidationError> {
    validated_current_registry().map(|_| ())
}

/// Returns the machine-readable matrix only after runtime validation passes.
pub fn current_support_matrix() -> Result<SupportMatrix, RegistryValidationError> {
    validated_current_registry().map(|registry| registry.support_matrix().clone())
}

/// Returns whether a catalog direction is deliberately native raw passthrough.
pub const fn is_native_raw_direction(direction: Direction) -> bool {
    matches!(
        direction,
        Direction::Request | Direction::Response | Direction::Stream
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_catalog_validates_a_complete_sixteen_route_matrix() {
        let registry = validated_current_registry().expect("runtime catalog validates");
        assert_eq!(registry.support_matrix().routes.len(), 16);
    }

    #[test]
    fn current_catalog_has_only_four_native_raw_runtime_routes() {
        let catalog = current_runtime_catalog();
        assert_eq!(catalog.routes.len(), 4);
        assert!(catalog.routes.iter().all(|route| {
            route.source == route.target
                && route.request_converter_id.is_some()
                && route.response_converter_id.is_some()
                && route.stream_converter_id.is_some()
                && route.stream_finalizer_id.is_some()
        }));
    }

    #[test]
    fn openai_chat_to_responses_is_unsupported_without_a_runtime_adaptor() {
        let registry = validated_current_registry().expect("runtime catalog validates");
        let route = registry
            .route(Protocol::OpenAi, Protocol::OpenAiResponses)
            .expect("complete matrix route");
        assert!(!route.request_supported);
        assert!(!route.response_supported);
        assert!(!route.stream_supported);
    }

    #[test]
    fn responses_to_claude_and_gemini_to_claude_are_unsupported() {
        let registry = validated_current_registry().expect("runtime catalog validates");
        for (source, target) in [
            (Protocol::OpenAiResponses, Protocol::Claude),
            (Protocol::Gemini, Protocol::Claude),
        ] {
            let route = registry
                .route(source, target)
                .expect("complete matrix route");
            assert!(!route.supports_any_direction());
        }
    }
}
