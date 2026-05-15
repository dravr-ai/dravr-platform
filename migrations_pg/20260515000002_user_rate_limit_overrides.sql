-- ABOUTME: Per-user rate-limit overrides (PostgreSQL) — tier defaults + per-user exemption pattern
-- ABOUTME: Mirrors GitHub Enterprise ghe-config exemption + Stripe per-account custom limit

CREATE TABLE IF NOT EXISTS user_rate_limit_overrides (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    daily_limit INTEGER,
    monthly_limit INTEGER,
    note TEXT,
    set_by UUID REFERENCES users(id) ON DELETE SET NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_user_rate_limit_overrides_set_by
    ON user_rate_limit_overrides(set_by);
