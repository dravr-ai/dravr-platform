// ABOUTME: Repository trait definitions for the agent-to-agent (A2A) protocol persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::a2a::{
    A2AClient, A2APushNotificationConfig, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};

use serde_json::Value;
use uuid::Uuid;

/// A2A (Agent-to-Agent) client and session management repository
#[async_trait]
pub trait A2ARepository: Send + Sync {
    /// Create a new A2A client
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String>;
    /// Get A2A client by ID
    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>>;
    /// Get A2A client by API key ID
    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>>;
    /// Get A2A client by name
    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>>;
    /// List all A2A clients for a user
    async fn list_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>>;
    /// Deactivate an A2A client
    async fn deactivate_client(&self, client_id: &str) -> AppResult<()>;
    /// Get client credentials for authentication
    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>>;
    /// Invalidate all active sessions for a client
    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()>;
    /// Deactivate all API keys associated with a client
    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()>;
    /// Create a new A2A session
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String>;
    /// Get A2A session by token
    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>>;
    /// Update A2A session activity timestamp
    async fn update_session_activity(&self, session_token: &str) -> AppResult<()>;
    /// Get active sessions for a specific client
    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>>;
    /// Create a new A2A task
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
        context_id: Option<&str>,
    ) -> AppResult<String>;
    /// Get A2A task by ID
    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>>;
    /// List A2A tasks for a client with optional filtering, ordered by
    /// status timestamp (`updated_at`) descending per A2A 1.0 `ListTasks`
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        context_id: Option<&str>,
        updated_after: Option<DateTime<Utc>>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>>;
    /// Update A2A task status, result, and the wire `TaskStatus.message` JSON
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        status_message: Option<&Value>,
    ) -> AppResult<()>;
    /// Replace the task's wire `history` (`Message[]`) and `artifacts`
    /// (`Artifact[]`) JSON documents
    async fn update_task_wire_state(
        &self,
        task_id: &str,
        history: Option<&Value>,
        artifacts: Option<&Value>,
    ) -> AppResult<()>;
    /// Store a push notification configuration for a task (A2A 1.0
    /// `CreateTaskPushNotificationConfig`)
    async fn create_push_config(&self, config: &A2APushNotificationConfig) -> AppResult<()>;
    /// Get one push notification configuration of a task
    async fn get_push_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> AppResult<Option<A2APushNotificationConfig>>;
    /// List all push notification configurations of a task
    async fn list_push_configs(&self, task_id: &str) -> AppResult<Vec<A2APushNotificationConfig>>;
    /// Delete a push notification configuration; returns whether a row existed
    async fn delete_push_config(&self, task_id: &str, config_id: &str) -> AppResult<bool>;
    /// Record A2A usage for analytics
    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()>;
    /// Get current A2A usage count for a client
    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32>;
    /// Get A2A usage statistics for a client
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<A2AUsageStats>;
    /// Get A2A client usage history
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>>;
}

// ── Statements, written once for both backends ──
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres, so one statement serves both drivers and cannot drift between
// them. `a2a_clients.user_id` and `a2a_sessions.user_id` are `uuid` on
// Postgres and `TEXT` on `SQLite`, so a [`Uuid`] goes through the backend's
// uuid codec; the list columns (`capabilities`, `redirect_uris`,
// `granted_scopes`, `client_capabilities`) are `TEXT[]` on Postgres and text
// on `SQLite`, so a list goes through the backend's list codec. The five
// JSON columns of `a2a_tasks` are `JSONB` on Postgres and `TEXT` on
// `SQLite`; both bind and read a [`Value`], and what comes back is a
// re-rendering of what was stored, never the caller's bytes. Timestamps bind
// and read as `DateTime<Utc>` on both, and every comparison against "now"
// binds the instant rather than naming the engine's clock, so `SQLite`
// holds one text form and compares it as one. Three spellings remain per
// engine and arrive as macro literals: the calendar day of a usage row, the
// expression that mints a usage row's id, and the cast an IP address needs.

/// The client projection every client read returns. The model's
/// `public_key` lives in `api_key_hash`, its `rate_limit_requests` in
/// `rate_limit_per_minute` and its `rate_limit_window_seconds` in
/// `rate_limit_per_day`.
macro_rules! client_columns {
    () => {
        "client_id, user_id, name, description, api_key_hash, capabilities, redirect_uris, \
         rate_limit_per_minute, rate_limit_per_day, is_active, created_at, updated_at"
    };
}

