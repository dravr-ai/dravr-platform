-- ABOUTME: Records whether a provider connection signed in with an athlete or a coach account (PostgreSQL version)
-- ABOUTME: NULL until the provider reports it; a reconnect clears it so the new login is read again

ALTER TABLE provider_connections ADD COLUMN IF NOT EXISTS account_role TEXT CHECK (account_role IS NULL OR account_role IN ('athlete', 'coach'));
