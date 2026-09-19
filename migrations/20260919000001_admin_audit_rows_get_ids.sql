-- ABOUTME: Gives admin_token_usage and admin_provisioned_keys a database-assigned integer id, as Postgres has
-- ABOUTME: Both were TEXT PRIMARY KEY that no insert ever filled, so every audit row read back with id 0
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Postgres declares both ids SERIAL. SQLite declared them TEXT PRIMARY KEY
-- with no default, and the inserts never name the column, so every row was
-- stored with a NULL key (SQLite permits that on a non-INTEGER primary key)
-- and decoded as 0 on the way out. INTEGER PRIMARY KEY AUTOINCREMENT is the
-- rowid alias: the engine assigns it, it is never reused, and the row reads
-- back with the same positive integer Postgres would give it. SQLite cannot
-- alter a primary key in place, so each table is rebuilt; the copy leaves
-- the old NULL ids behind and lets the engine number the rows.

CREATE TABLE IF NOT EXISTS admin_token_usage_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    admin_token_id TEXT NOT NULL REFERENCES admin_tokens(id) ON DELETE CASCADE,
    timestamp TEXT NOT NULL,
    action TEXT NOT NULL,
    target_resource TEXT,
    ip_address TEXT,
    user_agent TEXT,
    request_size_bytes INTEGER,
    success INTEGER NOT NULL,
    response_time_ms INTEGER,
    method TEXT
);

INSERT INTO admin_token_usage_new
    (admin_token_id, timestamp, action, target_resource, ip_address, user_agent,
     request_size_bytes, success, response_time_ms, method)
SELECT admin_token_id, timestamp, action, target_resource, ip_address, user_agent,
       request_size_bytes, success, response_time_ms, method
FROM admin_token_usage
ORDER BY timestamp ASC;

DROP TABLE admin_token_usage; -- idempotency-ok: table rebuild, the copy above only exists once the old table does
ALTER TABLE admin_token_usage_new RENAME TO admin_token_usage;

CREATE INDEX IF NOT EXISTS idx_admin_usage_token_id ON admin_token_usage(admin_token_id);
CREATE INDEX IF NOT EXISTS idx_admin_usage_timestamp ON admin_token_usage(timestamp);

CREATE TABLE IF NOT EXISTS admin_provisioned_keys_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    admin_token_id TEXT NOT NULL REFERENCES admin_tokens(id) ON DELETE CASCADE,
    api_key_id TEXT NOT NULL,
    user_email TEXT NOT NULL,
    requested_tier TEXT NOT NULL,
    provisioned_at TEXT NOT NULL,
    provisioned_by_service TEXT NOT NULL,
    rate_limit_requests INTEGER NOT NULL,
    rate_limit_period TEXT NOT NULL,
    key_status TEXT NOT NULL DEFAULT 'active',
    revoked_at TEXT,
    revoked_reason TEXT
);

INSERT INTO admin_provisioned_keys_new
    (admin_token_id, api_key_id, user_email, requested_tier, provisioned_at,
     provisioned_by_service, rate_limit_requests, rate_limit_period, key_status,
     revoked_at, revoked_reason)
SELECT admin_token_id, api_key_id, user_email, requested_tier, provisioned_at,
       provisioned_by_service, rate_limit_requests, rate_limit_period, key_status,
       revoked_at, revoked_reason
FROM admin_provisioned_keys
ORDER BY provisioned_at ASC;

DROP TABLE admin_provisioned_keys; -- idempotency-ok: table rebuild, the copy above only exists once the old table does
ALTER TABLE admin_provisioned_keys_new RENAME TO admin_provisioned_keys;

CREATE INDEX IF NOT EXISTS idx_admin_provisioned_token ON admin_provisioned_keys(admin_token_id);