/// Register a client. The secret is stored as its SHA-256 hex digest.
pub(crate) const INSERT_CLIENT_SQL: &str = r"
            INSERT INTO a2a_clients (
                client_id, user_id, name, description, api_key_hash, client_secret_hash,
                capabilities, redirect_uris,
                rate_limit_per_minute, rate_limit_per_day, is_active,
                created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ";

/// Bind a client to the API key it authenticates with.
pub(crate) const INSERT_CLIENT_API_KEY_SQL: &str = r"
            INSERT INTO a2a_client_api_keys (client_id, api_key_id, created_at)
            VALUES ($1, $2, $3)
            ";

/// One client by id, active or not.
pub(crate) const GET_CLIENT_SQL: &str = concat!(
    "SELECT ",
    client_columns!(),
    " FROM a2a_clients WHERE client_id = $1"
);

/// The active client behind an API key: how API-key auth resolves a caller.
pub(crate) const GET_CLIENT_BY_API_KEY_SQL: &str = r"
            SELECT c.client_id, c.user_id, c.name, c.description, c.api_key_hash, c.capabilities,
                   c.redirect_uris, c.rate_limit_per_minute, c.rate_limit_per_day, c.is_active,
                   c.created_at, c.updated_at
            FROM a2a_clients c
            INNER JOIN a2a_client_api_keys k ON c.client_id = k.client_id
            WHERE k.api_key_id = $1 AND c.is_active = TRUE
            ";

/// One client by name, active or not.
pub(crate) const GET_CLIENT_BY_NAME_SQL: &str = concat!(
    "SELECT ",
    client_columns!(),
    " FROM a2a_clients WHERE name = $1"
);

/// Every active client, newest first: the system-wide (admin) listing.
pub(crate) const LIST_ALL_CLIENTS_SQL: &str = concat!(
    "SELECT ",
    client_columns!(),
    " FROM a2a_clients WHERE is_active = TRUE ORDER BY created_at DESC"
);

/// A user's active clients, newest first.
pub(crate) const LIST_USER_CLIENTS_SQL: &str = concat!(
    "SELECT ",
    client_columns!(),
    " FROM a2a_clients WHERE is_active = TRUE AND user_id = $1 ORDER BY created_at DESC"
);

/// Retire a client; the caller checks that a row changed.
pub(crate) const DEACTIVATE_CLIENT_SQL: &str =
    "UPDATE a2a_clients SET is_active = FALSE, updated_at = $1 WHERE client_id = $2";

/// The id and secret digest an active client authenticates against.
pub(crate) const CLIENT_CREDENTIALS_SQL: &str =
    "SELECT client_id, client_secret_hash FROM a2a_clients WHERE client_id = $1 AND is_active = TRUE";

/// End every session of a client by moving its expiry into the past.
pub(crate) const INVALIDATE_CLIENT_SESSIONS_SQL: &str =
    "UPDATE a2a_sessions SET expires_at = $1 WHERE client_id = $2";

/// Retire every API key a client was bound to.
pub(crate) const DEACTIVATE_CLIENT_API_KEYS_SQL: &str = "UPDATE api_keys SET is_active = FALSE \
     WHERE id IN (SELECT api_key_id FROM a2a_client_api_keys WHERE client_id = $1)";

/// The session projection every session read returns.
macro_rules! session_columns {
    () => {
        "session_token, client_id, user_id, granted_scopes, expires_at, last_active_at, created_at"
    };
}

/// Open a session; `$6` stamps both `last_active_at` and `created_at`.
pub(crate) const CREATE_SESSION_SQL: &str = r"
            INSERT INTO a2a_sessions (
                session_token, client_id, user_id, granted_scopes,
                expires_at, last_active_at, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $6)
            ";

/// A session by token, only while its expiry is after `$2`.
pub(crate) const GET_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM a2a_sessions WHERE session_token = $1 AND expires_at > $2"
);

/// Stamp a session's last activity.
pub(crate) const TOUCH_SESSION_SQL: &str =
    "UPDATE a2a_sessions SET last_active_at = $1 WHERE session_token = $2";

/// A client's sessions whose expiry is after `$2`, most recently active first.
pub(crate) const ACTIVE_SESSIONS_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM a2a_sessions WHERE client_id = $1 AND expires_at > $2 ORDER BY last_active_at DESC"
);

/// The task projection every task read returns; the dynamic listing in the
/// backend body is built on it.
macro_rules! task_columns {
    () => {
        "task_id, session_token, task_type, parameters, result, status, context_id, \
         status_message, history, artifacts, created_at, updated_at"
    };
}
pub(crate) use task_columns;

/// Submit a task; `$7` stamps both `created_at` and `updated_at`.
pub(crate) const CREATE_TASK_SQL: &str = r"
            INSERT INTO a2a_tasks (
                task_id, session_token, task_type, parameters,
                status, context_id, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
            ";

/// One task by id.
pub(crate) const GET_TASK_SQL: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM a2a_tasks WHERE task_id = $1"
);

/// Move a task's status, stamping `updated_at` (the A2A status timestamp);
/// `result` and `status_message` are replaced only when provided.
pub(crate) const UPDATE_TASK_STATUS_SQL: &str = r"
            UPDATE a2a_tasks
            SET status = $2,
                result = COALESCE($3, result),
                status_message = COALESCE($4, status_message),
                updated_at = $5
            WHERE task_id = $1
            ";

