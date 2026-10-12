-- ABOUTME: athlete_gear and activity_gear — the shoes an athlete declares and the pair each activity is attributed to (Postgres)
-- ABOUTME: A pair's distance is its declared distance plus its attributed activities'; both tables are scoped by user_id (carnet#711)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The gear ledger is the person's, whichever tenant they are signed in to, so
-- both tables are keyed by user_id alone, and both go with the account.
--
-- athlete_gear: one row per pair the athlete declared. A retired pair keeps its
-- row, its distance and its history and is offered for no new activity; a pair
-- is deleted only while no activity is attributed to it. At most one pair per
-- sport is pinned as that sport's default.
CREATE TABLE IF NOT EXISTS athlete_gear (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('shoe')),
    name TEXT NOT NULL,
    sport TEXT NOT NULL,
    initial_distance_meters DOUBLE PRECISION NOT NULL DEFAULT 0,
    warn_distance_meters DOUBLE PRECISION NOT NULL,
    pinned BOOLEAN NOT NULL DEFAULT FALSE,
    added_at TIMESTAMPTZ NOT NULL,
    retired_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_athlete_gear_user ON athlete_gear(user_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_athlete_gear_pinned_sport
    ON athlete_gear(user_id, sport) WHERE pinned;

-- activity_gear: the pair one activity copy is attributed to, keyed like the
-- activity's other per-activity rows by the provider key the cached activity
-- is stored under, so the provider-disconnect purge reaches it. gear_id NULL
-- records that the athlete declared the activity was run in none of their
-- pairs, so no default is given to it. The distance is kept on the row as it
-- stood when attributed: a pair keeps its distance after the activity cache
-- lets the activity go.
CREATE TABLE IF NOT EXISTS activity_gear (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    gear_id UUID REFERENCES athlete_gear(id) ON DELETE CASCADE,
    source TEXT NOT NULL CHECK (source IN ('declared', 'default')),
    start_date TIMESTAMPTZ NOT NULL,
    distance_meters DOUBLE PRECISION NOT NULL DEFAULT 0,
    attributed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, provider, activity_id)
);

CREATE INDEX IF NOT EXISTS idx_activity_gear_user_gear ON activity_gear(user_id, gear_id);

CREATE INDEX IF NOT EXISTS idx_activity_gear_user_start ON activity_gear(user_id, start_date);
