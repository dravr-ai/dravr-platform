-- ABOUTME: Drops the unused key_versions table and its indexes (PostgreSQL)
-- ABOUTME: Table was written only by the never-instantiated KeyRotationManager; empty in practice

DROP INDEX IF EXISTS idx_key_versions_active;
DROP INDEX IF EXISTS idx_key_versions_tenant;
DROP TABLE IF EXISTS key_versions;
