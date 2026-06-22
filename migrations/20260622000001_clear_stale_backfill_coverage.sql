-- ABOUTME: One-time purge of activity_backfill_coverage rows from the removed killed-page-scrape
-- ABOUTME: whose count-based hit_feed_end falsely marked partial windows "covered" (the "2022 stuck at 46" bug)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The pre-interval-feed page scraper could be reclaimed by Cloud Run mid-walk;
-- backfill_hit_feed_end then read `fetched_count < fetch_limit` as "feed
-- exhausted" and stamped hit_feed_end=1 on a partial slice (e.g. only Jul-Dec of
-- a full 2022), so the gate marked that window permanently covered and never
-- self-healed. The month-at-a-time interval-feed scrape completes reliably
-- (~50s, no Cloud Run reclaim), so the coverage it records is trustworthy.
-- Purge every existing coverage row once so each athlete re-backfills through the
-- interval feed on their next historical ask and gets a correct
-- oldest_reached_ts / hit_feed_end. Coverage is a derived cache, not source data
-- — losing it costs at most one re-scrape per athlete.
DELETE FROM activity_backfill_coverage;
