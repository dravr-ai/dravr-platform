-- ABOUTME: Runtime-toggleable feature flags (PostgreSQL) — tenant defaults + per-user overrides
-- ABOUTME: Resolution: per-user override > tenant default > compile-time default

CREATE TABLE IF NOT EXISTS tenant_feature_defaults (
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    feature_key TEXT NOT NULL,
    enabled BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (tenant_id, feature_key)
);

CREATE INDEX IF NOT EXISTS idx_tenant_feature_defaults_tenant
    ON tenant_feature_defaults(tenant_id);

CREATE TABLE IF NOT EXISTS user_feature_overrides (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    feature_key TEXT NOT NULL,
    enabled BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (user_id, feature_key)
);

CREATE INDEX IF NOT EXISTS idx_user_feature_overrides_user
    ON user_feature_overrides(user_id);
