-- ABOUTME: The physiological profile becomes the one home of the thresholds training load scores against
-- ABOUTME: Adds a measured threshold_hr, reads lactate_threshold_percentage as a fraction of VO2max, moves overrides in

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- 1. A measured lactate threshold heart rate. Heart-rate training stress is
--    scored against it; without it the LTHR is estimated from the lactate
--    threshold and max HR (UserPhysiologicalProfile::lactate_threshold_hr).
ALTER TABLE user_physiological_profiles ADD COLUMN threshold_hr INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- 2. set_physiology documented lactate_threshold_percentage as a fraction of
--    max HR, and the TSS engine read max_hr * percentage as the LTHR. That LTHR
--    is what each such row described, so it is kept as the measured
--    threshold_hr — when it passes the checks set_physiology applies to one:
--    100-200 bpm, above resting HR, below max HR.
UPDATE user_physiological_profiles
SET threshold_hr = CAST(ROUND(max_hr * lactate_threshold_percentage) AS INTEGER)
WHERE lactate_threshold_percentage IS NOT NULL
  AND max_hr IS NOT NULL
  AND ROUND(max_hr * lactate_threshold_percentage) BETWEEN 100 AND 200
  AND ROUND(max_hr * lactate_threshold_percentage) < max_hr
  AND (resting_hr IS NULL OR ROUND(max_hr * lactate_threshold_percentage) > resting_hr);

-- 3. The column now holds a fraction of VO2max, the meaning the model, the
--    pace-zone engine and the configuration catalogue always gave it. Swain et
--    al. 1994 (%HRmax = 0.6463 x %VO2max + 37.182, the regression
--    max_hr_fraction_at_vo2max_fraction applies) converts the stored fraction
--    of max HR, clamped to the 0.65-0.95 set_physiology accepts.
UPDATE user_physiological_profiles
SET lactate_threshold_percentage =
    MIN(0.95, MAX(0.65, (lactate_threshold_percentage - 0.37182) / 0.6463))
WHERE lactate_threshold_percentage IS NOT NULL;

-- 4. analyze_training_load and the recovery tools read FTP, threshold HR, max
--    and resting HR and weight from update_user_configuration's
--    session_overrides; every training-load reader now reads the profile. Each
--    override moves to the profile of every tenant the athlete belongs to,
--    where the profile has no value of its own and the number passes the
--    ranges and orderings set_physiology enforces. The profile's own value
--    wins: set_physiology is the documented writer of these numbers.
CREATE TEMP TABLE threshold_overrides AS
SELECT
    c.user_id AS user_id,
    CASE WHEN json_type(c.config_data, '$.session_overrides.ftp') IN ('integer', 'real')
         THEN json_extract(c.config_data, '$.session_overrides.ftp') END AS ftp,
    COALESCE(
        CASE WHEN json_type(c.config_data, '$.session_overrides.lactate_threshold_hr') IN ('integer', 'real')
             THEN json_extract(c.config_data, '$.session_overrides.lactate_threshold_hr') END,
        CASE WHEN json_type(c.config_data, '$.session_overrides.threshold_hr') IN ('integer', 'real')
             THEN json_extract(c.config_data, '$.session_overrides.threshold_hr') END
    ) AS threshold_hr,
    CASE WHEN json_type(c.config_data, '$.session_overrides.max_hr') IN ('integer', 'real')
         THEN json_extract(c.config_data, '$.session_overrides.max_hr') END AS max_hr,
    CASE WHEN json_type(c.config_data, '$.session_overrides.resting_hr') IN ('integer', 'real')
         THEN json_extract(c.config_data, '$.session_overrides.resting_hr') END AS resting_hr,
    COALESCE(
        CASE WHEN json_type(c.config_data, '$.session_overrides.weight_kg') IN ('integer', 'real')
             THEN json_extract(c.config_data, '$.session_overrides.weight_kg') END,
        CASE WHEN json_type(c.config_data, '$.session_overrides.weight') IN ('integer', 'real')
             THEN json_extract(c.config_data, '$.session_overrides.weight') END
    ) AS weight
FROM user_configurations c
WHERE json_valid(c.config_data);

