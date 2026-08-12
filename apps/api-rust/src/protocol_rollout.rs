//! Pure rollout controls for the protocol-conversion migration.
//!
//! This module deliberately stops at decision boundaries. It does not own a
//! route, an HTTP client, a provider response, or a deployment switch. A
//! caller can therefore compile one decision before a request/stream starts,
//! run two local conversion summaries for shadow comparison, and evaluate
//! rollback telemetry without ever giving this API a way to repeat an
//! upstream call.

use std::{collections::BTreeMap, env, error::Error, fmt};

use lmm_contracts::relay::{
    Feature as RelayFeature, LossCode, LossPolicy, Protocol, SyntheticField,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum rollout percentage expressed in basis points.
pub const MAX_BASIS_POINTS: u16 = 10_000;

/// Response parsing error-rate increase that pauses a rollout, in percentage
/// points, matching the PLAN starting threshold.
pub const PARSE_ERROR_RATE_PAUSE_PERCENTAGE_POINTS: f64 = 0.1;

/// Gateway TTFT p95 increase that pauses a rollout, in percent.
pub const TTFT_P95_PAUSE_PERCENT: f64 = 10.0;

const ROLLOUT_HASH_DOMAIN: &[u8] = b"lmm-protocol-rollout-v1\0";

fn is_false(value: &bool) -> bool {
    !*value
}

/// PLAN's ordered canary stages.  Values are basis points, never floats, so
/// traffic selection has one exact representation across configuration and
/// metrics.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryStage {
    /// Internal test channel only; no public allocation.
    Internal,
    /// One percent plain-text canary.
    TextOnePercent,
    /// Five percent text/image canary.
    TextImageFivePercent,
    /// Five percent single-function-call canary.
    SingleFunctionFivePercent,
    /// Five percent parallel-function-call canary.
    ParallelFunctionFivePercent,
    /// Twenty-five percent full compatibility canary.
    FullFeatureTwentyFivePercent,
    /// Full allocation after all gates pass.
    FullTraffic,
}

impl CanaryStage {
    /// Stable PLAN order.
    pub const ALL: [Self; 7] = [
        Self::Internal,
        Self::TextOnePercent,
        Self::TextImageFivePercent,
        Self::SingleFunctionFivePercent,
        Self::ParallelFunctionFivePercent,
        Self::FullFeatureTwentyFivePercent,
        Self::FullTraffic,
    ];

    /// Returns the minimum allocation represented by this stage.
    #[must_use]
    pub const fn minimum_basis_points(self) -> u16 {
        match self {
            Self::Internal => 0,
            Self::TextOnePercent => 100,
            Self::TextImageFivePercent
            | Self::SingleFunctionFivePercent
            | Self::ParallelFunctionFivePercent => 500,
            Self::FullFeatureTwentyFivePercent => 2_500,
            Self::FullTraffic => MAX_BASIS_POINTS,
        }
    }

    /// Returns whether an allocation has reached this stage.
    #[must_use]
    pub const fn accepts(self, canary_basis_points: u16) -> bool {
        canary_basis_points >= self.minimum_basis_points()
            && canary_basis_points <= MAX_BASIS_POINTS
    }
}

/// Validates a requested canary allocation against a named rollout stage.
pub fn validate_canary_stage(
    stage: CanaryStage,
    canary_basis_points: u16,
) -> Result<(), RolloutConfigError> {
    validate_basis_points(canary_basis_points, "canary stage")?;
    if stage.accepts(canary_basis_points) {
        Ok(())
    } else {
        Err(RolloutConfigError::CanaryBelowStage {
            stage,
            canary_basis_points,
        })
    }
}

/// Stable feature switches described by PLAN ROLL-001.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutFlag {
    /// Select the v2 conversion engine.
    ConversionEngineV2,
    /// Select the loss handling policy.
    ConversionLossPolicy,
    /// Preserve Gemini function-call identifiers in v2 routes.
    GeminiFunctionIdV2,
    /// Preserve Gemini thought signatures in v2 routes.
    GeminiThoughtSignatureV2,
    /// Preserve opaque Claude thinking in v2 routes.
    ClaudeOpaqueThinkingV2,
    /// Select the complete-frame SSE parser.
    SseParserV2,
    /// Apply route-specific converter overrides.
    ConverterPairOverrides,
}

impl RolloutFlag {
    /// Every flag in the stable PLAN order.
    pub const ALL: [Self; 7] = [
        Self::ConversionEngineV2,
        Self::ConversionLossPolicy,
        Self::GeminiFunctionIdV2,
        Self::GeminiThoughtSignatureV2,
        Self::ClaudeOpaqueThinkingV2,
        Self::SseParserV2,
        Self::ConverterPairOverrides,
    ];

    /// Returns every rollout flag in stable declaration order.
    pub const fn all() -> &'static [Self] {
        &Self::ALL
    }

    /// Returns the environment-variable suffix for a flag.
    pub const fn env_suffix(self) -> &'static str {
        match self {
            Self::ConversionEngineV2 => "CONVERSION_ENGINE_V2",
            Self::ConversionLossPolicy => "CONVERSION_LOSS_POLICY",
            Self::GeminiFunctionIdV2 => "GEMINI_FUNCTION_ID_V2",
            Self::GeminiThoughtSignatureV2 => "GEMINI_THOUGHT_SIGNATURE_V2",
            Self::ClaudeOpaqueThinkingV2 => "CLAUDE_OPAQUE_THINKING_V2",
            Self::SseParserV2 => "SSE_PARSER_V2",
            Self::ConverterPairOverrides => "CONVERTER_PAIR_OVERRIDES",
        }
    }

    /// Returns whether this switch controls a v2 implementation.
    #[must_use]
    pub const fn is_v2(self) -> bool {
        !matches!(self, Self::ConversionLossPolicy)
    }
}

/// One deterministic rollout switch and its canary percentage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagConfig {
    /// Whether the switch is eligible for rollout.
    pub enabled: bool,
    /// Stable canary allocation in basis points, from 0 to 10,000.
    pub canary_basis_points: u16,
    /// Dimension-specific overrides, ordered by declaration for stable ties.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<FlagOverride>,
}

impl Default for FlagConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            canary_basis_points: 0,
            overrides: Vec::new(),
        }
    }
}

impl FlagConfig {
    /// Creates a disabled v1/default configuration.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            canary_basis_points: 0,
            overrides: Vec::new(),
        }
    }

    /// Creates an enabled configuration with a validated canary allocation.
    pub fn enabled(canary_basis_points: u16) -> Result<Self, RolloutConfigError> {
        validate_basis_points(canary_basis_points, "rollout flag")?;
        Ok(Self {
            enabled: true,
            canary_basis_points,
            overrides: Vec::new(),
        })
    }

    /// Adds one dimension-specific override after validating its allocation.
    pub fn push_override(&mut self, override_rule: FlagOverride) -> Result<(), RolloutConfigError> {
        override_rule.validate()?;
        self.overrides.push(override_rule);
        Ok(())
    }
}

