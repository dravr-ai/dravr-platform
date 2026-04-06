-- ABOUTME: Adds source column to coaches for tracking prompt origin
-- ABOUTME: Distinguishes contremaitre-managed coaches from user-created ones
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Source tracking for coach origin:
-- 'contremaitre' = managed by dravr-contremaitre repo (hot-reload enabled)
-- 'custom'       = user-created or admin-created (DB-only)
-- 'seed'         = originally seeded from coaches/ directory (transitional)
ALTER TABLE coaches ADD COLUMN source TEXT NOT NULL DEFAULT 'custom';

CREATE INDEX IF NOT EXISTS idx_coaches_source ON coaches(source);

-- Backfill: coaches with source_file set were seeded from markdown files
UPDATE coaches SET source = 'seed' WHERE source_file IS NOT NULL;
