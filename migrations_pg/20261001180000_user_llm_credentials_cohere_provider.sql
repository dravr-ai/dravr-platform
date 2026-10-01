-- ABOUTME: user_llm_credentials admits the 'cohere' provider the LLM settings routes and TenantLlmManager accept
-- ABOUTME: Replaces the provider CHECK under its PostgreSQL-assigned name with the widened vocabulary

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The provider vocabulary is pierre-auth's LlmProvider::as_str(): gemini,
-- groq, openai, anthropic, cohere, local. The CHECK from 20260311000001
-- predates Cohere, so saving Cohere credentials failed the constraint.
--
-- PostgreSQL named the inline column constraint
-- user_llm_credentials_provider_check; it is replaced under the same name so
-- the table keeps exactly one provider CHECK. Every stored row satisfies the
-- new constraint: the old vocabulary is a subset of it.
ALTER TABLE user_llm_credentials
    DROP CONSTRAINT IF EXISTS user_llm_credentials_provider_check;
ALTER TABLE user_llm_credentials
    ADD CONSTRAINT user_llm_credentials_provider_check
    CHECK (provider IN ('gemini', 'groq', 'openai', 'anthropic', 'cohere', 'local'));
