-- ABOUTME: Adds optional tenant_id to admin_tokens for per-tenant admin scoping
-- ABOUTME: Super-admin tokens leave tenant_id NULL; scoped tokens bind to one tenant

ALTER TABLE admin_tokens ADD COLUMN tenant_id TEXT;

CREATE INDEX IF NOT EXISTS idx_admin_tokens_tenant ON admin_tokens(tenant_id);
