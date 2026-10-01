-- ABOUTME: The detail-only fields of a cached activity — splits and laps a provider serves only on a detail read
-- ABOUTME: Kept in their own column, which the list write-through never names, so a later list sync cannot wipe them
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A list read never carries splits or laps, and every list write-through
-- replaces data_json whole. detail_json holds what a detail read of the
-- activity found, as the serde JSON of the repository's ActivityDetail; the
-- reads fill data_json's missing keys from it. It lives on the activity's own
-- row, so a purge, a prune or a user deletion takes it with the row. NULL
-- until a detail read stores one. TEXT, like data_json, so both engines bind
-- the same string.

ALTER TABLE cached_activities ADD COLUMN IF NOT EXISTS detail_json TEXT;