/// Replace a task's wire `history` and `artifacts`, each only when provided.
pub(crate) const UPDATE_TASK_WIRE_STATE_SQL: &str = r"
            UPDATE a2a_tasks
            SET history = COALESCE($2, history),
                artifacts = COALESCE($3, artifacts)
            WHERE task_id = $1
            ";

/// Store or refresh one push-notification config of a task.
pub(crate) const UPSERT_PUSH_CONFIG_SQL: &str = r"
            INSERT INTO a2a_push_notification_configs (
                config_id, task_id, url, token, auth_scheme, auth_credentials,
                created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (task_id, config_id) DO UPDATE SET
                url = EXCLUDED.url,
                token = EXCLUDED.token,
                auth_scheme = EXCLUDED.auth_scheme,
                auth_credentials = EXCLUDED.auth_credentials,
                updated_at = EXCLUDED.updated_at
            ";

/// The push-config projection.
macro_rules! push_config_columns {
    () => {
        "config_id, task_id, url, token, auth_scheme, auth_credentials, created_at, updated_at"
    };
}

/// One push-notification config of a task.
pub(crate) const GET_PUSH_CONFIG_SQL: &str = concat!(
    "SELECT ",
    push_config_columns!(),
    " FROM a2a_push_notification_configs WHERE task_id = $1 AND config_id = $2"
);

/// Every push-notification config of a task, oldest first.
pub(crate) const LIST_PUSH_CONFIGS_SQL: &str = concat!(
    "SELECT ",
    push_config_columns!(),
    " FROM a2a_push_notification_configs WHERE task_id = $1 ORDER BY created_at ASC"
);

/// Remove one push-notification config; the caller reads whether a row went.
pub(crate) const DELETE_PUSH_CONFIG_SQL: &str =
    "DELETE FROM a2a_push_notification_configs WHERE task_id = $1 AND config_id = $2";

/// Record one request. `$id` is the expression that mints the row's id:
/// `a2a_usage.id` is a `SERIAL` on Postgres (`DEFAULT`) and a `TEXT` primary
/// key with no default on `SQLite` (`lower(hex(randomblob(16)))`). `$inet`
/// is the cast the `INET` column needs on Postgres and nothing on `SQLite`,
/// whose column is `TEXT`. The model's `tool_name` is the `endpoint` column;
/// `method` is not carried by the model and is stored NULL.
macro_rules! record_usage_sql {
    ($id:literal, $inet:literal) => {
        concat!(
            "INSERT INTO a2a_usage (id, client_id, session_token, timestamp, endpoint, \
                 response_time_ms, status_code, method, request_size_bytes, response_size_bytes, \
                 ip_address, user_agent, protocol_version, client_capabilities, granted_scopes) \
             VALUES (",
            $id,
            ", $1, $2, $3, $4, $5, $6, $7, $8, $9, $10",
            $inet,
            ", $11, $12, $13, $14)"
        )
    };
}
pub(crate) use record_usage_sql;

/// A client's requests since `$2`: its current usage against its own
/// rate-limit window.
pub(crate) const CLIENT_USAGE_SINCE_SQL: &str =
    "SELECT COUNT(*) FROM a2a_usage WHERE client_id = $1 AND timestamp > $2";

/// Usage totals over a window. `successful` and `failed` partition every
/// request on the 400 boundary, so the two sum to `total`. The average is
/// cast to a double on both engines: Postgres averages an integer column as
/// `NUMERIC`, which sqlx will not decode as `f64`.
pub(crate) const USAGE_STATS_SQL: &str = r"
            SELECT
                COUNT(*) AS total_requests,
                COUNT(CASE WHEN status_code < 400 THEN 1 END) AS successful_requests,
                COUNT(CASE WHEN status_code >= 400 THEN 1 END) AS failed_requests,
                CAST(AVG(response_time_ms) AS DOUBLE PRECISION) AS avg_response_time,
                SUM(request_size_bytes) AS total_request_bytes,
                SUM(response_size_bytes) AS total_response_bytes
            FROM a2a_usage
            WHERE client_id = $1 AND timestamp >= $2 AND timestamp <= $3
            ";

/// Daily success and error counts since `$2`, newest day first. `$day` is
/// the engine's spelling of the UTC calendar day of a `timestamp` row, as a
/// `DATE`: `date(timestamp)` on `SQLite`, whose text the driver reads as a
/// date, and `CAST(timestamp AT TIME ZONE 'UTC' AS DATE)` on Postgres.
macro_rules! usage_history_sql {
    ($day:literal) => {
        concat!(
            "SELECT ",
            $day,
            " AS usage_date, \
                 COUNT(CASE WHEN status_code < 400 THEN 1 END) AS success_count, \
                 COUNT(CASE WHEN status_code >= 400 THEN 1 END) AS error_count \
             FROM a2a_usage \
             WHERE client_id = $1 AND timestamp >= $2 \
             GROUP BY ",
            $day,
            " ORDER BY usage_date DESC"
        )
    };
}
pub(crate) use usage_history_sql;
