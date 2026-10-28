-- ABOUTME: Records the data-sharing permissions a provider last reported for a connection (Garmin's userPermissionsChange)
-- ABOUTME: NULL until the provider reports them, read as every permission granted; a reconnect clears it

ALTER TABLE provider_connections ADD COLUMN provider_grants TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
