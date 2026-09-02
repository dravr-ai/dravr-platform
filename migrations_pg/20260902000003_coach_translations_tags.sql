-- ABOUTME: Adds coach_translations.tags, the per-locale tag list a coach's <locale>.md declares
-- ABOUTME: NULL leaves the English tags visible; a JSON array replaces them for that locale

-- Tags were the last English words on a French Discover card. A coach's
-- fr.md may declare its own tags; the seeder stores them here and the store
-- overlay applies them beside the title and description.
ALTER TABLE coach_translations ADD COLUMN IF NOT EXISTS tags TEXT;
