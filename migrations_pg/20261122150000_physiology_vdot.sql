-- ABOUTME: A stated VDOT with its day and method, and how the stored VO2max was found (carnet#1693 items 2 and 5)
-- ABOUTME: Additive only: nullable vdot, vdot_as_of, vdot_method and vo2_max_method; every existing row reads as having none

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A VDOT is Daniels' performance index, read from a race; a VO2max is the
-- athlete's aerobic capacity. They differ for most athletes, so a VDOT is
-- stored as its own value and never in vo2_max. vdot_as_of is the race's day,
-- or the day it was stated: a stated VDOT gives way to what recent running
-- shows once it is 12 weeks old. vdot_method says how it was found
-- ("race: 10 km in 41:21", "stated").
--
-- vo2_max_method records how each VO2max saved from now on was found
-- ("cooper_test", "lab test"). VO2max values stored before this migration
-- keep their value and read as having no recorded method. No backfill.

-- VDOT, the day it was true, and how it was found.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS vdot DOUBLE PRECISION;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS vdot_as_of DATE;
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS vdot_method TEXT;

-- How the stored VO2max was found.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS vo2_max_method TEXT;
