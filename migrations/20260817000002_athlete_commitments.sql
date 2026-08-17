-- ABOUTME: Athlete commitments — countable, time-boxed promises swept against real activity data (SQLite)
-- ABOUTME: Separate lifecycle columns for "counted" and "reported" so the sweep and the delivery never race

-- A COMMITMENT is the athlete's own promise ("three easy runs this week"),
-- recorded only after the coach confirmed it explicitly through a tool call.
-- Once `window_end` passes, an hourly sweep counts matching activities in
-- [window_start, window_end], records met / partial / missed, and a second pass
-- delivers the verdict back through the channel the promise was made on.
--
-- `status` and `reported_at` are deliberately distinct transitions. The sibling
-- coach_followups table overloads one `delivered` state to mean both "rendered
-- into a prompt" and "pushed to the athlete", which lets a chat turn and the
-- scheduler consume the same row out from under each other. Here the sweep
-- moves open -> labeled and only the reporter moves labeled -> reported.
--
-- No CHECK constraint on status/outcome, matching the sibling pending_advice
-- table: SQLite cannot widen a CHECK in place, so the constraint would make
-- every future lifecycle state a 12-step table rebuild. Both columns are parsed
-- strictly in Rust (CommitmentStatus::parse / CommitmentOutcome::parse return
-- None on anything unknown) and the repository surfaces that as an error.
--
-- coach_id / conversation_id / sport are NOT NULL DEFAULT '' rather
-- than nullable — the same reasoning as coaching_playbooks.coach_slug. The
-- duplicate guard on insert compares them, and NULLs compare distinct, which
-- would let a re-affirmed promise accumulate a second open row. The repository
-- maps Option<String> <-> '' at the boundary for all three.
--
-- tenant_id / user_id are TEXT on both backends (query keys, not FKs) to dodge
-- the VARCHAR-tenant-id-decoded-as-UUID bind trap. Every read except the two
-- documented cross-tenant sweeper scans is tenant-scoped (WHERE tenant_id = ?).
--
-- All timestamps are Unix epoch SECONDS (integers) so reads/writes are identical
-- across SQLite and PostgreSQL with no datetime-format ambiguity.
CREATE TABLE IF NOT EXISTS athlete_commitments (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    coach_id            TEXT NOT NULL DEFAULT '',
    conversation_id     TEXT NOT NULL DEFAULT '',
    statement           TEXT NOT NULL,
    sport               TEXT NOT NULL DEFAULT '',
    target_sessions     INTEGER NOT NULL,
    window_start        INTEGER NOT NULL,
    window_end          INTEGER NOT NULL,
    status              TEXT NOT NULL DEFAULT 'open',
    outcome             TEXT,
    completed_sessions  INTEGER,
    swept_at            INTEGER,
    reported_at         INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
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
