-- ABOUTME: Converts coaches.tenant_id from unconstrained TEXT to UUID with a FK to tenants(id) ON DELETE CASCADE.
-- ABOUTME: Rows whose tenant_id is not a UUID or names no existing tenant are quarantined into coaches_orphaned.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The SQLite schema has carried FOREIGN KEY(tenant_id) REFERENCES tenants(id)
-- ON DELETE CASCADE since the coaches table was created; PostgreSQL shipped
-- the column as unconstrained TEXT. Deployed databases hold real data, so a
-- row that cannot satisfy the constraint is quarantined — moved wholesale
-- into coaches_orphaned (same columns, no constraints) — never deleted.
--
-- Deleting the quarantined originals from coaches cascades through the
-- existing child FKs: coach_assignments / coach_versions / coach_relations /
-- store_listings rows of an orphaned coach are removed (ON DELETE CASCADE),
-- and a surviving fork of an orphaned origin keeps running with
-- forked_from = NULL (ON DELETE SET NULL).
CREATE TABLE IF NOT EXISTS coaches_orphaned (LIKE coaches INCLUDING DEFAULTS);

INSERT INTO coaches_orphaned
SELECT c.*
FROM coaches c
WHERE c.tenant_id !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
   OR NOT EXISTS (SELECT 1 FROM tenants t WHERE t.id::text = lower(c.tenant_id));

DELETE FROM coaches c
USING coaches_orphaned o
WHERE c.id = o.id;

ALTER TABLE coaches
    ALTER COLUMN tenant_id TYPE UUID USING tenant_id::uuid;

ALTER TABLE coaches
    ADD CONSTRAINT coaches_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
