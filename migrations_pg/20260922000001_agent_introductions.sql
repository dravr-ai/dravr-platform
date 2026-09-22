-- ABOUTME: Which agents have introduced themselves in which thread — a conversation, or a shared room (PostgreSQL)
-- ABOUTME: Backfilled from every thread an agent already answered in, so none of them is re-introduced
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the SQLite mirror for why the fact is kept rather than read off the
-- transcript. `thread_id` holds either a conversation id (VARCHAR) or a
-- coaching group id (UUID) rendered as text, which is how the pipeline holds
-- both; `tenant_id` matches `chat_conversations.tenant_id`.
CREATE TABLE IF NOT EXISTS agent_introductions (
    tenant_id VARCHAR(255) NOT NULL,
    thread_id VARCHAR(255) NOT NULL,
    agent_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, thread_id, agent_id)
);

INSERT INTO agent_introductions (tenant_id, thread_id, agent_id, created_at)
SELECT DISTINCT c.tenant_id, COALESCE(c.group_id::text, c.id), c.agent_id, now()
FROM chat_conversations c
WHERE c.agent_id IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM chat_messages m
      WHERE m.conversation_id = c.id AND m.role = 'assistant'
  )
ON CONFLICT DO NOTHING;
