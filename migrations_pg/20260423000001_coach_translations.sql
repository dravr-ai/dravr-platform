-- ABOUTME: Coach translations table (PostgreSQL) — per-locale overlay for coach content
-- ABOUTME: English stays canonical on coaches; locale fallback chain resolves at read time
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the SQLite sibling migration for the full semantic contract. PG uses
-- TIMESTAMPTZ for timestamps and an explicit UUID type is not required because
-- coach_translations.coach_id mirrors coaches.id which is TEXT on both backends.
CREATE TABLE IF NOT EXISTS coach_translations (
    coach_id TEXT NOT NULL,
    locale TEXT NOT NULL,
    title TEXT,
    description TEXT,
    purpose TEXT,
    instructions TEXT,
    source_sha TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (coach_id, locale),
    FOREIGN KEY (coach_id) REFERENCES coaches(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_coach_translations_locale ON coach_translations(locale);
