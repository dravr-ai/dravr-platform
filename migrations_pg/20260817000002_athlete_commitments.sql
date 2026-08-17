-- ABOUTME: Athlete commitments — countable, time-boxed promises swept against real activity data (`PostgreSQL`)
-- ABOUTME: Mirrors the SQLite DDL column for column; only the integer width differs (INTEGER -> BIGINT)

-- See migrations/20260817000002_athlete_commitments.sql for the full rationale.
-- Column NAMES are identical across backends (schema_parity_test asserts this);
-- only the integer width differs. Epoch-second timestamps and TEXT tenant/user
-- keys are deliberate on both backends — the newest tables sidestep both the
-- TIMESTAMPTZ-vs-RFC3339 divergence and the VARCHAR-tenant-id bind trap.
CREATE TABLE IF NOT EXISTS athlete_commitments (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    coach_id            TEXT NOT NULL DEFAULT '',
    conversation_id     TEXT NOT NULL DEFAULT '',
    statement           TEXT NOT NULL,
    sport               TEXT NOT NULL DEFAULT '',
    target_sessions     BIGINT NOT NULL,
    window_start        BIGINT NOT NULL,
    window_end          BIGINT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'open',
    outcome             TEXT,
    completed_sessions  BIGINT,
    swept_at            BIGINT,
    reported_at         BIGINT,
    created_at          BIGINT NOT NULL,
    updated_at          BIGINT NOT NULL
);

-- The sweep's hot scan: open commitments whose window has closed, oldest first.
CREATE INDEX IF NOT EXISTS idx_athlete_commitments_due
    ON athlete_commitments(status, window_end);
-- The reporter's scan: labeled commitments still waiting for a delivery route.
CREATE INDEX IF NOT EXISTS idx_athlete_commitments_report
    ON athlete_commitments(status, swept_at);
-- Prompt injection (a user's open commitments) and the per-user cadence cap.
CREATE INDEX IF NOT EXISTS idx_athlete_commitments_user
    ON athlete_commitments(tenant_id, user_id, status);
