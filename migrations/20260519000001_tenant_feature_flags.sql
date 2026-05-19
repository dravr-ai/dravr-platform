-- ABOUTME: Runtime-toggleable feature flags (SQLite) — tenant defaults + per-user overrides
-- ABOUTME: Resolution: per-user override > tenant default > compile-time default

-- Tenant-wide defaults. One row per (tenant, feature_key). Absence = use compile default.
CREATE TABLE IF NOT EXISTS tenant_feature_defaults (
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    feature_key TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at TEXT NOT NULL,
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (tenant_id, feature_key)
);

CREATE INDEX IF NOT EXISTS idx_tenant_feature_defaults_tenant
    ON tenant_feature_defaults(tenant_id);

-- Per-user overrides. One row per (user, feature_key). Absence = fall through to tenant default.
CREATE TABLE IF NOT EXISTS user_feature_overrides (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    feature_key TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at TEXT NOT NULL,
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (user_id, feature_key)
);

CREATE INDEX IF NOT EXISTS idx_user_feature_overrides_user
    ON user_feature_overrides(user_id);