/// Dimensions used to select a rollout decision.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolloutSelector {
    /// Optional channel, such as an internal or canary channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Optional source protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Protocol>,
    /// Optional target protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Protocol>,
    /// Optional model family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_family: Option<String>,
    /// Optional stream/non-stream selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

impl RolloutSelector {
    /// Returns whether this selector matches a request context.
    pub fn matches(&self, context: &RolloutContext<'_>) -> bool {
        self.channel.as_deref().is_none_or(|value| {
            context
                .channel
                .is_some_and(|actual| value.eq_ignore_ascii_case(actual))
        }) && self.source.is_none_or(|value| value == context.source)
            && self.target.is_none_or(|value| value == context.target)
            && self
                .model_family
                .as_deref()
                .is_none_or(|value| value.eq_ignore_ascii_case(context.model_family))
            && self.stream.is_none_or(|value| value == context.stream)
    }

    /// Returns how specific this selector is for deterministic precedence.
    pub const fn specificity(&self) -> u8 {
        self.channel.is_some() as u8
            + self.source.is_some() as u8
            + self.target.is_some() as u8
            + self.model_family.is_some() as u8
            + self.stream.is_some() as u8
    }

    fn validate(&self) -> Result<(), RolloutConfigError> {
        if self
            .channel
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
            || self
                .model_family
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err(RolloutConfigError::InvalidSelector);
        }
        Ok(())
    }
}

/// One dimension-specific value for a flag.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagOverride {
    /// Dimensions that must match.
    pub selector: RolloutSelector,
    /// Whether this matching scope is eligible for rollout.
    pub enabled: bool,
    /// Canary allocation for this scope.
    pub canary_basis_points: u16,
}

impl FlagOverride {
    /// Creates a validated override.
    pub fn new(
        selector: RolloutSelector,
        enabled: bool,
        canary_basis_points: u16,
    ) -> Result<Self, RolloutConfigError> {
        let value = Self {
            selector,
            enabled,
            canary_basis_points,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), RolloutConfigError> {
        self.selector.validate()?;
        validate_basis_points(self.canary_basis_points, "flag override")?;
        if !self.enabled && self.canary_basis_points != 0 {
            return Err(RolloutConfigError::DisabledWithNonzeroCanary);
        }
        Ok(())
    }
}

/// A pair-specific override for one rollout switch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConverterPairOverride {
    /// Flag controlled by this pair override.
    pub flag: RolloutFlag,
    /// Source protocol selected by the override.
    pub source: Protocol,
    /// Target protocol selected by the override.
    pub target: Protocol,
    /// Optional channel selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Optional model-family selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_family: Option<String>,
    /// Optional stream/non-stream selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Whether this route scope is eligible for rollout.
    pub enabled: bool,
    /// Canary allocation for this route scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_basis_points: Option<u16>,
}

impl ConverterPairOverride {
    /// Returns this pair override as a regular selector.
    pub fn selector(&self) -> RolloutSelector {
        RolloutSelector {
            channel: self.channel.clone(),
            source: Some(self.source),
            target: Some(self.target),
            model_family: self.model_family.clone(),
            stream: self.stream,
        }
    }

    /// Returns the effective canary value, defaulting explicit enablement to
    /// full allocation and explicit disablement to zero.
    pub fn effective_basis_points(&self) -> Result<u16, RolloutConfigError> {
        let basis_points =
            self.canary_basis_points
                .unwrap_or(if self.enabled { MAX_BASIS_POINTS } else { 0 });
        validate_basis_points(basis_points, "converter pair override")?;
        if !self.enabled && basis_points != 0 {
            return Err(RolloutConfigError::DisabledWithNonzeroCanary);
        }
        Ok(basis_points)
    }

    fn validate(&self) -> Result<(), RolloutConfigError> {
        self.selector().validate()?;
        self.effective_basis_points().map(|_| ())
    }
}

/// The complete, typed protocol rollout configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRolloutConfig {
    /// Emergency configuration-only rollback switch.
    ///
    /// When set, every v2 flag and pair override is fail-closed before a
    /// request can be selected. This is intentionally configuration data, so
    /// disabling v2 does not require rebuilding conversion code; installing a
    /// changed value in a live selector remains the caller's responsibility.
    #[serde(default, skip_serializing_if = "is_false")]
    pub rollback: bool,
    /// Conversion-engine v2 rollout.
    pub conversion_engine_v2: FlagConfig,
    /// Loss policy applied by conversion planning.
    pub conversion_loss_policy: LossPolicy,
    /// Gemini function-call ID rollout.
    pub gemini_function_id_v2: FlagConfig,
    /// Gemini thought-signature rollout.
    pub gemini_thought_signature_v2: FlagConfig,
    /// Claude opaque-thinking rollout.
    pub claude_opaque_thinking_v2: FlagConfig,
    /// Complete-frame SSE parser rollout.
    pub sse_parser_v2: FlagConfig,
    /// Route-specific converter overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub converter_pair_overrides: Vec<ConverterPairOverride>,
}

impl Default for ProtocolRolloutConfig {
    fn default() -> Self {
        Self {
            rollback: false,
            conversion_engine_v2: FlagConfig::default(),
            conversion_loss_policy: LossPolicy::Reject,
            gemini_function_id_v2: FlagConfig::default(),
            gemini_thought_signature_v2: FlagConfig::default(),
            claude_opaque_thinking_v2: FlagConfig::default(),
            sse_parser_v2: FlagConfig::default(),
            converter_pair_overrides: Vec::new(),
        }
    }
}

impl ProtocolRolloutConfig {
    /// Reads rollout flags from environment variables.
    ///
    /// Missing variables produce the disabled v1 configuration. Malformed
    /// values return an error rather than widening traffic.
    pub fn from_env() -> Result<Self, RolloutConfigError> {
        let rollback = parse_rollback_env()?;
        if rollback {
            // An emergency rollback must remain usable even when a stale v2
            // variable is malformed.  The safe default is reject-loss and no
            // v2 allocation; no malformed value can widen traffic here.
            return Ok(Self {
                rollback: true,
                ..Self::default()
            });
        }
        let config = Self {
            rollback,
            conversion_engine_v2: parse_flag_env(
                "LMM_CONVERSION_ENGINE_V2",
                "LMM_CONVERSION_ENGINE_V2_CANARY_BPS",
            )?,
            conversion_loss_policy: parse_loss_policy_env()?,
            gemini_function_id_v2: parse_flag_env(
                "LMM_GEMINI_FUNCTION_ID_V2",
                "LMM_GEMINI_FUNCTION_ID_V2_CANARY_BPS",
            )?,
            gemini_thought_signature_v2: parse_flag_env(
                "LMM_GEMINI_THOUGHT_SIGNATURE_V2",
                "LMM_GEMINI_THOUGHT_SIGNATURE_V2_CANARY_BPS",
            )?,
            claude_opaque_thinking_v2: parse_flag_env(
                "LMM_CLAUDE_OPAQUE_THINKING_V2",
                "LMM_CLAUDE_OPAQUE_THINKING_V2_CANARY_BPS",
            )?,
            sse_parser_v2: parse_flag_env("LMM_SSE_PARSER_V2", "LMM_SSE_PARSER_V2_CANARY_BPS")?,
            converter_pair_overrides: parse_pair_overrides()?,
        };
        for override_rule in &config.converter_pair_overrides {
            override_rule.validate()?;
        }
        Ok(config)
    }

