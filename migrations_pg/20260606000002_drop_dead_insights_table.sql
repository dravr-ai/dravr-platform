-- ABOUTME: Drops the dead generic `insights` analytics table on PostgreSQL.
-- ABOUTME: The canonical PG tree never recreated it; this clears the live-DB orphan.

-- The March-2026 PG consolidation already omitted `insights` from the tree, but
-- long-lived databases (dev) still carry the physical table (empty, 0 rows, no PG
-- code path touches it). DROP IF EXISTS removes that orphan and aligns the live DB
-- with the canonical tree; no-op on a fresh DB that never had it.
DROP TABLE IF EXISTS insights;
