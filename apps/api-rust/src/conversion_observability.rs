//! Bounded, low-cardinality telemetry for protocol conversion.
//!
//! This module deliberately stores only closed enums and numeric values.  A
//! request id, model name, tool name, prompt, response body, and provider
//! converter string can never become a label or enter a snapshot.  The
//! recorder is process-local and bounded; callers can export a stable,
//! sorted snapshot to the metrics boundary without retaining request data.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use lmm_contracts::relay::{LossCode, Protocol, SyntheticField};
use serde::{Deserialize, Serialize};

/// Maximum normalized hop count retained in a metric label.
pub const MAX_HOP_COUNT: u16 = 64;

/// Default number of distinct metric series retained by one recorder.
pub const DEFAULT_MAX_SERIES: usize = 512;

const NO_LOSS_CODE: u8 = u8::MAX;

/// A closed converter/runtime version label.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConverterVersion {
    /// OpenAI Chat request/response conversion boundary.
    OpenAiChatV1,
    /// OpenAI Responses request/response conversion boundary.
    OpenAiResponsesV1,
    /// Native same-protocol bytes, without JSON decoding.
    NativeRawV1,
}

impl ConverterVersion {
    const fn rank(self) -> u8 {
        match self {
            Self::OpenAiChatV1 => 0,
            Self::OpenAiResponsesV1 => 1,
            Self::NativeRawV1 => 2,
        }
    }

    const fn from_rank(rank: u8) -> Option<Self> {
        match rank {
            0 => Some(Self::OpenAiChatV1),
            1 => Some(Self::OpenAiResponsesV1),
            2 => Some(Self::NativeRawV1),
            _ => None,
        }
    }
}

/// A controlled feature class used instead of feature/prompt/tool strings.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureClass {
    /// Plain text and message content.
    Text,
    /// Image, audio, video, or file content.
    Multimodal,
    /// Function calls, tools, and tool results.
    Tool,
    /// Reasoning and provider signatures.
    Reasoning,
    /// Usage and accounting metadata.
    Usage,
    /// Streaming protocol events.
    Stream,
    /// A bounded catch-all for unclassified conversion data.
    Unknown,
}

impl FeatureClass {
    const fn rank(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Multimodal => 1,
            Self::Tool => 2,
            Self::Reasoning => 3,
            Self::Usage => 4,
            Self::Stream => 5,
            Self::Unknown => 6,
        }
    }

    const fn from_rank(rank: u8) -> Option<Self> {
        match rank {
            0 => Some(Self::Text),
            1 => Some(Self::Multimodal),
            2 => Some(Self::Tool),
            3 => Some(Self::Reasoning),
            4 => Some(Self::Usage),
            5 => Some(Self::Stream),
            6 => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// The bounded result dimension for conversion telemetry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionResult {
    /// Conversion or passthrough completed successfully.
    Success,
    /// Conversion or serialization failed.
    Failure,
    /// The selected route or feature was unsupported.
    Unsupported,
    /// The client or gateway cancelled the stream.
    Cancelled,
}

impl ConversionResult {
    const fn rank(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Unsupported => 2,
            Self::Cancelled => 3,
        }
    }

    const fn from_rank(rank: u8) -> Option<Self> {
        match rank {
            0 => Some(Self::Success),
            1 => Some(Self::Failure),
            2 => Some(Self::Unsupported),
            3 => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// The exact metric name exported by a snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// Total conversion time. Values are saturating nanoseconds.
    ConversionDurationSeconds,
    /// Time spent compiling a conversion plan. Values are saturating nanoseconds.
    ConversionPlanDurationSeconds,
    /// Number of conversion events.
    ConversionEventsTotal,
    /// Number of input bytes observed.
    ConversionInputBytes,
    /// Number of output bytes observed.
    ConversionOutputBytes,
    /// Number of conversion failures.
    ConversionFailuresTotal,
    /// Number of structured losses.
    ConversionLossesTotal,
    /// Number of gateway-synthesized fields.
    ConversionSyntheticFieldsTotal,
    /// Number of unknown events.
    ConversionUnknownEventsTotal,
    /// Gateway-only first-event-to-first-write duration. Values are nanoseconds.
    StreamGatewayTtftSeconds,
    /// Current number of active streams in the bounded queue.
    StreamQueueDepth,
    /// Number of client-aborted streams.
    StreamClientAbortTotal,
}

impl MetricKind {
    /// Returns the stable Prometheus-style metric name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConversionDurationSeconds => "conversion_duration_seconds",
            Self::ConversionPlanDurationSeconds => "conversion_plan_duration_seconds",
            Self::ConversionEventsTotal => "conversion_events_total",
            Self::ConversionInputBytes => "conversion_input_bytes",
            Self::ConversionOutputBytes => "conversion_output_bytes",
            Self::ConversionFailuresTotal => "conversion_failures_total",
            Self::ConversionLossesTotal => "conversion_losses_total",
            Self::ConversionSyntheticFieldsTotal => "conversion_synthetic_fields_total",
            Self::ConversionUnknownEventsTotal => "conversion_unknown_events_total",
            Self::StreamGatewayTtftSeconds => "stream_gateway_ttft_seconds",
            Self::StreamQueueDepth => "stream_queue_depth",
            Self::StreamClientAbortTotal => "stream_client_abort_total",
        }
    }
}

