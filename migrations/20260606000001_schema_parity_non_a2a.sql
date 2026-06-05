-- ABOUTME: Reconciles SQLite column names with the canonical PostgreSQL schema for
-- ABOUTME: the non-a2a shared tables, so the schema-parity guard holds across backends.

-- api_keys: PG carries updated_at; add the matching nullable column to SQLite.
ALTER TABLE api_keys ADD COLUMN updated_at TEXT;

-- tenant_oauth_credentials: PG carries configured_by; add the matching column.
ALTER TABLE tenant_oauth_credentials ADD COLUMN configured_by TEXT REFERENCES users(id);

-- admin_token_usage: canonical PG uses `method`; the SQLite-only `error_message`
-- column is vestigial (never written or read by admin code).
ALTER TABLE admin_token_usage ADD COLUMN method TEXT;
ALTER TABLE admin_token_usage DROP COLUMN error_message;

-- tenant_users: drop the three SQLite-only columns absent from canonical PG.
-- permissions/invited_by have no code consumers; is_active was only ever
-- written hard-coded and never read.
ALTER TABLE tenant_users DROP COLUMN permissions;
ALTER TABLE tenant_users DROP COLUMN invited_by;
ALTER TABLE tenant_users DROP COLUMN is_active;

-- user_llm_credentials_audit: drop the dead SQLite-only changed_by column
-- (no code references the column or the table).
ALTER TABLE user_llm_credentials_audit DROP COLUMN changed_by;

-- oauth_apps: canonical PG names the secret column `client_secret`.
ALTER TABLE oauth_apps RENAME COLUMN client_secret_hash TO client_secret;

-- api_key_usage: canonical PG uses `endpoint` + `method`; SQLite stored
-- `tool_name` and a vestigial `error_message`.
ALTER TABLE api_key_usage RENAME COLUMN tool_name TO endpoint;
ALTER TABLE api_key_usage ADD COLUMN method TEXT;
ALTER TABLE api_key_usage DROP COLUMN error_message;

-- tenants: canonical PG names the plan column `subscription_tier` and derives the
-- owner from the tenant_users join rather than an owner_user_id column. Drop the
-- owner index first — it depends on the column being removed.
ALTER TABLE tenants RENAME COLUMN plan TO subscription_tier;
DROP INDEX IF EXISTS idx_tenants_owner;
ALTER TABLE tenants DROP COLUMN owner_user_id;
