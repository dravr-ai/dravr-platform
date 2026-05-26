// ABOUTME: Chat stream event types — token-level streaming surface shared by chat pipeline and tool loops
// ABOUTME: Decoupled from upstream runner crates so consumers don't take transitive deps via this hop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat stream event surface.
//!
//! [`ChatStreamEvent`] is the lingua franca emitted by tool-loop strategies
//! that support token-level streaming (currently the headless Copilot ACP
//! branch) and consumed by channel adapters that wrap the event stream in
//! an SSE response. The type is intentionally decoupled from
//! `embacle::HeadlessStreamEvent` so the rest of the workspace doesn't take
//! a transitive dependency on the runner crate.

use tokio::sync::mpsc;

/// Event emitted to a chat-stream sink while the pipeline runs.
///
/// Decoupled from the upstream `embacle::HeadlessStreamEvent` so the rest
/// of the workspace doesn't take a transitive dependency on the runner
/// crate via the hooks layer. The headless tool loop translates each
/// embacle event into one of these variants before forwarding.
#[derive(Debug, Clone)]
pub enum ChatStreamEvent {
    /// A partial assistant text chunk — the next slice to append to the
    /// in-flight assistant message bubble on the client.
    TextDelta(String),
    /// A tool call was observed (start or status update). Each event is a
    /// snapshot of the call's latest known state, so consumers can either
    /// accumulate or replace by `id`.
    ToolCall {
        /// Stable id of the tool call (from the underlying ACP protocol).
        id: String,
        /// Human-readable title describing the tool action.
        title: String,
        /// Latest status string (`"Pending"`, `"InProgress"`, `"Completed"`, ...).
        status: String,
    },
}

/// Sink that the chat pipeline forwards [`ChatStreamEvent`]s into when
/// the calling channel adapter wants progressive token-level UX.
///
/// A simple `tokio::sync::mpsc::UnboundedSender` rather than a custom
/// trait so the route layer can drop the receiver to abort the stream
/// (the closed channel surfaces as a benign send error in the pipeline).
pub type ChatStreamSink = mpsc::UnboundedSender<ChatStreamEvent>;