/// The complete and closed label set allowed on conversion metrics.
///
/// No string or request-scoped identifier is present by construction. The
/// hop count is normalized to [`MAX_HOP_COUNT`] and all other dimensions are
/// closed enums or booleans.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MetricLabels {
    /// Source protocol label.
    pub source_format: Protocol,
    /// Target protocol label.
    pub target_format: Protocol,
    /// Controlled converter/runtime version.
    pub converter_version: ConverterVersion,
    /// Normalized conversion hop count.
    pub hop_count: u16,
    /// Whether the conversion is a stream.
    pub stream: bool,
    /// Controlled semantic feature class.
    pub feature_class: FeatureClass,
    /// Structured loss code, when this series concerns one loss.
    pub loss_code: Option<LossCode>,
    /// Result category.
    pub result: ConversionResult,
}

impl MetricLabels {
    /// Creates labels while normalizing the hop count to a bounded range.
    #[must_use]
    pub const fn new(
        source_format: Protocol,
        target_format: Protocol,
        converter_version: ConverterVersion,
        hop_count: u16,
        stream: bool,
        feature_class: FeatureClass,
        result: ConversionResult,
    ) -> Self {
        Self {
            source_format,
            target_format,
            converter_version,
            hop_count: if hop_count > MAX_HOP_COUNT {
                MAX_HOP_COUNT
            } else {
                hop_count
            },
            stream,
            feature_class,
            loss_code: None,
            result,
        }
    }

    /// Returns labels with a controlled loss dimension.
    #[must_use]
    pub const fn with_loss_code(mut self, loss_code: LossCode) -> Self {
        self.loss_code = Some(loss_code);
        self
    }

    /// Returns labels with a different controlled feature class.
    #[must_use]
    pub const fn with_feature_class(mut self, feature_class: FeatureClass) -> Self {
        self.feature_class = feature_class;
        self
    }

    /// Returns labels with a different controlled result.
    #[must_use]
    pub const fn with_result(mut self, result: ConversionResult) -> Self {
        self.result = result;
        self
    }

    /// Returns the labels used by a raw same-protocol byte passthrough.
    #[must_use]
    pub const fn native_raw(
        protocol: Protocol,
        stream: bool,
        result: ConversionResult,
    ) -> Self {
        Self::new(
            protocol,
            protocol,
            ConverterVersion::NativeRawV1,
            0,
            stream,
            FeatureClass::Unknown,
            result,
        )
    }
}