    /// Returns the boolean flag configuration for one switch.
    ///
    /// The loss-policy flag is read through [`Self::loss_policy`], while pair
    /// overrides are selected from [`Self::converter_pair_overrides`]. Those
    /// value-shaped controls intentionally do not pretend to be booleans.
    pub fn flag(&self, flag: RolloutFlag) -> Option<&FlagConfig> {
        match flag {
            RolloutFlag::ConversionEngineV2 => Some(&self.conversion_engine_v2),
            RolloutFlag::ConversionLossPolicy => None,
            RolloutFlag::GeminiFunctionIdV2 => Some(&self.gemini_function_id_v2),
            RolloutFlag::GeminiThoughtSignatureV2 => Some(&self.gemini_thought_signature_v2),
            RolloutFlag::ClaudeOpaqueThinkingV2 => Some(&self.claude_opaque_thinking_v2),
            RolloutFlag::SseParserV2 => Some(&self.sse_parser_v2),
            RolloutFlag::ConverterPairOverrides => None,
        }
    }

    /// Returns the configured loss policy.
    pub const fn loss_policy(&self) -> LossPolicy {
        self.conversion_loss_policy
    }

    /// Returns whether the configuration has disabled all v2 controls.
    #[must_use]
    pub const fn rollback_enabled(&self) -> bool {
        self.rollback
    }

    /// Returns a fail-closed copy after a correctness or operational gate
    /// fires. The caller remains responsible for installing this updated
    /// configuration in the live request selector.
    #[must_use]
    pub fn rolled_back(&self) -> Self {
        let mut rolled_back = self.clone();
        rolled_back.disable_v2_controls();
        rolled_back
    }

    /// Applies a rollback decision to this configuration.
    ///
    /// `pause` deliberately leaves the active allocation unchanged: a pause
    /// stops expansion, while `disable` closes v2 decisions in this value.
    /// The method has no external side effects or hot-reload behavior.
    pub fn apply_rollback(&mut self, decision: &RollbackDecision) {
        if decision.should_disable() {
            self.disable_v2_controls();
        }
    }

    fn disable_v2_controls(&mut self) {
        self.rollback = true;
        self.conversion_engine_v2 = FlagConfig::disabled();
        self.gemini_function_id_v2 = FlagConfig::disabled();
        self.gemini_thought_signature_v2 = FlagConfig::disabled();
        self.claude_opaque_thinking_v2 = FlagConfig::disabled();
        self.sse_parser_v2 = FlagConfig::disabled();
        self.converter_pair_overrides.clear();
    }

    /// Adds a pair override after validation.
    pub fn push_pair_override(
        &mut self,
        override_rule: ConverterPairOverride,
    ) -> Result<(), RolloutConfigError> {
        override_rule.validate()?;
        self.converter_pair_overrides.push(override_rule);
        Ok(())
    }

    /// Decides one flag for a stable request context.
    pub fn decide(&self, flag: RolloutFlag, context: &RolloutContext<'_>) -> FlagDecision {
        let bucket = stable_bucket(context.request_key);
        if self.rollback {
            return FlagDecision::disabled(flag, bucket, DecisionSource::ConfigRollback);
        }
        if context.request_key.is_empty() {
            return FlagDecision::disabled(flag, bucket, DecisionSource::EmptyRequestKey);
        }

        if flag == RolloutFlag::ConverterPairOverrides {
            let Some((index, pair_override)) = self
                .converter_pair_overrides
                .iter()
                .enumerate()
                .filter(|(_, value)| value.selector().matches(context))
                .max_by_key(|(index, value)| (value.selector().specificity(), *index))
            else {
                return FlagDecision::disabled(flag, bucket, DecisionSource::DefaultV1);
            };
            let Ok(basis_points) = pair_override.effective_basis_points() else {
                return FlagDecision::disabled(
                    flag,
                    bucket,
                    DecisionSource::ConverterPairOverride(index),
                );
            };
            return FlagDecision::from_config(
                flag,
                true,
                basis_points,
                bucket,
                DecisionSource::ConverterPairOverride(index),
            );
        }

        if let Some((index, pair_override)) = self
            .converter_pair_overrides
            .iter()
            .enumerate()
            .filter(|(_, value)| value.flag == flag && value.selector().matches(context))
            .max_by_key(|(index, value)| (value.selector().specificity(), *index))
        {
            let Ok(basis_points) = pair_override.effective_basis_points() else {
                return FlagDecision::disabled(
                    flag,
                    bucket,
                    DecisionSource::ConverterPairOverride(index),
                );
            };
            return FlagDecision::from_config(
                flag,
                pair_override.enabled,
                basis_points,
                bucket,
                DecisionSource::ConverterPairOverride(index),
            );
        }

        let Some(base) = self.flag(flag) else {
            return FlagDecision::disabled(flag, bucket, DecisionSource::DefaultV1);
        };
        if let Some((index, override_rule)) = base
            .overrides
            .iter()
            .enumerate()
            .filter(|(_, value)| value.selector.matches(context))
            .max_by_key(|(index, value)| (value.selector.specificity(), *index))
        {
            return FlagDecision::from_config(
                flag,
                override_rule.enabled,
                override_rule.canary_basis_points,
                bucket,
                DecisionSource::DimensionOverride(index),
            );
        }
        FlagDecision::from_config(
            flag,
            base.enabled,
            base.canary_basis_points,
            bucket,
            DecisionSource::BaseConfig,
        )
    }

    /// Returns whether one flag is active for a stable request context.
    pub fn is_enabled(&self, flag: RolloutFlag, context: &RolloutContext<'_>) -> bool {
        self.decide(flag, context).enabled
    }
}

/// Request dimensions used by deterministic rollout decisions.
#[derive(Clone, Copy, Debug)]
pub struct RolloutContext<'a> {
    /// Stable request key; no process-random state is used.
    pub request_key: &'a str,
    /// Optional channel label.
    pub channel: Option<&'a str>,
    /// Incoming protocol.
    pub source: Protocol,
    /// Upstream protocol.
    pub target: Protocol,
    /// Normalized model-family label.
    pub model_family: &'a str,
    /// Whether this is a streaming request.
    pub stream: bool,
}

impl<'a> RolloutContext<'a> {
    /// Creates a context with only the required stable dimensions.
    pub const fn new(
        request_key: &'a str,
        source: Protocol,
        target: Protocol,
        model_family: &'a str,
        stream: bool,
    ) -> Self {
        Self {
            request_key,
            channel: None,
            source,
            target,
            model_family,
            stream,
        }
    }

