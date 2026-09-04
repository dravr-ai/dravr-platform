-- Restore the error text of a failed API-key request.
--
-- `ApiKeyUsage::error_message` is populated at the call site from the MCP error
-- body, but PostgreSQL's api_key_usage never had a column to hold it. Both trees
-- carry it now, so the request-log read path serves the real message instead of
-- a constant.

ALTER TABLE api_key_usage ADD COLUMN IF NOT EXISTS error_message TEXT;
