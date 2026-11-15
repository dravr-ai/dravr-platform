-- ABOUTME: coach_data_reads — one row per read of an athlete's data by their group's human coach, the athlete's own record (PostgreSQL)
-- ABOUTME: Written at the read's chokepoint, listed to the athlete beside their coach sharing, purged after 12 months (carnet#1445)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A group's human coach reads a coached athlete's data through their agent:
-- in their thread about the athlete, over MCP or A2A with `athlete=`, and
-- through the groups tools; and in the group views built from every member's
-- training: the room's member cards, /stats, /report, /health and the weekly
-- digest. Each read the coaching gates let through writes one row here (a
-- group view at most one a day, read_day), and the athlete lists their own
-- rows.
--   tenant_id       the athlete's one home tenant, where the read data lives;
--                   every read and write filters on it with the athlete
--   user_id         the athlete whose data was read: the row's owner
--   coach_user_id   the coach who read it; NULL once that account is deleted,
--                   so the athlete keeps the record of the read
--   group_id        the group the coach coaches the athlete in
--   tool_name       the tool the coach's agent read through
--   data_class      'training' or 'wellness'
--   read_at         when the coach read it; rows older than 12 months are
--                   purged hourly
--   read_day        the UTC day ('YYYY-MM-DD') of a read recorded at most
--                   once a day per source: the group views a coach opens on
--                   every turn or page load (the room's member cards,
--                   /stats, /report, /health, the weekly digest). NULL for a
--                   read recorded each time it is made (a tool call, the
--                   plan view, the roster, the coach's thread)
-- Additive; no backfill: reads before this table have no record.
CREATE TABLE IF NOT EXISTS coach_data_reads (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    coach_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    group_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    data_class TEXT NOT NULL CHECK (data_class IN ('training', 'wellness')),
    read_at TIMESTAMPTZ NOT NULL,
    read_day TEXT
);

-- The athlete's listing: their reads in one group, newest first.
CREATE INDEX IF NOT EXISTS idx_coach_data_reads_athlete
    ON coach_data_reads (tenant_id, user_id, group_id, read_at);

-- A read recorded once a day per source: the second one that day is a
-- conflict here and writes nothing (INSERT ... ON CONFLICT DO NOTHING). The
-- group is in the key because the athlete lists their record per group. A
-- NULL read_day never conflicts, on SQLite and PostgreSQL alike, so a read
-- recorded each time it is made is never folded into another.
CREATE UNIQUE INDEX IF NOT EXISTS idx_coach_data_reads_daily
    ON coach_data_reads (tenant_id, user_id, coach_user_id, group_id, tool_name, data_class, read_day);

-- The retention purge: every row read before the cutoff.
CREATE INDEX IF NOT EXISTS idx_coach_data_reads_read_at
    ON coach_data_reads (read_at);