/// One stable metric sample from a recorder snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    /// Metric name.
    pub metric: MetricKind,
    /// Closed labels.
    pub labels: MetricLabels,
    /// Saturating value. Duration metrics use nanoseconds.
    pub value: u64,
}

/// An immutable, sorted export of bounded conversion metrics.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecorderSnapshot {
    /// Stable metric samples sorted by their closed key.
    pub samples: Vec<MetricSample>,
    /// Number of series rejected after the configured bound was reached.
    pub dropped_series: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SeriesKey {
    metric: MetricKind,
    source_format: u8,
    target_format: u8,
    converter_version: u8,
    hop_count: u16,
    stream: bool,
    feature_class: u8,
    loss_code: u8,
    result: u8,
}

impl SeriesKey {
    fn new(metric: MetricKind, labels: MetricLabels) -> Self {
        Self {
            metric,
            source_format: protocol_rank(labels.source_format),
            target_format: protocol_rank(labels.target_format),
            converter_version: labels.converter_version.rank(),
            hop_count: labels.hop_count,
            stream: labels.stream,
            feature_class: labels.feature_class.rank(),
            loss_code: labels.loss_code.map_or(NO_LOSS_CODE, loss_code_rank),
            result: labels.result.rank(),
        }
    }

    fn labels(self) -> Option<MetricLabels> {
        Some(MetricLabels {
            source_format: protocol_from_rank(self.source_format)?,
            target_format: protocol_from_rank(self.target_format)?,
            converter_version: ConverterVersion::from_rank(self.converter_version)?,
            hop_count: self.hop_count,
            stream: self.stream,
            feature_class: FeatureClass::from_rank(self.feature_class)?,
            loss_code: if self.loss_code == NO_LOSS_CODE {
                None
            } else {
                Some(loss_code_from_rank(self.loss_code)?)
            },
            result: ConversionResult::from_rank(self.result)?,
        })
    }
}

struct RecorderInner {
    series: BTreeMap<SeriesKey, u64>,
    max_series: usize,
    dropped_series: u64,
}

impl RecorderInner {
    fn reserve(&mut self, key: SeriesKey) -> bool {
        if self.series.contains_key(&key) {
            return true;
        }
        if self.series.len() >= self.max_series {
            self.dropped_series = self.dropped_series.saturating_add(1);
            return false;
        }
        self.series.insert(key, 0);
        true
    }
}

/// Thread-safe bounded metric recorder.
#[derive(Clone)]
pub struct ConversionObserver {
    inner: Arc<Mutex<RecorderInner>>,
}

/// Naming alias for callers that prefer the recorder terminology.
pub type ConversionRecorder = ConversionObserver;

impl Default for ConversionObserver {
    fn default() -> Self {
        Self::with_max_series(DEFAULT_MAX_SERIES)
    }
}

