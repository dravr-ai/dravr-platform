// ABOUTME: The seam the chat pipeline uses to hand an ACP provider Dravr's own MCP tools
// ABOUTME: Trait only — the impl lives in pierre-server, where auth and signing keys are

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use pierre_llm::McpServerConfig;

/// Mints the MCP servers an ACP-managed provider (Copilot Headless) exposes to
/// the model for native tool calling on a turn.
///
/// Implemented in `pierre-server` (where auth + signing keys live) so the chat
/// pipeline stays free of auth dependencies. The returned config points the
/// agent at Dravr's own `/mcp` endpoint with a freshly-minted, short-TTL,
/// `/mcp`-audience Bearer token scoped to `(user, tenant)`. Returns empty when
/// the bridge is disabled or the token cannot be minted.
#[async_trait::async_trait]
pub trait McpBridgeProvider: Send + Sync {
    /// Build the per-turn MCP server list for `(user_id, tenant_id)`.
    ///
    /// `conversation_id` is the turn's own conversation. It travels into the
    /// minted token so a tool the model calls natively — in a separate HTTP
    /// request, where the pipeline's task-local is out of scope — can still
    /// route detached follow-up work back to the channel that asked.
    async fn mcp_servers_for(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        conversation_id: &str,
    ) -> Vec<McpServerConfig>;
}
