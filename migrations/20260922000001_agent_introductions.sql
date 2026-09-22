-- ABOUTME: Which agents have introduced themselves in which thread — a conversation, or a shared room (SQLite)
-- ABOUTME: Backfilled from every thread an agent already answered in, so none of them is re-introduced
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An agent opens its first reply in a thread by naming itself and its role
-- (carnet#501). "First" cannot be read off the transcript: a messaging DM is
-- one long-lived conversation re-pointed at whichever agent the athlete picks
-- last, an `@handle` hands one turn to another agent, and a shared room keeps
-- one conversation row per member. So the fact is kept here, one row per agent
-- per thread, written once a delivered reply has actually named the agent.
--
-- `thread_id` is the coaching group id when the conversation belongs to a
-- room — every member's row there reads the same room — and the conversation
-- id otherwise. Rows are keyed under the conversation's tenant like every
-- other chat row.
CREATE TABLE IF NOT EXISTS agent_introductions (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id, agent_id)
);

-- A thread the conversation's agent has already answered in has met that
-- agent. Without this row it would introduce itself again mid-thread on the
-- first turn after the deploy.
INSERT INTO agent_introductions (tenant_id, thread_id, agent_id, created_at)
SELECT DISTINCT c.tenant_id, COALESCE(c.group_id, c.id), c.agent_id, CURRENT_TIMESTAMP
FROM chat_conversations c
WHERE c.agent_id IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM chat_messages m
      WHERE m.conversation_id = c.id AND m.role = 'assistant'
  )
ON CONFLICT DO NOTHING;
