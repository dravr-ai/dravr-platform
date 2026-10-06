-- ABOUTME: firebase_identity_deletions — Firebase sign-in identities still to delete at Google after their account went
-- ABOUTME: Written in the account delete's transaction, removed once Identity Toolkit confirms; a sweep retries the rest

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Deleting an account also deletes its Firebase Authentication user, which
-- holds the athlete's email and display name at Google (carnet#798). That
-- call can fail after the account delete committed, and the account row was
-- the only place the Firebase uid lived. So the delete writes the uid here in
-- its own transaction; the post-commit call removes the row once Firebase
-- confirms (or answers that the user is already gone), and the sweep retries
-- whatever is left, with backoff. The row holds the uid and the Firebase
-- project only: no user id, email or name.
CREATE TABLE IF NOT EXISTS firebase_identity_deletions (
    firebase_uid TEXT PRIMARY KEY,
    firebase_project TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_firebase_identity_deletions_due ON firebase_identity_deletions(next_attempt_at);