    /// Sets an optional channel for selector matching.
    pub const fn with_channel(mut self, channel: &'a str) -> Self {
        self.channel = Some(channel);
        self
    }
}

/// Why a flag decision won.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// No rollout configuration was supplied.
    DefaultV1,
    /// Base flag configuration.
    BaseConfig,
    /// A dimension-specific flag override.
    DimensionOverride(usize),
    /// A converter-pair override.
    ConverterPairOverride(usize),
    /// Empty request keys fail closed.
    EmptyRequestKey,
    /// An emergency configuration-only rollback is active.
    ConfigRollback,
}

/// One deterministic flag decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FlagDecision {
    /// Flag being evaluated.
    pub flag: RolloutFlag,
    /// Effective decision after canary bucketing.
    pub enabled: bool,
    /// Stable bucket in the range 0..10,000.
    pub bucket: u16,
    /// Allocation used for this decision.
    pub canary_basis_points: u16,
    /// Rule that supplied the decision.
    pub source: DecisionSource,
}

impl FlagDecision {
    fn from_config(
        flag: RolloutFlag,
        eligible: bool,
        canary_basis_points: u16,
        bucket: u16,
        source: DecisionSource,
    ) -> Self {
        Self {
            flag,
            enabled: eligible && bucket_is_in_rollout(bucket, canary_basis_points),
            bucket,
            canary_basis_points,
            source,
        }
    }

    fn disabled(flag: RolloutFlag, bucket: u16, source: DecisionSource) -> Self {
        Self {
            flag,
            enabled: false,
            bucket,
            canary_basis_points: 0,
            source,
        }
    }
}

/// Computes a deterministic bucket from a request key.
pub fn stable_bucket(request_key: &str) -> u16 {
    let mut hasher = Sha256::new();
    hasher.update(ROLLOUT_HASH_DOMAIN);
    hasher.update(request_key.as_bytes());
    let digest = hasher.finalize();
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(prefix) % u64::from(MAX_BASIS_POINTS)) as u16
}

/// Returns whether a bucket is included in a basis-point allocation.
pub const fn bucket_is_in_rollout(bucket: u16, canary_basis_points: u16) -> bool {
    bucket < canary_basis_points && canary_basis_points <= MAX_BASIS_POINTS
}

/// Parses a strict boolean environment value.
pub fn parse_boolean(value: &str, name: &'static str) -> Result<bool, RolloutConfigError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(RolloutConfigError::InvalidBoolean { name }),
    }
}

/// Parses a bounded basis-point value.
pub fn parse_basis_points(value: &str, name: &'static str) -> Result<u16, RolloutConfigError> {
    let parsed = value
        .parse::<u16>()
        .map_err(|_| RolloutConfigError::InvalidBasisPoints { name })?;
    validate_basis_points(parsed, name)?;
    Ok(parsed)
}

fn validate_basis_points(value: u16, name: &'static str) -> Result<(), RolloutConfigError> {
    if value > MAX_BASIS_POINTS {
        Err(RolloutConfigError::InvalidBasisPoints { name })
    } else {
        Ok(())
    }
}

fn parse_flag_env(
    enabled_name: &'static str,
    basis_points_name: &'static str,
) -> Result<FlagConfig, RolloutConfigError> {
    let enabled = match env::var(enabled_name) {
        Ok(value) => parse_boolean(&value, enabled_name)?,
        Err(env::VarError::NotPresent) => false,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(RolloutConfigError::InvalidBoolean { name: enabled_name });
        }
    };
    let basis_points = match env::var(basis_points_name) {
        Ok(value) => parse_basis_points(&value, basis_points_name)?,
        Err(env::VarError::NotPresent) if enabled => MAX_BASIS_POINTS,
        Err(env::VarError::NotPresent) => 0,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(RolloutConfigError::InvalidBasisPoints {
                name: basis_points_name,
            });
        }
    };
    if !enabled && basis_points != 0 {
        return Err(RolloutConfigError::DisabledWithNonzeroCanary);
    }
    Ok(FlagConfig {
        enabled,
        canary_basis_points: basis_points,
        overrides: Vec::new(),
    })
}

fn parse_loss_policy_env() -> Result<LossPolicy, RolloutConfigError> {
    match env::var("LMM_CONVERSION_LOSS_POLICY") {
        Ok(value) => parse_loss_policy(&value),
        Err(env::VarError::NotPresent) => Ok(LossPolicy::Reject),
        Err(env::VarError::NotUnicode(_)) => Err(RolloutConfigError::InvalidLossPolicy),
    }
}

fn parse_rollback_env() -> Result<bool, RolloutConfigError> {
    match env::var("LMM_PROTOCOL_ROLLBACK") {
        Ok(value) => parse_boolean(&value, "LMM_PROTOCOL_ROLLBACK"),
        Err(env::VarError::NotPresent) => Ok(false),
        Err(env::VarError::NotUnicode(_)) => Err(RolloutConfigError::InvalidBoolean {
            name: "LMM_PROTOCOL_ROLLBACK",
        }),
    }
}

/// Parses the administrator loss policy.
pub fn parse_loss_policy(value: &str) -> Result<LossPolicy, RolloutConfigError> {
    match value {
        "reject" => Ok(LossPolicy::Reject),
        "warn" => Ok(LossPolicy::Warn),
        "allow" => Ok(LossPolicy::Allow),
        _ => Err(RolloutConfigError::InvalidLossPolicy),
    }
}

fn parse_pair_overrides() -> Result<Vec<ConverterPairOverride>, RolloutConfigError> {
    let value = match env::var("LMM_CONVERTER_PAIR_OVERRIDES") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok(Vec::new()),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(RolloutConfigError::InvalidPairOverrides);
        }
    };
    let overrides = serde_json::from_str::<Vec<ConverterPairOverride>>(&value)
        .map_err(|_| RolloutConfigError::InvalidPairOverrides)?;
    for override_rule in &overrides {
        override_rule.validate()?;
    }
    Ok(overrides)
}

/// Categories emitted by local shadow comparison.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowDifference {
    /// Compiled conversion plans differ.
    Plan,
    /// Semantic output summaries differ.
    Semantic,
    /// Loss ledgers differ.
    LossLedger,
    /// Synthetic-field summaries differ.
    SyntheticFields,
    /// Converter identifiers differ.
    ConverterId,
    /// One local converter failed.
    ConversionFailure,
}

/// A request view passed to local converters.
///
/// The body is intentionally private and has no Debug/Serialize implementation
/// so shadow records cannot accidentally retain or log sensitive prompt data.
pub struct LocalRequest<'a> {
    body: &'a [u8],
}

impl<'a> LocalRequest<'a> {
    /// Creates a local conversion input view.
    pub const fn new(body: &'a [u8]) -> Self {
        Self { body }
    }

    /// Returns the input size without exposing its contents.
    pub const fn len(&self) -> usize {
        self.body.len()
    }

