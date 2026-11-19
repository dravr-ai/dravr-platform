// ABOUTME: The one gate streamed reply text crosses on its way to the turn sink, whichever tool loop produced it
// ABOUTME: Holds back a split canary and <tool_call> blocks, and takes back what was sent when it is not the reply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Streamed prose, filtered and retractable.
//!
//! Both tool loops that stream — the Copilot ACP turn and an API turn on an
//! Anthropic head — put model text on the athlete's screen before the reply
//! is assembled, so what the response boundary checks on the whole reply has
//! to be held at the stream too: a canary token must never cross, a
//! `<tool_call>` block is the platform's to run, never prose, and a
//! `dravr-` fence (a chart, the model's quick replies) is lifted out of the
//! stored reply, so its raw text must not flash in the bubble (carnet#1486).
//! [`StreamedProse`] applies the three filters, in that order, to every delta
//! either loop forwards.
//!
//! It also remembers whether anything reached the sink, so a loop that learns
//! the streamed text will not be the reply — a tool call followed it, the
//! stream broke off, the provider refused — takes it back with one
//! [`TurnEvent::ProseRetract`] rather than leaving it glued to whatever is
//! streamed next.

use std::mem;

use pierre_core::prompt_fingerprint::{detect_canary_in_response, extract_canary_marker};
use pierre_llm::ChatRequest;
use pierre_services::chat_stream::{TurnEvent, TurnEventSink};
use tracing::warn;

use crate::tool_execution::{TOOL_CALL_CLOSE, TOOL_CALL_OPEN};
use crate::tool_progress;

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

/// Streamed prose on its way to one turn's sink.
pub(crate) struct StreamedProse<'a> {
    sink: &'a TurnEventSink,
    canary: CanaryFilteredProse<'a>,
    blocks: ToolCallBlocksWithheld,
    fences: PlatformFencesWithheld,
    /// Whether any prose reached the sink since the last retraction.
    sent: bool,
}

impl<'a> StreamedProse<'a> {
    /// Prose for `sink`, filtered against `canary`. An empty `canary`
    /// filters nothing.
    pub(crate) fn new(sink: &'a TurnEventSink, canary: &'a str) -> Self {
        Self {
            sink,
            canary: CanaryFilteredProse::new(canary),
            blocks: ToolCallBlocksWithheld::default(),
            fences: PlatformFencesWithheld::default(),
            sent: false,
        }
    }

    /// Forward what of `delta` may be read now.
    pub(crate) fn push(&mut self, delta: &str) {
        if let Some(text) = self
            .canary
            .push(delta)
            .and_then(|t| self.blocks.push(&t))
            .and_then(|t| self.fences.push(&t))
        {
            self.send(text);
        }
    }

    /// The model has stopped: release the text the filters were holding
    /// against a canary or a tag split across deltas, which nothing can
    /// complete any more.
    pub(crate) fn finish(&mut self) {
        let blocks_tail = self
            .canary
            .flush()
            .and_then(|t| self.blocks.push(&t))
            .into_iter()
            .chain(self.blocks.flush())
            .collect::<String>();
        let tail = self
            .fences
            .push(&blocks_tail)
            .into_iter()
            .chain(self.fences.flush())
            .collect::<String>();
        if !tail.is_empty() {
            self.send(tail);
        }
    }

    /// The text streamed so far will not be the reply: drop what is held and
    /// tell the sink to empty the bubble. Nothing is sent when nothing was.
    pub(crate) fn retract(&mut self) {
        self.canary.discard();
        self.blocks = ToolCallBlocksWithheld::default();
        self.fences = PlatformFencesWithheld::default();
        if mem::take(&mut self.sent) {
            tool_progress::deliver(self.sink, TurnEvent::ProseRetract);
        }
    }

    /// Whether any prose reached the sink since the last retraction: what a
    /// caller that learns later the text is not the reply has to take back.
    pub(crate) const fn sent(&self) -> bool {
        self.sent
    }

    fn send(&mut self, text: String) {
        self.sent = true;
        tool_progress::deliver(self.sink, TurnEvent::ProseDelta(text));
    }
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
/// A match stops prose for the rest of the stream. The assembled reply still
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
    /// Set once the canary matched; nothing more is released this stream.
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

    /// Drop what is held without releasing it. A trip stays tripped.
    fn discard(&mut self) {
        self.carry.clear();
    }

    fn trip(&mut self) {
        self.tripped = true;
        self.carry.clear();
        warn!(
            "canary token appeared in the streamed reply; prose forwarding \
             stopped for this stream and the held text was dropped"
        );
    }
}

