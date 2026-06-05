-- ABOUTME: Per-user admin tier override marker (SQLite) — admin-set tier wins over Stripe webhook
-- ABOUTME: Row presence makes the billing webhook skip set_tier/set_plan for that user

-- A row in user_tier_overrides records that an operator manually set the
-- user's billing tier outside the Stripe loop. While the row exists, the
-- billing webhook (subscription upsert / cancel) MUST NOT change users.tier
-- or the tenant plan — it still upserts the subscription row and logs the
-- skip. The admin handler writes users.tier as before AND records this row.
CREATE TABLE IF NOT EXISTS user_tier_overrides (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    tier TEXT NOT NULL,
    note TEXT,
    set_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    set_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_user_tier_overrides_set_by
    ON user_tier_overrides(set_by);
