-- ABOUTME: An append-only record of every answer an account gave a provider notice: each grant and each withdrawal, with the version
-- ABOUTME: provider_terms_consents keeps the standing answer and overwrites it; this keeps the history a provider may audit (carnet#1157)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per grant or withdrawal, never updated or deleted while the account
-- exists: Garmin's agreement lets Garmin audit the AI and transfer consents,
-- so grant -> withdraw -> grant leaves three rows. The account's deletion
-- takes its rows with it.
--
-- event       'granted' or 'withdrawn'
-- version     the notice version granted, or the one standing when withdrawn
-- Rows start with this table; answers given before it have only their
-- standing row in provider_terms_consents.
CREATE TABLE IF NOT EXISTS provider_consent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    version TEXT NOT NULL,
    event TEXT NOT NULL CHECK (event IN ('granted', 'withdrawn')),
    occurred_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provider_consent_events_user
    ON provider_consent_events (user_id, provider, occurred_at);
