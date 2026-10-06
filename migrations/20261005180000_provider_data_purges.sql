-- ABOUTME: provider_data_purges — the fact and the time of every provider data deletion, kept for attestation
-- ABOUTME: data_point_series.synced_at — when each point was last copied from its provider, for the cache TTL sweep

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A provider's terms can ask for written attestation that its data was
-- deleted (Nolio API terms §7.3, carnet#725). Every purge — a disconnect, a
-- provider's revocation notice, an ended coach link, the operator's
-- termination purge, a cache TTL eviction — writes one row here in the same
-- transaction as its deletes. The row holds no provider data: who, which
-- provider, why, how many rows, and when. user_id and tenant_id are NULL on
-- the operator's whole-provider purge. A user's rows go with their account:
-- the account delete clears them (user_references.rs), so no user id outlives
-- the account.
CREATE TABLE IF NOT EXISTS provider_data_purges (
    id TEXT PRIMARY KEY,
    tenant_id TEXT,
    user_id TEXT,
    provider TEXT NOT NULL,
    reason TEXT NOT NULL,
    rows_removed INTEGER NOT NULL,
    purged_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_provider_data_purges_provider ON provider_data_purges(provider, purged_at);

-- Points carried no sync time, so a cache TTL could only be judged by when a
-- point was recorded, which says nothing about how long the copy has been
-- held. NULL on the points stored before this column: a TTL-bound provider's
-- sweep treats them as expired, since their age cannot be shown.
ALTER TABLE data_point_series ADD COLUMN synced_at TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
