-- ABOUTME: Cost/cached-token columns on llm_usage, channel_type on chat_conversations, embedding_usage table
-- ABOUTME: Phase 1 of billing foundation — every LLM call persists real USD cost; Gemini cache hits are tracked
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Cost and cache accounting on llm_usage.
-- cost_usd: calculated by pierre_llm::pricing::calculate_cost at insert time;
--           cached tokens bill at 25% of the model's input rate.
-- cached_tokens: prompt tokens that hit the provider's context cache; Gemini
--                returns this as cachedContentTokenCount, OpenAI as cached_tokens.
-- call_sequence: 1-based position of this LLM call within the owning turn_id.
--                Replaces the legacy turn_summary row with per-call rows.
ALTER TABLE llm_usage ADD COLUMN cost_usd REAL NOT NULL DEFAULT 0.0;
ALTER TABLE llm_usage ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE llm_usage ADD COLUMN call_sequence INTEGER;

-- Channel of origin (web | mobile | telegram | slack | discord | whatsapp | messenger).
-- Distinguishes web vs mobile billing + per-channel analytics.
ALTER TABLE chat_conversations ADD COLUMN channel_type TEXT NOT NULL DEFAULT 'web';

-- Hot path for per-user billing queries.
CREATE INDEX IF NOT EXISTS idx_llm_usage_tenant_user_created ON llm_usage(tenant_id, user_id, created_at DESC);

-- Embedding calls are not chat LLM calls and live in their own counter.
-- Separate table so embedding volume never dilutes chat-token billing.
CREATE TABLE IF NOT EXISTS embedding_usage (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL DEFAULT 0.0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_embedding_usage_tenant_user_created ON embedding_usage(tenant_id, user_id, created_at DESC);