impl ConversionObserver {
    /// Creates a recorder with a hard cap on distinct metric series.
    #[must_use]
    pub fn with_max_series(max_series: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecorderInner {
                series: BTreeMap::new(),
                max_series: max_series.max(1),
                dropped_series: 0,
            })),
        }
    }

    /// Records an arbitrary bounded metric value using saturating arithmetic.
    pub fn record(&self, metric: MetricKind, labels: MetricLabels, value: u64) {
        self.with_inner(|inner| {
            let key = SeriesKey::new(metric, labels);
            if !inner.reserve(key) {
                return;
            }
            if let Some(current) = inner.series.get_mut(&key) {
                *current = current.saturating_add(value);
            }
        });
    }

    /// Records a duration metric in saturating nanoseconds.
    pub fn record_duration(&self, metric: MetricKind, labels: MetricLabels, duration: Duration) {
        self.record(metric, labels, duration_nanos(duration));
    }

    /// Records conversion duration.
    pub fn record_conversion_duration(&self, labels: MetricLabels, duration: Duration) {
        self.record_duration(MetricKind::ConversionDurationSeconds, labels, duration);
    }

    /// Records conversion-plan compilation duration.
    pub fn record_plan_duration(&self, labels: MetricLabels, duration: Duration) {
        self.record_duration(
            MetricKind::ConversionPlanDurationSeconds,
            labels,
            duration,
        );
    }

    /// Records conversion events.
    pub fn record_events(&self, labels: MetricLabels, count: u64) {
        self.record(MetricKind::ConversionEventsTotal, labels, count);
    }

    /// Records input bytes.
    pub fn record_input_bytes(&self, labels: MetricLabels, count: usize) {
        self.record(MetricKind::ConversionInputBytes, labels, usize_to_u64(count));
    }

    /// Records output bytes.
    pub fn record_output_bytes(&self, labels: MetricLabels, count: usize) {
        self.record(MetricKind::ConversionOutputBytes, labels, usize_to_u64(count));
    }

    /// Records a failed conversion and fixes its result label to `failure`.
    pub fn record_failure(&self, labels: MetricLabels) {
        self.record(
            MetricKind::ConversionFailuresTotal,
            labels.with_result(ConversionResult::Failure),
            1,
        );
    }

    /// Records a structured loss under its closed loss-code label.
    pub fn record_loss(&self, labels: MetricLabels, loss_code: LossCode) {
        self.record(
            MetricKind::ConversionLossesTotal,
            labels.with_loss_code(loss_code),
            1,
        );
    }

    /// Records one gateway-synthesized field without exposing its raw name.
    pub fn record_synthetic_field(&self, labels: MetricLabels, field: SyntheticField) {
        let feature_class = match field {
            SyntheticField::ToolCallId => FeatureClass::Tool,
            SyntheticField::ThoughtSignature => FeatureClass::Reasoning,
        };
        self.record(
            MetricKind::ConversionSyntheticFieldsTotal,
            labels.with_feature_class(feature_class),
            1,
        );
    }

    /// Records one unknown stream event without retaining an event name.
    pub fn record_unknown_event(&self, labels: MetricLabels) {
        self.record(
            MetricKind::ConversionUnknownEventsTotal,
            labels.with_feature_class(FeatureClass::Stream),
            1,
        );
    }

    /// Records the gateway-only first-event-to-first-write duration.
    pub fn record_gateway_ttft(&self, labels: MetricLabels, duration: Duration) {
        self.record_duration(MetricKind::StreamGatewayTtftSeconds, labels, duration);
    }

    /// Records one client abort using the cancelled result label.
    pub fn record_client_abort(&self, labels: MetricLabels) {
        self.record(
            MetricKind::StreamClientAbortTotal,
            labels.with_result(ConversionResult::Cancelled),
            1,
        );
    }

    /// Enters a bounded stream queue and returns a guard that decrements on drop.
    #[must_use]
    pub fn enter_queue(&self, labels: MetricLabels) -> QueueDepthGuard {
        self.adjust_queue_depth(labels, true);
        QueueDepthGuard {
            observer: self.clone(),
            labels,
            active: true,
        }
    }

    /// Returns a stable sorted snapshot suitable for a metrics exporter.
    #[must_use]
    pub fn snapshot(&self) -> RecorderSnapshot {
        self.with_inner(|inner| RecorderSnapshot {
            samples: inner
                .series
                .iter()
                .filter_map(|(key, value)| {
                    key.labels().map(|labels| MetricSample {
                        metric: key.metric,
                        labels,
                        value: *value,
                    })
                })
                .collect(),
            dropped_series: inner.dropped_series,
        })
    }

    /// Exports the same immutable data as [`Self::snapshot`].
    #[must_use]
    pub fn export(&self) -> RecorderSnapshot {
        self.snapshot()
    }

    fn adjust_queue_depth(&self, labels: MetricLabels, increment: bool) {
        self.with_inner(|inner| {
            let key = SeriesKey::new(MetricKind::StreamQueueDepth, labels);
            if increment && !inner.reserve(key) {
                return;
            }
            let Some(current) = inner.series.get_mut(&key) else {
                return;
            };
            if increment {
                *current = current.saturating_add(1);
            } else {
                *current = current.saturating_sub(1);
            }
        });
    }

    fn with_inner<T>(&self, operation: impl FnOnce(&mut RecorderInner) -> T) -> T {
        match self.inner.lock() {
            Ok(mut guard) => operation(&mut guard),
            Err(poisoned) => operation(&mut poisoned.into_inner()),
        }
    }
}

