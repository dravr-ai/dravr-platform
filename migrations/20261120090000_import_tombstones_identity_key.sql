-- ABOUTME: import_tombstones.identity_key — each tombstone's identity digest keyed under the platform's Index key
-- ABOUTME: Expand only: written beside identity_hash and backfilled; carnet#1550 drops identity_hash once approved

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- identity_hash is a plain SHA-256 of a provider-side id or an email: anyone
-- holding the table can confirm a guessed identity. identity_key is the
-- HMAC of that hash under the Index key (pierre_database::keyed, domain
-- import_tombstone), stored verbatim with its key version. Lookups match it
-- under either live key, or identity_hash while a row is not keyed yet; the
-- record_log worker keys the NULL rows (carnet#1542).
ALTER TABLE import_tombstones ADD COLUMN identity_key TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
CREATE INDEX IF NOT EXISTS idx_import_tombstones_live_key
    ON import_tombstones (kind, identity_key)
    WHERE cleared_at IS NULL;
