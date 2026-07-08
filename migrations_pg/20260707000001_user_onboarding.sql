-- ABOUTME: user_onboarding table — durable per-user onboarding step completion state (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration with PG-native TIMESTAMPTZ; all id columns kept TEXT on purpose

-- See the matching SQLite migration for the full rationale. Summary: one row per
-- (user_id, step_id) the user has reached ('complete' | 'skipped'); a missing row
-- means pending. Server-driven onboarding state replaces the web flow's per-device
-- localStorage flags so progress follows the user across devices.
--
-- user_id / tenant_id are TEXT (not native uuid) so binds are plain strings — this
-- deliberately dodges the VARCHAR-tenant-id-decoded-as-native-UUID trap. tenant_id
-- is a nullable best-effort audit column; resolution is by user_id.
CREATE TABLE IF NOT EXISTS user_onboarding (
    user_id        TEXT NOT NULL,
    step_id        TEXT NOT NULL,
    status         TEXT NOT NULL,
    chosen_channel TEXT,
    tenant_id      TEXT,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, step_id)
);
