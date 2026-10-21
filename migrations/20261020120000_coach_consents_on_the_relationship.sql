-- ABOUTME: Moves the coach's calendar-write and wellness-read consents from each delegated link onto the member's coach relationship (SQLite)
-- ABOUTME: Backfills every confirmed link's grants onto its member row, then drops the link's four consent columns (carnet#1055)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A member's consents to their human coach live on their membership of the
-- group that coach coaches, beside coach_sharing_consent, not on a link:
--
--   coach_calendar_write  the group's coach writes the member's training
--                         calendar on a coaching platform, through the coach's
--                         own account there, whether the member reaches the
--                         platform through a link or through a connection of
--                         their own.
--   coach_wellness_read   the member's sleep, HRV, resting heart rate, weight
--                         and daily note sync through the coach's account, for
--                         a member who reads the platform through a link.
--
-- Each is off until the member grants it from Dravr's own app, and the member
-- can revoke it at any time. Each *_at records when the member last granted or
-- revoked it, NULL while it was never granted. A link now only binds a
-- platform athlete to a member; it carries no consent.
ALTER TABLE coaching_group_members ADD COLUMN coach_calendar_write INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_calendar_write_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_wellness_read INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_wellness_read_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A grant the member gave on a confirmed link carries over to their
-- membership of the link's group, with the instant they gave it, so no member
-- loses a consent they gave. Links that were proposed or ended carry nothing.
UPDATE coaching_group_members
   SET coach_calendar_write = 1,
       coach_calendar_write_at = (
           SELECT MAX(d.calendar_write_at)
             FROM delegated_connections d
            WHERE d.group_id = coaching_group_members.group_id
              AND d.member_user_id = coaching_group_members.user_id
              AND d.status = 'confirmed'
              AND d.calendar_write = 1)
 WHERE EXISTS (
           SELECT 1
             FROM delegated_connections d
            WHERE d.group_id = coaching_group_members.group_id
              AND d.member_user_id = coaching_group_members.user_id
              AND d.status = 'confirmed'
              AND d.calendar_write = 1);

UPDATE coaching_group_members
   SET coach_wellness_read = 1,
       coach_wellness_read_at = (
           SELECT MAX(d.wellness_read_at)
             FROM delegated_connections d
            WHERE d.group_id = coaching_group_members.group_id
              AND d.member_user_id = coaching_group_members.user_id
              AND d.status = 'confirmed'
              AND d.wellness_read = 1)
 WHERE EXISTS (
           SELECT 1
             FROM delegated_connections d
            WHERE d.group_id = coaching_group_members.group_id
              AND d.member_user_id = coaching_group_members.user_id
              AND d.status = 'confirmed'
              AND d.wellness_read = 1);

ALTER TABLE delegated_connections DROP COLUMN calendar_write; -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
ALTER TABLE delegated_connections DROP COLUMN calendar_write_at; -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
ALTER TABLE delegated_connections DROP COLUMN wellness_read; -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
ALTER TABLE delegated_connections DROP COLUMN wellness_read_at; -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
