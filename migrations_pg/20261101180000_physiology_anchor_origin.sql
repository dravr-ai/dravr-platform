-- ABOUTME: Who set the profile's resting heart rate, VO2max and weight, and as of which day (DECISION 5, carnet#1307)
-- ABOUTME: Additive only: origin (athlete | imported | estimated | promoted) and as-of date per anchor; NULL is the athlete's own

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The three anchors stored training load and the heart-rate zones are scored
-- against. A weekly promotion fills one from the athlete's own measurements
-- (resting heart rate: the 7-day median of morning readings; VO2max and
-- weight: the latest), but only an anchor that is empty or that nobody
-- vouches for: an estimate, or an earlier promotion. The origin is what tells
-- a promoted number from one the athlete stated, so the promotion can refresh
-- its own value and never overwrite theirs. NULL beside a value is the
-- athlete's own: every value stored before this migration was written by
-- set_physiology or moved from the athlete's own configuration. No backfill.

-- Resting heart rate, bpm.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS resting_hr_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS resting_hr_as_of DATE;

-- VO2max, ml/kg/min.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS vo2_max_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS vo2_max_as_of DATE;

-- Weight, kg.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS weight_origin TEXT;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS weight_as_of DATE;

-- A revision written before these columns rewrites the anchors without
-- naming them, so its upsert leaves a promoted origin beside a number it just
-- took from the athlete, and the next promotion would overwrite that number.
-- A value that changes while its origin and as-of stay as they were is
-- therefore such a writer, and the anchor becomes the athlete's own (NULL).
-- This revision's writers always move the stamp with the value: set_physiology
-- stamps the athlete and the day, and a promotion never changes a value as of
-- the day it already stands for.
CREATE OR REPLACE FUNCTION release_unstamped_physiology_anchor()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.resting_hr_origin = 'promoted'
       AND NEW.resting_hr IS DISTINCT FROM OLD.resting_hr
       AND NEW.resting_hr_origin IS NOT DISTINCT FROM OLD.resting_hr_origin
       AND NEW.resting_hr_as_of IS NOT DISTINCT FROM OLD.resting_hr_as_of THEN
        NEW.resting_hr_origin := NULL;
        NEW.resting_hr_as_of := NULL;
    END IF;
    IF OLD.vo2_max_origin = 'promoted'
       AND NEW.vo2_max IS DISTINCT FROM OLD.vo2_max
       AND NEW.vo2_max_origin IS NOT DISTINCT FROM OLD.vo2_max_origin
       AND NEW.vo2_max_as_of IS NOT DISTINCT FROM OLD.vo2_max_as_of THEN
        NEW.vo2_max_origin := NULL;
        NEW.vo2_max_as_of := NULL;
    END IF;
    IF OLD.weight_origin = 'promoted'
       AND NEW.weight IS DISTINCT FROM OLD.weight
       AND NEW.weight_origin IS NOT DISTINCT FROM OLD.weight_origin
       AND NEW.weight_as_of IS NOT DISTINCT FROM OLD.weight_as_of THEN
        NEW.weight_origin := NULL;
        NEW.weight_as_of := NULL;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_physiology_anchor_unstamped_write ON user_physiological_profiles;
CREATE TRIGGER trg_physiology_anchor_unstamped_write
    BEFORE UPDATE ON user_physiological_profiles
    FOR EACH ROW EXECUTE FUNCTION release_unstamped_physiology_anchor();
