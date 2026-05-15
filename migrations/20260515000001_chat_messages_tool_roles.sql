-- ABOUTME: Extend chat_messages.role CHECK constraint to allow tool_call and tool_result rows
-- ABOUTME: Persists in-loop tool dispatch round (assistant call + result) so it survives the turn
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- SQLite cannot ALTER a CHECK constraint in place. Rebuild the table preserving
-- every column, copy the data, drop the original, and rename. The new constraint
-- accepts the two roles the chat pipeline emits when it persists a tool round:
-- - tool_call:   assistant message that contained a tool request (text-inlined
--                or native), preserved verbatim so replay reproduces the same
--                Vec<ChatMessage> the in-memory tool loop built.
-- - tool_result: formatted tool output (one row per round, holds either a
--                synthesized "[Tool Result for X]: ..." block or the joined
--                multi-tool text the loop currently pushes as a user message).

CREATE TABLE chat_messages_new (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool_call', 'tool_result')),
    content TEXT NOT NULL,
    token_count INTEGER,
    finish_reason TEXT,
    created_at TEXT NOT NULL,
    prompt_tokens INTEGER,
    model TEXT
);

INSERT INTO chat_messages_new (
    id, conversation_id, role, content, token_count, finish_reason, created_at, prompt_tokens, model
)
SELECT id, conversation_id, role, content, token_count, finish_reason, created_at, prompt_tokens, model
FROM chat_messages;

DROP TABLE chat_messages;
ALTER TABLE chat_messages_new RENAME TO chat_messages;

CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation ON chat_messages(conversation_id);
CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation_created ON chat_messages(conversation_id, created_at ASC);
