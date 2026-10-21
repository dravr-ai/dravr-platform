-- ABOUTME: Moves the coach's calendar-write and wellness-read consents from each delegated link onto the member's coach relationship (PostgreSQL)
-- ABOUTME: Backfills every confirmed link's grants onto its member row, then drops the link's four consent columns (carnet#1055)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. A member's
-- consents to their human coach live on their membership of the group that
-- coach coaches; a link only binds a platform athlete to a member.
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_calendar_write BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_calendar_write_at TIMESTAMPTZ;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_wellness_read BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_wellness_read_at TIMESTAMPTZ;

-- A grant the member gave on a confirmed link carries over to their
-- membership of the link's group, with the instant they gave it. The guard on
-- the dropped column keeps a re-run after the drop a no-op.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
         WHERE table_name = 'delegated_connections' AND column_name = 'calendar_write'
    ) THEN
        UPDATE coaching_group_members m
           SET coach_calendar_write = TRUE,
               coach_calendar_write_at = g.granted_at
          FROM (SELECT group_id, member_user_id, MAX(calendar_write_at) AS granted_at
                  FROM delegated_connections
                 WHERE status = 'confirmed' AND calendar_write
                 GROUP BY group_id, member_user_id) g
         WHERE m.group_id = g.group_id
           AND m.user_id = g.member_user_id;

        UPDATE coaching_group_members m
           SET coach_wellness_read = TRUE,
               coach_wellness_read_at = g.granted_at
          FROM (SELECT group_id, member_user_id, MAX(wellness_read_at) AS granted_at
                  FROM delegated_connections
                 WHERE status = 'confirmed' AND wellness_read
                 GROUP BY group_id, member_user_id) g
         WHERE m.group_id = g.group_id
           AND m.user_id = g.member_user_id;
    END IF;
END $$;

ALTER TABLE delegated_connections DROP COLUMN IF EXISTS calendar_write;
ALTER TABLE delegated_connections DROP COLUMN IF EXISTS calendar_write_at;
ALTER TABLE delegated_connections DROP COLUMN IF EXISTS wellness_read;
ALTER TABLE delegated_connections DROP COLUMN IF EXISTS wellness_read_at;
