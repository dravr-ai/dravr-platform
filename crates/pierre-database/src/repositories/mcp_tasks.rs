// ABOUTME: Repository trait for MCP Tasks extension handles (io.modelcontextprotocol/tasks)
// ABOUTME: Owner-scoped task rows backing the tronc TaskStore seam with durable persistence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};

/// One durable MCP task handle, stored exactly as the engine shaped it.
///
/// `created_at` / `last_updated_at` are the RFC3339 strings the engine minted
/// (never re-parsed into a timestamp column) so the wire `Task` round-trips
/// byte-identical. `status` uses the extension's `snake_case` vocabulary
/// (`working` / `input_required` / `completed` / `failed` / `cancelled`), and
/// exactly one of `input_requests` / `result` / `error` is populated for the
/// statuses that carry a payload. `expires_at_ms` is the precomputed
/// unix-millisecond expiry (`None` = unlimited retention).
#[derive(Debug, Clone)]
pub struct McpTaskRow {
    /// Server-minted opaque task identifier.
    pub task_id: String,
    /// Tenant the task belongs to — every lookup filters on it.
    pub tenant_id: String,
    /// User the task belongs to — every lookup filters on it.
    pub user_id: String,
    /// Lifecycle state, `snake_case` wire vocabulary.
    pub status: String,
    /// Optional human-readable description of the current state.
    pub status_message: Option<String>,
    /// ISO 8601 creation timestamp, as minted by the engine.
    pub created_at: String,
    /// ISO 8601 timestamp of the most recent state change.
    pub last_updated_at: String,
    /// Lifetime from `created_at` in milliseconds; `None` = unlimited.
    pub ttl_ms: Option<i64>,
    /// Polling interval advertised to the client, in milliseconds.
    pub poll_interval_ms: Option<i64>,
    /// Precomputed expiry in unix milliseconds; `None` = never expires.
    pub expires_at_ms: Option<i64>,
    /// JSON object of outstanding input requests (`input_required` only).
    pub input_requests: Option<String>,
    /// JSON object holding the terminal result (`completed` only).
    pub result: Option<String>,
    /// JSON object holding the JSON-RPC error (`failed` only).
    pub error: Option<String>,
}

/// Persistence for MCP task handles.
///
/// Owner scoping is enforced here, not by callers: `get_task` matches
/// `(tenant_id, user_id, task_id)` together, so a task id guessed or leaked
/// across tenants reads as absent. `upsert_task` refuses to overwrite a row
/// owned by someone else for the same reason.
#[async_trait]
pub trait McpTaskRepository: Send + Sync {
    /// Insert a task row, or overwrite it when the owner matches.
    async fn upsert_task(&self, row: &McpTaskRow) -> AppResult<()>;

    /// Fetch a task visible to the owner and not expired at `now_ms`
    /// (unix milliseconds), or `None` when absent, foreign, or expired.
    async fn get_task(
        &self,
        tenant_id: &str,
        user_id: &str,
        task_id: &str,
        now_ms: i64,
    ) -> AppResult<Option<McpTaskRow>>;

    /// List an owner's unexpired tasks at `now_ms` (unix milliseconds).
    ///
    /// Scoped by `(tenant_id, user_id)` like [`Self::get_task`], so it can
    /// never surface another owner's work. Ordered by `task_id` so a caller
    /// diffing successive snapshots sees a stable sequence rather than
    /// whatever order the backend happens to return.
    async fn active_tasks(
        &self,
        tenant_id: &str,
        user_id: &str,
        now_ms: i64,
    ) -> AppResult<Vec<McpTaskRow>>;

    /// Delete tasks whose expiry has passed at `now_ms` (unix milliseconds),
    /// returning how many rows were removed.
    async fn delete_expired_tasks(&self, now_ms: i64) -> AppResult<u64>;
}

/// The thirteen columns every read of `mcp_tasks` returns, in the order
/// [`task_from_row`] reads them. One list, so a column added to
/// [`McpTaskRow`] reaches every statement at once.
macro_rules! task_columns {
    () => {
        "task_id, tenant_id, user_id, status, status_message, \
         created_at, last_updated_at, ttl_ms, poll_interval_ms, \
         expires_at_ms, input_requests, result, error"
    };
}

