-- ABOUTME: Coach translations table — per-locale title/description/purpose/instructions overlay
-- ABOUTME: English stays canonical on the coaches row; locale fallback chain resolves at read time
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Per-locale overlay for coach-authored content. The canonical English copy
-- stays on the `coaches` row (title / description / purpose / instructions
-- columns). A coach_translations row provides the localized copy for a given
-- BCP-47 short locale ("fr", "en", "es", "de", "pt").
--
-- source_sha holds the first 16 hex chars of sha256(en.md) captured at
-- translation time. At read time the repository compares it to the current
-- English-source hash; a mismatch flags the translation as stale and the
-- reader falls back to the canonical English fields on `coaches`.
CREATE TABLE IF NOT EXISTS coach_translations (
    coach_id TEXT NOT NULL,
    locale TEXT NOT NULL,
    title TEXT,
    description TEXT,
    purpose TEXT,
    instructions TEXT,
    source_sha TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (coach_id, locale),
    FOREIGN KEY (coach_id) REFERENCES coaches(id) ON DELETE CASCADE
);

-- locale-first lookup is the hot path (list all translations in fr, etc.)
CREATE INDEX IF NOT EXISTS idx_coach_translations_locale ON coach_translations(locale);
