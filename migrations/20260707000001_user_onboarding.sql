-- ABOUTME: user_onboarding table — durable per-user onboarding step completion state (SQLite)
-- ABOUTME: Survives device changes / cache clears so the onboarding flow is server-driven, not localStorage-only

-- One row per (user_id, step_id) the user has reached. `status` is 'complete' or
-- 'skipped'; a missing row means the step is still pending. This replaces the
-- per-device localStorage flags the web flow used, so onboarding progress follows
-- the user across devices and unifies web + mobile.
--
-- All columns are TEXT (chosen_channel / tenant_id nullable) so there is no
-- INTEGER-as-i64 or VARCHAR-tenant-id-as-native-UUID decode trap on Postgres:
-- user_id / tenant_id are bound as strings. Resolution is by user_id (the
-- auth-token-derived isolation boundary, same as the provider-connection gate);
-- tenant_id is a best-effort audit column (the onboarding JWT may carry no tenant).
CREATE TABLE IF NOT EXISTS user_onboarding (
    user_id        TEXT NOT NULL,
    step_id        TEXT NOT NULL,
    status         TEXT NOT NULL,
    chosen_channel TEXT,
    tenant_id      TEXT,
    updated_at     TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, step_id)
);
