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
ALTER TABLE user_physiological_profiles ADD COLUMN resting_hr_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN resting_hr_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- VO2max, ml/kg/min.
ALTER TABLE user_physiological_profiles ADD COLUMN vo2_max_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN vo2_max_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Weight, kg.
ALTER TABLE user_physiological_profiles ADD COLUMN weight_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN weight_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A revision written before these columns rewrites the anchors without
-- naming them, so its upsert leaves a promoted origin beside a number it just
-- took from the athlete, and the next promotion would overwrite that number.
-- A value that changes while its origin and as-of stay as they were is
-- therefore such a writer, and the anchor becomes the athlete's own (NULL).
-- This revision's writers always move the stamp with the value: set_physiology
-- stamps the athlete and the day, and a promotion never changes a value as of
-- the day it already stands for.

CREATE TRIGGER IF NOT EXISTS trg_physiology_resting_hr_unstamped_write
AFTER UPDATE OF resting_hr ON user_physiological_profiles
FOR EACH ROW
WHEN OLD.resting_hr_origin = 'promoted'
 AND NEW.resting_hr IS NOT OLD.resting_hr
 AND NEW.resting_hr_origin IS OLD.resting_hr_origin
 AND NEW.resting_hr_as_of IS OLD.resting_hr_as_of
BEGIN
    UPDATE user_physiological_profiles
    SET resting_hr_origin = NULL, resting_hr_as_of = NULL
    WHERE tenant_id = NEW.tenant_id AND user_id = NEW.user_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_physiology_vo2_max_unstamped_write
AFTER UPDATE OF vo2_max ON user_physiological_profiles
FOR EACH ROW
WHEN OLD.vo2_max_origin = 'promoted'
 AND NEW.vo2_max IS NOT OLD.vo2_max
 AND NEW.vo2_max_origin IS OLD.vo2_max_origin
 AND NEW.vo2_max_as_of IS OLD.vo2_max_as_of
BEGIN
    UPDATE user_physiological_profiles
    SET vo2_max_origin = NULL, vo2_max_as_of = NULL
    WHERE tenant_id = NEW.tenant_id AND user_id = NEW.user_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_physiology_weight_unstamped_write
AFTER UPDATE OF weight ON user_physiological_profiles
FOR EACH ROW
WHEN OLD.weight_origin = 'promoted'
 AND NEW.weight IS NOT OLD.weight
 AND NEW.weight_origin IS OLD.weight_origin
 AND NEW.weight_as_of IS OLD.weight_as_of
BEGIN
    UPDATE user_physiological_profiles
    SET weight_origin = NULL, weight_as_of = NULL
    WHERE tenant_id = NEW.tenant_id AND user_id = NEW.user_id;
END;
