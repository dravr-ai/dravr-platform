-- ABOUTME: Extend chat_messages.role CHECK constraint to allow tool_call and tool_result rows
-- ABOUTME: Persists in-loop tool dispatch round (assistant call + result) so it survives the turn
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Drop the existing CHECK constraint (auto-named by Postgres on table creation)
-- and replace it with one that accepts the two new tool round roles. The new
-- values mirror what the in-memory tool loop already pushes to llm_messages:
-- - tool_call:   assistant message carrying a tool request (text-inlined or native).
-- - tool_result: formatted tool output (synthesized "[Tool Result for X]: ..." block).

ALTER TABLE chat_messages DROP CONSTRAINT IF EXISTS chat_messages_role_check;
ALTER TABLE chat_messages
    ADD CONSTRAINT chat_messages_role_check
    CHECK (role IN ('system', 'user', 'assistant', 'tool_call', 'tool_result'));
