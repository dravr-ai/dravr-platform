-- ABOUTME: Adds nullable theme column to users table (SQLite)
-- ABOUTME: 'light'/'dark' pins a scheme across devices; NULL = follow the system (server-side chart renders read NULL as dark)

ALTER TABLE users ADD COLUMN theme TEXT CHECK (theme IN ('light', 'dark')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
