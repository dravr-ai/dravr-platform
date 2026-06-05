-- ABOUTME: Per-user admin tier override marker (PostgreSQL) — admin-set tier wins over Stripe webhook
-- ABOUTME: Row presence makes the billing webhook skip set_tier/set_plan for that user

CREATE TABLE IF NOT EXISTS user_tier_overrides (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    tier TEXT NOT NULL,
    note TEXT,
    set_by UUID REFERENCES users(id) ON DELETE SET NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_user_tier_overrides_set_by
    ON user_tier_overrides(set_by);
