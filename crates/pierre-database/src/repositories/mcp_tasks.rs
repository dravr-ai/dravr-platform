// ABOUTME: Repository trait for MCP Tasks extension handles (io.modelcontextprotocol/tasks)
// ABOUTME: Owner-scoped task rows backing the tronc TaskStore seam with durable persistence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

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
