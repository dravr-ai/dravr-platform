-- ABOUTME: Add respond_mode to coaching_groups — when the AI coach replies in the bound chat
-- ABOUTME: 'all' answers every member message (original behavior); 'mentions' only explicitly-addressed ones

ALTER TABLE coaching_groups ADD COLUMN respond_mode TEXT NOT NULL DEFAULT 'all'; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
