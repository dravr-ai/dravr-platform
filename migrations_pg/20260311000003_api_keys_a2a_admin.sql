-- ABOUTME: Consolidated PostgreSQL migration for API keys, A2A protocol, admin tokens, and audit tables.
-- ABOUTME: Creates api_keys, a2a_clients/sessions/tasks/usage, admin tokens/config, JWT usage, and audit events.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- api_keys
-- ============================================================================
CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    key_prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL,
    description TEXT,
    tier TEXT NOT NULL CHECK (tier IN ('trial', 'starter', 'professional', 'enterprise')),
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    rate_limit_requests INTEGER NOT NULL,
    rate_limit_window_seconds INTEGER NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- api_key_usage
-- ============================================================================
CREATE TABLE IF NOT EXISTS api_key_usage (
    id SERIAL PRIMARY KEY,
    api_key_id TEXT NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    endpoint TEXT NOT NULL,
    response_time_ms INTEGER,
    status_code SMALLINT NOT NULL,
    method TEXT,
    request_size_bytes INTEGER,
    response_size_bytes INTEGER,
    ip_address INET,
    user_agent TEXT
);

-- ============================================================================
-- a2a_clients
-- ============================================================================
CREATE TABLE IF NOT EXISTS a2a_clients (
    client_id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    client_secret_hash TEXT NOT NULL,
    api_key_hash TEXT NOT NULL,
    capabilities TEXT[] NOT NULL DEFAULT '{}',
    redirect_uris TEXT[] NOT NULL DEFAULT '{}',
    contact_email TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    rate_limit_per_minute INTEGER NOT NULL DEFAULT 100,
    rate_limit_per_day INTEGER DEFAULT 10000,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- a2a_client_api_keys
-- ============================================================================
CREATE TABLE IF NOT EXISTS a2a_client_api_keys (
    client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
    api_key_id TEXT NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (client_id, api_key_id)
);
CREATE INDEX IF NOT EXISTS idx_a2a_client_api_keys_api_key_id ON a2a_client_api_keys(api_key_id);

-- ============================================================================
-- a2a_sessions
-- ============================================================================
CREATE TABLE IF NOT EXISTS a2a_sessions (
    session_token TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    granted_scopes TEXT[] NOT NULL DEFAULT '{}',
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ NOT NULL,
    last_active_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- a2a_tasks
-- ============================================================================
CREATE TABLE IF NOT EXISTS a2a_tasks (
    task_id TEXT PRIMARY KEY,
    session_token TEXT NOT NULL REFERENCES a2a_sessions(session_token) ON DELETE CASCADE,
    task_type TEXT NOT NULL,
    parameters JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    result JSONB,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- ============================================================================
-- a2a_usage
-- ============================================================================
CREATE TABLE IF NOT EXISTS a2a_usage (
    id SERIAL PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
    session_token TEXT REFERENCES a2a_sessions(session_token) ON DELETE SET NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    endpoint TEXT NOT NULL,
    response_time_ms INTEGER,
    status_code SMALLINT NOT NULL,
    method TEXT,
    request_size_bytes INTEGER,
    response_size_bytes INTEGER,
    ip_address INET,
    user_agent TEXT,
    protocol_version TEXT NOT NULL DEFAULT 'v1',
    client_capabilities TEXT[] DEFAULT '{}',
    granted_scopes TEXT[] DEFAULT '{}'
);

-- ============================================================================
-- admin_tokens
-- ============================================================================
CREATE TABLE IF NOT EXISTS admin_tokens (
    id TEXT PRIMARY KEY,
    service_name TEXT NOT NULL,
    service_description TEXT,
    token_hash TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    jwt_secret_hash TEXT NOT NULL,
    permissions TEXT NOT NULL DEFAULT '[]',
    is_super_admin BOOLEAN NOT NULL DEFAULT FALSE,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    last_used_ip INET,
    usage_count BIGINT NOT NULL DEFAULT 0
);

-- ============================================================================
-- admin_token_usage
-- ============================================================================
CREATE TABLE IF NOT EXISTS admin_token_usage (
    id SERIAL PRIMARY KEY,
    admin_token_id TEXT NOT NULL REFERENCES admin_tokens(id) ON DELETE CASCADE,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    action TEXT NOT NULL,
    target_resource TEXT,
    ip_address INET,
    user_agent TEXT,
    request_size_bytes INTEGER,
    success BOOLEAN NOT NULL,
    method TEXT,
    response_time_ms INTEGER
);

-- ============================================================================
-- admin_provisioned_keys
-- ============================================================================
CREATE TABLE IF NOT EXISTS admin_provisioned_keys (
    id SERIAL PRIMARY KEY,
    admin_token_id TEXT NOT NULL REFERENCES admin_tokens(id) ON DELETE CASCADE,
    api_key_id TEXT NOT NULL,
    user_email TEXT NOT NULL,
    requested_tier TEXT NOT NULL,
    provisioned_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    provisioned_by_service TEXT NOT NULL,
    rate_limit_requests INTEGER NOT NULL,
    rate_limit_period TEXT NOT NULL,
    key_status TEXT NOT NULL DEFAULT 'active',
    revoked_at TIMESTAMPTZ,
    revoked_reason TEXT
);

-- ============================================================================
-- admin_config_categories
-- ============================================================================
CREATE TABLE IF NOT EXISTS admin_config_categories (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    display_order INTEGER NOT NULL DEFAULT 0,
    icon TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE
);
CREATE INDEX IF NOT EXISTS idx_admin_config_categories_order ON admin_config_categories(display_order);

-- ============================================================================
-- admin_config_overrides
-- ============================================================================
CREATE TABLE IF NOT EXISTS admin_config_overrides (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    config_key TEXT NOT NULL,
    config_value TEXT NOT NULL,
    data_type TEXT NOT NULL CHECK (data_type IN ('float', 'integer', 'boolean', 'string', 'enum')),
    tenant_id TEXT,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    reason TEXT,
    UNIQUE(category, config_key, tenant_id)
);
CREATE INDEX IF NOT EXISTS idx_admin_config_overrides_tenant ON admin_config_overrides(tenant_id);
CREATE INDEX IF NOT EXISTS idx_admin_config_overrides_category ON admin_config_overrides(category);
CREATE INDEX IF NOT EXISTS idx_admin_config_overrides_key ON admin_config_overrides(config_key);
CREATE INDEX IF NOT EXISTS idx_admin_config_overrides_category_key ON admin_config_overrides(category, config_key);

-- ============================================================================
-- admin_config_audit
-- ============================================================================
CREATE TABLE IF NOT EXISTS admin_config_audit (
    id TEXT PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    admin_user_id UUID NOT NULL REFERENCES users(id),
    admin_email TEXT NOT NULL,
    category TEXT NOT NULL,
    config_key TEXT NOT NULL,
    old_value TEXT,
    new_value TEXT NOT NULL,
    data_type TEXT NOT NULL,
    reason TEXT,
    tenant_id TEXT,
    ip_address TEXT,
    user_agent TEXT
);
CREATE INDEX IF NOT EXISTS idx_admin_config_audit_timestamp ON admin_config_audit(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_admin_config_audit_category ON admin_config_audit(category);
CREATE INDEX IF NOT EXISTS idx_admin_config_audit_key ON admin_config_audit(config_key);
CREATE INDEX IF NOT EXISTS idx_admin_config_audit_admin ON admin_config_audit(admin_user_id);
CREATE INDEX IF NOT EXISTS idx_admin_config_audit_tenant ON admin_config_audit(tenant_id);

-- ============================================================================
-- jwt_usage
-- ============================================================================
CREATE TABLE IF NOT EXISTS jwt_usage (
    id SERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    endpoint TEXT NOT NULL,
    response_time_ms INTEGER,
    status_code INTEGER NOT NULL,
    method TEXT,
    request_size_bytes INTEGER,
    response_size_bytes INTEGER,
    ip_address INET,
    user_agent TEXT
);

-- ============================================================================
-- request_logs
-- ============================================================================
CREATE TABLE IF NOT EXISTS request_logs (
    id TEXT PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    api_key_id TEXT REFERENCES api_keys(id) ON DELETE CASCADE,
    timestamp TIMESTAMPTZ NOT NULL,
    method TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    status_code INTEGER NOT NULL,
    response_time_ms INTEGER,
    error_message TEXT
);
CREATE INDEX IF NOT EXISTS idx_request_logs_timestamp ON request_logs(timestamp);

-- ============================================================================
-- key_versions
-- ============================================================================
CREATE TABLE IF NOT EXISTS key_versions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT,
    version INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    algorithm TEXT NOT NULL DEFAULT 'AES-256-GCM',
    UNIQUE(tenant_id, version)
);
CREATE INDEX IF NOT EXISTS idx_key_versions_tenant ON key_versions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_key_versions_active ON key_versions(tenant_id, is_active);

-- ============================================================================
-- audit_events
-- ============================================================================
CREATE TABLE IF NOT EXISTS audit_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    source TEXT NOT NULL,
    result TEXT NOT NULL,
    tenant_id TEXT,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    ip_address TEXT,
    user_agent TEXT,
    metadata JSONB,
    timestamp TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_events_tenant ON audit_events(tenant_id);
CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp ON audit_events(timestamp);

-- ============================================================================
-- API, A2A, and admin indexes
-- ============================================================================
CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX IF NOT EXISTS idx_api_key_usage_api_key_id ON api_key_usage(api_key_id);
CREATE INDEX IF NOT EXISTS idx_api_key_usage_timestamp ON api_key_usage(timestamp);
CREATE INDEX IF NOT EXISTS idx_a2a_clients_user_id ON a2a_clients(user_id);
CREATE INDEX IF NOT EXISTS idx_a2a_usage_client_id ON a2a_usage(client_id);
CREATE INDEX IF NOT EXISTS idx_a2a_usage_timestamp ON a2a_usage(timestamp);
CREATE INDEX IF NOT EXISTS idx_admin_tokens_service ON admin_tokens(service_name);
CREATE INDEX IF NOT EXISTS idx_admin_tokens_prefix ON admin_tokens(token_prefix);
CREATE INDEX IF NOT EXISTS idx_admin_usage_token_id ON admin_token_usage(admin_token_id);
CREATE INDEX IF NOT EXISTS idx_admin_usage_timestamp ON admin_token_usage(timestamp);
CREATE INDEX IF NOT EXISTS idx_admin_provisioned_token ON admin_provisioned_keys(admin_token_id);
CREATE INDEX IF NOT EXISTS idx_jwt_usage_user_id ON jwt_usage(user_id);
CREATE INDEX IF NOT EXISTS idx_jwt_usage_timestamp ON jwt_usage(timestamp);

-- ============================================================================
-- Seed admin_config_categories
-- ============================================================================
INSERT INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('cat_rate_limiting', 'rate_limiting', 'Rate Limiting', 'API rate limits and throttling settings', 10, 'gauge'),
    ('cat_feature_flags', 'feature_flags', 'Feature Flags', 'Enable or disable system features', 20, 'toggle'),
    ('cat_heart_rate', 'heart_rate_zones', 'Heart Rate Zones', 'Heart rate zone thresholds based on sports science', 30, 'heart'),
    ('cat_training_zones', 'training_zones', 'Training Zones', 'Training zone configurations for different sports', 40, 'activity'),
    ('cat_effort', 'effort_thresholds', 'Effort Thresholds', 'RPE and effort level thresholds', 50, 'flame'),
    ('cat_recommendation', 'recommendation_engine', 'Recommendations', 'Recommendation engine thresholds and limits', 60, 'lightbulb'),
    ('cat_sleep', 'sleep_recovery', 'Sleep & Recovery', 'Sleep and recovery analysis thresholds', 70, 'moon'),
    ('cat_tsb', 'training_stress', 'Training Stress', 'Training stress balance (TSB) thresholds', 80, 'trending'),
    ('cat_weather', 'weather_analysis', 'Weather Analysis', 'Weather impact analysis thresholds', 90, 'cloud'),
    ('cat_nutrition', 'nutrition', 'Nutrition', 'Nutrition and macronutrient recommendations', 100, 'utensils'),
    ('cat_algorithms', 'algorithms', 'Algorithms', 'Algorithm selection for physiological calculations', 110, 'calculator'),
    ('cat_tokio_runtime', 'tokio_runtime', 'Tokio Runtime', 'Async runtime worker threads and stack settings', 120, 'cpu'),
    ('cat_sqlx_config', 'sqlx_config', 'Database Pool', 'SQLx connection pool configuration', 130, 'database'),
    ('cat_llm_pricing', 'llm_pricing', 'LLM Pricing', 'Token pricing per-model for cost estimation', 140, 'dollar-sign'),
    ('cat_usage_quotas', 'usage_quotas', 'Usage Quotas', 'Per-tenant and per-user usage limits', 150, 'bar-chart')
ON CONFLICT (name) DO NOTHING;
