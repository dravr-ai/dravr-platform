-- ABOUTME: Records the TrainingPeaks exposure notice an account accepted, and when
-- ABOUTME: NULL until accepted; a newer notice version than the one stored asks again

ALTER TABLE users ADD COLUMN trainingpeaks_terms_version TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE users ADD COLUMN trainingpeaks_terms_consented_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
