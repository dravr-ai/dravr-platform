-- ABOUTME: Keeps a provider OAuth flow's post-OAuth return URL in its server-side state row (PostgreSQL version)
-- ABOUTME: The state the provider sees becomes opaque; the callback reads the URL back from the row it redeems

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#1873. See the SQLite twin for why the return URL moved out of the
-- `state` string. NULL = the flow returns to the frontend.
ALTER TABLE oauth_client_states ADD COLUMN IF NOT EXISTS return_redirect TEXT;
