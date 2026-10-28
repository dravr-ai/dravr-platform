-- ABOUTME: Records the data-sharing permissions a provider last reported for a connection (PostgreSQL version)
-- ABOUTME: NULL until the provider reports them, read as every permission granted; a reconnect clears it

ALTER TABLE provider_connections ADD COLUMN IF NOT EXISTS provider_grants TEXT;
