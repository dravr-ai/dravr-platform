-- ABOUTME: resource on oauth2_auth_codes and oauth2_refresh_tokens — the RFC 8707 resource a grant is bound to (SQLite)
-- ABOUTME: The token endpoint mints access tokens whose audience is this resource; NULL is an unbound grant
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- `/oauth2/authorize` and `/oauth2/token` accept an RFC 8707 `resource`
-- parameter naming the resource server a token is for (carnet#484). The
-- authorization request's resource has to survive until the code is exchanged,
-- and the grant's resource has to survive every refresh, or a refreshed access
-- token would come back unbound. NULL is a grant made without a resource, which
-- mints the platform audience exactly as before.
ALTER TABLE oauth2_auth_codes ADD COLUMN resource TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE oauth2_refresh_tokens ADD COLUMN resource TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
