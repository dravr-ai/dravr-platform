-- ABOUTME: Composite (api_key_id, timestamp) index on api_key_usage for the per-request window read
-- ABOUTME: Every API-key request counts its key's calls inside the key's sliding window and reads the oldest

-- The rate limiter runs `COUNT(*), MIN(timestamp) ... WHERE api_key_id = ? AND
-- timestamp > ?` on every authenticated API-key request, and every such request
-- now writes a row. The single-column indexes answer one predicate each, so the
-- read would scan all of a key's rows; the composite answers both from one range.
CREATE INDEX IF NOT EXISTS idx_api_key_usage_key_timestamp
    ON api_key_usage(api_key_id, timestamp);
