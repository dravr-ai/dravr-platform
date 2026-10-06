-- ABOUTME: When an account withdrew a provider notice it had accepted; NULL while the acceptance stands
-- ABOUTME: Additive only: a consent to AI use (WHOOP's owner authorization) is withdrawn as simply as it was given (carnet#726)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A withdrawn acceptance is no acceptance: the account is asked again on its
-- next connect, no model reads the provider's data, and accepting again
-- clears the stamp. The row stays, so the account's history of answers is
-- kept beside the version it last accepted.
ALTER TABLE provider_terms_consents ADD COLUMN withdrawn_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
