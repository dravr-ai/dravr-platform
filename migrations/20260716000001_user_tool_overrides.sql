-- ABOUTME: Per-user admin tool override (SQLite) — allow/deny one MCP tool for one user
-- ABOUTME: Overlay above tenant plan + tenant overrides; below GlobalDisabled, above everything else

-- A row records that an operator explicitly enabled or disabled a single MCP
-- tool for one user, independent of the user's tenant plan or any tenant-level
-- tool override. ToolSelectionService consults it as a per-request overlay on
-- top of the (cached) tenant computation: a user override wins over plan
-- restriction, tenant override, and catalog default — but a globally-disabled
-- tool (PIERRE_DISABLED_TOOLS) stays off and is never resurrected here.
CREATE TABLE IF NOT EXISTS user_tool_overrides (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL REFERENCES tool_catalog(tool_name) ON DELETE CASCADE,
    is_enabled INTEGER NOT NULL,
    set_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, tool_name)
);

CREATE INDEX IF NOT EXISTS idx_user_tool_overrides_set_by
    ON user_tool_overrides(set_by);
