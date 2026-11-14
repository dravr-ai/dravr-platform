-- ABOUTME: coach_access_requests.decline_reason and superseded_at — a decline's optional reason, and a decline a new ask replaced (SQLite)
-- ABOUTME: Additive only: two nullable columns; nothing is renamed, dropped or re-checked (carnet#1521, #1522)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A decline now reaches the coach: an in-app notice, an email and the
-- declined state where they asked. The super-admin may say why in the
-- console's queue; the words are stored on the request so every one of those
-- surfaces reads the same reason. NULL for a decline given without one, and
-- for every request that is not declined. No backfill: an older decline had
-- no reason to give.
ALTER TABLE coach_access_requests ADD COLUMN decline_reason TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Asking again replaces an earlier decline: when a coach opens a new request,
-- their declined requests are stamped superseded_at, and the request the coach
-- reads skips a superseded decline. A coach who was declined, asked again and
-- then withdrew that ask therefore reads no request at all — the plain ask —
-- rather than the old decline. The console queue and the account's history
-- keep every declined row and its reason. NULL for every request not
-- superseded; no backfill.
ALTER TABLE coach_access_requests ADD COLUMN superseded_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
