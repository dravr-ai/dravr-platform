-- ABOUTME: Durable store for MCP Tasks extension handles (io.modelcontextprotocol/tasks)
-- ABOUTME: Backs the tronc TaskStore seam so a task handle survives a server restart
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Deliberately NOT a2a_tasks: that table has no tenant_id/user_id, its
-- session_token is NOT NULL (meaningless for the stateless 2026-07-28
-- transport), and its status CHECK pins the A2A vocabulary. MCP tasks are
-- owner-scoped from day one — every lookup filters on (tenant_id, user_id)
-- so a task id leaked across tenants reads as absent, never as forbidden.
--
-- created_at / last_updated_at hold the exact RFC3339 strings the engine
-- minted, so the wire Task round-trips byte-identical. expires_at_ms is the
-- precomputed unix-millisecond expiry (created_at + ttl_ms; NULL = unlimited
-- retention) so expiry filtering and the sweep are single indexed comparisons.
CREATE TABLE IF NOT EXISTS mcp_tasks (
    task_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('working', 'input_required', 'completed', 'failed', 'cancelled')),
    status_message TEXT,
    created_at TEXT NOT NULL,
    last_updated_at TEXT NOT NULL,
    ttl_ms BIGINT,
    poll_interval_ms BIGINT,
    expires_at_ms BIGINT,
    input_requests TEXT,
    result TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_mcp_tasks_owner ON mcp_tasks(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_mcp_tasks_expires ON mcp_tasks(expires_at_ms) WHERE expires_at_ms IS NOT NULL;
