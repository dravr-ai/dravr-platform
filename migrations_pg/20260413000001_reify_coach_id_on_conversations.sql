-- ABOUTME: Reify coach_id on chat_conversations as a first-class FK to coaches(id) (PostgreSQL)
-- ABOUTME: Backfills coach_id from the legacy system_prompt text match, then drops the column

-- See the matching SQLite migration for the full rationale. Summary: coach context
-- becomes a first-class FK on the conversation, the legacy system_prompt string is
-- removed, and existing data is backfilled by matching against coaches.system_prompt.

-- Step 1: add coach_id column as a nullable FK
ALTER TABLE chat_conversations
    ADD COLUMN coach_id TEXT REFERENCES coaches(id);

-- Step 2: backfill coach_id. Prefer same-tenant coaches, fall back to system coaches.
-- Two independent correlated subqueries combined via COALESCE for SQLite parity.
UPDATE chat_conversations cc
SET coach_id = COALESCE(
    (
        SELECT c.id
        FROM coaches c
        WHERE c.system_prompt = cc.system_prompt
          AND c.tenant_id = cc.tenant_id
        ORDER BY c.created_at ASC
        LIMIT 1
    ),
    (
        SELECT c.id
        FROM coaches c
        WHERE c.system_prompt = cc.system_prompt
          AND c.is_system = TRUE
        ORDER BY c.created_at ASC
        LIMIT 1
    )
)
WHERE cc.system_prompt IS NOT NULL;

-- Step 3: drop the legacy column. Pre-1.0, no external consumers, clean cutover.
ALTER TABLE chat_conversations
    DROP COLUMN system_prompt;

-- Step 4: index for coach-scoped lookups.
CREATE INDEX IF NOT EXISTS idx_chat_conversations_coach_id
    ON chat_conversations(coach_id)
    WHERE coach_id IS NOT NULL;
