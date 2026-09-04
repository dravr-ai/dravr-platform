-- Align the SQLite tenant_users.role CHECK with TenantRole and with PostgreSQL.
--
-- TenantRole is Owner | Admin | Billing | Member (pierre-auth/src/tenant/schema.rs),
-- and from_db_string reads "viewer" as an alias for Member. PostgreSQL's constraint
-- already matches the enum: ('owner','admin','billing','member'). SQLite's did not —
-- it accepted the alias and rejected 'billing', so a Billing assignment succeeded in
-- production and constraint-failed in dev/CI, while a 'viewer' write did the reverse.
-- Since role reaches TenantContext::is_admin through from_db_string, the divergence is
-- authorization-relevant rather than cosmetic.
--
-- Existing 'viewer' rows are folded to 'member' first, which is the value they already
-- resolved to in code, so no membership changes meaning.
--
-- SQLite cannot alter a CHECK constraint, hence the table rebuild.

UPDATE tenant_users SET role = 'member' WHERE role = 'viewer';

CREATE TABLE tenant_users_rebuild (  -- idempotency-ok: table rebuild, dropped at the end of this migration
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'billing', 'member')),
    invited_at TEXT NOT NULL,
    joined_at TEXT,
    selected_coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL,
    UNIQUE(tenant_id, user_id)
);

INSERT INTO tenant_users_rebuild (id, tenant_id, user_id, role, invited_at, joined_at, selected_coach_id)
SELECT id, tenant_id, user_id, role, invited_at, joined_at, selected_coach_id FROM tenant_users;

DROP TABLE tenant_users;  -- idempotency-ok: table rebuild, the replacement is renamed in below

ALTER TABLE tenant_users_rebuild RENAME TO tenant_users;

CREATE INDEX IF NOT EXISTS idx_tenant_users_tenant ON tenant_users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tenant_users_user ON tenant_users(user_id);
CREATE INDEX IF NOT EXISTS idx_tenant_users_selected_coach ON tenant_users(selected_coach_id);
