-- ABOUTME: Records where an app-started Slack/Discord link hands the provider's code back to the app
-- ABOUTME: A state without it completes only in a browser signed in as the Dravr account that started it
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The OAuth link callback used to accept a provider code from any browser
-- that carried the link's `state`, so a link started on one account and
-- forwarded to someone else linked that person's channel identity to the
-- sender (carnet#1580). The callback now completes a link only in a browser
-- signed in as the account that started it. The phone app starts its links
-- in a system auth session that has no Dravr session, so the callback hands
-- the code back to the app at this URI and the app finishes the link with its
-- own login. NULL for links started on the web. Validated at init against the
-- first-party app's fixed callback, never stored as the caller sent it.

ALTER TABLE messaging_link_states ADD COLUMN app_return_uri TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
