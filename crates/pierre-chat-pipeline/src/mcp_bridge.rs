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
use pierre_core::models::TenantId;

/// Opens the turn-scoped tool session an ACP-managed provider calls into.
///
/// Implemented in `pierre-server`, where the tool registry and the executor
/// live. Returns `None` when native tool calling is disabled or a session
/// cannot be opened, in which case the turn proceeds with no tools rather than
/// failing — a coach that cannot reach data should say so, not error.
#[async_trait::async_trait]
pub trait McpBridgeProvider: Send + Sync {
    /// Open a session exposing this turn's tools to the agent.
    ///
    /// The caller holds the returned guard for exactly as long as the turn may
    /// legitimately call tools.
    ///
    /// `budget` is the turn's tool-call ceiling, already resolved by
    /// `tool_budget::resolve_max_iterations`. It is passed rather than
    /// re-derived so the agent's loop and the platform's own loop are bounded
    /// by one number from one resolution.
    async fn open_tool_session(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        conversation_id: &str,
        budget: usize,
    ) -> Option<ToolSession>;
}
