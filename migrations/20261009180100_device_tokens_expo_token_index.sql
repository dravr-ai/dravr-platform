-- ABOUTME: Index on the active rows of device_tokens by Expo push token (SQLite)
-- ABOUTME: Registering a token looks up every other account's active row for it (carnet#301)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A token addresses one install, so dravr-commere's upsert retires the active
-- rows other accounts hold for it. The table's unique key leads with user_id,
-- which that lookup does not have, and the mobile app registers on every
-- entry; without this index each registration scans the table.
CREATE INDEX IF NOT EXISTS idx_device_tokens_expo_token_active
    ON device_tokens(expo_push_token) WHERE active = 1;
