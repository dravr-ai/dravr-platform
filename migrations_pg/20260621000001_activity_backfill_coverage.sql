-- ABOUTME: Records how deep a historical backfill reached per (tenant,user,provider) so the gate self-heals partial windows
-- ABOUTME: oldest_reached_ts + hit_feed_end let the gate tell "cached but shallow" from "cached and complete"
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per (tenant, user, provider): the deepest a background activity
-- backfill has confirmed for that athlete's feed.
--
-- `oldest_reached_ts` is the oldest activity start (unix seconds) the most recent
-- deeper backfill fetched. `hit_feed_end` is true when that scrape exhausted the
-- provider feed (next-page disabled before reaching the requested `after`), so
-- there is genuinely no older data — distinguishing "cached but only the recent
-- slice of a deep window" (re-backfill) from "cached and the feed ends here"
-- (covered, never re-scrape). Coverage only deepens: a query shallower than the
-- recorded floor is already covered and never overwrites it.
-- No FK: a pure coverage log, decoupled so a churned user/provider never cascades.
-- tenant_id/user_id are UUID to match users.id / tenants.id (cf. backfill_push_log).
CREATE TABLE IF NOT EXISTS activity_backfill_coverage (
    tenant_id          UUID    NOT NULL,
    user_id            UUID    NOT NULL,
    provider           TEXT    NOT NULL,
    oldest_reached_ts  BIGINT  NOT NULL,  -- oldest activity start (unix seconds) reached
    hit_feed_end       BOOLEAN NOT NULL DEFAULT FALSE,  -- true when the provider feed ended (no older data)
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, user_id, provider)
);
