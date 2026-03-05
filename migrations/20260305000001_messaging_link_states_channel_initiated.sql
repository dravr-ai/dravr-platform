-- ABOUTME: Extend link_states for webhook-initiated auth flow (nullable user_id, channel identity)
-- ABOUTME: Enables bots to generate login URLs so unlinked users can authenticate and link
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- SQLite cannot ALTER COLUMN, so we recreate the table with:
--   user_id TEXT (nullable — webhook-initiated links start without a user)
--   channel_user_id TEXT (sender's platform ID from webhook)
--   sender_name TEXT (display name from platform, for login page greeting)

-- Step 1: Rename old table
ALTER TABLE messaging_link_states RENAME TO messaging_link_states_old;

-- Step 2: Recreate with nullable user_id and new columns
CREATE TABLE messaging_link_states (
    id              TEXT    NOT NULL PRIMARY KEY,
    tenant_id       TEXT    NOT NULL,
    user_id         TEXT,            -- nullable: webhook-initiated links have no user yet
    channel_type    TEXT    NOT NULL, -- whatsapp, messenger, discord, slack, telegram
    code            TEXT    NOT NULL, -- Verification code (cryptographically random)
    method          TEXT    NOT NULL, -- deep_link or oauth
    used            INTEGER NOT NULL DEFAULT 0,
    channel_user_id TEXT,            -- Sender's platform ID (set by webhook handler)
    sender_name     TEXT,            -- Display name from platform (for login page greeting)
    expires_at      TEXT    NOT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Step 3: Copy existing data (old rows have user_id set, new columns default to NULL)
INSERT INTO messaging_link_states
    (id, tenant_id, user_id, channel_type, code, method, used, expires_at, created_at)
SELECT id, tenant_id, user_id, channel_type, code, method, used, expires_at, created_at
FROM messaging_link_states_old;

-- Step 4: Drop old table
DROP TABLE messaging_link_states_old;

-- Step 5: Recreate indexes
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_link_states_code
    ON messaging_link_states(code);

CREATE INDEX IF NOT EXISTS idx_messaging_link_states_expiry
    ON messaging_link_states(expires_at) WHERE used = 0;

-- Updated index: user_id is now nullable, so only index rows that have it
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_user_channel
    ON messaging_link_states(tenant_id, user_id, channel_type)
    WHERE used = 0 AND user_id IS NOT NULL;
