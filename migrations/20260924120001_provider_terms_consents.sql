-- ABOUTME: Records, per provider, the exposure notice an account accepted and when; replaces the TrainingPeaks-only users columns
-- ABOUTME: A provider scraped through the athlete's own sign-in (TrainingPeaks, COROS) asks again when its notice version moves

-- One row per (account, provider) once the account accepted that provider's
-- notice. The notice belongs to the account, not to a session: a disconnect
-- leaves the row and a reconnect reads it.
CREATE TABLE IF NOT EXISTS provider_terms_consents (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    version TEXT NOT NULL,
    consented_at TEXT NOT NULL,
    PRIMARY KEY (user_id, provider)
);

-- The TrainingPeaks acceptances recorded so far move over under the backend
-- the notice guards, so no account that accepted is asked again.
INSERT OR IGNORE INTO provider_terms_consents (user_id, provider, version, consented_at)
    SELECT id, 'sciotte_trainingpeaks', trainingpeaks_terms_version,
           COALESCE(trainingpeaks_terms_consented_at, CURRENT_TIMESTAMP)
    FROM users
    WHERE trainingpeaks_terms_version IS NOT NULL;

ALTER TABLE users DROP COLUMN trainingpeaks_terms_version;  -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
ALTER TABLE users DROP COLUMN trainingpeaks_terms_consented_at;  -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
