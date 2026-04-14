-- ABOUTME: Reify coach_id on chat_conversations as a first-class FK to coaches(id)
-- ABOUTME: Backfills coach_id from the legacy system_prompt text match, then drops the column

-- Context: historically, `chat_conversations.system_prompt` stored a copy of the
-- selected coach's system prompt as a string. Tier -1 of the coaching harness makes
-- `coach_id` the single source of truth: the conversation references a coach, and the
-- coach owns the system prompt. Backfill best-effort matches existing conversations
-- to a coach via exact system_prompt text match, preferring same-tenant coaches and
-- falling back to system coaches. Conversations with no match land with coach_id = NULL
-- (default Pierre prompt at runtime).

-- SQLite does not support ALTER TABLE DROP COLUMN reliably across versions, so we
-- rebuild the table. Existing references from chat_messages.conversation_id are
-- preserved because the replacement table retains the same name and PRIMARY KEY.

CREATE TABLE chat_conversations_new (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    title TEXT NOT NULL,
    model TEXT NOT NULL DEFAULT 'gemini-2.0-flash-exp',
    coach_id TEXT REFERENCES coaches(id),
    total_tokens INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    group_id TEXT REFERENCES coaching_groups(id)
);

-- Copy existing rows with coach_id backfilled from system_prompt text match.
-- Prefer same-tenant coach match; fall back to system coaches. Ties broken
-- by earliest created_at for determinism. Two independent correlated
-- subqueries combined via COALESCE avoids SQLite's stricter resolution
-- of outer columns inside a subquery's ORDER BY CASE.
INSERT INTO chat_conversations_new (
    id, user_id, tenant_id, title, model, coach_id,
    total_tokens, created_at, updated_at, group_id
)
SELECT
    cc.id,
    cc.user_id,
    cc.tenant_id,
    cc.title,
    cc.model,
    COALESCE(
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
              AND c.is_system = 1
            ORDER BY c.created_at ASC
            LIMIT 1
        )
    ) AS coach_id,
    cc.total_tokens,
    cc.created_at,
    cc.updated_at,
    cc.group_id
FROM chat_conversations cc;

DROP TABLE chat_conversations;
ALTER TABLE chat_conversations_new RENAME TO chat_conversations;

-- Recreate the indexes that existed on the original table, plus a new coach_id lookup index.
CREATE INDEX IF NOT EXISTS idx_chat_conversations_user ON chat_conversations(user_id);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_tenant ON chat_conversations(tenant_id);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_user_tenant ON chat_conversations(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_updated ON chat_conversations(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_coach_id ON chat_conversations(coach_id) WHERE coach_id IS NOT NULL;