-- A profile row for each membership that has a number to receive. The enum
-- columns hold their serde JSON text, as the repository writes them.
INSERT OR IGNORE INTO user_physiological_profiles (
    user_id, tenant_id, fitness_level, primary_sport, created_at, updated_at
)
SELECT tu.user_id, tu.tenant_id, '"Recreational"', '"run"', datetime('now'), datetime('now')
FROM threshold_overrides o
JOIN tenant_users tu ON tu.user_id = o.user_id
WHERE (o.ftp BETWEEN 50 AND 600)
   OR (o.threshold_hr BETWEEN 100 AND 200)
   OR (o.max_hr BETWEEN 100 AND 220)
   OR (o.resting_hr BETWEEN 30 AND 100)
   OR (o.weight BETWEEN 30 AND 250);

UPDATE user_physiological_profiles
SET ftp_watts = (SELECT CAST(ROUND(o.ftp) AS INTEGER) FROM threshold_overrides o
                 WHERE o.user_id = user_physiological_profiles.user_id),
    updated_at = datetime('now')
WHERE ftp_watts IS NULL
  AND EXISTS (SELECT 1 FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id
                AND o.ftp BETWEEN 50 AND 600);

UPDATE user_physiological_profiles
SET max_hr = (SELECT CAST(ROUND(o.max_hr) AS INTEGER) FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id),
    updated_at = datetime('now')
WHERE max_hr IS NULL
  AND EXISTS (SELECT 1 FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id
                AND o.max_hr BETWEEN 100 AND 220
                AND ROUND(o.max_hr) > COALESCE(user_physiological_profiles.resting_hr, 0)
                AND ROUND(o.max_hr) > COALESCE(user_physiological_profiles.threshold_hr, 0));

UPDATE user_physiological_profiles
SET resting_hr = (SELECT CAST(ROUND(o.resting_hr) AS INTEGER) FROM threshold_overrides o
                  WHERE o.user_id = user_physiological_profiles.user_id),
    updated_at = datetime('now')
WHERE resting_hr IS NULL
  AND EXISTS (SELECT 1 FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id
                AND o.resting_hr BETWEEN 30 AND 100
                AND ROUND(o.resting_hr) < COALESCE(user_physiological_profiles.max_hr, 1000)
                AND ROUND(o.resting_hr) < COALESCE(user_physiological_profiles.threshold_hr, 1000));

UPDATE user_physiological_profiles
SET threshold_hr = (SELECT CAST(ROUND(o.threshold_hr) AS INTEGER) FROM threshold_overrides o
                    WHERE o.user_id = user_physiological_profiles.user_id),
    updated_at = datetime('now')
WHERE threshold_hr IS NULL
  AND EXISTS (SELECT 1 FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id
                AND o.threshold_hr BETWEEN 100 AND 200
                AND ROUND(o.threshold_hr) < COALESCE(user_physiological_profiles.max_hr, 1000)
                AND ROUND(o.threshold_hr) > COALESCE(user_physiological_profiles.resting_hr, 0));

UPDATE user_physiological_profiles
SET weight = (SELECT o.weight FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id),
    updated_at = datetime('now')
WHERE weight IS NULL
  AND EXISTS (SELECT 1 FROM threshold_overrides o
              WHERE o.user_id = user_physiological_profiles.user_id
                AND o.weight BETWEEN 30 AND 250);

DROP TABLE IF EXISTS threshold_overrides;

-- Every measurement key update_user_configuration now refuses leaves the
-- configuration (its MEASUREMENT_KEYS): the five moved above, and the ones it
-- once stored wholesale that nothing ever read from here (vo2_max, age...).
-- Left behind, such a key would sit beside the profile's number and disagree
-- with it, and a null could not remove it: the tool refuses the key itself.
UPDATE user_configurations
SET config_data = json_remove(
        config_data,
        '$.session_overrides.ftp_watts',
        '$.session_overrides.threshold_pace_sec_per_km',
        '$.session_overrides.max_hr',
        '$.session_overrides.resting_hr',
        '$.session_overrides.threshold_hr',
        '$.session_overrides.lactate_threshold_percentage',
        '$.session_overrides.vo2_max',
        '$.session_overrides.weight',
        '$.session_overrides.age',
        '$.session_overrides.fitness_level',
        '$.session_overrides.primary_sport',
        '$.session_overrides.training_experience_years',
        '$.session_overrides.ftp',
        '$.session_overrides.lactate_threshold_hr',
        '$.session_overrides.lactate_threshold',
        '$.session_overrides.weight_kg',
        '$.session_overrides.threshold_pace'
    )
WHERE json_valid(config_data)
  AND json_type(config_data, '$.session_overrides') = 'object';
