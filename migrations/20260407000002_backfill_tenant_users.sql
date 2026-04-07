-- Backfill tenant_users junction table from users.tenant_id
--
-- Root cause: update_tenant_id() only set users.tenant_id but never inserted
-- into tenant_users. All user listing queries INNER JOIN on tenant_users,
-- so users without entries were invisible to admins.

INSERT OR IGNORE INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at, is_active)
SELECT lower(hex(randomblob(16))), u.tenant_id, u.id, 'member', u.created_at, u.created_at, 1
FROM users u
WHERE u.tenant_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM tenant_users tu
    WHERE tu.tenant_id = u.tenant_id AND tu.user_id = u.id
  );
