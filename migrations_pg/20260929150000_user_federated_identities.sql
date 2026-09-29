-- ABOUTME: user_federated_identities — the external sign-in identities (a Google account id) each Dravr account is reached by (PostgreSQL)
-- ABOUTME: Keyed by (provider, subject) so a sign-in finds its account by the identity before any email match
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A Google account is named by its stable `sub`, which is not the Firebase
-- UID users.firebase_uid holds: Firebase mints its own. The hosted
-- authorization server signs athletes in with Google directly, and the web
-- app through Firebase; both record the Google `sub` here, so either path
-- finds the account the other linked even after the Google account's email
-- changes, and an address reassigned to another Google account is refused
-- instead of attached.
--
-- Scoped by user_id (no tenant_id): an identity belongs to a person, not a
-- workspace. The primary key makes one identity reach one account.
-- user_id is UUID to match users.id (SQLite keeps both as TEXT).
CREATE TABLE IF NOT EXISTS user_federated_identities (
    provider   TEXT NOT NULL,
    subject    TEXT NOT NULL,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (provider, subject)
);

-- "Which Google account is this user linked to" reads by user.
CREATE INDEX IF NOT EXISTS idx_user_federated_identities_user
    ON user_federated_identities(user_id);
