-- ABOUTME: peer_data_reads — one row per read of a member's data by a fellow group member under their peer consent, the member's own record (SQLite)
-- ABOUTME: Written at the read's chokepoint, listed in the member's "who read my data" feed, purged with coach_data_reads after 12 months (carnet#1543)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A group member who shares their training with their peers
-- (peer_sharing_consent, under the group's peer_data_sharing switch) is read
-- by a fellow member through Dravr's own surfaces (a peer's sharing never
-- reaches an outside assistant, carnet#1511): get_group_member_activities,
-- and the views built from every member's training (the room's member
-- cards, /stats, /report, /health, a manager's copy of the weekly digest).
-- Each such read writes one row here (a group view at most one a day,
-- read_day), and the member lists their own rows. A read by the group's
-- human coach is coach_data_reads, never here.
--   tenant_id       a tenant the member whose data was read belongs to: the
--                   group membership's when they belong to it, else their
--                   first; every read and write filters on it with the member
--   user_id         the member whose data was read: the row's owner
--   peer_user_id    the fellow member who read it; NULL once that account is
--                   deleted, so the member keeps the record of the read
--   group_id        the group whose peer sharing let the read through
--   tool_name       the tool the reader's agent read through, or the group
--                   view's label (group_room, group_stats, group_report,
--                   group_health, weekly_digest)
--   read_at         when it was read; rows older than 12 months are purged
--                   hourly, with coach_data_reads
--   read_day        the UTC day ('YYYY-MM-DD') of a group view recorded at
--                   most once a day per source; NULL for a read recorded
--                   each time it is made (a tool call, a support session's
--                   read)
--   transport       pierre_core::transport::Transport::label_of of the read
--                   ('undeclared' when it declared none)
--   impersonator_user_id  the admin whose support session made the read
--                   while acting as the reader; NULL for the reader's own.
--                   No foreign key, as coach_data_reads: the record keeps
--                   saying "support" after that admin's account is gone
-- Additive; no backfill: reads before this table have no record.
CREATE TABLE IF NOT EXISTS peer_data_reads (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    peer_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    group_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    read_at TEXT NOT NULL,
    read_day TEXT,
    transport TEXT NOT NULL,
    impersonator_user_id TEXT
);

-- The member's feed: their reads under one tenant since a time.
CREATE INDEX IF NOT EXISTS idx_peer_data_reads_member
    ON peer_data_reads (tenant_id, user_id, read_at);

-- A group view recorded once a day per source: the second one that day is a
-- conflict here and writes nothing (INSERT ... ON CONFLICT DO NOTHING). A
-- NULL read_day never conflicts, on SQLite and PostgreSQL alike, so a read
-- recorded each time it is made is never folded into another.
CREATE UNIQUE INDEX IF NOT EXISTS idx_peer_data_reads_daily
    ON peer_data_reads (tenant_id, user_id, peer_user_id, group_id, tool_name, read_day);

-- The retention purge: every row read before the cutoff.
CREATE INDEX IF NOT EXISTS idx_peer_data_reads_read_at
    ON peer_data_reads (read_at);
