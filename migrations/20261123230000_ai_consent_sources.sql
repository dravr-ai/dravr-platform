-- ABOUTME: Names on each derived row the AI-consent keys of the provider data it was drawn from (garmin, whoop)
-- ABOUTME: Additive and nullable: a withdrawal then keeps the row out of every prompt; earlier rows name none (carnet#1157, #1158)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A reply, a memory fact or a compaction summary built from a provider's data
-- is that provider's data too (Garmin agreement v9 §15.11). Each such row names
-- the consent keys of what it was drawn from, comma-separated: once the athlete
-- withdraws one, no model reads the row, and no model ever reads a fact or a
-- summary naming a key kept out of the agent's memory (Garmin, until Garmin
-- authorizes it in writing). NULL names none. Rows written before this column
-- name none: what they were drawn from cannot be attributed after the fact.
ALTER TABLE chat_messages ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_facts ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE compaction_blocks ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
