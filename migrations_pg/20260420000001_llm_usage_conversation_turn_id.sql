-- ABOUTME: Thread ConversationTurnId + tools_called onto llm_usage for per-turn observability
-- ABOUTME: Rows predating this migration keep the nil UUID as a sentinel turn_id
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE llm_usage
    ADD COLUMN turn_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000';

-- tools_called is JSON-encoded text in both backends so that a single
-- `&str` binding in `InsertLlmUsage` works against SQLite and Postgres.
ALTER TABLE llm_usage
    ADD COLUMN tools_called TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS idx_llm_usage_turn_id ON llm_usage(turn_id);
