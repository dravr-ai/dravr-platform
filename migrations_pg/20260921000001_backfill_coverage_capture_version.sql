-- ABOUTME: Stamps each backfill coverage row with the capture version that produced its rows
-- ABOUTME: A row below the provider's current version reads as not covered, so history re-captures itself

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A coverage row vouches that the durable cache holds a window in full, and the
-- historical gate then serves that window forever without calling the provider.
-- It said nothing about WHICH capture wrote those rows. When the sciotte Strava
-- capture lost elevation for two weeks, every row backfilled in that period was
-- complete except for the field, the coverage row went on vouching for it, and
-- no later fix to the capture could ever reach that history.
--
-- capture_version is written only by a completed backfill, never by the
-- retention clamp, so it names the capture that produced the rows rather than
-- the last time the row was touched. Existing rows take 0, the baseline every
-- provider starts at; a provider whose capture has since been corrected
-- compares above that and re-captures on the next deep ask.
ALTER TABLE activity_backfill_coverage ADD COLUMN IF NOT EXISTS capture_version BIGINT NOT NULL DEFAULT 0;