    /// Provides the body only to the local converter implementation.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.body
    }
}

/// A body-free summary produced by one local converter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalConversionSummary {
    /// Converter implementation identifier.
    pub converter_id: String,
    /// Hash of the compiled plan/route semantics.
    pub plan_fingerprint: [u8; 32],
    /// Hash of the normalized semantic output.
    pub semantic_fingerprint: [u8; 32],
    /// Loss codes observed by the local conversion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<LossCode>,
    /// Synthetic fields observed by the local conversion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synthetic: Vec<SyntheticField>,
}

/// Error categories safe to retain in a shadow record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalConversionErrorKind {
    /// Input could not be decoded.
    InvalidInput,
    /// This local converter does not support the input.
    Unsupported,
    /// A local conversion invariant failed.
    Internal,
}

/// Body-free local conversion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalConversionError {
    /// Safe failure category.
    pub kind: LocalConversionErrorKind,
}

/// A converter API intentionally limited to local conversion work.
pub trait LocalConverter {
    /// Converts one request locally and returns only a semantic summary.
    fn convert_local(
        &self,
        request: &LocalRequest<'_>,
    ) -> Result<LocalConversionSummary, LocalConversionError>;
}

impl<F> LocalConverter for F
where
    F: Fn(&LocalRequest<'_>) -> Result<LocalConversionSummary, LocalConversionError>,
{
    fn convert_local(
        &self,
        request: &LocalRequest<'_>,
    ) -> Result<LocalConversionSummary, LocalConversionError> {
        self(request)
    }
}

/// Body-free shadow comparison result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowRecord {
    /// Source protocol of the compared route.
    pub source: Protocol,
    /// Target protocol of the compared route.
    pub target: Protocol,
    /// Whether the compared route is streaming.
    pub stream: bool,
    /// Old converter identifier, when conversion succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_converter_id: Option<String>,
    /// New converter identifier, when conversion succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_converter_id: Option<String>,
    /// Aggregatable difference categories; no body or prompt is retained.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub differences: Vec<ShadowDifference>,
}

impl ShadowRecord {
    /// Returns whether both local converters produced equivalent summaries.
    pub fn is_identical(&self) -> bool {
        self.differences.is_empty()
    }
}

/// Runs old and new local converters once each for one request.
pub struct ShadowRunner<Old, New> {
    old: Old,
    new: New,
    source: Protocol,
    target: Protocol,
    stream: bool,
}

impl<Old, New> ShadowRunner<Old, New>
where
    Old: LocalConverter,
    New: LocalConverter,
{
    /// Creates a runner that has no upstream client or upstream-call method.
    pub const fn new(old: Old, new: New, source: Protocol, target: Protocol, stream: bool) -> Self {
        Self {
            old,
            new,
            source,
            target,
            stream,
        }
    }

    /// Executes exactly one local conversion per implementation and compares
    /// body-free summaries.
    pub fn compare(&self, request: &LocalRequest<'_>) -> ShadowRecord {
        let old_result = self.old.convert_local(request);
        let new_result = self.new.convert_local(request);
        compare_local_results(
            self.source,
            self.target,
            self.stream,
            old_result,
            new_result,
        )
    }
}

fn compare_local_results(
    source: Protocol,
    target: Protocol,
    stream: bool,
    old_result: Result<LocalConversionSummary, LocalConversionError>,
    new_result: Result<LocalConversionSummary, LocalConversionError>,
) -> ShadowRecord {
    let old_converter_id = old_result
        .as_ref()
        .ok()
        .map(|value| value.converter_id.clone());
    let new_converter_id = new_result
        .as_ref()
        .ok()
        .map(|value| value.converter_id.clone());
    let mut differences = Vec::new();
    match (&old_result, &new_result) {
        (Ok(old), Ok(new)) => {
            if old.plan_fingerprint != new.plan_fingerprint {
                differences.push(ShadowDifference::Plan);
            }
            if old.semantic_fingerprint != new.semantic_fingerprint {
                differences.push(ShadowDifference::Semantic);
            }
            if old.losses != new.losses {
                differences.push(ShadowDifference::LossLedger);
            }
            if old.synthetic != new.synthetic {
                differences.push(ShadowDifference::SyntheticFields);
            }
            if old.converter_id != new.converter_id {
                differences.push(ShadowDifference::ConverterId);
            }
        }
        (Err(_), Err(_)) if old_result == new_result => {}
        _ => differences.push(ShadowDifference::ConversionFailure),
    }
    ShadowRecord {
        source,
        target,
        stream,
        old_converter_id,
        new_converter_id,
        differences,
    }
}

/// Aggregated shadow counters suitable for metrics.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowAggregate {
    /// Number of compared requests.
    pub total: u64,
    /// Number of equivalent comparisons.
    pub identical: u64,
    /// Difference count by safe category.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub differences: BTreeMap<ShadowDifference, u64>,
    /// Per-route counters with only source/target/stream dimensions.
    ///
    /// Keeping this key closed makes shadow telemetry useful for canary
    /// decisions without retaining a request key, model name, or body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ShadowRouteAggregateEntry>,
}

/// Closed route key for body-free shadow aggregation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowRouteKey {
    /// Source protocol.
    pub source: Protocol,
    /// Target protocol.
    pub target: Protocol,
    /// Whether the route is streaming.
    pub stream: bool,
}

impl Ord for ShadowRouteKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        route_protocol_rank(self.source)
            .cmp(&route_protocol_rank(other.source))
            .then_with(|| route_protocol_rank(self.target).cmp(&route_protocol_rank(other.target)))
            .then_with(|| self.stream.cmp(&other.stream))
    }
}

impl PartialOrd for ShadowRouteKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Body-free totals for one shadow route.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowRouteAggregate {
    /// Number of comparisons for this route.
    pub total: u64,
    /// Number of identical old/new summaries.
    pub identical: u64,
    /// Difference count by closed category.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub differences: BTreeMap<ShadowDifference, u64>,
}

/// One serializable route key and its body-free shadow totals.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowRouteAggregateEntry {
    /// Closed route dimensions.
    pub route: ShadowRouteKey,
    /// Aggregated comparison totals.
    pub aggregate: ShadowRouteAggregate,
}

impl ShadowRouteAggregate {
    fn record(&mut self, comparison: &ShadowRecord) {
        self.total = self.total.saturating_add(1);
        if comparison.is_identical() {
            self.identical = self.identical.saturating_add(1);
        }
        for category in &comparison.differences {
            let count = self.differences.entry(*category).or_default();
            *count = (*count).saturating_add(1);
        }
    }
}

