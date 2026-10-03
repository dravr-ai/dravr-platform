// ABOUTME: The seam an ACP-managed provider reaches Dravr's own tools through
// ABOUTME: Returns a session guard, because the credential must die with the turn

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How a native-tool-calling provider is handed Dravr's tools.
//!
//! An ACP agent runs its own tool loop in its own subprocess and reaches its
//! caller only over MCP, so the tools it may call have to be published on a
//! listener it can dial. `embacle-tool-host` owns that listener; this seam is
//! how the pipeline asks for a turn-scoped session on it.
//!
//! The return type is a guard, not a config, and that is the whole point.
//! Dropping a [`ToolSession`] revokes its bearer, so a turn that ends —
//! normally, by error, or because the athlete walked away — leaves no live
//! credential an orphaned agent subprocess can still spend on an irreversible
//! action. A seam returning a bare `Vec<McpServerConfig>` would have to leak
//! the session to keep it valid.

use embacle_tool_host::ToolSession;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_tool_runtime::coach_seat::TurnSeat;

/// The turn a tool session is opened for.
///
/// `budget` is the turn's tool-call ceiling, already resolved by
/// `tool_budget::resolve_max_iterations`. It is passed rather than re-derived
/// so the agent's loop and the platform's own loop are bounded by one number
/// from one resolution.
///
/// `turn_id` is the Guardian turn key for this utterance, the same value the
/// in-process `ReAct` loop binds. The agent's loop runs in another process and
/// reaches the executor from a task outside any tool body, so the task-local
/// inherit cannot supply it; passing it here is what makes taint and the
/// per-turn blast-radius budgets accumulate across the loopback calls of one
/// message instead of resetting on every call.
///
/// `turn_agent_id` is the agent the turn answers as, passed for the same
/// reason: the loopback executor cannot inherit it, and a tool that records
/// authorship must name the agent the athlete was talking to.
///
/// `seat` is [`TurnSeat::Coach`] when the sender coaches the turn's group: the session
/// then withholds and refuses every tool that would read or write the coach's
/// own data (`pierre_tool_runtime::coach_seat`, carnet#742).
#[derive(Debug, Clone, Copy)]
pub struct ToolSessionTurn<'a> {
    /// The caller every tool runs as.
    pub user_id: &'a str,
    /// The tenant the caller's tools run under.
    pub tenant_id: TenantId,
    /// The conversation the turn belongs to.
    pub conversation_id: &'a str,
    /// The Guardian turn key for this utterance.
    pub turn_id: ConversationTurnId,
    /// The agent the turn answers as.
    pub turn_agent_id: Option<&'a str>,
    /// The turn's tool-call ceiling.
    pub budget: usize,
    /// Whose data the turn's tools run against.
    pub seat: TurnSeat,
}

/// Opens the turn-scoped tool session an ACP-managed provider calls into.
///
/// Implemented in `pierre-server`, where the tool registry and the executor
/// live. Returns `None` when native tool calling is disabled or a session
/// cannot be opened, in which case the turn proceeds with no tools rather than
/// failing — an agent that cannot reach data should say so, not error.
#[async_trait::async_trait]
pub trait McpBridgeProvider: Send + Sync {
    /// Open a session exposing this turn's tools to the agent.
    ///
    /// The caller holds the returned guard for exactly as long as the turn may
    /// legitimately call tools.
    async fn open_tool_session(&self, turn: ToolSessionTurn<'_>) -> Option<ToolSession>;
}
