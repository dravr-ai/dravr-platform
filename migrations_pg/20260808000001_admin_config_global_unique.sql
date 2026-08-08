-- ABOUTME: Deduplicates system-wide admin config overrides and makes them uniquely keyed
-- ABOUTME: NULL is distinct from NULL under UNIQUE, so every tenant-less save appended a row
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- `UNIQUE(category, config_key, tenant_id)` cannot constrain system-wide rows:
-- PostgreSQL's default NULLS DISTINCT means a tenant-less override never
-- collided with its predecessor and each save inserted another row. Reads then
-- took an arbitrary row, which in practice was the oldest, so the effective
-- value stayed pinned to the first-ever write.
--
-- Collapse each (category, config_key) group down to its most recent row: that
-- is the value the operator last intended.
DELETE FROM admin_config_overrides
WHERE tenant_id IS NULL
  AND id NOT IN (
      SELECT id
      FROM (
          SELECT id,
                 ROW_NUMBER() OVER (
                     PARTITION BY category, config_key
                     ORDER BY updated_at DESC, id DESC
                 ) AS rn
          FROM admin_config_overrides
          WHERE tenant_id IS NULL
      ) ranked
      WHERE rn = 1
  );

-- Partial unique index over the system-wide rows only. This is the arbiter that
-- `ON CONFLICT (category, config_key) WHERE tenant_id IS NULL` infers, which is
-- what turns the tenant-less INSERT into a genuine upsert. Tenant-scoped rows
-- are outside this index and keep using the table's UNIQUE constraint.
CREATE UNIQUE INDEX IF NOT EXISTS idx_admin_config_overrides_global_unique
    ON admin_config_overrides(category, config_key)
    WHERE tenant_id IS NULL;
