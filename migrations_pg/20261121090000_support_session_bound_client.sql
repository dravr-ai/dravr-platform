-- ABOUTME: Binds an admin's support session to the assistant connection that started it over MCP or A2A (PostgreSQL)
-- ABOUTME: Adds impersonation_sessions.bound_client: NULL for a web session, else the credential it acts through (carnet#1511)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- bound_client: the credential through which an admin's assistant started
-- the support session, and the only one it changes. An OAuth application's
-- client id, or `api_key` for the admin's own API keys. NULL for a session
-- the admin console started on the web, which never turns an assistant
-- connection into the user; a session bound here never changes the web
-- console's either. Additive; no backfill: every existing row is a web
-- session.
ALTER TABLE impersonation_sessions ADD COLUMN IF NOT EXISTS bound_client TEXT;
