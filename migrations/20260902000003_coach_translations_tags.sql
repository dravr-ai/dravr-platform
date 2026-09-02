-- ABOUTME: Adds coach_translations.tags, the per-locale tag list a coach's <locale>.md declares
-- ABOUTME: NULL leaves the English tags visible; a JSON array replaces them for that locale
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Tags were the last English words on a French Discover card. A coach's
-- fr.md may declare its own tags; the seeder stores them here and the store
-- overlay applies them beside the title and description.
--
-- SQLite has no `ADD COLUMN IF NOT EXISTS`, so the table is rebuilt the way
-- 20260902000001 rebuilds user_facts: same shape plus `tags`, rows copied,
-- swap, index recreated. Every statement is re-runnable.
PRAGMA defer_foreign_keys = ON;

DROP TABLE IF EXISTS coach_translations_new;

CREATE TABLE IF NOT EXISTS coach_translations_new (
    coach_id TEXT NOT NULL,
    locale TEXT NOT NULL,
    title TEXT,
    description TEXT,
    purpose TEXT,
    instructions TEXT,
    source_sha TEXT,
    tags TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (coach_id, locale),
    FOREIGN KEY (coach_id) REFERENCES coaches(id) ON DELETE CASCADE
);

INSERT INTO coach_translations_new
    (coach_id, locale, title, description, purpose, instructions, source_sha, created_at, updated_at)
SELECT coach_id, locale, title, description, purpose, instructions, source_sha, created_at, updated_at
FROM coach_translations;

DROP TABLE coach_translations; -- idempotency-ok: the rebuild swap, guarded by the copy above

ALTER TABLE coach_translations_new RENAME TO coach_translations;

CREATE INDEX IF NOT EXISTS idx_coach_translations_locale ON coach_translations(locale);
