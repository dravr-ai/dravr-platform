-- ABOUTME: Keeps a provider OAuth flow's post-OAuth return URL in its server-side state row
-- ABOUTME: The state the provider sees becomes opaque; the callback reads the URL back from the row it redeems

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The return URL used to ride base64-encoded inside the `state` string, and
-- a hosted-connect start's return URL carries the athlete's connect link
-- token: it reached the provider's authorize URL and every request log of our
-- callback (carnet#1873). It is now stored with the flow's state and read back
-- when the callback redeems it. NULL = the flow returns to the frontend.
-- Additive and nullable: a server that does not know the column inserts and
-- returns the columns it names, so rolling back leaves it unread.
ALTER TABLE oauth_client_states ADD COLUMN return_redirect TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
