-- ABOUTME: Adds the per-user scope to admin config overrides (global -> tenant -> user)
-- ABOUTME: Re-cuts the global partial index, which would otherwise collide user rows with it

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Overrides gain a third scope. A row now belongs to exactly one of:
--   global  -> tenant_id IS NULL AND user_id IS NULL
--   tenant  -> tenant_id IS NOT NULL
--   user    -> user_id IS NOT NULL
-- and a lookup walks user -> tenant -> global, taking the first that answers.
ALTER TABLE admin_config_overrides
    ADD COLUMN user_id TEXT REFERENCES users(id) ON DELETE CASCADE;  -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS form

-- The audit log records which user a per-user change targeted. Without this a
-- per-user edit is indistinguishable from a system-wide one after the fact.
-- SET NULL rather than CASCADE: deleting a user must not erase the record that
-- an operator once changed configuration for them.
ALTER TABLE admin_config_audit
    ADD COLUMN user_id TEXT REFERENCES users(id) ON DELETE SET NULL;  -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS form

-- The existing global index is `WHERE tenant_id IS NULL`, and a per-user row
-- also has a NULL tenant_id — so without re-cutting it, the first per-user
-- override for a key would be rejected as a duplicate of the global row.
DROP INDEX IF EXISTS idx_admin_config_overrides_global_unique;

CREATE UNIQUE INDEX IF NOT EXISTS idx_admin_config_overrides_global_unique
    ON admin_config_overrides(category, config_key)
    WHERE tenant_id IS NULL AND user_id IS NULL;

-- Per-user rows get their own arbiter, which
-- `ON CONFLICT (category, config_key, user_id) WHERE user_id IS NOT NULL`
-- infers to make the per-user INSERT a genuine upsert. The table's
-- UNIQUE(category, config_key, tenant_id) still governs tenant rows; user rows
-- carry a NULL tenant_id and so never collide under it.
CREATE UNIQUE INDEX IF NOT EXISTS idx_admin_config_overrides_user_unique
    ON admin_config_overrides(category, config_key, user_id)
    WHERE user_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_admin_config_overrides_user
    ON admin_config_overrides(user_id);

CREATE INDEX IF NOT EXISTS idx_admin_config_audit_user
    ON admin_config_audit(user_id);
