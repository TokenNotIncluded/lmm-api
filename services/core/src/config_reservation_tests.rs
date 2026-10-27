//! Parser/startup checks for the host-only reservation profile.
use super::*;
use serde_json::{Value, json};

// @context-fold begin host-reservation-profile-config-tests
fn document() -> Value {
    json!({
        "version":1,
        "routing":{"version":1,"groups":[],"upstreams":[],"routes":[],"aliases":[]},
        "prices":{"version":1,"rules":[]},
        "credentials":[],"key_groups":[],"input_token_ceiling":1024,
        "lease_seconds":20,"timeout_seconds":3,"max_output_tokens":128,
        "external_models":[{
            "upstream_id":1,"upstream_model":"gpt-4.1-2025-04-14",
            "protocol":"openai_responses","endpoint":"https://api.openai.com/v1",
            "max_input_tokens":1024,"max_output_tokens":128,
            "contract_revision":"synthetic-reviewed-count-output-cache-rates-v1"
        }]
    })
}

#[test]
fn input_count_profile_default_exact_and_strict_tagged_union() {
    let default: ModelConfig = serde_json::from_value(document()).unwrap();
    crate::model_http::ProviderPolicy::new(default.external_models, 1024).unwrap();
    let mut exact = document();
    exact["external_models"][0]["reservation"] =
        json!({"mode":"openai_responses_exact_v1","count_timeout_ms":2000});
    let config: ModelConfig = serde_json::from_value(exact).unwrap();
    crate::model_http::ProviderPolicy::new(config.external_models, 1024).unwrap();
    for profile in [
        json!({"mode":"unknown"}),
        json!({"mode":"context_max","count_timeout_ms":2000}),
        json!({"mode":"context_max","counter_url":"https://api.openai.com"}),
        json!({"mode":"openai_responses_exact_v1"}),
        json!({"mode":"openai_responses_exact_v1","count_timeout_ms":0}),
        json!({"mode":"openai_responses_exact_v1","count_timeout_ms":-1}),
        json!({"mode":"openai_responses_exact_v1","count_timeout_ms":1.5}),
        json!({"mode":"openai_responses_exact_v1","count_timeout_ms":1,"extra":true}),
    ] {
        let mut invalid = document();
        invalid["external_models"][0]["reservation"] = profile;
        assert!(serde_json::from_value::<ModelConfig>(invalid).is_err());
    }
}

#[test]
fn input_count_profile_rejects_alias_third_party_or_different_protocol_at_startup() {
    for (field, value) in [
        ("upstream_model", "gpt-4.1"),
        ("upstream_model", "gpt-4.1-latest"),
        ("endpoint", "https://third-party.example/v1"),
        ("endpoint", "https://api.openai.com/other"),
        ("endpoint", "https://api.openai.com/v1/responses"),
        ("endpoint", "http://api.openai.com/v1"),
        ("protocol", "openai_chat"),
    ] {
        let mut invalid = document();
        invalid["external_models"][0]["reservation"] =
            json!({"mode":"openai_responses_exact_v1","count_timeout_ms":2000});
        invalid["external_models"][0][field] = json!(value);
        let config: ModelConfig = serde_json::from_value(invalid).unwrap();
        assert!(
            crate::model_http::ProviderPolicy::new(config.external_models, 1024).is_err(),
            "{field}"
        );
    }
}
// @context-fold end host-reservation-profile-config-tests
