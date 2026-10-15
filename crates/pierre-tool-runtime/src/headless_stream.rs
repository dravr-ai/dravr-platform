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

use crate::tool_execution::{TOOL_CALL_CLOSE, TOOL_CALL_OPEN};

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

/// Withholds `<tool_call>` blocks from streamed prose.
///
/// The shared system prompt teaches every tier to ask for a tool by writing a
/// `<tool_call>` block, and a Copilot SDK turn does so instead of calling its
/// MCP tools natively (carnet#1032). The block is the platform's to run, never
/// the athlete's to read: the final reply strips it, but every delta reaches
/// the live bubble as it arrives, so the raw JSON was on screen until the turn
/// ended. A tag split across deltas is held until it can be told apart from
/// prose.
#[derive(Default)]
struct ToolCallBlocksWithheld {
    /// Text seen but not yet released or dropped.
    carry: String,
    /// Whether `carry` starts inside an open block.
    inside: bool,
}

impl ToolCallBlocksWithheld {
    /// Take released prose, return what may reach the sink now.
    fn push(&mut self, text: &str) -> Option<String> {
        self.carry.push_str(text);
        let mut released = String::new();
        loop {
            let tag = if self.inside {
                TOOL_CALL_CLOSE
            } else {
                TOOL_CALL_OPEN
            };
            if let Some(at) = self.carry.find(tag) {
                if !self.inside {
                    released.push_str(&self.carry[..at]);
                }
                self.carry.drain(..at + tag.len());
                self.inside = !self.inside;
                continue;
            }
            let cut = self.carry.len() - held_tag_prefix_len(&self.carry, tag);
            if !self.inside {
                released.push_str(&self.carry[..cut]);
            }
            self.carry.drain(..cut);
            break;
        }
        (!released.is_empty()).then_some(released)
    }

    /// Release what is still held once the model has stopped producing. A
    /// block left open is dropped: it was never prose.
    fn flush(&mut self) -> Option<String> {
        let rest = mem::take(&mut self.carry);
        (!self.inside && !rest.is_empty()).then_some(rest)
    }
}

/// The length of the longest end of `text` that could still grow into `tag`.
fn held_tag_prefix_len(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&n| {
            n <= text.len()
                && text.is_char_boundary(text.len() - n)
                && tag.starts_with(&text[text.len() - n..])
        })
        .unwrap_or(0)
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
    headless_runner: &dyn pierre_llm::HeadlessTurnProvider,
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
    let mut blocks = ToolCallBlocksWithheld::default();

    while let Some(item) = stream.next().await {
        let event = item.map_err(AppError::from)?;
        match event {
            HeadlessStreamEvent::TextDelta(delta) => {
                // Send may fail if the receiver was dropped (client disconnected
                // or the pipeline aborted) — treat as a benign no-op so the ACP
                // session keeps draining and the run still completes cleanly.
                if let Some(text) = prose.push(&delta).and_then(|t| blocks.push(&t)) {
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
    let tail = prose
        .flush()
        .and_then(|t| blocks.push(&t))
        .into_iter()
        .chain(blocks.flush())
        .collect::<String>();
    if !tail.is_empty() {
        let _ = sink.send(TurnEvent::ProseDelta(tail));
    }

    final_response.ok_or_else(|| {
        AppError::external_service(
            "copilot-headless",
            "converse_stream completed without a Done event",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::ToolCallBlocksWithheld;

    fn run(deltas: &[&str]) -> String {
        let mut blocks = ToolCallBlocksWithheld::default();
        let mut out: String = deltas.iter().filter_map(|d| blocks.push(d)).collect();
        out.extend(blocks.flush());
        out
    }

    #[test]
    fn a_tool_call_block_never_reaches_the_reader() {
        assert_eq!(
            run(&["Je regarde. <tool_call>\n{\"name\": \"get_activities\"}\n</tool_call> Voilà."]),
            "Je regarde.  Voilà."
        );
    }

    #[test]
    fn tags_split_across_deltas_are_still_withheld() {
        assert_eq!(
            run(&[
                "Je regarde. <tool",
                "_call>\n{\"name\": ",
                "\"x\"}\n</tool_c",
                "all> Voilà."
            ]),
            "Je regarde.  Voilà."
        );
    }

    #[test]
    fn prose_that_only_looks_like_a_tag_is_released() {
        assert_eq!(run(&["a <tool", " belt"]), "a <tool belt");
        assert_eq!(run(&["fin <"]), "fin <");
    }

    #[test]
    fn a_multibyte_character_beside_a_held_tag_survives() {
        assert_eq!(run(&["é", "<tool_call>{}</tool_call>", "ok"]), "éok");
        assert_eq!(
            run(&["café <", "tool_call>{}</tool_call> été"]),
            "café  été"
        );
        assert_eq!(run(&["à <", "é"]), "à <é");
    }

    #[test]
    fn a_block_left_open_is_dropped() {
        assert_eq!(run(&["avant <tool_call>{\"name\""]), "avant ");
    }
}
