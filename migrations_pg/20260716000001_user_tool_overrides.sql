-- ABOUTME: Per-user admin tool override (PostgreSQL) — allow/deny one MCP tool for one user
-- ABOUTME: Overlay above tenant plan + tenant overrides; below GlobalDisabled, above everything else

CREATE TABLE IF NOT EXISTS user_tool_overrides (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tool_name VARCHAR(255) NOT NULL REFERENCES tool_catalog(tool_name) ON DELETE CASCADE,
    is_enabled BOOLEAN NOT NULL,
    set_by UUID REFERENCES users(id) ON DELETE SET NULL,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, tool_name)
);

CREATE INDEX IF NOT EXISTS idx_user_tool_overrides_set_by
    ON user_tool_overrides(set_by);
