-- ABOUTME: user_home_preferences — the per-user choices Home's layout honours on every device (Postgres)
-- ABOUTME: Today one: whether the athlete set aside the suggestion to build a training plan (carnet#820)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per user, written the first time the athlete changes a choice; no
-- row reads as every default. Scoped by user_id alone: the choice is the
-- person's, whichever tenant they are signed in to.
CREATE TABLE IF NOT EXISTS user_home_preferences (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    plan_suggestion_hidden BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL
);
