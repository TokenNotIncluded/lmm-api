//! Native client rendering; transport, checkpoints and billing remain shared.
use super::{ClientProtocol, Event, FinishReason, Limits, RelayError, Request, Usage};

pub(super) enum NativeEncoder {
    Anthropic(super::native_anthropic::NativeEncoder),
    Gemini(super::native_gemini::NativeEncoder),
}
impl NativeEncoder {
    pub(super) fn new(request: &Request, id: &str, limits: &Limits) -> Option<Self> {
        match request.client {
            ClientProtocol::Anthropic => Some(Self::Anthropic(
                super::native_anthropic::NativeEncoder::new(
                    &request.model,
                    id,
                    request.stream,
                    limits,
                ),
            )),
            ClientProtocol::Gemini => Some(Self::Gemini(super::native_gemini::NativeEncoder::new(
                &request.model,
                id,
                request.stream,
                limits,
            ))),
            _ => None,
        }
    }
    pub(super) fn consume(&mut self, event: &Event) -> Result<Vec<Vec<u8>>, RelayError> {
        match self {
            Self::Anthropic(e) => e.consume(event),
            Self::Gemini(e) => e.consume(event),
        }
    }
    pub(super) fn terminal(&mut self) -> Result<Vec<Vec<u8>>, RelayError> {
        match self {
            Self::Anthropic(e) => e.terminal(),
            Self::Gemini(e) => e.terminal(),
        }
    }
    pub(super) fn error_packet(&mut self, error: &RelayError) -> Result<Vec<u8>, RelayError> {
        match self {
            Self::Anthropic(e) => e.error_packet(error),
            Self::Gemini(e) => e.error_packet(error),
        }
    }
    pub(super) fn usage(&self) -> Usage {
        match self {
            Self::Anthropic(e) => e.usage(),
            Self::Gemini(e) => e.usage(),
        }
    }
    pub(super) fn finish_reason(&self) -> Option<FinishReason> {
        match self {
            Self::Anthropic(e) => e.finish_reason(),
            Self::Gemini(e) => e.finish_reason(),
        }
    }
}
