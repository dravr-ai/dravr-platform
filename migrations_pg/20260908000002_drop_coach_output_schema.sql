-- ABOUTME: Drops coaches.output_schema — the builder agents no longer emit a JSON plan document for the platform to extract (PostgreSQL)
-- ABOUTME: A plan reaches a client as the workout_plan block projected from the rows save_training_plan stored, one session vocabulary

-- The column named the JSON schema a builder agent's whole reply had to
-- conform to. That vocabulary (the structured-workout Block) was a second
-- way of saying what PlannedDay.steps already says; the card is now
-- projected from the saved plan and the model writes prose. Nothing reads
-- the column, so it goes.

ALTER TABLE coaches DROP COLUMN IF EXISTS output_schema;
