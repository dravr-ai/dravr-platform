-- ABOUTME: Remove synthetic provider connections from production environments
-- ABOUTME: Synthetic providers are excluded from production builds via server-production feature flag

-- Remove synthetic provider connections so they no longer appear as connected
DELETE FROM provider_connections WHERE provider IN ('synthetic', 'synthetic_sleep');
