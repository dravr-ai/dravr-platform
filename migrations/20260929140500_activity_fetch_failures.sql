-- ABOUTME: Records when a provider activity fetch last failed per (tenant,user,provider), beside the success mark
-- ABOUTME: Lets Home say a sync failed, and when the last good one was, instead of reading a failed scrape as synced
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per (tenant, user, provider): when the last activity fetch that did
-- not count as a sync happened, and why. A fetch counts as a sync only when
-- the provider answered with a list it vouched for; a transport or scrape
-- error, a capture missing its list head, and an empty answer over a window
-- the cache holds activities in all land here instead of in
-- activity_fetch_freshness. A failure is current only while it is newer than
-- the provider's last successful fetch, so a later sync supersedes it without
-- a delete. `consecutive` counts the failures since that last good fetch, so a
-- refresh of a provider that keeps failing backs off further each time.
-- `streak` counts the failures in a row for this same reason, and
-- `streak_started_at` is when the first of them happened: an empty answer
-- repeated across an hour is believed as an honest empty list.
-- No FK: a pure attempt log, decoupled so a churned user/provider never cascades.
CREATE TABLE IF NOT EXISTS activity_fetch_failures (
    tenant_id   TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    provider    TEXT NOT NULL,
    failed_at   TEXT NOT NULL,  -- RFC3339 UTC of the last failed fetch
    reason      TEXT NOT NULL,  -- slug: fetch_error | head_incomplete | empty_over_cached
    consecutive INTEGER NOT NULL DEFAULT 1,  -- failures since the last good fetch
    streak      INTEGER NOT NULL DEFAULT 1,  -- failures in a row for this reason
    streak_started_at TEXT NOT NULL,  -- RFC3339 UTC of the first of them
    PRIMARY KEY (tenant_id, user_id, provider)
);
