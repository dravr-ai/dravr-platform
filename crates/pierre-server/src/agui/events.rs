// ABOUTME: AG-UI protocol event types mirroring the ag-ui-protocol wire format
// ABOUTME: Types serialize as JSON objects tagged by `type` — fully interop with AG-UI clients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! AG-UI event vocabulary.
//!
//! Mirror of the [AG-UI protocol](https://github.com/ag-ui-protocol/ag-ui)
//! event schema. Kept in-crate so `pierre-server` does not depend on a
//! specific `embacle` release carrying the same types; the wire format
//! (JSON objects tagged by `type`) is identical so any AG-UI client can
//! consume events emitted here.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

/// AG-UI event emitted by an agent run. See module docs for lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgUiEvent {
    /// A new agent run has started.
    #[serde(rename = "RUN_STARTED")]
    RunStarted {
        /// Unique identifier for this run.
        run_id: String,
        /// Conversation/thread the run belongs to.
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// A logical step inside a run has started.
    #[serde(rename = "STEP_STARTED")]
    StepStarted {
        /// Run identifier.
        run_id: String,
        /// Human-readable step name.
        step_name: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// A logical step inside a run has finished.
    #[serde(rename = "STEP_FINISHED")]
    StepFinished {
        /// Run identifier.
        run_id: String,
        /// Step name that matches a prior `STEP_STARTED`.
        step_name: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// The run has completed successfully.
    #[serde(rename = "RUN_FINISHED")]
    RunFinished {
        /// Run identifier.
        run_id: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// The run has terminated with an error.
    #[serde(rename = "RUN_ERROR")]
    RunError {
        /// Run identifier.
        run_id: String,
        /// Short machine-readable error code.
        code: String,
        /// Human-readable error message.
        message: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Start of a streamed assistant text message.
    #[serde(rename = "TEXT_MESSAGE_START")]
    TextMessageStart {
        /// Run identifier.
        run_id: String,
        /// Unique identifier for the streamed message.
        message_id: String,
        /// Author role (typically `"assistant"`).
        role: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Streamed text delta for an open `TEXT_MESSAGE_START`.
    #[serde(rename = "TEXT_MESSAGE_CONTENT")]
    TextMessageContent {
        /// Run identifier.
        run_id: String,
        /// Message identifier matching a prior `TEXT_MESSAGE_START`.
        message_id: String,
        /// Delta text to append.
        delta: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// End of a streamed assistant text message.
    #[serde(rename = "TEXT_MESSAGE_END")]
    TextMessageEnd {
        /// Run identifier.
        run_id: String,
        /// Message identifier matching a prior `TEXT_MESSAGE_START`.
        message_id: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// The agent has decided to invoke a tool.
    #[serde(rename = "TOOL_CALL_START")]
    ToolCallStart {
        /// Run identifier.
        run_id: String,
        /// Unique identifier for this tool call.
        tool_call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Streamed tool-call argument delta.
    #[serde(rename = "TOOL_CALL_ARGS")]
    ToolCallArgs {
        /// Run identifier.
        run_id: String,
        /// Tool call identifier matching a prior `TOOL_CALL_START`.
        tool_call_id: String,
        /// Delta of the serialized JSON arguments.
        delta: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Tool invocation has completed.
    #[serde(rename = "TOOL_CALL_END")]
    ToolCallEnd {
        /// Run identifier.
        run_id: String,
        /// Tool call identifier matching a prior `TOOL_CALL_START`.
        tool_call_id: String,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Tool execution produced a result.
    #[serde(rename = "TOOL_CALL_RESULT")]
    ToolCallResult {
        /// Run identifier.
        run_id: String,
        /// Tool call identifier matching a prior `TOOL_CALL_START`.
        tool_call_id: String,
        /// Result payload as an opaque JSON value.
        result: Value,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Full snapshot of the agent's shared state.
    #[serde(rename = "STATE_SNAPSHOT")]
    StateSnapshot {
        /// Run identifier.
        run_id: String,
        /// Full state document.
        snapshot: Value,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Incremental state update as a JSON Patch document.
    #[serde(rename = "STATE_DELTA")]
    StateDelta {
        /// Run identifier.
        run_id: String,
        /// JSON Patch array describing the delta.
        delta: Value,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Full snapshot of the current message list.
    #[serde(rename = "MESSAGES_SNAPSHOT")]
    MessagesSnapshot {
        /// Run identifier.
        run_id: String,
        /// Messages as an opaque JSON array.
        messages: Value,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Raw, provider-specific event passthrough.
    #[serde(rename = "RAW")]
    Raw {
        /// Run identifier.
        run_id: String,
        /// Provider label.
        source: String,
        /// Raw payload.
        payload: Value,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },

    /// Custom application-defined event.
    #[serde(rename = "CUSTOM")]
    Custom {
        /// Run identifier.
        run_id: String,
        /// Custom event name.
        name: String,
        /// Arbitrary payload.
        payload: Value,
        /// Milliseconds since the Unix epoch.
        timestamp: u64,
    },
}

impl AgUiEvent {
    /// Return the discriminant kind without cloning the event.
    #[must_use]
    pub const fn kind(&self) -> AgUiEventKind {
        match self {
            Self::RunStarted { .. } => AgUiEventKind::RunStarted,
            Self::StepStarted { .. } => AgUiEventKind::StepStarted,
            Self::StepFinished { .. } => AgUiEventKind::StepFinished,
            Self::RunFinished { .. } => AgUiEventKind::RunFinished,
            Self::RunError { .. } => AgUiEventKind::RunError,
            Self::TextMessageStart { .. } => AgUiEventKind::TextMessageStart,
            Self::TextMessageContent { .. } => AgUiEventKind::TextMessageContent,
            Self::TextMessageEnd { .. } => AgUiEventKind::TextMessageEnd,
            Self::ToolCallStart { .. } => AgUiEventKind::ToolCallStart,
            Self::ToolCallArgs { .. } => AgUiEventKind::ToolCallArgs,
            Self::ToolCallEnd { .. } => AgUiEventKind::ToolCallEnd,
            Self::ToolCallResult { .. } => AgUiEventKind::ToolCallResult,
            Self::StateSnapshot { .. } => AgUiEventKind::StateSnapshot,
            Self::StateDelta { .. } => AgUiEventKind::StateDelta,
            Self::MessagesSnapshot { .. } => AgUiEventKind::MessagesSnapshot,
            Self::Raw { .. } => AgUiEventKind::Raw,
            Self::Custom { .. } => AgUiEventKind::Custom,
        }
    }

    /// Return the run identifier this event belongs to.
    #[must_use]
    pub fn run_id(&self) -> &str {
        match self {
            Self::RunStarted { run_id, .. }
            | Self::StepStarted { run_id, .. }
            | Self::StepFinished { run_id, .. }
            | Self::RunFinished { run_id, .. }
            | Self::RunError { run_id, .. }
            | Self::TextMessageStart { run_id, .. }
            | Self::TextMessageContent { run_id, .. }
            | Self::TextMessageEnd { run_id, .. }
            | Self::ToolCallStart { run_id, .. }
            | Self::ToolCallArgs { run_id, .. }
            | Self::ToolCallEnd { run_id, .. }
            | Self::ToolCallResult { run_id, .. }
            | Self::StateSnapshot { run_id, .. }
            | Self::StateDelta { run_id, .. }
            | Self::MessagesSnapshot { run_id, .. }
            | Self::Raw { run_id, .. }
            | Self::Custom { run_id, .. } => run_id,
        }
    }

    /// Convenience constructor for `RUN_STARTED`.
    #[must_use]
    pub fn run_started(run_id: impl Into<String>, thread_id: Option<&str>) -> Self {
        Self::RunStarted {
            run_id: run_id.into(),
            thread_id: thread_id.map(String::from),
            timestamp: now_ms(),
        }
    }

    /// Convenience constructor for `STEP_STARTED`.
    #[must_use]
    pub fn step_started(run_id: impl Into<String>, step_name: impl Into<String>) -> Self {
        Self::StepStarted {
            run_id: run_id.into(),
            step_name: step_name.into(),
            timestamp: now_ms(),
        }
    }

    /// Convenience constructor for `STEP_FINISHED`.
    #[must_use]
    pub fn step_finished(run_id: impl Into<String>, step_name: impl Into<String>) -> Self {
        Self::StepFinished {
            run_id: run_id.into(),
            step_name: step_name.into(),
            timestamp: now_ms(),
        }
    }

    /// Convenience constructor for `RUN_FINISHED`.
    #[must_use]
    pub fn run_finished(run_id: impl Into<String>) -> Self {
        Self::RunFinished {
            run_id: run_id.into(),
            timestamp: now_ms(),
        }
    }

    /// Convenience constructor for `RUN_ERROR`.
    #[must_use]
    pub fn run_error(
        run_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::RunError {
            run_id: run_id.into(),
            code: code.into(),
            message: message.into(),
            timestamp: now_ms(),
        }
    }

    /// Convenience constructor for `TOOL_CALL_START`.
    #[must_use]
    pub fn tool_call_start(
        run_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        Self::ToolCallStart {
            run_id: run_id.into(),
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            timestamp: now_ms(),
        }
    }

    /// Convenience constructor for `TOOL_CALL_RESULT`.
    #[must_use]
    pub fn tool_call_result(
        run_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        result: Value,
    ) -> Self {
        Self::ToolCallResult {
            run_id: run_id.into(),
            tool_call_id: tool_call_id.into(),
            result,
            timestamp: now_ms(),
        }
    }
}

/// Discriminant for [`AgUiEvent`]. Used by the filter to allow or deny
/// classes of events at emission time.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum AgUiEventKind {
    /// Corresponds to `AgUiEvent::RunStarted`.
    RunStarted,
    /// Corresponds to `AgUiEvent::StepStarted`.
    StepStarted,
    /// Corresponds to `AgUiEvent::StepFinished`.
    StepFinished,
    /// Corresponds to `AgUiEvent::RunFinished`.
    RunFinished,
    /// Corresponds to `AgUiEvent::RunError`.
    RunError,
    /// Corresponds to `AgUiEvent::TextMessageStart`.
    TextMessageStart,
    /// Corresponds to `AgUiEvent::TextMessageContent`.
    TextMessageContent,
    /// Corresponds to `AgUiEvent::TextMessageEnd`.
    TextMessageEnd,
    /// Corresponds to `AgUiEvent::ToolCallStart`.
    ToolCallStart,
    /// Corresponds to `AgUiEvent::ToolCallArgs`.
    ToolCallArgs,
    /// Corresponds to `AgUiEvent::ToolCallEnd`.
    ToolCallEnd,
    /// Corresponds to `AgUiEvent::ToolCallResult`.
    ToolCallResult,
    /// Corresponds to `AgUiEvent::StateSnapshot`.
    StateSnapshot,
    /// Corresponds to `AgUiEvent::StateDelta`.
    StateDelta,
    /// Corresponds to `AgUiEvent::MessagesSnapshot`.
    MessagesSnapshot,
    /// Corresponds to `AgUiEvent::Raw`.
    Raw,
    /// Corresponds to `AgUiEvent::Custom`.
    Custom,
}

impl AgUiEventKind {
    /// Every kind, in stable declaration order.
    pub const ALL: [Self; 17] = [
        Self::RunStarted,
        Self::StepStarted,
        Self::StepFinished,
        Self::RunFinished,
        Self::RunError,
        Self::TextMessageStart,
        Self::TextMessageContent,
        Self::TextMessageEnd,
        Self::ToolCallStart,
        Self::ToolCallArgs,
        Self::ToolCallEnd,
        Self::ToolCallResult,
        Self::StateSnapshot,
        Self::StateDelta,
        Self::MessagesSnapshot,
        Self::Raw,
        Self::Custom,
    ];
}

/// Milliseconds since the Unix epoch. Saturates rather than panics on
/// system clocks reporting times outside the supported range.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
