-- ABOUTME: Adds a subject to the platform post claim key, so a post owed per item claims one row per item
-- ABOUTME: A landing reaction per ride claims <provider>:<activity_id>; every existing claim keeps subject ''
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The coach's reaction to a synced workout (carnet#1486) is owed once per
-- local day by default, or once per ride when the athlete's
-- activity_landed.frequency says per_ride. A per-ride claim needs the ride in
-- the key, so the key becomes (tenant_id, user_id, purpose, subject):
--
--   subject   '' for a purpose owed once per athlete or once per period
--             (first_sync, check_in, activity_landed under per_day), and
--             '<provider>:<activity_id>' for activity_landed under per_ride.
--             A provider key never contains ':', so the pair is unambiguous.
--
-- The primary key is dropped by the name the catalogue holds rather than by
-- PostgreSQL's default name, so a database whose key was named otherwise is
-- rebuilt the same way, and a re-run replaces the new key with itself.
--
-- One-way: a binary older than this migration claims with
-- ON CONFLICT (tenant_id, user_id, purpose), which no longer names a unique
-- constraint, so its first-sync and check-in claims fail and it posts nothing
-- until every replica runs the new binary.
ALTER TABLE platform_post_claims ADD COLUMN IF NOT EXISTS subject TEXT NOT NULL DEFAULT '';

DO $$
DECLARE cname text;
BEGIN
    SELECT conname INTO cname FROM pg_constraint
    WHERE conrelid = 'platform_post_claims'::regclass AND contype = 'p';
    IF cname IS NOT NULL THEN
        EXECUTE format('ALTER TABLE platform_post_claims DROP CONSTRAINT %I', cname);
    END IF;
END $$;

ALTER TABLE platform_post_claims ADD PRIMARY KEY (tenant_id, user_id, purpose, subject);
