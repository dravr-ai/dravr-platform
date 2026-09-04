// ABOUTME: Streaming Copilot ACP turn — forwards deltas and tool-call observations to the turn sink
// ABOUTME: Accumulates the same final HeadlessToolResponse the non-streaming converse() produces
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::mem;

use pierre_core::errors::AppError;
use pierre_core::prompt_fingerprint::{detect_canary_in_response, extract_canary_marker};
use pierre_llm::ChatRequest;
use pierre_services::chat_stream::{TurnEvent, TurnEventSink, TurnProgress};
use tracing::warn;

/// The turn's canary token, recovered from the prompt the request carries.
///
/// The forwarder holds the assembled request and not the pipeline's
/// `PromptGuard`, and the marker `inject_canary_marker` appends travels inside
/// the prompt, so the token is read back from there. Every message is searched
/// rather than only the system one: which role carries the hardened prompt is
/// the assembler's choice, and a marker reproduced in any of them is the same
/// token.
#[must_use]
pub fn request_canary(request: &ChatRequest) -> Option<&str> {
    request
        .messages
        .iter()
        .find_map(|m| extract_canary_marker(&m.content))
}

/// Releases streamed prose to the sink while the canary token cannot cross it.
///
/// A per-delta scan is not enough on its own: the model emits the token a few
/// characters at a time, so the first half of a canary would already be on the
/// athlete's screen by the time the second half made the match. Text is
/// therefore released one canary behind the stream — the last
/// `canary.len() - 1` characters seen so far are held back, because those are
/// exactly the prefix a canary continuing into the next delta would begin
/// with. The delay is under two dozen characters and invisible at reading
/// speed.
///
/// A match stops prose for the rest of the turn. The assembled reply still
/// reaches the response boundary, which withholds it wholesale and puts the
/// canned string on the terminal frame — so the athlete's live bubble is
/// superseded by the withheld reply rather than ever having shown the token.
struct CanaryFilteredProse<'a> {
    canary: &'a str,
    /// Text scanned but not yet released, because it could still be the start
    /// of a canary the next delta completes.
    carry: String,
    /// How many trailing characters stay held back.
    hold: usize,
    /// Set once the canary matched; nothing more is released this turn.
    tripped: bool,
}

impl<'a> CanaryFilteredProse<'a> {
    fn new(canary: &'a str) -> Self {
        Self {
            canary,
            carry: String::new(),
            // One short of the token: a held-back run that long is the longest
            // that can still grow into a match. An absent canary holds nothing
            // and the stream passes through untouched.
            hold: canary.chars().count().saturating_sub(1),
            tripped: false,
        }
    }

    /// Take one delta, return the text that may be forwarded now.
    fn push(&mut self, delta: &str) -> Option<String> {
        if self.tripped {
            return None;
        }
        self.carry.push_str(delta);
        if detect_canary_in_response(self.canary, &self.carry) {
            self.trip();
            return None;
        }
        let total = self.carry.chars().count();
        if total <= self.hold {
            return None;
        }
        let split = self
            .carry
            .char_indices()
            .nth(total - self.hold)
            .map_or(self.carry.len(), |(i, _)| i);
        let released: String = self.carry.drain(..split).collect();
        (!released.is_empty()).then_some(released)
    }

    /// Release what is still held once the model has stopped producing.
    fn flush(&mut self) -> Option<String> {
        if self.tripped || self.carry.is_empty() {
            return None;
        }
        if detect_canary_in_response(self.canary, &self.carry) {
            self.trip();
            return None;
        }
        Some(mem::take(&mut self.carry))
    }

    fn trip(&mut self) {
        self.tripped = true;
        self.carry.clear();
        warn!(
            "canary token appeared in the streamed reply; prose forwarding \
             stopped for this turn and the held text was dropped"
        );
    }
}

/// Run a streaming Copilot ACP turn, forwarding text deltas and tool-call
/// observations to `sink` while accumulating the same final
/// [`HeadlessToolResponse`] that the non-streaming `converse()` produces.
///
/// Prose is filtered against the turn's canary on the way out, so the one leak
/// signal that is conclusive on its own — a verbatim canary — is caught while
/// the reply is still being produced instead of only at the response boundary,
/// which sees the assembled text after the athlete has read it.
///
/// Returns the aggregated response so the caller can record per-call usage
/// and fold it into `ToolLoopResult` exactly like the non-streaming branch.
///
/// # Errors
///
/// Returns [`AppError`] when the ACP session cannot be opened, when the
/// underlying stream yields a transport error, or when the run ends without a
/// terminal response frame.
pub async fn run_headless_streaming(
    headless_runner: &pierre_llm::CopilotHeadlessRunner,
    request: &ChatRequest,
    sink: &TurnEventSink,
) -> Result<pierre_llm::HeadlessToolResponse, AppError> {
    let stream = headless_runner
        .converse_stream(request)
        .await
        .map_err(AppError::from)?;
    forward_headless_stream(stream, request_canary(request).unwrap_or_default(), sink).await
}

/// Drain one ACP turn's events into `sink`, filtering prose against `canary`.
///
/// Split from [`run_headless_streaming`] so the boundary that decides what
/// reaches the athlete's screen is exercised against a stream of events rather
/// than only against a live subprocess. An empty `canary` filters nothing.
///
/// # Errors
/// Propagates a runner error off the stream, and returns an external-service
/// error when the stream ends without a `Done` event.
pub async fn forward_headless_stream(
    mut stream: pierre_llm::HeadlessEventStream,
    canary: &str,
    sink: &TurnEventSink,
) -> Result<pierre_llm::HeadlessToolResponse, AppError> {
    use pierre_llm::HeadlessStreamEvent;
    use tokio_stream::StreamExt;

    let mut final_response: Option<pierre_llm::HeadlessToolResponse> = None;
    let mut prose = CanaryFilteredProse::new(canary);

    while let Some(item) = stream.next().await {
        let event = item.map_err(AppError::from)?;
        match event {
            HeadlessStreamEvent::TextDelta(delta) => {
                // Send may fail if the receiver was dropped (client disconnected
                // or the pipeline aborted) — treat as a benign no-op so the ACP
                // session keeps draining and the run still completes cleanly.
                if let Some(text) = prose.push(&delta) {
                    let _ = sink.send(TurnEvent::ProseDelta(text));
                }
            }
            HeadlessStreamEvent::ToolCall(tc) => {
                let _ = sink.send(TurnEvent::Progress(TurnProgress::tool(
                    tc.id, tc.title, tc.status,
                )));
            }
            HeadlessStreamEvent::Done(response) => {
                final_response = Some(response);
            }
        }
    }

    // The tail the filter was holding against a split canary: the model has
    // stopped, so nothing can complete a match any more and the text is owed
    // to the reader.
    if let Some(text) = prose.flush() {
        let _ = sink.send(TurnEvent::ProseDelta(text));
    }

    final_response.ok_or_else(|| {
        AppError::external_service(
            "copilot-headless",
            "converse_stream completed without a Done event",
        )
    })
}
