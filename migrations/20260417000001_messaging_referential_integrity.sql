-- ABOUTME: Add FK constraints to messaging_* tables so orphan ids can never be inserted
-- ABOUTME: Rebuilds tables with REFERENCES to users/tenants/chat_conversations and CASCADE

-- See the matching PostgreSQL migration for the full rationale. Summary: messaging_*
-- tables historically had no referential integrity on user_id/tenant_id/
-- pierre_conversation_id. A production incident on 2026-04-16 exposed orphan ids
-- breaking WhatsApp dispatch. This migration sweeps orphans then rebuilds each
-- table with ON DELETE CASCADE foreign keys.

-- ============================================================================
-- Step 1: pre-flight orphan cleanup.
-- ============================================================================

DELETE FROM messaging_link_states
WHERE user_id IS NOT NULL
  AND user_id NOT IN (SELECT id FROM users);

DELETE FROM messaging_link_states
WHERE tenant_id NOT IN (SELECT id FROM tenants);

DELETE FROM messaging_channel_links
WHERE user_id NOT IN (SELECT id FROM users);

DELETE FROM messaging_channel_links
WHERE tenant_id NOT IN (SELECT id FROM tenants);

-- messaging_messages.session_id references messaging_sessions(id); delete
-- dependent rows before the session DELETE to avoid FK restrict.
DELETE FROM messaging_messages
WHERE session_id IN (
    SELECT id FROM messaging_sessions
    WHERE user_id NOT IN (SELECT id FROM users)
       OR tenant_id NOT IN (SELECT id FROM tenants)
);

DELETE FROM messaging_sessions
WHERE user_id NOT IN (SELECT id FROM users);

DELETE FROM messaging_sessions
WHERE tenant_id NOT IN (SELECT id FROM tenants);

UPDATE messaging_sessions
SET pierre_conversation_id = NULL
WHERE pierre_conversation_id IS NOT NULL
  AND pierre_conversation_id NOT IN (SELECT id FROM chat_conversations);

-- ============================================================================
-- Step 2: rebuild messaging_link_states with FK constraints.
-- SQLite does not support ALTER TABLE ADD CONSTRAINT, so we rebuild.
-- ============================================================================

CREATE TABLE messaging_link_states_new (
    id              TEXT    NOT NULL PRIMARY KEY,
    tenant_id       TEXT    NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id         TEXT    REFERENCES users(id) ON DELETE CASCADE,
    channel_type    TEXT    NOT NULL,
    code            TEXT    NOT NULL,
    method          TEXT    NOT NULL,
    used            INTEGER NOT NULL DEFAULT 0,
    channel_user_id TEXT,
    sender_name     TEXT,
    expires_at      TEXT    NOT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    otp_step        TEXT,
    email           TEXT,
    otp_hash        TEXT,
    otp_attempts    INTEGER NOT NULL DEFAULT 0
);

INSERT INTO messaging_link_states_new
    (id, tenant_id, user_id, channel_type, code, method, used,
     channel_user_id, sender_name, expires_at, created_at,
     otp_step, email, otp_hash, otp_attempts)
SELECT
    id, tenant_id, user_id, channel_type, code, method, used,
    channel_user_id, sender_name, expires_at, created_at,
    otp_step, email, otp_hash, otp_attempts
FROM messaging_link_states;

DROP TABLE messaging_link_states;
ALTER TABLE messaging_link_states_new RENAME TO messaging_link_states;

CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_link_states_code
    ON messaging_link_states(code);
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_expiry
    ON messaging_link_states(expires_at) WHERE used = 0;
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_user_channel
    ON messaging_link_states(tenant_id, user_id, channel_type)
    WHERE used = 0 AND user_id IS NOT NULL;

-- ============================================================================
-- Step 3: rebuild messaging_channel_links.
-- ============================================================================

CREATE TABLE messaging_channel_links_new (
    id              TEXT    NOT NULL PRIMARY KEY,
    tenant_id       TEXT    NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id         TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_type    TEXT    NOT NULL,
    channel_user_id TEXT    NOT NULL,
    display_name    TEXT,
    linked_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(tenant_id, channel_type, channel_user_id)
);

INSERT INTO messaging_channel_links_new
    (id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at)
SELECT
    id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
FROM messaging_channel_links;

DROP TABLE messaging_channel_links;
ALTER TABLE messaging_channel_links_new RENAME TO messaging_channel_links;

CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_user
    ON messaging_channel_links(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_channel_identity
    ON messaging_channel_links(tenant_id, channel_type, channel_user_id);

-- ============================================================================
-- Step 4: rebuild messaging_sessions with FK on user_id, tenant_id, and
-- pierre_conversation_id. ON DELETE SET NULL for the conversation ref — the
-- self-heal layer reconstructs a fresh conversation when it sees NULL.
-- ============================================================================

CREATE TABLE messaging_sessions_new (
    id                          TEXT    NOT NULL PRIMARY KEY,
    user_id                     TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id                   TEXT    NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    channel_type                TEXT    NOT NULL,
    channel_user_id             TEXT    NOT NULL,
    channel_conversation_id     TEXT,
    pierre_conversation_id      TEXT    REFERENCES chat_conversations(id) ON DELETE SET NULL,
    last_message_at             TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at                  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

INSERT INTO messaging_sessions_new
    (id, user_id, tenant_id, channel_type, channel_user_id,
     channel_conversation_id, pierre_conversation_id,
     last_message_at, created_at)
SELECT
    id, user_id, tenant_id, channel_type, channel_user_id,
    channel_conversation_id, pierre_conversation_id,
    last_message_at, created_at
FROM messaging_sessions;

DROP TABLE messaging_sessions;
ALTER TABLE messaging_sessions_new RENAME TO messaging_sessions;

CREATE INDEX IF NOT EXISTS idx_messaging_sessions_tenant
    ON messaging_sessions(tenant_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_sessions_channel_identity
    ON messaging_sessions(tenant_id, channel_type, channel_user_id);
CREATE INDEX IF NOT EXISTS idx_messaging_sessions_user
    ON messaging_sessions(user_id, tenant_id);
