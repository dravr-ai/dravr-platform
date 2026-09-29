-- ABOUTME: The physiological profile becomes the one home of the thresholds training load scores against
-- ABOUTME: Adds a measured threshold_hr, reads lactate_threshold_percentage as a fraction of VO2max, moves overrides in

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- 1. A measured lactate threshold heart rate. Heart-rate training stress is
--    scored against it; without it the LTHR is estimated from the lactate
--    threshold and max HR (UserPhysiologicalProfile::lactate_threshold_hr).
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS threshold_hr INTEGER;

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
    LEAST(0.95, GREATEST(0.65, (lactate_threshold_percentage - 0.37182) / 0.6463))
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
    CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,ftp}') = 'number'
         THEN (c.config_data::jsonb #>> '{session_overrides,ftp}')::double precision END AS ftp,
    COALESCE(
        CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,lactate_threshold_hr}') = 'number'
             THEN (c.config_data::jsonb #>> '{session_overrides,lactate_threshold_hr}')::double precision END,
        CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,threshold_hr}') = 'number'
             THEN (c.config_data::jsonb #>> '{session_overrides,threshold_hr}')::double precision END
    ) AS threshold_hr,
    CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,max_hr}') = 'number'
         THEN (c.config_data::jsonb #>> '{session_overrides,max_hr}')::double precision END AS max_hr,
    CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,resting_hr}') = 'number'
         THEN (c.config_data::jsonb #>> '{session_overrides,resting_hr}')::double precision END AS resting_hr,
    COALESCE(
        CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,weight_kg}') = 'number'
             THEN (c.config_data::jsonb #>> '{session_overrides,weight_kg}')::double precision END,
        CASE WHEN jsonb_typeof(c.config_data::jsonb #> '{session_overrides,weight}') = 'number'
             THEN (c.config_data::jsonb #>> '{session_overrides,weight}')::double precision END
    ) AS weight
FROM user_configurations c;

-- A profile row for each membership that has a number to receive. The enum
-- columns hold their serde JSON text, as the repository writes them.
INSERT INTO user_physiological_profiles (user_id, tenant_id, fitness_level, primary_sport)
SELECT tu.user_id, tu.tenant_id, '"Recreational"', '"run"'
FROM threshold_overrides o
JOIN tenant_users tu ON tu.user_id = o.user_id
WHERE (o.ftp BETWEEN 50 AND 600)
   OR (o.threshold_hr BETWEEN 100 AND 200)
   OR (o.max_hr BETWEEN 100 AND 220)
   OR (o.resting_hr BETWEEN 30 AND 100)
   OR (o.weight BETWEEN 30 AND 250)
ON CONFLICT (tenant_id, user_id) DO NOTHING;

UPDATE user_physiological_profiles p
SET ftp_watts = CAST(ROUND(o.ftp) AS INTEGER),
    updated_at = NOW()
FROM threshold_overrides o
WHERE o.user_id = p.user_id
  AND p.ftp_watts IS NULL
  AND o.ftp BETWEEN 50 AND 600;

UPDATE user_physiological_profiles p
SET max_hr = CAST(ROUND(o.max_hr) AS INTEGER),
    updated_at = NOW()
FROM threshold_overrides o
WHERE o.user_id = p.user_id
  AND p.max_hr IS NULL
  AND o.max_hr BETWEEN 100 AND 220
  AND ROUND(o.max_hr) > COALESCE(p.resting_hr, 0)
  AND ROUND(o.max_hr) > COALESCE(p.threshold_hr, 0);

UPDATE user_physiological_profiles p
SET resting_hr = CAST(ROUND(o.resting_hr) AS INTEGER),
    updated_at = NOW()
FROM threshold_overrides o
WHERE o.user_id = p.user_id
  AND p.resting_hr IS NULL
  AND o.resting_hr BETWEEN 30 AND 100
  AND ROUND(o.resting_hr) < COALESCE(p.max_hr, 1000)
  AND ROUND(o.resting_hr) < COALESCE(p.threshold_hr, 1000);

UPDATE user_physiological_profiles p
SET threshold_hr = CAST(ROUND(o.threshold_hr) AS INTEGER),
    updated_at = NOW()
FROM threshold_overrides o
WHERE o.user_id = p.user_id
  AND p.threshold_hr IS NULL
  AND o.threshold_hr BETWEEN 100 AND 200
  AND ROUND(o.threshold_hr) < COALESCE(p.max_hr, 1000)
  AND ROUND(o.threshold_hr) > COALESCE(p.resting_hr, 0);

UPDATE user_physiological_profiles p
SET weight = o.weight,
    updated_at = NOW()
FROM threshold_overrides o
WHERE o.user_id = p.user_id
  AND p.weight IS NULL
  AND o.weight BETWEEN 30 AND 250;

DROP TABLE IF EXISTS threshold_overrides;

-- Every measurement key update_user_configuration now refuses leaves the
-- configuration (its MEASUREMENT_KEYS): the five moved above, and the ones it
-- once stored wholesale that nothing ever read from here (vo2_max, age...).
-- Left behind, such a key would sit beside the profile's number and disagree
-- with it, and a null could not remove it: the tool refuses the key itself.
UPDATE user_configurations
SET config_data = jsonb_set(
        config_data::jsonb,
        '{session_overrides}',
        (config_data::jsonb -> 'session_overrides')
            - ARRAY['ftp_watts', 'threshold_pace_sec_per_km', 'max_hr', 'resting_hr',
                    'threshold_hr', 'lactate_threshold_percentage', 'vo2_max', 'weight',
                    'age', 'fitness_level', 'primary_sport', 'training_experience_years',
                    'ftp', 'lactate_threshold_hr', 'lactate_threshold', 'weight_kg',
                    'threshold_pace']
    )::text
WHERE jsonb_typeof(config_data::jsonb -> 'session_overrides') = 'object';
