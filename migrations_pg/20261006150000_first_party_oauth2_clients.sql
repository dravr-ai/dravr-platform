-- ABOUTME: Registers Dravr's own web and mobile apps (dravr-web, dravr-mobile) as OAuth2 clients (PostgreSQL)
-- ABOUTME: They sign in through the hosted login page with authorization code + PKCE, replacing the password grant
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- carnet#787: the apps no longer send the athlete's password to /oauth/token
-- (RFC 9700 §2.4 forbids the password grant). They redirect to the hosted
-- login page and redeem the authorization code it issues, so each needs the
-- registration row its codes, states and refresh tokens reference.
--
-- They are public clients (RFC 8252 §8.4): PKCE proves them, no secret does.
-- `client_secret_hash` holds '!', which is not an Argon2 hash, so no secret
-- ever verifies against it; the confidential-client token endpoint refuses
-- these ids before it would try. `redirect_uris` stays empty because the
-- deployment's first-party redirect policy decides where their codes go.
-- `expires_at` is NULL, so the registration retention sweep never deletes
-- them. ON CONFLICT DO NOTHING: rerunnable, and a row already present is kept.

INSERT INTO oauth2_clients (id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at) VALUES
('first-party-dravr-web', 'dravr-web', '!', '[]', '["authorization_code"]', '["code"]', 'Dravr', NULL, 'fitness:read profile:read', '2026-10-06T15:00:00+00:00', NULL),
('first-party-dravr-mobile', 'dravr-mobile', '!', '[]', '["authorization_code"]', '["code"]', 'Dravr', NULL, 'fitness:read profile:read', '2026-10-06T15:00:00+00:00', NULL)
ON CONFLICT (client_id) DO NOTHING;
