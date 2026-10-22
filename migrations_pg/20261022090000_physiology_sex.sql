-- ABOUTME: The athlete's physiological sex, for the published equations that take one (Keytel energy, LT1 pace ratios)
-- ABOUTME: Additive only: one nullable column, male | female; NULL means not stated and not reported (carnet#1113)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Set by the athlete with set_physiology, or filled once from a provider that
-- reports it (Strava's and intervals.icu's athlete `sex`) when the profile has
-- none. Never inferred. No backfill: every existing row reads as not stated.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS sex TEXT CHECK (sex IS NULL OR sex IN ('male', 'female'));
