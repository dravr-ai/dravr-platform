-- ABOUTME: One row per (tenant, athlete, first-run flow, local day) on which the flow's done rule was met (carnet#1099)
-- ABOUTME: Getting started reads which flows are done from it; flow.completed fires on the row that makes one done (carnet#1100)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- flow_id is the use-case catalogue id of a first-run flow (meet_team,
-- last_workout, about_me_more, check_in). A flow is done at its first row,
-- except the daily check-in, which is done once three distinct local days hold
-- one. local_date is the athlete's own civil date (YYYY-MM-DD) when the row was
-- written, so the primary key keeps one row a day. recorded_at is epoch
-- milliseconds, identical on both engines. A row for an id the catalogue no
-- longer lists is simply never read.
CREATE TABLE IF NOT EXISTS first_run_progress (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    flow_id TEXT NOT NULL,
    local_date TEXT NOT NULL,
    recorded_at BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, flow_id, local_date)
);
