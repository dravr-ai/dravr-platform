// ABOUTME: AG-UI protocol integration — event schema, filter, emitter, per-run broadcast registry
// ABOUTME: In-process progress vocabulary for messaging channels; no HTTP transport of its own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # AG-UI (Agent-User Interaction) integration
//!
//! Implements the [AG-UI protocol](https://github.com/ag-ui-protocol/ag-ui)
//! event schema so the messaging channels can render live progress feedback
//! while the chat pipeline runs an agent turn.
//!
//! ## Components
//!
//! - [`events`] — the AG-UI event enum and kind discriminant. Serializes
//!   to the AG-UI wire format (JSON objects tagged by `type`) so
//!   off-the-shelf AG-UI clients interop.
//! - [`filter`] — [`AgUiEventFilter`] configuration that narrows the set
//!   of event kinds a sink forwards. Operators can opt out of
//!   high-volume streams like `TEXT_MESSAGE_CONTENT` without changing
//!   the producer code.
//! - [`registry`] — [`RunRegistry`] maps `run_id` to a tokio broadcast
//!   channel; the pipeline publishes and the in-process status bridge
//!   consumes.
//! - [`emitter`] — [`AgUiSink`] trait with a [`NoopSink`] default and a
//!   registry-backed [`BroadcastSink`] for production use.
//!
//! ## Pipeline wiring
//!
//! The chat pipeline threads an optional `&dyn AgUiSink` through its hooks.
//! The messaging dispatcher constructs a [`BroadcastSink`] bound to a fresh
//! `run_id`, passes it in, and spawns
//! `pierre_services::messaging_status_bridge::spawn_status_consumer` against
//! the same id — both ends in process, inside one turn. The sink is silently
//! skipped when absent, so the feature is strictly opt-in per turn.
//!
//! There is no HTTP transport here and no client-facing `run_id`: web and
//! mobile read their progress off the turn's own event stream
//! (`pierre_services::chat_stream`), which carries the reply on the same
//! body.

pub mod emitter;
pub mod events;
pub mod filter;
pub mod registry;

pub use emitter::{AgUiSink, BroadcastSink, NoopSink};
pub use events::{AgUiEvent, AgUiEventKind};
pub use filter::AgUiEventFilter;
pub use registry::{PublishOutcome, RunRegistry, RunScope, RunSubscription};

/// Maximum number of recent AG-UI events retained per run for replay.
///
/// The status consumer, spawned just after the run is registered, receives
/// the buffered events before switching to live. Typical runs emit
/// 6–20 events (`RUN_STARTED`, a handful of `STEP_*`, `RUN_FINISHED`)
/// plus any tool-call + text-delta bursts; 256 leaves plenty of
/// headroom before the oldest entries start dropping.
pub const AGUI_RUN_REPLAY_BUFFER_SIZE: usize = 256;
