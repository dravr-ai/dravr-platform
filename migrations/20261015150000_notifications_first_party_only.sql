-- ABOUTME: Stores a notification's push restriction in its own first_party_only column, backfilled from its data (carnet#985)
-- ABOUTME: Additive only: one flag on notifications; a row written before it reads as unrestricted unless its data said otherwise

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- dravr-commere 0.6.0 types the first-party-only restriction (carnet#789) as a
-- field of the notification instead of a "first_party_only" key inside its
-- deep-link `data`, and reads it from this column. Rows written before it
-- carried the restriction in `data`, so they are backfilled from that key: a
-- restricted row must stay restricted. The key itself is left in the stored
-- `data`; nothing reads it any more. json_extract raises on text that is not
-- JSON, and SQLite may evaluate WHERE terms in any order, so the validity check
-- guards the extract inside one CASE: a malformed row reads as unrestricted
-- instead of failing the migration and, with it, the server's startup.
ALTER TABLE notifications ADD COLUMN first_party_only INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

UPDATE notifications
SET first_party_only = 1
WHERE data IS NOT NULL
  AND CASE WHEN json_valid(data) THEN json_extract(data, '$.first_party_only') ELSE 0 END = 1;