/// Process-wide recorder used by route slices that do not receive a metrics
/// state object. It remains bounded and exposes only [`RecorderSnapshot`].
pub fn global_observer() -> &'static ConversionObserver {
    static OBSERVER: OnceLock<ConversionObserver> = OnceLock::new();
    OBSERVER.get_or_init(ConversionObserver::default)
}

/// A queue-depth guard. Dropping it decrements the corresponding depth once.
#[must_use]
pub struct QueueDepthGuard {
    observer: ConversionObserver,
    labels: MetricLabels,
    active: bool,
}

impl QueueDepthGuard {
    /// Marks the queue item as completed before the guard is dropped.
    pub fn complete(&mut self) {
        if self.active {
            self.observer.adjust_queue_depth(self.labels, false);
            self.active = false;
        }
    }
}

impl Drop for QueueDepthGuard {
    fn drop(&mut self) {
        self.complete();
    }
}

/// A stream guard that records a client abort unless explicitly completed.
#[must_use]
pub struct ClientAbortGuard {
    observer: ConversionObserver,
    labels: MetricLabels,
    completed: bool,
}

impl ClientAbortGuard {
    /// Creates an abort guard for one stream.
    #[must_use]
    pub fn new(observer: ConversionObserver, labels: MetricLabels) -> Self {
        Self {
            observer,
            labels,
            completed: false,
        }
    }

    /// Marks normal stream completion so drop does not count an abort.
    pub fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for ClientAbortGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.observer.record_client_abort(self.labels);
        }
    }
}

/// Monotonic first-event timing for a gateway stream.
///
/// Only the interval from `first_upstream_event_at` to
/// `first_downstream_write_at` is exported. No provider/model generation time
/// is included, and `checked_duration_since` prevents negative values after a
/// clock-ordering mistake.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamTiming {
    /// First observed upstream event timestamp.
    pub first_upstream_event_at: Option<Instant>,
    /// First downstream write timestamp.
    pub first_downstream_write_at: Option<Instant>,
}

impl StreamTiming {
    /// Records the first upstream event using the monotonic clock.
    pub fn mark_upstream_event(&mut self) {
        self.mark_upstream_event_at(Instant::now());
    }

    /// Records an injected first upstream timestamp for deterministic callers.
    pub fn mark_upstream_event_at(&mut self, timestamp: Instant) {
        if self.first_upstream_event_at.is_none() {
            self.first_upstream_event_at = Some(timestamp);
        }
    }

    /// Records the first downstream write using the monotonic clock.
    pub fn mark_downstream_write(&mut self) {
        self.mark_downstream_write_at(Instant::now());
    }

    /// Records an injected first downstream timestamp for deterministic callers.
    pub fn mark_downstream_write_at(&mut self, timestamp: Instant) {
        if self.first_downstream_write_at.is_none() {
            self.first_downstream_write_at = Some(timestamp);
        }
    }

    /// Returns only the gateway TTFT tax, never a negative duration.
    #[must_use]
    pub fn gateway_ttft_tax(&self) -> Option<Duration> {
        Some(
            self.first_downstream_write_at?
                .checked_duration_since(self.first_upstream_event_at?)?,
        )
    }

    /// Records the gateway TTFT tax when both first timestamps are ordered.
    pub fn record_gateway_ttft(&self, observer: &ConversionObserver, labels: MetricLabels) {
        if let Some(duration) = self.gateway_ttft_tax() {
            observer.record_gateway_ttft(labels, duration);
        }
    }
}

