-- ABOUTME: user_home_preferences — the per-user choices Home's layout honours on every device (SQLite)
-- ABOUTME: Today one: whether the athlete set aside the suggestion to build a training plan (carnet#820)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per user, written the first time the athlete changes a choice; no
-- row reads as every default. Scoped by user_id alone: the choice is the
-- person's, whichever tenant they are signed in to.
CREATE TABLE IF NOT EXISTS user_home_preferences (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    plan_suggestion_hidden INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
