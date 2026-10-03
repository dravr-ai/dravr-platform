-- ABOUTME: Adds agent_translations.sample_prompts, the per-locale Example Inputs an agent's <locale>.md declares
-- ABOUTME: NULL leaves the English samples visible; a JSON array replaces them for that locale
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An agent bound into a thread opens it with a few starter questions taken
-- from its sample prompts (carnet#735). Those were English only: the overlay
-- translated the title and description but not the samples, so a French
-- athlete would read « Pour commencer, tu peux me demander : » above English
-- questions. The seeder stores each locale's Example Inputs here.
--
-- SQLite has no `ADD COLUMN IF NOT EXISTS`, so the table is rebuilt the way
-- 20260902000003 rebuilt it for `tags`: same shape plus `sample_prompts`, rows
-- copied, swap, index recreated. Every statement is re-runnable.
PRAGMA defer_foreign_keys = ON;

DROP TABLE IF EXISTS agent_translations_new;

CREATE TABLE IF NOT EXISTS agent_translations_new (
    agent_id TEXT NOT NULL,
    locale TEXT NOT NULL,
    title TEXT,
    description TEXT,
    purpose TEXT,
    instructions TEXT,
    source_sha TEXT,
    tags TEXT,
    sample_prompts TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (agent_id, locale),
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

INSERT INTO agent_translations_new
    (agent_id, locale, title, description, purpose, instructions, source_sha, tags, created_at, updated_at)
SELECT agent_id, locale, title, description, purpose, instructions, source_sha, tags, created_at, updated_at
FROM agent_translations;

DROP TABLE agent_translations; -- idempotency-ok: the rebuild swap, guarded by the copy above

ALTER TABLE agent_translations_new RENAME TO agent_translations;

CREATE INDEX IF NOT EXISTS idx_agent_translations_locale ON agent_translations(locale);
