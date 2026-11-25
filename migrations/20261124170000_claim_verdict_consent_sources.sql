-- ABOUTME: Names on each claim verdict the AI-consent keys of the provider data the reply it judged was drawn from
-- ABOUTME: Additive and nullable: evaluation metrics leave out a verdict naming Garmin; earlier rows name none (carnet#1170)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Garmin's agreement (v9 §15.11) bars using Garmin data to train, test,
-- evaluate or benchmark an AI model, and a verdict on a reply drawn from Garmin
-- data is derived from it. Each verdict names the consent keys of what that
-- reply was drawn from, comma-separated, as chat_messages does: the verifier
-- health rates, the verdict calibration dashboard and the coach's corrections
-- count leave out a verdict naming a key barred from evaluation. NULL names
-- none. Rows written before this column name none: what their replies were
-- drawn from cannot be attributed after the fact.
ALTER TABLE claim_verdicts ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
