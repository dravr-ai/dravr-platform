-- ABOUTME: Restores the shared_insights activity-link columns the March-2026 PG
-- ABOUTME: consolidation dropped; postgres/social_features.rs already reads/writes them.

-- The consolidated PG schema omitted these two columns, but the Postgres backend
-- code (backends/postgres/social_features.rs) INSERTs and SELECTs both — so every
-- shared-insight create/read errored on Postgres. SQLite has carried them since
-- 20250125000001_social_activity_link. Add them to PG to restore parity and fix
-- the broken path. coach_generated mirrors SQLite's INTEGER DEFAULT 0 as a BOOLEAN.
ALTER TABLE shared_insights ADD COLUMN source_activity_id TEXT;
ALTER TABLE shared_insights ADD COLUMN coach_generated BOOLEAN NOT NULL DEFAULT FALSE;
