-- ABOUTME: A fact the agent inferred waits as a draft, with the words it came from, until the athlete keeps it
-- ABOUTME: Adds status, evidence, evidence_source and an idempotency key to user_facts; existing rows stay approved

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Until now every fact the extractor or the remember_fact tool wrote reached
-- the agent's prompt at once, on the agent's word alone. A fact the athlete
-- stated in their own words is still approved on write; one the agent
-- inferred is a draft that only the review endpoints read, and that reaches a
-- prompt once the athlete keeps it. Every row written before this migration
-- was already in prompts, so the default keeps it there.

-- approved: in prompts. draft: waiting on the athlete.
ALTER TABLE user_facts ADD COLUMN status TEXT NOT NULL DEFAULT 'approved' CHECK (status IN ('approved', 'draft')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The words the fact was taken from, verbatim; NULL for a form answer.
ALTER TABLE user_facts ADD COLUMN evidence TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Whose words evidence quotes: the athlete's message or the agent's reply.
ALTER TABLE user_facts ADD COLUMN evidence_source TEXT CHECK (evidence_source IN ('athlete', 'agent')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The fact's identity for a writer that may run twice (a resumed extraction
-- job): the same claim from the same writer lands on one row. NULL for the
-- writers that legitimately write a claim again (onboarding, PAR-Q).
ALTER TABLE user_facts ADD COLUMN idempotency_key TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- NULLs are distinct in a unique index on both engines, so unkeyed rows never collide.
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_facts_idempotency
    ON user_facts (tenant_id, user_id, idempotency_key);

-- The review list reads only drafts.
CREATE INDEX IF NOT EXISTS idx_user_facts_drafts
    ON user_facts (tenant_id, user_id, updated_at)
    WHERE status = 'draft';
