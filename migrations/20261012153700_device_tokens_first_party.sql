-- ABOUTME: Records whether Dravr's own app registered each push device, so a first-party-only push skips the rest (carnet#789)
-- ABOUTME: Additive only: one first_party flag on device_tokens; a row written before it reads as externally registered

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A notification stamped first_party_only (carnet#769) carries content some
-- provider terms keep on Dravr's own surfaces, so dravr-commere pushes it only
-- to a device whose row says Dravr's own app registered it. A registration
-- through an API key or an MCP client writes 0, the app writes 1, and a
-- re-registration overwrites it. The default is the safe side: a device no
-- surface vouched for still receives every unstamped push, never a stamped one.
ALTER TABLE device_tokens ADD COLUMN first_party INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
