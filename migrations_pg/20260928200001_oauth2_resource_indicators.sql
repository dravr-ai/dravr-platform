-- ABOUTME: resource on oauth2_auth_codes and oauth2_refresh_tokens — the RFC 8707 resource a grant is bound to (PostgreSQL)
-- ABOUTME: The token endpoint mints access tokens whose audience is this resource; NULL is an unbound grant
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. Summary: an RFC
-- 8707 `resource` parameter binds a grant to one resource server (carnet#484);
-- the code carries it to the token endpoint and the refresh token carries it
-- through every rotation. NULL is an unbound grant.
ALTER TABLE oauth2_auth_codes ADD COLUMN IF NOT EXISTS resource TEXT;
ALTER TABLE oauth2_refresh_tokens ADD COLUMN IF NOT EXISTS resource TEXT;
