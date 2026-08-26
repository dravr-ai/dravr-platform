// ABOUTME: The one live vocabulary of a chat turn — progress, prose deltas, blocks, and the terminal frame
// ABOUTME: Shared by the pipeline that produces the events and the route that serializes them as SSE frames
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn event stream.
//!
//! One rail carries everything a client learns while a turn runs and when it
//! ends: the stages and tool calls the pipeline is working through, the
//! assistant text as the model produces it, each renderable block the reply
//! resolved to, and exactly one terminal event. There is no second stream to
//! correlate against and no per-turn handshake to open one.
//!
//! [`TurnEvent::frame`] owns the wire naming, so the event a producer emits
//! and the SSE frame a client reads cannot drift apart: adding a variant
//! forces its frame name to be decided in the same place.
//!
//! The type is deliberately decoupled from `embacle::HeadlessStreamEvent` and
//! from the HTTP crate's response DTOs — the terminal payloads arrive
//! pre-serialized — so neither the tool-loop crates nor the pipeline take a
//! dependency on the other's types through this hop.

use serde_json::{json, Value};
use tokio::sync::mpsc;

/// Where a piece of progress came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressKind {
    /// A pipeline stage — prompt assembly, dispatch, and the rest of the
    /// turn's fixed sequence.
    Stage,
    /// A tool the model asked for during the turn's tool loop.
    Tool,
}

impl ProgressKind {
    /// The wire word for this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Tool => "tool",
        }
    }
}

/// One thing the turn is doing right now.
///
/// A snapshot of the activity's latest known state rather than a delta, so a
/// consumer can either accumulate by `id` or simply render the most recent
/// one it received.
#[derive(Debug, Clone)]
pub struct TurnProgress {
    /// Whether this is a pipeline stage or a tool call.
    pub kind: ProgressKind,
    /// Stable identifier: the stage's name, or the tool call's protocol id.
    pub id: String,
    /// What to name in a status line — the stage name, or the tool's title.
    pub title: String,
    /// Latest state, in the producer's own vocabulary: `"started"` /
    /// `"finished"` for a stage, and the ACP call state (`"Pending"`,
    /// `"InProgress"`, `"Completed"`, …) for a tool.
    pub status: String,
}

impl TurnProgress {
    /// A pipeline stage entering `status`.
    #[must_use]
    pub fn stage(name: &str, status: &str) -> Self {
        Self {
            kind: ProgressKind::Stage,
            id: name.to_owned(),
            title: name.to_owned(),
            status: status.to_owned(),
        }
    }

    /// A tool call observed by the tool loop.
    #[must_use]
    pub const fn tool(id: String, title: String, status: String) -> Self {
        Self {
            kind: ProgressKind::Tool,
            id,
            title,
            status,
        }
    }
}

/// Status a stage reports as it is entered.
pub const STAGE_STATUS_STARTED: &str = "started";
/// Status a stage reports as it is left.
pub const STAGE_STATUS_FINISHED: &str = "finished";

/// One event on a turn's stream.
///
/// A turn emits any number of [`Self::Progress`], [`Self::ProseDelta`] and
/// [`Self::Block`] events, then exactly one of [`Self::Done`] or
/// [`Self::Failed`]. Nothing follows the terminal event.
#[derive(Debug, Clone)]
pub enum TurnEvent {
    /// A stage or tool call reported its latest state.
    Progress(TurnProgress),
    /// The next slice of assistant text to append to the in-flight bubble.
    ProseDelta(String),
    /// One renderable piece of the reply, in the order the server decided it.
    ///
    /// Pre-serialized because the block shape is owned by the HTTP layer's
    /// response DTOs, which sit above this crate.
    Block(Value),
    /// The turn finished. Carries the whole turn envelope — the same document
    /// a non-streaming caller receives as the response body.
    Done(Value),
    /// The turn did not finish. Carries the sanitized, client-safe reason;
    /// raw internals never reach this variant.
    Failed(String),
}

impl TurnEvent {
    /// Whether this event ends the stream.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done(_) | Self::Failed(_))
    }

    /// Serialize this event as an SSE `(event name, data)` pair.
    ///
    /// The single place the wire names live. A client reads exactly these
    /// five frame names off a turn body.
    #[must_use]
    pub fn frame(self) -> (&'static str, String) {
        match self {
            Self::Progress(progress) => (
                "progress",
                json!({
                    "kind": progress.kind.as_str(),
                    "id": progress.id,
                    "title": progress.title,
                    "status": progress.status,
                })
                .to_string(),
            ),
            Self::ProseDelta(delta) => ("delta", json!({ "delta": delta }).to_string()),
            Self::Block(block) => ("block", block.to_string()),
            Self::Done(turn) => ("done", turn.to_string()),
            Self::Failed(reason) => ("failed", json!({ "error": reason }).to_string()),
        }
    }
}

/// Sink the pipeline and the tool loop forward [`TurnEvent`]s into.
///
/// A plain `tokio::sync::mpsc::UnboundedSender` rather than a trait so the
/// route layer can drop the receiver to abort the stream (the closed channel
/// surfaces as a benign send error upstream), and so the terminal event rides
/// the same ordered channel as everything before it.
pub type TurnEventSink = mpsc::UnboundedSender<TurnEvent>;
