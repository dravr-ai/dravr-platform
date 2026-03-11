-- ABOUTME: Consolidated PostgreSQL migration for tenant management and OAuth tables.
-- ABOUTME: Creates tenants, OAuth apps, authorization codes, tokens, clients, and provider connections.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- tenants
-- ============================================================================
CREATE TABLE IF NOT EXISTS tenants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) UNIQUE NOT NULL,
    domain VARCHAR(255) UNIQUE,
    subscription_tier VARCHAR(50) DEFAULT 'starter' CHECK (subscription_tier IN ('starter', 'professional', 'enterprise')),
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- tenant_oauth_credentials
-- ============================================================================
CREATE TABLE IF NOT EXISTS tenant_oauth_credentials (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider VARCHAR(50) NOT NULL,
    client_id VARCHAR(255) NOT NULL,
    client_secret_encrypted TEXT NOT NULL,
    redirect_uri VARCHAR(500) NOT NULL,
    scopes TEXT[] DEFAULT '{}',
    rate_limit_per_day INTEGER DEFAULT 15000,
    is_active BOOLEAN DEFAULT TRUE,
    configured_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, provider)
);

-- ============================================================================
-- tenant_users
-- ============================================================================
CREATE TABLE IF NOT EXISTS tenant_users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role VARCHAR(50) DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'billing', 'member')),
    invited_at TIMESTAMPTZ NOT NULL,
    joined_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, user_id)
);

-- ============================================================================
-- tenant_provider_usage
-- ============================================================================
CREATE TABLE IF NOT EXISTS tenant_provider_usage (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider VARCHAR(50) NOT NULL,
    usage_date DATE NOT NULL,
    request_count INTEGER DEFAULT 0,
    error_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, provider, usage_date)
);

-- ============================================================================
-- oauth_apps
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth_apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    client_id VARCHAR(255) UNIQUE NOT NULL,
    client_secret VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    redirect_uris TEXT[] NOT NULL DEFAULT '{}',
    scopes TEXT[] NOT NULL DEFAULT '{}',
    app_type VARCHAR(50) DEFAULT 'web' CHECK (app_type IN ('desktop', 'web', 'mobile', 'server')),
    owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- authorization_codes
-- ============================================================================
CREATE TABLE IF NOT EXISTS authorization_codes (
    code VARCHAR(255) PRIMARY KEY,
    client_id VARCHAR(255) NOT NULL REFERENCES oauth_apps(client_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri VARCHAR(500) NOT NULL,
    scope VARCHAR(500) NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ NOT NULL
);

-- ============================================================================
-- user_oauth_tokens
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_oauth_tokens (
    id VARCHAR(255) PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id VARCHAR(255) NOT NULL,
    provider VARCHAR(50) NOT NULL,
    access_token TEXT NOT NULL,
    refresh_token TEXT,
    token_type VARCHAR(50) DEFAULT 'bearer',
    expires_at TIMESTAMPTZ,
    scope TEXT,
    last_sync TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, tenant_id, provider)
);

-- ============================================================================
-- oauth2_clients
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth2_clients (
    id TEXT PRIMARY KEY,
    client_id TEXT UNIQUE NOT NULL,
    client_secret_hash TEXT NOT NULL,
    redirect_uris TEXT NOT NULL,
    grant_types TEXT NOT NULL,
    response_types TEXT NOT NULL,
    client_name TEXT,
    client_uri TEXT,
    scope TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_oauth2_clients_client_id ON oauth2_clients(client_id);

-- ============================================================================
-- oauth2_auth_codes
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth2_auth_codes (
    code TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    scope TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE,
    state TEXT,
    code_challenge TEXT,
    code_challenge_method TEXT,
    FOREIGN KEY (client_id) REFERENCES oauth2_clients(client_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_oauth2_auth_codes_expires_at ON oauth2_auth_codes(expires_at);
CREATE INDEX IF NOT EXISTS idx_oauth2_auth_codes_tenant_user ON oauth2_auth_codes(tenant_id, user_id);

-- ============================================================================
-- oauth2_refresh_tokens
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth2_refresh_tokens (
    token TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    scope TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE,
    FOREIGN KEY (client_id) REFERENCES oauth2_clients(client_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_oauth2_refresh_tokens_tenant_user ON oauth2_refresh_tokens(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_oauth2_refresh_tokens_user_id ON oauth2_refresh_tokens(user_id);

-- ============================================================================
-- oauth2_states
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth2_states (
    state TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    user_id TEXT,
    tenant_id TEXT,
    redirect_uri TEXT NOT NULL,
    scope TEXT,
    code_challenge TEXT,
    code_challenge_method TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE,
    FOREIGN KEY (client_id) REFERENCES oauth2_clients(client_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_oauth2_states_expires_at ON oauth2_states(expires_at);

-- ============================================================================
-- oauth_client_states
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth_client_states (
    state TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    user_id TEXT,
    tenant_id TEXT,
    redirect_uri TEXT NOT NULL,
    scope TEXT,
    pkce_code_verifier TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS idx_oauth_client_states_expires_at ON oauth_client_states(expires_at);
CREATE INDEX IF NOT EXISTS idx_oauth_client_states_provider ON oauth_client_states(provider);

-- ============================================================================
-- provider_connections
-- ============================================================================
CREATE TABLE IF NOT EXISTS provider_connections (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    connection_type TEXT NOT NULL,
    connected_at TIMESTAMPTZ NOT NULL,
    metadata TEXT,
    UNIQUE(user_id, tenant_id, provider)
);
CREATE INDEX IF NOT EXISTS idx_provider_connections_user ON provider_connections(user_id);
CREATE INDEX IF NOT EXISTS idx_provider_connections_tenant ON provider_connections(tenant_id, provider);

-- ============================================================================
-- Tenant and OAuth indexes
-- ============================================================================
CREATE INDEX IF NOT EXISTS idx_tenant_oauth_credentials_tenant_provider ON tenant_oauth_credentials(tenant_id, provider);
CREATE INDEX IF NOT EXISTS idx_tenant_users_tenant ON tenant_users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tenant_users_user ON tenant_users(user_id);
CREATE INDEX IF NOT EXISTS idx_user_oauth_tokens_user ON user_oauth_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_user_oauth_tokens_tenant_provider ON user_oauth_tokens(tenant_id, provider);
CREATE INDEX IF NOT EXISTS idx_tenant_usage_date ON tenant_provider_usage(tenant_id, provider, usage_date);
