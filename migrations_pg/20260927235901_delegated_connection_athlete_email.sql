-- ABOUTME: provider_athlete_email on delegated_connections — the email the coach's roster lists for the athlete (PostgreSQL)
-- ABOUTME: A link binds only when it is the member's verified Dravr email; a row without one cannot be confirmed

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale (carnet#602).
ALTER TABLE delegated_connections ADD COLUMN IF NOT EXISTS provider_athlete_email TEXT;
