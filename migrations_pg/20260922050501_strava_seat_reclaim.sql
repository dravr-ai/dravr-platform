-- ABOUTME: The strava_seat_reclaim config category, and one row per Strava seat holder warned before a reclaim
-- ABOUTME: A reclaim disconnects only an athlete warned at least warn_lead_days earlier and not active since
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The catalog lists a parameter only under a category it has a row for, so
-- without this one `pierre-cli config show` and `config get` cannot see the
-- five strava_seat_reclaim.* parameters (carnet#505).
INSERT INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('strava_seat_reclaim', 'strava_seat_reclaim', 'Strava Seat Reclaim', 'When idle athletes give their Strava OAuth seat back so others can connect', 155, 'users')
ON CONFLICT (id) DO NOTHING;

-- One row per (athlete, tenant) Strava token the sweeper has warned. The
-- warning is what makes a later disconnect legitimate, so it is recorded; the
-- row goes when the athlete comes back, stops being a candidate, reconnects,
-- lets it grow stale, or is reclaimed. warned_at_ms is epoch milliseconds,
-- identical on both engines. reached is whether the warning got to the athlete
-- outside the app (a push to a device, or a linked chat channel); one that
-- only landed in the in-app list is kept, so it is not sent again every pass,
-- but it never justifies a disconnect.
CREATE TABLE IF NOT EXISTS strava_seat_reclaim_warnings (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL,
    warned_at_ms BIGINT NOT NULL,
    reached BOOLEAN NOT NULL,
    PRIMARY KEY (user_id, tenant_id)
);
