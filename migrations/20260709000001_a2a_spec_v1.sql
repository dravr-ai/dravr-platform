-- A2A protocol 1.0 alignment: task lifecycle states, wire-state columns,
-- and push notification configurations.
--
-- a2a_tasks: the status CHECK pins the pre-1.0 states, so the table is
-- rebuilt (SQLite cannot alter a CHECK) with:
--   * the nine A2A 1.0 task states (compact form; wire uses TASK_STATE_*),
--   * new columns context_id / status_message / history / artifacts holding
--     the A2A wire JSON,
--   * no FK on session_token: the column is a dual-use soft key (session
--     token for session-keyed tasks, client_id for client-keyed tasks
--     created without a session), so a hard FK to a2a_sessions is wrong.
-- Existing rows are migrated: pending->submitted, running->working,
-- cancelled->canceled.

CREATE TABLE a2a_tasks_v1 ( -- idempotency-ok: rebuild target; SQLite cannot alter the status CHECK in place
    task_id TEXT PRIMARY KEY,
    session_token TEXT NOT NULL,
    task_type TEXT NOT NULL,
    parameters TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'submitted', 'working', 'completed', 'failed', 'canceled',
        'input-required', 'rejected', 'auth-required'
    )),
    result TEXT,
    context_id TEXT,
    status_message TEXT,
    history TEXT,
    artifacts TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO a2a_tasks_v1 (task_id, session_token, task_type, parameters, status,
    result, context_id, status_message, history, artifacts, created_at, updated_at)
  SELECT task_id, session_token, task_type, parameters,
    CASE status
        WHEN 'pending' THEN 'submitted'
        WHEN 'running' THEN 'working'
        WHEN 'cancelled' THEN 'canceled'
        ELSE status
    END,
    result, NULL, NULL, NULL, NULL, created_at, updated_at
  FROM a2a_tasks;

DROP TABLE a2a_tasks; -- idempotency-ok: rebuild swap; the copy above requires the old table to exist
ALTER TABLE a2a_tasks_v1 RENAME TO a2a_tasks;

CREATE INDEX IF NOT EXISTS idx_a2a_tasks_session_token ON a2a_tasks(session_token);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_status ON a2a_tasks(status);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_context_id ON a2a_tasks(context_id);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_updated_at ON a2a_tasks(updated_at);

-- A2A 1.0 TaskPushNotificationConfig storage: one row per webhook
-- registration on a task; the server POSTs task state changes to url using
-- the stored authentication material.
CREATE TABLE IF NOT EXISTS a2a_push_notification_configs (
    config_id TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES a2a_tasks(task_id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    token TEXT,
    auth_scheme TEXT,
    auth_credentials TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (task_id, config_id)
);

CREATE INDEX IF NOT EXISTS idx_a2a_push_configs_task_id
    ON a2a_push_notification_configs(task_id);
