-- ABOUTME: Records when a claimed coach post actually landed, so a claim abandoned mid-post can be retaken (carnet#911)
-- ABOUTME: Additive only: one nullable column, and the first-sync claims already held are read as posted

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The first-sync post is claimed before its model call and was only ever
-- released when nothing posted, so an instance stopped mid-turn left a claim
-- nothing could retake and the athlete never got the post. posted_at_ms
-- (epoch milliseconds, as claimed_at_ms) is set once the post lands; a
-- first-sync claim still NULL here after its lease is presumed abandoned and
-- the next landing retakes it.
ALTER TABLE platform_post_claims ADD COLUMN posted_at_ms INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A first-sync claim held before this column existed cannot say whether its
-- post landed. Reading it as posted keeps every athlete who already got
-- theirs from being posted to a second time; an abandoned one stays lost, as
-- it was.
UPDATE platform_post_claims
SET posted_at_ms = claimed_at_ms
WHERE purpose = 'first_sync' AND posted_at_ms IS NULL;
