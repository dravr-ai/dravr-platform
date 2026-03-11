-- ABOUTME: Consolidated PostgreSQL migration for core user and authentication tables.
-- ABOUTME: Creates users, profiles, OAuth credentials, tokens, LLM credentials, and delegation tables.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- users
-- ============================================================================
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    display_name TEXT,
    password_hash TEXT NOT NULL,
    tier TEXT NOT NULL DEFAULT 'starter' CHECK (tier IN ('starter', 'professional', 'enterprise')),
    tenant_id TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    user_status TEXT NOT NULL DEFAULT 'pending' CHECK (user_status IN ('pending', 'active', 'suspended')),
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('super_admin', 'admin', 'user')),
    approved_by UUID REFERENCES users(id),
    approved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    last_active TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    firebase_uid TEXT,
    auth_provider TEXT NOT NULL DEFAULT 'email'
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_firebase_uid ON users(firebase_uid) WHERE firebase_uid IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_users_auth_provider ON users(auth_provider);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

-- ============================================================================
-- user_profiles
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_profiles (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    profile_data JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- user_oauth_app_credentials
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_oauth_app_credentials (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('strava', 'garmin', 'fitbit', 'whoop', 'terra')),
    client_id TEXT NOT NULL,
    client_secret TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(user_id, provider)
);
CREATE INDEX IF NOT EXISTS idx_user_oauth_apps_user ON user_oauth_app_credentials(user_id);
CREATE INDEX IF NOT EXISTS idx_user_oauth_apps_provider ON user_oauth_app_credentials(provider);
CREATE INDEX IF NOT EXISTS idx_user_oauth_apps_user_provider ON user_oauth_app_credentials(user_id, provider);

-- ============================================================================
-- user_configurations
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_configurations (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    config_data TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

-- ============================================================================
-- user_mcp_tokens
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_mcp_tokens (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    usage_count INTEGER NOT NULL DEFAULT 0,
    is_revoked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_user_tokens_user ON user_mcp_tokens(user_id, is_revoked);
CREATE INDEX IF NOT EXISTS idx_user_tokens_prefix ON user_mcp_tokens(token_prefix);

-- ============================================================================
-- user_llm_credentials
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_llm_credentials (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('gemini', 'groq', 'openai', 'anthropic', 'local')),
    api_key_encrypted TEXT NOT NULL,
    base_url TEXT,
    default_model TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    created_by UUID NOT NULL REFERENCES users(id),
    UNIQUE(tenant_id, user_id, provider)
);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_tenant ON user_llm_credentials(tenant_id);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_user ON user_llm_credentials(user_id);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_provider ON user_llm_credentials(provider);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_lookup ON user_llm_credentials(tenant_id, user_id, provider);
CREATE INDEX IF NOT EXISTS idx_user_llm_credentials_tenant_default ON user_llm_credentials(tenant_id, provider) WHERE user_id IS NULL;

-- ============================================================================
-- user_llm_credentials_audit
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_llm_credentials_audit (
    id TEXT PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('create', 'update', 'delete', 'rotate')),
    credential_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id UUID NOT NULL,
    provider TEXT NOT NULL,
    ip_address TEXT,
    user_agent TEXT,
    reason TEXT
);
CREATE INDEX IF NOT EXISTS idx_user_llm_audit_timestamp ON user_llm_credentials_audit(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_user_llm_audit_tenant ON user_llm_credentials_audit(tenant_id);
CREATE INDEX IF NOT EXISTS idx_user_llm_audit_user ON user_llm_credentials_audit(user_id);

-- ============================================================================
-- password_reset_tokens
-- ============================================================================
CREATE TABLE IF NOT EXISTS password_reset_tokens (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_user_id ON password_reset_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_token_hash ON password_reset_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_expires_at ON password_reset_tokens(expires_at);

-- ============================================================================
-- impersonation_sessions
-- ============================================================================
CREATE TABLE IF NOT EXISTS impersonation_sessions (
    id TEXT PRIMARY KEY,
    impersonator_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_impersonation_active ON impersonation_sessions(impersonator_id, is_active);
CREATE INDEX IF NOT EXISTS idx_impersonation_target ON impersonation_sessions(target_user_id);

-- ============================================================================
-- permission_delegations
-- ============================================================================
CREATE TABLE IF NOT EXISTS permission_delegations (
    id TEXT PRIMARY KEY,
    grantor_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    grantee_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permissions INTEGER NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(grantor_id, grantee_id)
);
CREATE INDEX IF NOT EXISTS idx_delegation_grantee ON permission_delegations(grantee_id);
CREATE INDEX IF NOT EXISTS idx_delegation_grantor ON permission_delegations(grantor_id);

-- ============================================================================
-- fitness_configurations
-- ============================================================================
CREATE TABLE IF NOT EXISTS fitness_configurations (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id UUID,
    configuration_name TEXT NOT NULL DEFAULT 'default',
    config_data TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(tenant_id, user_id, configuration_name)
);
CREATE INDEX IF NOT EXISTS idx_fitness_configs_tenant ON fitness_configurations(tenant_id);
CREATE INDEX IF NOT EXISTS idx_fitness_configs_user ON fitness_configurations(user_id);
CREATE INDEX IF NOT EXISTS idx_fitness_configs_tenant_user ON fitness_configurations(tenant_id, user_id);
