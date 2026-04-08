-- Backfill tenant_users junction table from users.tenant_id
--
-- Root cause: update_tenant_id() only set users.tenant_id but never inserted
-- into tenant_users. All user listing queries INNER JOIN on tenant_users,
-- so users without entries were invisible to admins.
--
-- Note: users.tenant_id is TEXT, tenant_users.tenant_id is UUID

INSERT INTO tenant_users (tenant_id, user_id, role, invited_at, joined_at)
SELECT u.tenant_id::uuid, u.id, 'member', u.created_at, u.created_at
FROM users u
WHERE u.tenant_id IS NOT NULL
  AND u.tenant_id != ''
  AND NOT EXISTS (
    SELECT 1 FROM tenant_users tu
    WHERE tu.tenant_id::text = u.tenant_id AND tu.user_id = u.id
  );
