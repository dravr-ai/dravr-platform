-- ABOUTME: Adds optional tenant_id to admin_tokens for per-tenant admin scoping
-- ABOUTME: Mirrors the SQLite migration 20260419000001 — PG branch was missing this column
--
-- Production incident on 2026-05-07: GET /api/admin/tokens returned 500 with
-- `column "tenant_id" does not exist` because the PG `admin_tokens` table never
-- got the column the SQLite branch added on 2026-04-19. Both backends now match.

ALTER TABLE admin_tokens ADD COLUMN IF NOT EXISTS tenant_id TEXT;

CREATE INDEX IF NOT EXISTS idx_admin_tokens_tenant ON admin_tokens(tenant_id);
