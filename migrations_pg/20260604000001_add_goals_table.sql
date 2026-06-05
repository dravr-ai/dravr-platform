-- Create the `goals` table on PostgreSQL.
--
-- The SQLite tree (migrations/20250120000003_analytics_schema.sql) has always
-- created `goals`, but no PostgreSQL migration did. The SetGoalTool's
-- create_goal / get_goals / update_goal_progress paths
-- (crates/pierre-database/src/backends/postgres/user.rs) issue
-- `INSERT INTO goals` / `FROM goals` / `UPDATE goals`, so on Postgres
-- (Cloud Run) those failed at runtime with `relation "goals" does not exist`.
--
-- Column types match the sqlx binds in that backend exactly:
--   id         -> bound as a String (Uuid::new_v4().to_string())  => TEXT
--   user_id    -> bound as Uuid                                   => UUID (FK)
--   goal_data  -> bound as serde_json::Value, jsonb-path updated  => JSONB
--   created_at -> bound as DateTime<Utc>                          => TIMESTAMPTZ
--   updated_at -> bound as DateTime<Utc>                          => TIMESTAMPTZ
CREATE TABLE IF NOT EXISTS goals (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    goal_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_goals_user_id ON goals (user_id);