/// Record or advance a task handle.
///
/// The conflict update is owner-guarded: a row owned by a different
/// (tenant, user) is left untouched and the zero-row result surfaces as an
/// error, so an id collision can never hand one owner's task state to
/// another.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind on this table is a plain `&str`/`Option<String>`/
/// `i64`, so one statement serves both backends and cannot drift between them.
pub(crate) const UPSERT_TASK_SQL: &str = concat!(
    "INSERT INTO mcp_tasks (",
    task_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
     ON CONFLICT (task_id) DO UPDATE SET \
        status = excluded.status, \
        status_message = excluded.status_message, \
        last_updated_at = excluded.last_updated_at, \
        ttl_ms = excluded.ttl_ms, \
        poll_interval_ms = excluded.poll_interval_ms, \
        expires_at_ms = excluded.expires_at_ms, \
        input_requests = excluded.input_requests, \
        result = excluded.result, \
        error = excluded.error \
     WHERE mcp_tasks.tenant_id = excluded.tenant_id \
       AND mcp_tasks.user_id = excluded.user_id"
);

/// One owner's task by id, hidden once its TTL has elapsed.
pub(crate) const GET_TASK_SQL: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM mcp_tasks \
     WHERE task_id = $1 AND tenant_id = $2 AND user_id = $3 \
       AND (expires_at_ms IS NULL OR expires_at_ms >= $4)"
);

/// Every unexpired task one owner holds.
pub(crate) const ACTIVE_TASKS_SQL: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM mcp_tasks \
     WHERE tenant_id = $1 AND user_id = $2 \
       AND (expires_at_ms IS NULL OR expires_at_ms >= $3) \
     ORDER BY task_id"
);

/// Storage hygiene: drop every handle whose TTL has elapsed.
pub(crate) const SWEEP_EXPIRED_TASKS_SQL: &str =
    "DELETE FROM mcp_tasks WHERE expires_at_ms IS NOT NULL AND expires_at_ms < $1";

/// Extract an [`McpTaskRow`] from a row of either backend via `try_get` only —
/// `Row::get` is `try_get().unwrap()` and panics the whole read path on a
/// width or NULL surprise.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn task_from_row<R>(row: &R) -> AppResult<McpTaskRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("mcp_tasks {name}: {e}")))
    };
    let opt = |name: &str| -> AppResult<Option<String>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("mcp_tasks {name}: {e}")))
    };
    let millis = |name: &str| -> AppResult<Option<i64>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("mcp_tasks {name}: {e}")))
    };
    Ok(McpTaskRow {
        task_id: col("task_id")?,
        tenant_id: col("tenant_id")?,
        user_id: col("user_id")?,
        status: col("status")?,
        status_message: opt("status_message")?,
        created_at: col("created_at")?,
        last_updated_at: col("last_updated_at")?,
        ttl_ms: millis("ttl_ms")?,
        poll_interval_ms: millis("poll_interval_ms")?,
        expires_at_ms: millis("expires_at_ms")?,
        input_requests: opt("input_requests")?,
        result: opt("result")?,
        error: opt("error")?,
    })
}

/// Emit the whole [`McpTaskRepository`] implementation for one backend type.
/// The body is written once here; each backend's shell invokes it with its own
/// type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_mcp_task_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl McpTaskRepository for $ty {
            async fn upsert_task(&self, row: &McpTaskRow) -> AppResult<()> {
                let outcome = sqlx::query(UPSERT_TASK_SQL)
                    .bind(&row.task_id)
                    .bind(&row.tenant_id)
                    .bind(&row.user_id)
                    .bind(&row.status)
                    .bind(&row.status_message)
                    .bind(&row.created_at)
                    .bind(&row.last_updated_at)
                    .bind(row.ttl_ms)
                    .bind(row.poll_interval_ms)
                    .bind(row.expires_at_ms)
                    .bind(&row.input_requests)
                    .bind(&row.result)
                    .bind(&row.error)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to upsert MCP task: {e}")))?;

                if outcome.rows_affected() == 0 {
                    return Err(AppError::database(format!(
                        "MCP task '{}' exists under a different owner",
                        row.task_id
                    )));
                }
                Ok(())
            }

            async fn get_task(
                &self,
                tenant_id: &str,
                user_id: &str,
                task_id: &str,
                now_ms: i64,
            ) -> AppResult<Option<McpTaskRow>> {
                let row = sqlx::query(GET_TASK_SQL)
                    .bind(task_id)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(now_ms)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to fetch MCP task: {e}")))?;
                row.as_ref().map(task_from_row).transpose()
            }

            async fn active_tasks(
                &self,
                tenant_id: &str,
                user_id: &str,
                now_ms: i64,
            ) -> AppResult<Vec<McpTaskRow>> {
                let rows = sqlx::query(ACTIVE_TASKS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(now_ms)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list MCP tasks: {e}")))?;
                rows.iter().map(task_from_row).collect()
            }

            async fn delete_expired_tasks(&self, now_ms: i64) -> AppResult<u64> {
                let outcome = sqlx::query(SWEEP_EXPIRED_TASKS_SQL)
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to sweep expired MCP tasks: {e}"))
                    })?;
                Ok(outcome.rows_affected())
            }
        }
    };
}
pub(crate) use impl_mcp_task_repository;
