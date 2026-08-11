-- ABOUTME: guardian_pending_actions — parked destructive tool calls awaiting /confirm or /deny (SQLite)
-- ABOUTME: Backs the Guardian's TaintedDestructive::Confirm human-in-the-loop flow; single-use, expiry at resolution

-- When a destructive tool fires in a tainted turn and the Guardian policy says
-- `confirm`, the call is parked here instead of executing, and the user gets a
-- deterministic prompt carrying the row id. `/confirm <id>` atomically claims
-- the row (single-use, owner-checked) and re-dispatches the stored call;
-- `/deny <id>` discards it. Expiry is checked at resolution time (the
-- short_links pattern) — no background job is load-bearing.
--
-- `id` is a uuid-simple token (32 hex chars, 122-bit entropy) so ids cannot be
-- guessed across users; ownership is still enforced on claim. `arguments` is
-- the tool-call JSON verbatim — stored to re-dispatch, NEVER echoed to the
-- user (it can carry injected content; that is why the row exists at all).
-- tenant_id / user_id are TEXT in both backends (the short_links precedent)
-- to dodge the VARCHAR-vs-native-UUID bind trap. `expires_at` is a Unix epoch
-- (seconds) so the TTL comparison is integer-pure and identical across
-- SQLite and PostgreSQL.

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
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at      INTEGER NOT NULL,
    resolved_at     TEXT
);

-- Claims filter on id + owner + status; the sweep deletes expired rows.
CREATE INDEX IF NOT EXISTS idx_guardian_pending_actions_expires_at
    ON guardian_pending_actions(expires_at);