fn duration_nanos(duration: Duration) -> u64 {
    match u64::try_from(duration.as_nanos()) {
        Ok(value) => value,
        Err(_) => u64::MAX,
    }
}

fn usize_to_u64(value: usize) -> u64 {
    if value > u64::MAX as usize {
        u64::MAX
    } else {
        value as u64
    }
}

fn protocol_rank(protocol: Protocol) -> u8 {
    match protocol {
        Protocol::OpenAi => 0,
        Protocol::OpenAiResponses => 1,
        Protocol::Claude => 2,
        Protocol::Gemini => 3,
    }
}

fn protocol_from_rank(rank: u8) -> Option<Protocol> {
    match rank {
        0 => Some(Protocol::OpenAi),
        1 => Some(Protocol::OpenAiResponses),
        2 => Some(Protocol::Claude),
        3 => Some(Protocol::Gemini),
        _ => None,
    }
}

fn loss_code_rank(loss_code: LossCode) -> u8 {
    match loss_code {
        LossCode::LossStatefulContext => 0,
        LossCode::LossBuiltinTool => 1,
        LossCode::LossCustomTool => 2,
        LossCode::LossToolCallId => 3,
        LossCode::LossOpaqueReasoning => 4,
        LossCode::LossRedactedReasoning => 5,
        LossCode::LossContentOrder => 6,
        LossCode::LossCitation => 7,
        LossCode::LossGroundingMetadata => 8,
        LossCode::LossSafetyMetadata => 9,
        LossCode::LossCacheControl => 10,
        LossCode::LossUnknownEvent => 11,
        LossCode::SyntheticToolCallId => 12,
        LossCode::SyntheticThoughtSignature => 13,
    }
}

