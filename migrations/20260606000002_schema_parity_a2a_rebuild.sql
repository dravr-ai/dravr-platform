-- ABOUTME: Rebuilds the SQLite a2a_* tables to the canonical PostgreSQL schema
-- ABOUTME: (client_id PK, api_key_hash, session-keyed tasks) for cross-backend parity.

-- sqlx runs each migration in a transaction; `PRAGMA foreign_keys` is a no-op
-- inside a transaction, so defer FK checks to COMMIT instead. On a fresh DB the
-- copies move zero rows, so deferred checks pass trivially.
PRAGMA defer_foreign_keys = ON;

-- a2a_clients: id -> client_id (PK), public_key -> api_key_hash,
-- client_secret -> client_secret_hash, drop permissions, rate_limit_requests ->
-- rate_limit_per_minute, rate_limit_window_seconds -> rate_limit_per_day,
-- + contact_email. The child FKs below are recreated to point at client_id.
CREATE TABLE a2a_clients_new (
    client_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    client_secret_hash TEXT NOT NULL,
    api_key_hash TEXT NOT NULL,
    capabilities TEXT NOT NULL DEFAULT '[]',
    redirect_uris TEXT NOT NULL DEFAULT '[]',
    contact_email TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    rate_limit_per_minute INTEGER NOT NULL DEFAULT 100,
    rate_limit_per_day INTEGER NOT NULL DEFAULT 10000,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(api_key_hash)
);
INSERT INTO a2a_clients_new (client_id, user_id, name, description, client_secret_hash,
    api_key_hash, capabilities, redirect_uris, contact_email, is_active,
    rate_limit_per_minute, rate_limit_per_day, created_at, updated_at)
  SELECT id, user_id, name, description, client_secret, public_key, capabilities,
    redirect_uris, NULL, is_active, rate_limit_requests, rate_limit_window_seconds,
    created_at, updated_at
  FROM a2a_clients;
DROP TABLE a2a_clients;
ALTER TABLE a2a_clients_new RENAME TO a2a_clients;

-- a2a_sessions: last_activity -> last_active_at, + is_active, - requests_count.
CREATE TABLE a2a_sessions_new (
    session_token TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    granted_scopes TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_active_at TEXT NOT NULL
);
INSERT INTO a2a_sessions_new (session_token, client_id, user_id, granted_scopes,
    is_active, created_at, expires_at, last_active_at)
  SELECT session_token, client_id, user_id, granted_scopes, 1, created_at,
    expires_at, last_activity
  FROM a2a_sessions;
DROP TABLE a2a_sessions;
ALTER TABLE a2a_sessions_new RENAME TO a2a_sessions;

-- a2a_tasks: id -> task_id (PK), client_id -> session_token (re-key),
-- input_data -> parameters, output_data -> result, drop error_message/completed_at.
-- Legacy rows were client-keyed with no session; carry client_id into
-- session_token as a best-effort identifier (deferred FKs allow the empty-DB copy).
CREATE TABLE a2a_tasks_new (
    task_id TEXT PRIMARY KEY,
    session_token TEXT NOT NULL REFERENCES a2a_sessions(session_token) ON DELETE CASCADE,
    task_type TEXT NOT NULL,
    parameters TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    result TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
INSERT INTO a2a_tasks_new (task_id, session_token, task_type, parameters, status,
    result, created_at, updated_at)
  SELECT id, client_id, task_type, input_data, status, output_data, created_at, updated_at
  FROM a2a_tasks;
DROP TABLE a2a_tasks;
ALTER TABLE a2a_tasks_new RENAME TO a2a_tasks;

-- a2a_usage: tool_name -> endpoint, + method, - error_message; FK -> client_id.
CREATE TABLE a2a_usage_new (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
    session_token TEXT REFERENCES a2a_sessions(session_token) ON DELETE SET NULL,
    timestamp TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    response_time_ms INTEGER,
    status_code INTEGER NOT NULL,
    method TEXT,
    request_size_bytes INTEGER,
    response_size_bytes INTEGER,
    ip_address TEXT,
    user_agent TEXT,
    protocol_version TEXT NOT NULL DEFAULT '1.0',
    client_capabilities TEXT NOT NULL DEFAULT '[]',
    granted_scopes TEXT NOT NULL DEFAULT '[]'
);
INSERT INTO a2a_usage_new (id, client_id, session_token, timestamp, endpoint,
    response_time_ms, status_code, method, request_size_bytes, response_size_bytes,
    ip_address, user_agent, protocol_version, client_capabilities, granted_scopes)
  SELECT id, client_id, session_token, timestamp, tool_name, response_time_ms,
    status_code, NULL, request_size_bytes, response_size_bytes, ip_address,
    user_agent, protocol_version, client_capabilities, granted_scopes
  FROM a2a_usage;
DROP TABLE a2a_usage;
ALTER TABLE a2a_usage_new RENAME TO a2a_usage;

-- a2a_client_api_keys: rebuild only to repoint the FK at a2a_clients(client_id).
CREATE TABLE a2a_client_api_keys_new (
    client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
    api_key_id TEXT NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (client_id, api_key_id)
);
INSERT INTO a2a_client_api_keys_new (client_id, api_key_id, created_at)
  SELECT client_id, api_key_id, created_at FROM a2a_client_api_keys;
DROP TABLE a2a_client_api_keys;
ALTER TABLE a2a_client_api_keys_new RENAME TO a2a_client_api_keys;

-- Recreate the a2a indexes against the rebuilt tables.
CREATE INDEX IF NOT EXISTS idx_a2a_sessions_client_id ON a2a_sessions(client_id);
CREATE INDEX IF NOT EXISTS idx_a2a_sessions_expires_at ON a2a_sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_session_token ON a2a_tasks(session_token);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_status ON a2a_tasks(status);
CREATE INDEX IF NOT EXISTS idx_a2a_usage_client_id ON a2a_usage(client_id);
CREATE INDEX IF NOT EXISTS idx_a2a_usage_timestamp ON a2a_usage(timestamp);
