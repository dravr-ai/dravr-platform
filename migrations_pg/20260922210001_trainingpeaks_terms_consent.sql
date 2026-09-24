-- ABOUTME: Records the TrainingPeaks exposure notice an account accepted, and when (PostgreSQL version)
-- ABOUTME: NULL until accepted; a newer notice version than the one stored asks again

ALTER TABLE users ADD COLUMN IF NOT EXISTS trainingpeaks_terms_version TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS trainingpeaks_terms_consented_at TIMESTAMPTZ;
