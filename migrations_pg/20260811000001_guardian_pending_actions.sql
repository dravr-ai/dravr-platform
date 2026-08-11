-- ABOUTME: guardian_pending_actions — parked destructive tool calls awaiting /confirm or /deny (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration with PG-native types (BIGINT epoch, TIMESTAMPTZ timestamps)

-- See the matching SQLite migration for the full rationale. Summary: the
-- Guardian's TaintedDestructive::Confirm mode parks a destructive tool call
-- here instead of executing it; /confirm atomically claims the single-use row
-- and re-dispatches, /deny discards, expiry is checked at resolution time.
--
-- tenant_id / user_id stay TEXT (the short_links precedent) to dodge the
-- VARCHAR-tenant-id-decoded-as-native-UUID bind trap; `expires_at` is a Unix
-- epoch (seconds) so the TTL comparison stays integer-pure and identical to
-- SQLite. `arguments` is the tool-call JSON verbatim — re-dispatched, never
-- echoed to the user.

CREATE TABLE IF NOT EXISTS guardian_pending_actions (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    user_id         TEXT NOT NULL,
    conversation_id TEXT,
    tool_name       TEXT NOT NULL,
    arguments       TEXT NOT NULL,
    deny_reason     TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'confirmed', 'denied', 'expired')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      BIGINT NOT NULL,
    resolved_at     TIMESTAMPTZ
);

-- Claims filter on id + owner + status; the sweep deletes expired rows.
CREATE INDEX IF NOT EXISTS idx_guardian_pending_actions_expires_at
    ON guardian_pending_actions(expires_at);
