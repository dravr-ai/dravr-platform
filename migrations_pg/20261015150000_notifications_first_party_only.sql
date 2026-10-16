-- ABOUTME: Stores a notification's push restriction in its own first_party_only column, backfilled from its data (carnet#985)
-- ABOUTME: Additive only: one flag on notifications; a row written before it reads as unrestricted unless its data said otherwise

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- dravr-commere 0.6.0 types the first-party-only restriction (carnet#789) as a
-- field of the notification instead of a "first_party_only" key inside its
-- deep-link `data`, and reads it from this column. Rows written before it
-- carried the restriction in `data`, so they are backfilled from that key: a
-- restricted row must stay restricted. The key itself is left in the stored
-- `data`; nothing reads it any more. `data` is TEXT holding the JSON object
-- dravr-commere wrote with serde_json's compact output, so the stamp is
-- matched as that text rather than through a jsonb cast: one row whose
-- `data` is not valid JSON would fail the cast and, with it, the migration
-- and the server's startup.
ALTER TABLE notifications ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE notifications
SET first_party_only = TRUE
WHERE data LIKE '%"first_party_only":true%';
