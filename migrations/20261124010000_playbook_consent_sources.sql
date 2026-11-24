-- ABOUTME: Names on each advice, playbook and commitment the AI-consent keys of the provider data it was drawn from
-- ABOUTME: Additive and nullable: a withdrawal then keeps the row out of every prompt; earlier rows name none (carnet#1157)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Advice captured from a reply, the playbook its outcome reinforces and a
-- commitment the agent records are derived from the provider data the turn
-- (and, for a playbook or a verdict, the window it was judged over) served,
-- as a reply or a memory fact is (20261123230000). Each names the consent keys
-- of what it was drawn from, comma-separated: once the athlete withdraws one,
-- no model reads the row; no model ever reads a playbook naming a key kept out
-- of the agent's memory. Advice carries its keys into the playbook it labels.
-- NULL names none. Rows written before this column name none: what they were
-- drawn from cannot be attributed after the fact.
ALTER TABLE pending_advice ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_playbooks ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE athlete_commitments ADD COLUMN ai_consent_sources TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
