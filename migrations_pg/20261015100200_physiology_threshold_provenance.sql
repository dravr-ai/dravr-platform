-- ABOUTME: Where the profile's FTP, threshold heart rate, max heart rate and threshold pace came from, for the ones a provider supplied
-- ABOUTME: Additive only: kind (measured | estimated), origin and as-of date per threshold; NULL means the athlete's own value (carnet#1021)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An import from a provider (intervals.icu's sport settings) fills one of
-- these thresholds only when the profile has none, or when the value is that
-- provider's own earlier one; a value the athlete set is never replaced. The
-- provenance columns are what tell the two apart: NULL is the athlete's own
-- value — every row written before this migration, and every value
-- set_physiology writes — while an imported value carries kind 'estimated',
-- the provider as its origin and the day the provider's setting last
-- changed. No backfill: every stored threshold is the athlete's own.

-- Functional threshold power, watts.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS ftp_watts_kind TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS ftp_watts_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS ftp_watts_as_of DATE;

-- Lactate threshold heart rate, bpm.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_hr_kind TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_hr_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_hr_as_of DATE;

-- Maximum heart rate, bpm.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS max_hr_kind TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS max_hr_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS max_hr_as_of DATE;

-- Threshold pace, seconds per kilometre.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_pace_sec_per_km_kind TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_pace_sec_per_km_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_pace_sec_per_km_as_of DATE;