impl ShadowAggregate {
    /// Records one body-free comparison.
    pub fn record(&mut self, comparison: &ShadowRecord) {
        self.total = self.total.saturating_add(1);
        if comparison.is_identical() {
            self.identical = self.identical.saturating_add(1);
        }
        for category in &comparison.differences {
            let count = self.differences.entry(*category).or_default();
            *count = (*count).saturating_add(1);
        }
        let route = ShadowRouteKey {
            source: comparison.source,
            target: comparison.target,
            stream: comparison.stream,
        };
        if let Some(entry) = self.routes.iter_mut().find(|entry| entry.route == route) {
            entry.aggregate.record(comparison);
        } else {
            let mut aggregate = ShadowRouteAggregate::default();
            aggregate.record(comparison);
            self.routes
                .push(ShadowRouteAggregateEntry { route, aggregate });
            self.routes.sort_by_key(|entry| entry.route);
        }
    }
}

fn route_protocol_rank(protocol: Protocol) -> u8 {
    match protocol {
        Protocol::OpenAi => 0,
        Protocol::OpenAiResponses => 1,
        Protocol::Claude => 2,
        Protocol::Gemini => 3,
    }
}

/// Immediate-close and pause conditions from PLAN ROLL-001.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackReason {
    /// Tool/signature-related upstream HTTP 400 rate increased.
    ToolSignature400RateIncreased,
    /// Authentic provider signature changed.
    SignatureModified,
    /// A semantic loss was silently dropped.
    SilentLoss,
    /// Usage or billing semantics differ.
    UsageBillingDifference,
    /// Response parsing error rate exceeded the pause threshold.
    ParseErrorRateExceeded,
    /// Gateway TTFT p95 exceeded the pause threshold.
    TtftP95Exceeded,
    /// SSE interruption rate was elevated.
    SseInterruptionRateElevated,
    /// Telemetry itself was invalid.
    InvalidMetric,
}

/// Telemetry and correctness signals used by pure rollback evaluation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollbackSignals {
    /// Tool/signature-related upstream HTTP 400 rate increased.
    pub tool_or_signature_400_rate_increased: bool,
    /// Authentic signature was modified.
    pub signature_modified: bool,
    /// A loss was observed without a ledger entry.
    pub silent_loss: bool,
    /// Usage or billing differed from the baseline.
    pub usage_billing_difference: bool,
    /// Response parse error-rate increase in percentage points.
    pub parse_error_rate_percentage_points: Option<f64>,
    /// Gateway TTFT p95 increase in percent.
    pub ttft_p95_increase_percent: Option<f64>,
    /// External monitor marked SSE interruptions as materially elevated.
    pub sse_interruption_rate_elevated: bool,
}

/// Action returned by rollback evaluation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackAction {
    /// No rollback condition is active.
    Continue,
    /// Pause further canary expansion while retaining current traffic.
    Pause,
    /// Immediately disable the v2 route/flag.
    Disable,
}

/// Pure rollback decision and all matching reasons.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RollbackDecision {
    /// Action that the caller should apply.
    pub action: RollbackAction,
    /// Stable reasons in declaration order.
    pub reasons: Vec<RollbackReason>,
}

impl RollbackDecision {
    /// Whether the decision immediately closes the rollout.
    pub const fn should_disable(&self) -> bool {
        matches!(self.action, RollbackAction::Disable)
    }

    /// Whether the decision pauses expansion.
    pub const fn should_pause(&self) -> bool {
        matches!(self.action, RollbackAction::Pause)
    }
}

/// Evaluates every PLAN rollback condition without mutating rollout state.
pub fn evaluate_rollback(signals: &RollbackSignals) -> RollbackDecision {
    let mut immediate = Vec::new();
    if signals.tool_or_signature_400_rate_increased {
        immediate.push(RollbackReason::ToolSignature400RateIncreased);
    }
    if signals.signature_modified {
        immediate.push(RollbackReason::SignatureModified);
    }
    if signals.usage_billing_difference {
        immediate.push(RollbackReason::UsageBillingDifference);
    }
    if signals.sse_interruption_rate_elevated {
        immediate.push(RollbackReason::SseInterruptionRateElevated);
    }

    let mut invalid_metric = false;
    let parse_exceeded = signals
        .parse_error_rate_percentage_points
        .is_some_and(|value| {
            if !value.is_finite() || value < 0.0 {
                invalid_metric = true;
                false
            } else {
                value > PARSE_ERROR_RATE_PAUSE_PERCENTAGE_POINTS
            }
        });
    let ttft_exceeded = signals.ttft_p95_increase_percent.is_some_and(|value| {
        if !value.is_finite() || value < 0.0 {
            invalid_metric = true;
            false
        } else {
            value > TTFT_P95_PAUSE_PERCENT
        }
    });
    if invalid_metric {
        immediate.push(RollbackReason::InvalidMetric);
    }
    if !immediate.is_empty() {
        return RollbackDecision {
            action: RollbackAction::Disable,
            reasons: immediate,
        };
    }

    let mut pause = Vec::new();
    if signals.silent_loss {
        pause.push(RollbackReason::SilentLoss);
    }
    if parse_exceeded {
        pause.push(RollbackReason::ParseErrorRateExceeded);
    }
    if ttft_exceeded {
        pause.push(RollbackReason::TtftP95Exceeded);
    }
    RollbackDecision {
        action: if pause.is_empty() {
            RollbackAction::Continue
        } else {
            RollbackAction::Pause
        },
        reasons: pause,
    }
}

/// Errors produced while parsing rollout configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloutConfigError {
    /// An environment boolean was not strict.
    InvalidBoolean { name: &'static str },
    /// A basis point value was out of range or malformed.
    InvalidBasisPoints { name: &'static str },
    /// A nonzero canary was paired with disabled traffic.
    DisabledWithNonzeroCanary,
    /// Loss policy was not reject, warn, or allow.
    InvalidLossPolicy,
    /// Pair override JSON was malformed.
    InvalidPairOverrides,
    /// Selector contained an empty dimension.
    InvalidSelector,
    /// A requested canary allocation has not reached its named stage.
    CanaryBelowStage {
        /// Required PLAN stage.
        stage: CanaryStage,
        /// Configured allocation.
        canary_basis_points: u16,
    },
}

impl RolloutConfigError {
    /// Environment name used when mapping the error into the process config.
    pub const fn env_name(self) -> &'static str {
        match self {
            Self::InvalidBoolean { name } | Self::InvalidBasisPoints { name } => name,
            Self::DisabledWithNonzeroCanary => "LMM_CONVERSION_ENGINE_V2_CANARY_BPS",
            Self::InvalidLossPolicy => "LMM_CONVERSION_LOSS_POLICY",
            Self::InvalidPairOverrides => "LMM_CONVERTER_PAIR_OVERRIDES",
            Self::InvalidSelector => "LMM_CONVERTER_PAIR_OVERRIDES",
            Self::CanaryBelowStage { .. } => "LMM_CONVERSION_ENGINE_V2_CANARY_BPS",
        }
    }
}

