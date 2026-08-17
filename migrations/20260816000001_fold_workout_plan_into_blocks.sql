-- ABOUTME: Moves stored workout plans onto the content_blocks rail and drops structured_content
-- ABOUTME: One content rail, so a client learns one shape instead of two

-- Plans used to travel their own column while charts and tables travelled
-- content_blocks: two systems carrying one idea. Every stored plan becomes a
-- single-element block array of the same shape the pipeline now writes, so
-- history renders through the same path as new replies.
--
-- source_tool names the schema the plan was validated against rather than a
-- tool call. Unlike a chart, a plan is authored from the athlete's whole
-- context rather than read out of one tool's response, so there is no single
-- call to cite.
UPDATE chat_messages
SET content_blocks = json_array(
        json_object(
            'type', 'workout_plan',
            'source_tool', 'structured-workout',
            'plan', json(structured_content)
        )
    )
WHERE structured_content IS NOT NULL
  AND TRIM(structured_content) <> ''
  AND content_blocks IS NULL
  AND json_valid(structured_content);

-- Pre-1.0 with no external consumers: a single-commit cutover, not a
-- deprecation window. Leaving the column would leave two places to read a plan
-- from and guarantee they drift.
ALTER TABLE chat_messages DROP COLUMN structured_content;  -- idempotency-ok: SQLite has no IF EXISTS on DROP COLUMN; adding it is a syntax error
