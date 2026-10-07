-- ABOUTME: app_attest_keys — the iOS app installs whose Secure Enclave key Apple attested, with each key's counter
-- ABOUTME: Registered on the first sign-in from an install; every later sign-in's assertion must move the counter forward

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#810. The key belongs to the app install, not to an account, so the
-- row names no user or tenant and holds nothing personal: the key id (the
-- SHA-256 of the public key, base64), the public key, which App Attest
-- service vouched for it, and the counter of the last accepted assertion.
CREATE TABLE IF NOT EXISTS app_attest_keys (
    key_id TEXT PRIMARY KEY,
    public_key BLOB NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('production', 'development')),
    sign_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_used_at TEXT NOT NULL
);