/// Withholds `<tool_call>` blocks from streamed prose.
///
/// The shared system prompt teaches every tier to ask for a tool by writing a
/// `<tool_call>` block, and a Copilot SDK turn does so instead of calling its
/// MCP tools natively (carnet#1032); an API model on the same prompt can too,
/// and the API loop runs those blocks. The block is the platform's to run,
/// never the athlete's to read: the final reply strips it, but every delta
/// reaches the live bubble as it arrives, so the raw JSON was on screen until
/// the turn ended. A tag split across deltas is held until it can be told
/// apart from prose.
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

/// What opens a fence the platform lifts out of the reply: a chart
/// (`dravr-viz`) or the model's quick replies (`dravr-actions`).
const PLATFORM_FENCE_OPEN: &str = "```dravr-";

/// Any code fence's delimiter.
const FENCE: &str = "```";

/// Where the fence filter stands in the reply.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum FenceState {
    /// Prose: a line opening a platform fence starts one.
    #[default]
    Prose,
    /// Inside another code fence (```` ```json ````): forwarded as prose, and
    /// a platform opener inside it opens nothing.
    OtherFence,
    /// Inside a platform fence: dropped until it closes.
    PlatformFence,
}

/// Withholds the platform's own fences from streamed prose.
///
/// A reply can end on a `dravr-actions` fence (the model's quick replies) or
/// carry a `dravr-viz` fence (a chart); post-processing lifts both out of the
/// stored reply, and the `done` frame redraws the bubble from it. Every delta
/// reaches the live bubble as it arrives, so without this the raw fence would
/// flash there until the turn ended (carnet#1486). A line that opens one is
/// held, with everything after it, until the fence closes, and then dropped;
/// a line that could still grow into an opener is held until it cannot. A
/// fence still open when the stream ends is dropped.
#[derive(Default)]
struct PlatformFencesWithheld {
    /// The current line so far, its released start included.
    line: String,
    /// How much of `line` already reached the sink.
    released: usize,
    /// Where the filter stands.
    state: FenceState,
}

