-- ABOUTME: Records when a provider activity fetch last completed per (tenant,user,provider) — freshness without rows
-- ABOUTME: Lets a freshness read tell "we checked and there was nothing" from "we never checked"
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per (tenant, user, provider): when the last successful activity
-- fetch completed. `cached_activities.synced_at` only advances when a fetch
-- returns rows, so an athlete whose provider truthfully reports no activities
-- looks forever stale to any freshness read over rows alone — the commitment
-- sweep could then never believe an honest zero. This row advances on every
-- successful fetch, empty or not.
-- No FK: a pure freshness log, decoupled so a churned user/provider never cascades.
-- tenant_id/user_id are UUID to match users.id / tenants.id (cf. activity_backfill_coverage).
CREATE TABLE IF NOT EXISTS activity_fetch_freshness (
    tenant_id   UUID NOT NULL,
    user_id     UUID NOT NULL,
    provider    TEXT NOT NULL,
    fetched_at  TIMESTAMPTZ NOT NULL,  -- when the last successful fetch completed
    PRIMARY KEY (tenant_id, user_id, provider)
);
