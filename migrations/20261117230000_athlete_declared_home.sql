-- ABOUTME: athlete_declared_home — what the athlete said about home, which replaces the inferred home exclusion
-- ABOUTME: athlete_place_runs.home_declared_at_seen — the declaration the last successful places recompute clustered with
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The athlete says where home is (carnet#1593): `at` a seed, `elsewhere`
-- (home is not where they train: nothing is excluded), or `inferred`, the
-- clustering's own guess, which is also what no row means. "Forget" writes
-- `inferred` rather than deleting the row, because declared_at is what makes
-- the athlete due a recompute.
--
-- The seed is a privacy-trimmed start or a place centroid, the fidelity of
-- athlete_places.centroid_*, never a raw recorded start. It is never served by
-- any tool, route or prompt, and never sent to another host: one service, the
-- places recompute, reads this table (check-activity-writers.sh rules 4 and
-- 5), and the athlete's own export carries the state and date without the
-- seed. Stored unencrypted, at parity with the centroids.
--
-- A recompute records the declared_at it clustered with in its runs row; a
-- newer declaration, or one no successful recompute saw, keeps the athlete due.
-- A failed recompute leaves home_declared_at_seen alone, so they stay due.
-- Ids are TEXT, like athlete_places; timestamps are RFC 3339 text.

CREATE TABLE IF NOT EXISTS athlete_declared_home (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK (state IN ('inferred', 'at', 'elsewhere')),
    seed_lat REAL,
    seed_lon REAL,
    declared_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id),
    CHECK ((state = 'at' AND seed_lat IS NOT NULL AND seed_lon IS NOT NULL)
        OR (state <> 'at' AND seed_lat IS NULL AND seed_lon IS NULL))
);

ALTER TABLE athlete_place_runs ADD COLUMN home_declared_at_seen TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The admin catalogue text, only where nobody edited the seeded one.
UPDATE tool_catalog
SET description = 'Name or rename a place where the athlete trains, or say where home is'
WHERE tool_name = 'name_place'
  AND description = 'Name or rename a place where the athlete trains';
