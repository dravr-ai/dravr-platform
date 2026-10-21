-- ABOUTME: import_tombstones — a deleted account's provider accounts and email, by hash, refused until explicitly cleared (PostgreSQL version)
-- ABOUTME: Written in the account deletion's transaction; webhooks, connects and sign-up admission read the live ones

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Deleting an account deletes its provider data, but the provider keeps
-- sending: a Strava webhook names the athlete's Strava id, a reconnect hands
-- the same account back, and a sign-up with the same email used to land
-- straight in Active through a standing pre-approval. A tombstone records
-- that the identity belonged to a deleted account, so each of those is
-- refused and logged until the person explicitly clears it.
--
-- No identity is stored in clear: `identity_hash` is the SHA-256 of the
-- provider-side id (`kind = 'provider_account'`, with its `provider`) or of
-- the normalized email (`kind = 'email'`). `subject_hash` is the deleted
-- account's, as its deletion receipt names it. A tombstone is live while
-- `cleared_at` is NULL, and at most one per identity is live at a time.
--
-- `claimed_by` and `claimed_at` record the account that last proved it holds
-- a tombstoned provider account (it completed that provider's authorization)
-- and was refused: that proof is what lets the account clear the tombstone.
-- `claimed_by` and `cleared_by` name accounts by `athlete:` plus their id's
-- SHA-256, or an operator, never by a user id.
CREATE TABLE IF NOT EXISTS import_tombstones (
    id TEXT PRIMARY KEY,
    subject_hash TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('provider_account', 'email')),
    provider TEXT,
    identity_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    cleared_at TIMESTAMPTZ,
    cleared_by TEXT,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    CHECK ((kind = 'email' AND provider IS NULL) OR (kind = 'provider_account' AND provider IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_import_tombstones_live
    ON import_tombstones (kind, COALESCE(provider, ''), identity_hash)
    WHERE cleared_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_import_tombstones_subject ON import_tombstones(subject_hash);
