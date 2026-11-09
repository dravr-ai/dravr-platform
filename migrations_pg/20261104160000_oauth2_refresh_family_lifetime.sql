-- ABOUTME: family_created_at on oauth2_refresh_tokens — when a rotation chain began, for an absolute family lifetime (PostgreSQL)
-- ABOUTME: Additive and nullable: NULL is a legacy family with no absolute limit, so the previous image keeps working

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. NULL is a legacy
-- family with no absolute limit. No backfill.
ALTER TABLE oauth2_refresh_tokens ADD COLUMN IF NOT EXISTS family_created_at TIMESTAMPTZ;
