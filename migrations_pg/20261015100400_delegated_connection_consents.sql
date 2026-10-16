-- ABOUTME: Per-link consents on delegated_connections — what the member lets a confirmed link do beyond reading workouts (PostgreSQL)
-- ABOUTME: Additive only: calendar_write and wellness_read, each off until the member grants it, with when each last changed

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. Every existing
-- link starts with both consents off; each *_at records when the member last
-- granted or revoked it, NULL while it was never granted.
ALTER TABLE delegated_connections ADD COLUMN IF NOT EXISTS calendar_write BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE delegated_connections ADD COLUMN IF NOT EXISTS calendar_write_at TIMESTAMPTZ;
ALTER TABLE delegated_connections ADD COLUMN IF NOT EXISTS wellness_read BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE delegated_connections ADD COLUMN IF NOT EXISTS wellness_read_at TIMESTAMPTZ;
