-- ABOUTME: Records how each member read a stored row drew on was made (peer or coach way, class, tier), and which rows carry a record (SQLite)
-- ABOUTME: Schema only, no backfill: a link row from before reads as the strictest way, and a chat row or block from before is legacy

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#1511 design call 19: every member read a stored row drew on is
-- recorded in one carrier, the link tables, with how it was read and the data
-- tier. A link row names the member only, so a coach's read of a member's
-- wellness read as a peer's read of their training to everyone who could read
-- the member at all (round-11 room-gate blocker). Each link row now records:
--
-- via_kind    'peer' (the group's peer view), 'coach' (the coach's view of
--             the member, by via_coach) or 'thread' (another participant's
--             own words in the same thread).
-- via_group   the group the read was made under.
-- via_coach   the coach who read it, for a coach read.
-- data_class  'training' or 'wellness' (read_subject::DataClass).
-- tier        'T2' or 'T3' (dravr_barrage::Tier) when the read reached one.
--
-- The defaults are the strictest record: a link row written before this
-- (branch databases only; no deployed database has the table) reads as a
-- coach's wellness read by no coach, which only the member reads.
ALTER TABLE chat_message_members ADD COLUMN via_kind TEXT NOT NULL DEFAULT 'coach' CHECK (via_kind IN ('peer', 'coach', 'thread')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE chat_message_members ADD COLUMN via_group TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE chat_message_members ADD COLUMN via_coach TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE chat_message_members ADD COLUMN data_class TEXT NOT NULL DEFAULT 'wellness' CHECK (data_class IN ('training', 'wellness')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE chat_message_members ADD COLUMN tier TEXT CHECK (tier IS NULL OR tier IN ('T2', 'T3')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

ALTER TABLE group_transcript_entry_members ADD COLUMN via_kind TEXT NOT NULL DEFAULT 'coach' CHECK (via_kind IN ('peer', 'coach', 'thread')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE group_transcript_entry_members ADD COLUMN via_group TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE group_transcript_entry_members ADD COLUMN via_coach TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE group_transcript_entry_members ADD COLUMN data_class TEXT NOT NULL DEFAULT 'wellness' CHECK (data_class IN ('training', 'wellness')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE group_transcript_entry_members ADD COLUMN tier TEXT CHECK (tier IS NULL OR tier IN ('T2', 'T3')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- audience_recorded  TRUE when the row was written with its record (its
--                    carried reads, possibly none). A row from before has
--                    none and none can be rebuilt (call 17): its author reads
--                    it on screen as before, and no room replays it.
-- author_user_id     the turn's sender who wrote the row, user or reply: the
--                    participant a replay of the row reads besides its
--                    carried members (R2). NULL on a row from before.
ALTER TABLE chat_messages ADD COLUMN audience_recorded INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE chat_messages ADD COLUMN author_user_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A compaction block written from now on never covers a row that carries a
-- member read, so its record is that it carries none.
ALTER TABLE compaction_blocks ADD COLUMN audience_recorded INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
