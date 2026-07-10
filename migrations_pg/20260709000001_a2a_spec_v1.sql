-- A2A protocol 1.0 alignment: task lifecycle states, wire-state columns,
-- and push notification configurations (PostgreSQL variant).
--
-- session_token is a dual-use soft key (session token for session-keyed
-- tasks, client_id for client-keyed tasks created without a session), so the
-- hard FK to a2a_sessions is dropped — it rejects the client-keyed form.
ALTER TABLE a2a_tasks DROP CONSTRAINT IF EXISTS a2a_tasks_session_token_fkey;

ALTER TABLE a2a_tasks ADD COLUMN IF NOT EXISTS context_id TEXT;
ALTER TABLE a2a_tasks ADD COLUMN IF NOT EXISTS status_message JSONB;
ALTER TABLE a2a_tasks ADD COLUMN IF NOT EXISTS history JSONB;
ALTER TABLE a2a_tasks ADD COLUMN IF NOT EXISTS artifacts JSONB;
UPDATE a2a_tasks SET updated_at = COALESCE(updated_at, created_at, CURRENT_TIMESTAMP)
WHERE updated_at IS NULL;
ALTER TABLE a2a_tasks ALTER COLUMN updated_at SET DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE a2a_tasks ALTER COLUMN updated_at SET NOT NULL;

-- Migrate pre-1.0 states to the A2A 1.0 compact names.
UPDATE a2a_tasks SET status = CASE status
    WHEN 'pending' THEN 'submitted'
    WHEN 'running' THEN 'working'
    WHEN 'cancelled' THEN 'canceled'
    ELSE status
END
WHERE status IN ('pending', 'running', 'cancelled');

ALTER TABLE a2a_tasks ALTER COLUMN status SET DEFAULT 'submitted';

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
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (task_id, config_id)
);

CREATE INDEX IF NOT EXISTS idx_a2a_push_configs_task_id
    ON a2a_push_notification_configs(task_id);
