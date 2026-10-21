-- ABOUTME: user_home_preferences gains getting_started_hidden — whether the athlete hid Home's Getting started (carnet#1098)
-- ABOUTME: Additive only: one boolean column defaulting to shown; existing rows keep every other choice

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Getting started sits at the top of Today until the athlete's first steps are
-- done. Its Hide link is a per-person choice that web and phone agree on, and
-- Settings brings it back, so it lives beside plan_suggestion_hidden.
ALTER TABLE user_home_preferences ADD COLUMN IF NOT EXISTS getting_started_hidden BOOLEAN NOT NULL DEFAULT FALSE;