fn loss_code_from_rank(rank: u8) -> Option<LossCode> {
    match rank {
        0 => Some(LossCode::LossStatefulContext),
        1 => Some(LossCode::LossBuiltinTool),
        2 => Some(LossCode::LossCustomTool),
        3 => Some(LossCode::LossToolCallId),
        4 => Some(LossCode::LossOpaqueReasoning),
        5 => Some(LossCode::LossRedactedReasoning),
        6 => Some(LossCode::LossContentOrder),
        7 => Some(LossCode::LossCitation),
        8 => Some(LossCode::LossGroundingMetadata),
        9 => Some(LossCode::LossSafetyMetadata),
        10 => Some(LossCode::LossCacheControl),
        11 => Some(LossCode::LossUnknownEvent),
        12 => Some(LossCode::SyntheticToolCallId),
        13 => Some(LossCode::SyntheticThoughtSignature),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> MetricLabels {
        MetricLabels::new(
            Protocol::OpenAi,
            Protocol::OpenAiResponses,
            ConverterVersion::OpenAiChatV1,
            1,
            true,
            FeatureClass::Text,
            ConversionResult::Success,
        )
    }

    #[test]
    fn labels_are_closed_and_sensitive_text_cannot_be_serialized() {
        let serialized = serde_json::to_string(&labels()).unwrap_or_default();
        assert!(!serialized.contains("secret prompt"));
        assert_eq!(MetricLabels::new(
            Protocol::OpenAi,
            Protocol::Gemini,
            ConverterVersion::NativeRawV1,
            u16::MAX,
            false,
            FeatureClass::Unknown,
            ConversionResult::Success,
        ).hop_count, MAX_HOP_COUNT);
    }

    #[test]
    fn recorder_covers_each_metric_with_saturating_values() {
        let observer = ConversionObserver::default();
        let base = labels();
        observer.record_conversion_duration(base, Duration::from_nanos(2));
        observer.record_plan_duration(base, Duration::from_nanos(3));
        observer.record_events(base, 1);
        observer.record_input_bytes(base, 4);
        observer.record_output_bytes(base, 5);
        observer.record_failure(base);
        observer.record_loss(base, LossCode::LossCitation);
        observer.record_synthetic_field(base, SyntheticField::ToolCallId);
        observer.record_unknown_event(base);
        observer.record_gateway_ttft(base, Duration::from_nanos(6));
        observer.record_client_abort(base);
        let snapshot = observer.snapshot();
        let expected = [
            MetricKind::ConversionDurationSeconds,
            MetricKind::ConversionPlanDurationSeconds,
            MetricKind::ConversionEventsTotal,
            MetricKind::ConversionInputBytes,
            MetricKind::ConversionOutputBytes,
            MetricKind::ConversionFailuresTotal,
            MetricKind::ConversionLossesTotal,
            MetricKind::ConversionSyntheticFieldsTotal,
            MetricKind::ConversionUnknownEventsTotal,
            MetricKind::StreamGatewayTtftSeconds,
            MetricKind::StreamClientAbortTotal,
        ];
        for metric in expected {
            assert!(snapshot.samples.iter().any(|sample| sample.metric == metric));
        }
    }

    #[test]
    fn recorder_is_bounded_and_snapshot_order_is_stable() {
        let observer = ConversionObserver::with_max_series(1);
        observer.record_events(labels(), 1);
        observer.record_input_bytes(labels(), 2);
        let first = observer.snapshot();
        let second = observer.snapshot();
        assert_eq!(first, second);
        assert_eq!(first.samples.len(), 1);
        assert_eq!(first.dropped_series, 1);
    }

    #[test]
    fn queue_guard_increments_and_decrements_without_underflow() {
        let observer = ConversionObserver::default();
        let mut guard = observer.enter_queue(labels());
        assert!(observer.snapshot().samples.iter().any(|sample| {
            sample.metric == MetricKind::StreamQueueDepth && sample.value == 1
        }));
        guard.complete();
        drop(guard);
        assert!(observer.snapshot().samples.iter().any(|sample| {
            sample.metric == MetricKind::StreamQueueDepth && sample.value == 0
        }));
    }

    #[test]
    fn ttft_uses_first_ordered_instants_only() {
        let start = Instant::now();
        let mut timing = StreamTiming::default();
        timing.mark_upstream_event_at(start);
        timing.mark_upstream_event_at(start + Duration::from_secs(1));
        timing.mark_downstream_write_at(start + Duration::from_millis(25));
        timing.mark_downstream_write_at(start + Duration::from_secs(2));
        assert_eq!(timing.gateway_ttft_tax(), Some(Duration::from_millis(25)));

        let mut reversed = StreamTiming::default();
        reversed.mark_upstream_event_at(start + Duration::from_secs(1));
        reversed.mark_downstream_write_at(start);
        assert_eq!(reversed.gateway_ttft_tax(), None);
    }

    #[test]
    fn abort_guard_records_only_when_dropped_incomplete() {
        let observer = ConversionObserver::default();
        {
            let _guard = ClientAbortGuard::new(observer.clone(), labels());
        }
        assert!(observer.snapshot().samples.iter().any(|sample| {
            sample.metric == MetricKind::StreamClientAbortTotal && sample.value == 1
        }));

        let completed_observer = ConversionObserver::default();
        let mut guard = ClientAbortGuard::new(completed_observer.clone(), labels());
        guard.complete();
        drop(guard);
        assert!(!completed_observer.snapshot().samples.iter().any(|sample| {
            sample.metric == MetricKind::StreamClientAbortTotal
        }));
    }

    #[test]
    fn concurrent_recording_is_saturating_and_thread_safe() {
        let observer = ConversionObserver::default();
        let mut workers = Vec::new();
        for _ in 0..4 {
            let worker = observer.clone();
            workers.push(std::thread::spawn(move || {
                for _ in 0..10 {
                    worker.record_events(labels(), 1);
                }
            }));
        }
        for worker in workers {
            let _ = worker.join();
        }
        let total = observer
            .snapshot()
            .samples
            .iter()
            .find(|sample| sample.metric == MetricKind::ConversionEventsTotal)
            .map_or(0, |sample| sample.value);
        assert_eq!(total, 40);
    }
}
