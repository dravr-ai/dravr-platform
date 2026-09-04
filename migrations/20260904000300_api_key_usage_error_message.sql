-- Restore the error text of a failed API-key request.
--
-- `ApiKeyUsage::error_message` is populated at the call site from the MCP error
-- body, but the column was dropped from SQLite to match PostgreSQL, which never
-- had it. Both trees carry it now, so the request-log read path serves the real
-- message instead of a constant.

ALTER TABLE api_key_usage ADD COLUMN error_message TEXT;  -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; the PostgreSQL twin uses it
