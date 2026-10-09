-- ABOUTME: user_unit_preferences — the unit system each athlete reads distances in, and what decides it (Postgres)
-- ABOUTME: An explicit choice, the connected provider's own setting, and the device locale, resolved in that order (carnet#835)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per user, written the first time any of the three inputs is
-- known; no row reads as automatic with nothing known. Scoped by user_id
-- alone: the units are the person's, whichever tenant they are signed in to.
--   preference      'automatic' | 'metric' | 'imperial' — the Settings choice
--   provider_units  'metric' | 'imperial' — the provider's own setting, when read
--   provider        the provider that setting was read from (e.g. 'strava')
--   device_locale   the BCP 47 tag the athlete's device last reported (e.g. 'en-US')
CREATE TABLE IF NOT EXISTS user_unit_preferences (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    preference TEXT NOT NULL DEFAULT 'automatic',
    provider_units TEXT,
    provider TEXT,
    device_locale TEXT,
    updated_at TIMESTAMPTZ NOT NULL
);