impl fmt::Display for RolloutConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBoolean { name } => write!(formatter, "invalid boolean in {name}"),
            Self::InvalidBasisPoints { name } => {
                write!(formatter, "invalid basis points in {name}")
            }
            Self::DisabledWithNonzeroCanary => {
                formatter.write_str("disabled rollout cannot have nonzero canary basis points")
            }
            Self::InvalidLossPolicy => formatter.write_str("invalid conversion loss policy"),
            Self::InvalidPairOverrides => formatter.write_str("invalid converter pair overrides"),
            Self::InvalidSelector => formatter.write_str("invalid rollout selector"),
            Self::CanaryBelowStage {
                stage,
                canary_basis_points,
            } => write!(
                formatter,
                "canary allocation {canary_basis_points} is below rollout stage {stage:?}"
            ),
        }
    }
}

impl Error for RolloutConfigError {}

/// Keeps the relay feature import visible to downstream users documenting
/// loss-aware rollout decisions without exposing provider DTOs here.
pub type RolloutFeature = RelayFeature;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn context<'a>(key: &'a str) -> RolloutContext<'a> {
        RolloutContext::new(
            key,
            Protocol::OpenAi,
            Protocol::OpenAiResponses,
            "gpt",
            false,
        )
    }

    #[test]
    fn default_configuration_is_v1_and_closed() {
        let config = ProtocolRolloutConfig::default();
        let decision = config.decide(RolloutFlag::ConversionEngineV2, &context("request-1"));
        assert!(!decision.enabled);
        assert_eq!(decision.source, DecisionSource::BaseConfig);
        assert_eq!(config.loss_policy(), LossPolicy::Reject);
    }

    #[test]
    fn explicit_flag_defaults_to_full_allocation_only_when_enabled() {
        let mut config = ProtocolRolloutConfig::default();
        config.conversion_engine_v2 = FlagConfig::enabled(MAX_BASIS_POINTS).expect("valid");
        assert!(
            config
                .decide(RolloutFlag::ConversionEngineV2, &context("request-1"))
                .enabled
        );
    }

    #[test]
    fn dimension_override_beats_base_configuration() {
        let mut config = ProtocolRolloutConfig::default();
        config.conversion_engine_v2 = FlagConfig::enabled(0).expect("valid");
        let selector = RolloutSelector {
            channel: Some("internal".to_owned()),
            ..RolloutSelector::default()
        };
        config
            .conversion_engine_v2
            .push_override(FlagOverride::new(selector, true, MAX_BASIS_POINTS).expect("valid"))
            .expect("valid");
        let internal = context("request-1").with_channel("internal");
        let public = context("request-1");
        assert!(
            config
                .decide(RolloutFlag::ConversionEngineV2, &internal)
                .enabled
        );
        assert!(
            !config
                .decide(RolloutFlag::ConversionEngineV2, &public)
                .enabled
        );
    }

    #[test]
    fn pair_override_beats_dimension_override() {
        let mut config = ProtocolRolloutConfig::default();
        config.conversion_engine_v2 = FlagConfig::enabled(MAX_BASIS_POINTS).expect("valid");
        config
            .push_pair_override(ConverterPairOverride {
                flag: RolloutFlag::ConversionEngineV2,
                source: Protocol::OpenAi,
                target: Protocol::OpenAiResponses,
                channel: None,
                model_family: None,
                stream: None,
                enabled: false,
                canary_basis_points: None,
            })
            .expect("valid");
        let decision = config.decide(RolloutFlag::ConversionEngineV2, &context("request-1"));
        assert!(!decision.enabled);
        assert!(matches!(
            decision.source,
            DecisionSource::ConverterPairOverride(_)
        ));
    }

    #[test]
    fn pair_override_flag_reports_matching_route_scope() {
        let mut config = ProtocolRolloutConfig::default();
        config
            .push_pair_override(ConverterPairOverride {
                flag: RolloutFlag::ConversionEngineV2,
                source: Protocol::OpenAi,
                target: Protocol::OpenAiResponses,
                channel: None,
                model_family: None,
                stream: None,
                enabled: true,
                canary_basis_points: None,
            })
            .expect("valid");
        let decision = config.decide(RolloutFlag::ConverterPairOverrides, &context("request-1"));
        assert!(decision.enabled);
        assert!(matches!(
            decision.source,
            DecisionSource::ConverterPairOverride(_)
        ));
    }

    #[test]
    fn stable_bucket_is_repeatable_and_boundary_is_explicit() {
        let first = stable_bucket("stable-request-key");
        assert_eq!(first, stable_bucket("stable-request-key"));
        assert!(!bucket_is_in_rollout(0, 0));
        assert!(bucket_is_in_rollout(0, 1));
        assert!(!bucket_is_in_rollout(MAX_BASIS_POINTS, MAX_BASIS_POINTS));
        assert!(bucket_is_in_rollout(MAX_BASIS_POINTS - 1, MAX_BASIS_POINTS));
    }

    #[test]
    fn malformed_values_fail_closed() {
        assert!(parse_boolean("yes", "FLAG").is_err());
        assert!(parse_basis_points("10001", "FLAG_BPS").is_err());
        assert!(parse_loss_policy("maybe").is_err());
        assert!(matches!(
            FlagOverride::new(RolloutSelector::default(), false, 1),
            Err(RolloutConfigError::DisabledWithNonzeroCanary)
        ));
    }

    #[test]
    fn canary_stages_use_exact_basis_point_thresholds() {
        assert!(validate_canary_stage(CanaryStage::TextOnePercent, 100).is_ok());
        assert!(validate_canary_stage(CanaryStage::TextImageFivePercent, 499).is_err());
        assert!(validate_canary_stage(CanaryStage::FullFeatureTwentyFivePercent, 2_500).is_ok());
        assert!(validate_canary_stage(CanaryStage::FullTraffic, MAX_BASIS_POINTS).is_ok());
    }

    #[test]
    fn updated_configuration_snapshot_closes_every_v2_decision() {
        let mut config = ProtocolRolloutConfig::default();
        config.conversion_engine_v2 = FlagConfig::enabled(MAX_BASIS_POINTS).expect("valid");
        let before = config.decide(RolloutFlag::ConversionEngineV2, &context("request-1"));
        assert!(before.enabled);
        config.apply_rollback(&RollbackDecision {
            action: RollbackAction::Disable,
            reasons: vec![RollbackReason::SignatureModified],
        });
        let after = config.decide(RolloutFlag::ConversionEngineV2, &context("request-1"));
        assert!(!after.enabled);
        assert_eq!(after.source, DecisionSource::ConfigRollback);
        assert!(config.rollback_enabled());
        assert_eq!(config.converter_pair_overrides.len(), 0);
    }

    #[test]
    fn invalid_direct_pair_override_fails_closed_at_decision_time() {
        let mut config = ProtocolRolloutConfig::default();
        config.conversion_engine_v2 = FlagConfig::enabled(MAX_BASIS_POINTS).expect("valid");
        config.converter_pair_overrides.push(ConverterPairOverride {
            flag: RolloutFlag::ConversionEngineV2,
            source: Protocol::OpenAi,
            target: Protocol::OpenAiResponses,
            channel: None,
            model_family: None,
            stream: None,
            enabled: true,
            canary_basis_points: Some(MAX_BASIS_POINTS + 1),
        });

        let decision = config.decide(RolloutFlag::ConversionEngineV2, &context("request-1"));
        assert!(!decision.enabled);
        assert_eq!(decision.canary_basis_points, 0);
    }

    #[test]
    fn shadow_runner_calls_only_local_converters_once_and_keeps_body_out_of_record() {
        let old_calls = AtomicUsize::new(0);
        let new_calls = AtomicUsize::new(0);
        let old = |request: &LocalRequest<'_>| {
            assert_eq!(request.as_bytes(), b"secret prompt");
            old_calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, LocalConversionError>(LocalConversionSummary {
                converter_id: "old".to_owned(),
                plan_fingerprint: [1; 32],
                semantic_fingerprint: [2; 32],
                losses: Vec::new(),
                synthetic: Vec::new(),
            })
        };
        let new = |_request: &LocalRequest<'_>| {
            new_calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, LocalConversionError>(LocalConversionSummary {
                converter_id: "new".to_owned(),
                plan_fingerprint: [1; 32],
                semantic_fingerprint: [2; 32],
                losses: Vec::new(),
                synthetic: Vec::new(),
            })
        };
        let runner =
            ShadowRunner::new(old, new, Protocol::OpenAi, Protocol::OpenAiResponses, false);
        let record = runner.compare(&LocalRequest::new(b"secret prompt"));
        assert_eq!(old_calls.load(Ordering::SeqCst), 1);
        assert_eq!(new_calls.load(Ordering::SeqCst), 1);
        assert!(record.differences.contains(&ShadowDifference::ConverterId));
        let serialized = serde_json::to_string(&record).expect("record serializes");
        assert!(!serialized.contains("secret prompt"));
    }

    #[test]
    fn shadow_comparison_classifies_plan_semantic_loss_and_synthetic_differences() {
        let old = LocalConversionSummary {
            converter_id: "same".to_owned(),
            plan_fingerprint: [1; 32],
            semantic_fingerprint: [2; 32],
            losses: vec![LossCode::LossCitation],
            synthetic: vec![SyntheticField::ToolCallId],
        };
        let mut new = old.clone();
        new.plan_fingerprint = [3; 32];
        new.semantic_fingerprint = [4; 32];
        new.losses.clear();
        new.synthetic.clear();
        let record = compare_local_results(
            Protocol::OpenAi,
            Protocol::OpenAiResponses,
            true,
            Ok(old),
            Ok(new),
        );
        assert!(record.differences.contains(&ShadowDifference::Plan));
        assert!(record.differences.contains(&ShadowDifference::Semantic));
        assert!(record.differences.contains(&ShadowDifference::LossLedger));
        assert!(
            record
                .differences
                .contains(&ShadowDifference::SyntheticFields)
        );
    }

    #[test]
    fn shadow_aggregate_keeps_route_counters_without_request_body_or_key() {
        let record = compare_local_results(
            Protocol::OpenAi,
            Protocol::Claude,
            true,
            Ok(LocalConversionSummary {
                converter_id: "old".to_owned(),
                plan_fingerprint: [1; 32],
                semantic_fingerprint: [1; 32],
                losses: Vec::new(),
                synthetic: Vec::new(),
            }),
            Ok(LocalConversionSummary {
                converter_id: "new".to_owned(),
                plan_fingerprint: [1; 32],
                semantic_fingerprint: [1; 32],
                losses: Vec::new(),
                synthetic: Vec::new(),
            }),
        );
        let mut aggregate = ShadowAggregate::default();
        aggregate.record(&record);
        let route = aggregate
            .routes
            .iter()
            .find(|entry| {
                entry.route
                    == (ShadowRouteKey {
                        source: Protocol::OpenAi,
                        target: Protocol::Claude,
                        stream: true,
                    })
            })
            .map(|entry| &entry.aggregate)
            .expect("route aggregate");
        assert_eq!(route.total, 1);
        assert_eq!(route.identical, 1);
        let serialized = serde_json::to_string(&aggregate).expect("aggregate serializes");
        assert!(!serialized.contains("prompt"));
    }

    #[test]
    fn rollback_immediately_disables_for_each_correctness_condition() {
        for signals in [
            RollbackSignals {
                tool_or_signature_400_rate_increased: true,
                ..RollbackSignals::default()
            },
            RollbackSignals {
                signature_modified: true,
                ..RollbackSignals::default()
            },
            RollbackSignals {
                usage_billing_difference: true,
                ..RollbackSignals::default()
            },
        ] {
            assert_eq!(evaluate_rollback(&signals).action, RollbackAction::Disable);
        }
    }

    #[test]
    fn rollback_pauses_for_each_operational_threshold() {
        for signals in [
            RollbackSignals {
                silent_loss: true,
                ..RollbackSignals::default()
            },
            RollbackSignals {
                parse_error_rate_percentage_points: Some(
                    PARSE_ERROR_RATE_PAUSE_PERCENTAGE_POINTS + 0.01,
                ),
                ..RollbackSignals::default()
            },
            RollbackSignals {
                ttft_p95_increase_percent: Some(TTFT_P95_PAUSE_PERCENT + 0.01),
                ..RollbackSignals::default()
            },
        ] {
            assert_eq!(evaluate_rollback(&signals).action, RollbackAction::Pause);
        }
    }

    #[test]
    fn sse_interruption_disables_the_sse_parser_rollout() {
        let decision = evaluate_rollback(&RollbackSignals {
            sse_interruption_rate_elevated: true,
            ..RollbackSignals::default()
        });
        assert_eq!(decision.action, RollbackAction::Disable);
        assert!(
            decision
                .reasons
                .contains(&RollbackReason::SseInterruptionRateElevated)
        );
    }

    #[test]
    fn tool_signature_400_increase_disables_the_converter_rollout() {
        let decision = evaluate_rollback(&RollbackSignals {
            tool_or_signature_400_rate_increased: true,
            ..RollbackSignals::default()
        });
        assert_eq!(decision.action, RollbackAction::Disable);
        assert!(
            decision
                .reasons
                .contains(&RollbackReason::ToolSignature400RateIncreased)
        );
    }

    #[test]
    fn rollback_continues_below_threshold_and_disables_invalid_metrics() {
        let safe = RollbackSignals {
            parse_error_rate_percentage_points: Some(PARSE_ERROR_RATE_PAUSE_PERCENTAGE_POINTS),
            ttft_p95_increase_percent: Some(TTFT_P95_PAUSE_PERCENT),
            ..RollbackSignals::default()
        };
        assert_eq!(evaluate_rollback(&safe).action, RollbackAction::Continue);
        let invalid = RollbackSignals {
            ttft_p95_increase_percent: Some(f64::NAN),
            ..RollbackSignals::default()
        };
        let decision = evaluate_rollback(&invalid);
        assert_eq!(decision.action, RollbackAction::Disable);
        assert!(decision.reasons.contains(&RollbackReason::InvalidMetric));
    }
}
