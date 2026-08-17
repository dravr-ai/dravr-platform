-- ABOUTME: Moves stored workout plans onto the content_blocks rail and drops structured_content
-- ABOUTME: One content rail, so a client learns one shape instead of two

-- See the SQLite twin for the reasoning. The shape differs only because
-- PostgreSQL builds JSON with jsonb_build_* rather than SQLite's json_*.
UPDATE chat_messages
SET content_blocks = jsonb_build_array(
        jsonb_build_object(
            'type', 'workout_plan',
            'source_tool', 'structured-workout',
            'plan', structured_content::jsonb
        )
    )::text
WHERE structured_content IS NOT NULL
  AND TRIM(structured_content) <> ''
  AND content_blocks IS NULL;

ALTER TABLE chat_messages DROP COLUMN IF EXISTS structured_content;
