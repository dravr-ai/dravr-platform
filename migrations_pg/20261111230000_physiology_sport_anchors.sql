-- ABOUTME: Per-sport physiology anchors with range, kind, origin, as-of, method and confidence, and the day the athlete stated each threshold (Postgres)
-- ABOUTME: Additive only: one child table of user_physiological_profiles and four as-of columns; nothing is renamed or dropped (carnet#1496)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The profile row holds one value per threshold for every sport, but a run
-- critical speed never sets bike power and a bike max heart rate never sets
-- run heart-rate zones (dravr-cageux rule I9). Workout families resolve
-- their targets on each sport's own anchors, so each anchor is stored per
-- sport family, never as a bare point: the value, the range it stands for,
-- whether it was measured, who set it, on which day, by which test (for MAP,
-- the definition it follows) and how far to trust it.
--   sport        'ride' | 'run' | 'swim' — the cageux SportFamily
--   anchor       'ftp' | 'critical_power' | 'map' | 'critical_speed' |
--                'vvo2max' | 'threshold_speed' | 'max_heart_rate' |
--                'threshold_heart_rate' — a cageux SportAnchors field
--   value        watts, metres per second or beats per minute
--   low, high    the range the value stands for, when stated
--   kind         'measured' | 'estimated'
--   origin       'athlete', or the provider, app or test that produced it
--   as_of        the day it was measured or stated
--   method       how it was measured; for MAP, its definition
--   confidence   'low' | 'medium' | 'high'
-- The vocabularies are enforced where the row is written, as on the parent
-- table, so a new anchor needs no table rebuild. A row lives only beside its
-- profile row, whose transport policy gates reading it.
CREATE TABLE IF NOT EXISTS user_physiology_sport_anchors (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    sport TEXT NOT NULL,
    anchor TEXT NOT NULL,
    value DOUBLE PRECISION NOT NULL CHECK (value > 0),
    low DOUBLE PRECISION,
    high DOUBLE PRECISION,
    kind TEXT NOT NULL,
    origin TEXT NOT NULL,
    as_of DATE NOT NULL,
    method TEXT,
    confidence TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_id, sport, anchor),
    FOREIGN KEY (tenant_id, user_id)
        REFERENCES user_physiological_profiles(tenant_id, user_id) ON DELETE CASCADE,
    CHECK (low IS NULL OR low <= value),
    CHECK (high IS NULL OR high >= value)
);

CREATE INDEX IF NOT EXISTS idx_user_physio_sport_anchors_tenant_user
    ON user_physiology_sport_anchors(tenant_id, user_id);

-- The day the athlete stated each threshold. The *_kind, *_origin and
-- *_as_of columns describe only a provider's import, and NULL there is what
-- marks the athlete's own value, which an import never replaces; so the day
-- an athlete-set value was stated lives in its own column, filled exactly
-- when the import provenance is empty. No backfill: an older value has no
-- known day.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS ftp_watts_stated_on DATE;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_hr_stated_on DATE;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS max_hr_stated_on DATE;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_pace_sec_per_km_stated_on DATE;
