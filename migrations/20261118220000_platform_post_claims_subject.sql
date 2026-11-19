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
-- SQLite cannot change a primary key, so the table is rebuilt with every
-- column it carries (20261008150000 plus posted_at_ms from 20261016020000 and
-- lease_token from 20261021090200), and every row is copied with subject ''.
-- No foreign key points at platform_post_claims.
--
-- One-way: a binary older than this migration claims with
-- ON CONFLICT (tenant_id, user_id, purpose), which no longer names a unique
-- constraint, so its first-sync and check-in claims fail and it posts nothing
-- until every replica runs the new binary.
CREATE TABLE platform_post_claims_new ( -- idempotency-ok: table rebuild — SQLite cannot change a primary key; sqlx applies each migration exactly once
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    claimed_at_ms INTEGER NOT NULL,
    posted_at_ms INTEGER,
    lease_token TEXT,
    PRIMARY KEY (tenant_id, user_id, purpose, subject)
);
INSERT INTO platform_post_claims_new
    (tenant_id, user_id, purpose, subject, claimed_at_ms, posted_at_ms, lease_token)
SELECT tenant_id, user_id, purpose, '', claimed_at_ms, posted_at_ms, lease_token
FROM platform_post_claims;
DROP TABLE platform_post_claims; -- idempotency-ok: table rebuild — the rebuilt table is renamed over it on the next line
ALTER TABLE platform_post_claims_new RENAME TO platform_post_claims;
