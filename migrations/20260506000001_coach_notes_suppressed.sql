-- Coach Notes Audit enforcement: admin-flagged notes are excluded from
-- memory recall while remaining visible in the audit panel.
--
-- Adds a `suppressed` boolean (default false) plus an audit timestamp
-- and the admin service name that flipped it. The chat pipeline's
-- memory-recall stage filters out rows where `suppressed=TRUE` so the
-- coach never re-injects an admin-rejected note into a user prompt.

ALTER TABLE coach_notes
    ADD COLUMN suppressed INTEGER NOT NULL DEFAULT 0;

ALTER TABLE coach_notes
    ADD COLUMN suppressed_at TEXT;

ALTER TABLE coach_notes
    ADD COLUMN suppressed_by TEXT;

-- Index on the active subset only — most notes are not suppressed, so
-- a partial index on the unsuppressed plane keeps recall queries fast
-- without indexing the audit-tail.
CREATE INDEX IF NOT EXISTS idx_coach_notes_active_suppressed
    ON coach_notes(tenant_id, user_id, coach_id, created_at DESC)
    WHERE suppressed = 0;
