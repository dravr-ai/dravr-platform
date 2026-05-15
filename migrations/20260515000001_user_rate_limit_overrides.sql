-- ABOUTME: Per-user rate-limit overrides (SQLite) — tier defaults + per-user exemption pattern
-- ABOUTME: Mirrors GitHub Enterprise ghe-config exemption + Stripe per-account custom limit

-- A row in user_rate_limit_overrides wins over the tier-keyed default for
-- that user. NULL daily_limit or monthly_limit on the override row means
-- "unlimited for this dimension" (same semantics as UserTier::Enterprise
-- returning None from monthly_limit()). No row at all means "use the tier
-- default" (current behaviour). Admin sets via PUT, clears via DELETE.
CREATE TABLE IF NOT EXISTS user_rate_limit_overrides (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    daily_limit INTEGER,
    monthly_limit INTEGER,
    note TEXT,
    set_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    set_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_user_rate_limit_overrides_set_by
    ON user_rate_limit_overrides(set_by);
