-- ABOUTME: Drops user_rate_limit_overrides.daily_limit; a per-user override sets the monthly request limit only
-- ABOUTME: Per-user daily quotas are ConfigScope::User settings, so a daily cap on this row would be a second system

-- The authentication gate meters a user's JWT, cookie and channel requests by
-- the UTC month: an override row's monthly_limit replaces the tier's, and a
-- NULL one lifts the ceiling. Per-user daily limits already have their home in
-- the ConfigScope::User quota settings, so the row carries no daily cap.
ALTER TABLE user_rate_limit_overrides DROP COLUMN daily_limit;  -- idempotency-ok: _sqlx_migrations prevents re-run; SQLite DROP COLUMN has no IF EXISTS
