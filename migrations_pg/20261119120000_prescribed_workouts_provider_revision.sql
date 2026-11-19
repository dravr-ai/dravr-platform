-- ABOUTME: provider_revision on prescribed_workouts: the provider's own change stamp as Dravr's write left the entry (PostgreSQL)
-- ABOUTME: A plan push compares it with the stamp a detail read returns, so an edit made on the provider is not overwritten
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- See the matching SQLite migration for the full rationale (carnet#1389).
-- Opaque TEXT, never parsed as a time. NULL for a provider that sends no such
-- stamp and for every row written before this column.

ALTER TABLE prescribed_workouts ADD COLUMN IF NOT EXISTS provider_revision TEXT;