impl PlatformFencesWithheld {
    /// Take released prose, return what may reach the sink now.
    fn push(&mut self, text: &str) -> Option<String> {
        let mut out = String::new();
        for piece in text.split_inclusive('\n') {
            self.line.push_str(piece);
            if piece.ends_with('\n') {
                self.end_line(&mut out);
            } else {
                self.release_partial(&mut out);
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// The current line is whole, its newline included.
    fn end_line(&mut self, out: &mut String) {
        let line = mem::take(&mut self.line);
        let released = mem::take(&mut self.released);
        let opener = line.trim_start();
        let closes = opener.trim_end() == FENCE;
        match self.state {
            FenceState::Prose if opener.starts_with(PLATFORM_FENCE_OPEN) => {
                self.state = FenceState::PlatformFence;
            }
            FenceState::Prose => {
                if opener.starts_with(FENCE) {
                    self.state = FenceState::OtherFence;
                }
                out.push_str(&line[released..]);
            }
            FenceState::OtherFence => {
                if closes {
                    self.state = FenceState::Prose;
                }
                out.push_str(&line[released..]);
            }
            FenceState::PlatformFence => {
                if closes {
                    self.state = FenceState::Prose;
                }
            }
        }
    }

    /// Release the unfinished line's new text once the line can no longer
    /// open a platform fence.
    fn release_partial(&mut self, out: &mut String) {
        let start = self.line.trim_start();
        let held = match self.state {
            FenceState::PlatformFence => true,
            FenceState::OtherFence => false,
            FenceState::Prose => {
                PLATFORM_FENCE_OPEN.starts_with(start) || start.starts_with(PLATFORM_FENCE_OPEN)
            }
        };
        if !held {
            out.push_str(&self.line[self.released..]);
            self.released = self.line.len();
        }
    }

    /// Release what is still held once the model has stopped producing. A
    /// platform fence, or its opener, left unfinished is dropped.
    fn flush(&mut self) -> Option<String> {
        let line = mem::take(&mut self.line);
        let released = mem::take(&mut self.released);
        let state = mem::take(&mut self.state);
        let dropped = state == FenceState::PlatformFence
            || (state == FenceState::Prose && line.trim_start().starts_with(PLATFORM_FENCE_OPEN));
        let rest = &line[released..];
        (!dropped && !rest.is_empty()).then(|| rest.to_owned())
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

#[cfg(test)]
mod tests {
    use pierre_services::chat_stream::TurnEvent;
    use tokio::sync::mpsc;

    use super::{PlatformFencesWithheld, StreamedProse, ToolCallBlocksWithheld};

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

    /// Every event the sink received, deltas as their text and a retraction as
    /// `None`.
    fn received(rx: &mut mpsc::UnboundedReceiver<TurnEvent>) -> Vec<Option<String>> {
        let mut seen = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                TurnEvent::ProseDelta(text) => seen.push(Some(text)),
                TurnEvent::ProseRetract => seen.push(None),
                other => panic!("prose sends only deltas and retractions, got {other:?}"),
            }
        }
        seen
    }

    #[test]
    fn a_retraction_follows_only_prose_that_was_sent() {
        let (sink, mut rx) = mpsc::unbounded_channel();
        let mut prose = StreamedProse::new(&sink, "");
        prose.retract();
        assert!(
            received(&mut rx).is_empty(),
            "nothing was sent, so there is nothing to take back"
        );
        prose.push("Je regarde");
        prose.retract();
        prose.retract();
        assert_eq!(
            received(&mut rx),
            vec![Some("Je regarde".to_owned()), None],
            "one retraction takes back what was sent, and a second has nothing left to take"
        );
    }

    #[test]
    fn a_retraction_drops_text_still_held_back() {
        let (sink, mut rx) = mpsc::unbounded_channel();
        let mut prose = StreamedProse::new(&sink, "");
        prose.push("Je regarde <tool");
        prose.retract();
        prose.finish();
        assert_eq!(
            received(&mut rx),
            vec![Some("Je regarde ".to_owned()), None],
            "the held tag prefix belongs to the retracted text and is never released after it"
        );
    }

    /// What reaches the reader when the reply streams as `deltas`.
    fn fenced(deltas: &[&str]) -> String {
        let mut fences = PlatformFencesWithheld::default();
        let mut out: String = deltas.iter().filter_map(|d| fences.push(d)).collect();
        out.extend(fences.flush());
        out
    }

    /// Every way to cut `reply` in two at a character boundary.
    fn every_split(reply: &str) -> Vec<String> {
        reply
            .char_indices()
            .map(|(at, _)| at)
            .chain([reply.len()])
            .map(|at| fenced(&[&reply[..at], &reply[at..]]))
            .collect()
    }

    #[test]
    fn a_quick_reply_fence_never_reaches_the_reader_however_it_is_split() {
        let reply = "Easy spin or rest?\n```dravr-actions\n[\"Easy spin\", \"Rest day\"]\n```\n";
        for streamed in every_split(reply) {
            assert_eq!(
                streamed, "Easy spin or rest?\n",
                "bug: the dravr-actions fence's opener or body reached the bubble"
            );
        }
        let one_char_at_a_time: Vec<String> = reply.chars().map(String::from).collect();
        let deltas: Vec<&str> = one_char_at_a_time.iter().map(String::as_str).collect();
        assert_eq!(
            fenced(&deltas),
            "Easy spin or rest?\n",
            "bug: a fence streamed one character at a time reached the bubble"
        );
    }

    #[test]
    fn a_chart_fence_is_held_the_same_way() {
        let reply = "Here:\n```dravr-viz\n{\"kind\":\"bar\"}\n```\nDone.";
        for streamed in every_split(reply) {
            assert_eq!(
                streamed, "Here:\nDone.",
                "bug: the dravr-viz fence reached the bubble"
            );
        }
    }

    #[test]
    fn a_fence_left_open_is_dropped_and_the_prose_before_it_kept() {
        assert_eq!(
            fenced(&["Rest well.\n", "```dravr-actions\n[\"Ok\""]),
            "Rest well.\n",
            "bug: an unterminated platform fence must be dropped"
        );
        assert_eq!(
            fenced(&["Rest well.\n```dravr-act"]),
            "Rest well.\n",
            "bug: an opener cut off by the end of the stream must be dropped"
        );
    }

    #[test]
    fn another_code_fence_is_forwarded_as_today() {
        let reply = "Config:\n```json\n{\"a\": 1}\n```dravr-actions\n```\nEnd";
        for streamed in every_split(reply) {
            assert_eq!(
                streamed, reply,
                "bug: a json fence must be forwarded untouched"
            );
        }
    }

    #[test]
    fn the_stream_gate_drops_the_fence_end_to_end() {
        let (sink, mut rx) = mpsc::unbounded_channel();
        let mut prose = StreamedProse::new(&sink, "");
        for delta in [
            "Easy ",
            "spin?\n``",
            "`dravr-actions\n[\"Easy",
            " spin\"]\n```",
        ] {
            prose.push(delta);
        }
        prose.finish();
        let text: String = received(&mut rx).into_iter().flatten().collect();
        assert_eq!(
            text, "Easy spin?\n",
            "bug: the stream must never carry the fence"
        );
    }
}
