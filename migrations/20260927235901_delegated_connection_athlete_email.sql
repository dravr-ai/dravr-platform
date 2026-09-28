-- ABOUTME: provider_athlete_email on delegated_connections — the email the coach's roster lists for the athlete (SQLite)
-- ABOUTME: A link binds only when it is the member's verified Dravr email; a row without one cannot be confirmed

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A coach could link any athlete on their TrainingPeaks roster to any member
-- of a group they coach, and that member's confirm let them and their agent
-- read a third party's workouts (carnet#602). A proposal now stores the
-- roster athlete's email and is made only when it is the member's verified
-- email; the member's confirm checks it again. Proposals stored before this
-- column carry none, so they cannot be confirmed: the coach proposes again.
ALTER TABLE delegated_connections ADD COLUMN provider_athlete_email TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
