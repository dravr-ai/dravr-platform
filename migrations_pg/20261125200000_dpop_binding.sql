-- ABOUTME: dpop_jkt on authorization codes and refresh tokens: the DPoP key (RFC 9449) a grant is bound to (PostgreSQL)
-- ABOUTME: Additive only: two nullable columns; codes and refresh tokens minted before it, or without a proof, stay NULL (unbound)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A client that proves possession of a key (a DPoP header at PAR or the
-- token endpoint, or dpop_jkt on authorize, RFC 9449 §10) binds its grant to
-- that key's RFC 7638 SHA-256 thumbprint, base64url. The code carries the
-- thumbprint to the token endpoint, which then requires a proof with that key;
-- the refresh token carries it so every refresh needs the same key (RFC 9449
-- §5). NULL means unbound: Bearer tokens, exactly as before this migration.
-- Proof replay is remembered in the cache, not here.
ALTER TABLE oauth2_auth_codes ADD COLUMN IF NOT EXISTS dpop_jkt TEXT;
ALTER TABLE oauth2_refresh_tokens ADD COLUMN IF NOT EXISTS dpop_jkt TEXT;
