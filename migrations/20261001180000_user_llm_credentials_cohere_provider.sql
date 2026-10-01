-- ABOUTME: user_llm_credentials admits the 'cohere' provider the LLM settings routes and TenantLlmManager accept
-- ABOUTME: Rebuilds the table under the widened provider CHECK; SQLite cannot ALTER a CHECK constraint

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The provider vocabulary is pierre-auth's LlmProvider::as_str(): gemini,
-- groq, openai, anthropic, cohere, local. The CHECK from 20250120000025
-- predates Cohere, so saving Cohere credentials failed the constraint.
--
-- SQLite has no ALTER for a CHECK constraint: the table is copied under the
-- widened constraint, the original dropped, the copy renamed and the indexes
-- recreated. sqlx runs the migration in one transaction and no other table
-- references this one by foreign key, so no PRAGMA is needed. Every row
-- survives the copy: the old vocabulary is a subset of the new one.
CREATE TABLE IF NOT EXISTS user_llm_credentials_new (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,  -- NULL = tenant-level default
    provider TEXT NOT NULL CHECK (provider IN ('gemini', 'groq', 'openai', 'anthropic', 'cohere', 'local')),
    api_key_encrypted TEXT NOT NULL,  -- AES-256-GCM encrypted with AAD
    base_url TEXT,  -- For local/custom providers only
    default_model TEXT,  -- Optional model override
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES users(id),
    UNIQUE(tenant_id, user_id, provider)
);
INSERT INTO user_llm_credentials_new (id, tenant_id, user_id, provider, api_key_encrypted,
    base_url, default_model, is_active, created_at, updated_at, created_by)
  SELECT id, tenant_id, user_id, provider, api_key_encrypted, base_url, default_model,
    is_active, created_at, updated_at, created_by
  FROM user_llm_credentials;
DROP TABLE IF EXISTS user_llm_credentials;
ALTER TABLE user_llm_credentials_new RENAME TO user_llm_credentials;

CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_tenant ON user_llm_credentials(tenant_id);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_user ON user_llm_credentials(user_id);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_provider ON user_llm_credentials(provider);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_lookup ON user_llm_credentials(tenant_id, user_id, provider);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_tenant_default ON user_llm_credentials(tenant_id, provider) WHERE user_id IS NULL;
