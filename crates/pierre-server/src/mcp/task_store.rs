// ABOUTME: Durable TaskStore backing the MCP Tasks extension over McpTaskRepository
// ABOUTME: Adapts tronc DetailedTask handles to owner-scoped mcp_tasks rows in both DB backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Durable [`TaskStore`] implementation for the `io.modelcontextprotocol/tasks`
//! extension.
//!
//! The tronc engine ships an in-memory store whose tasks vanish on restart —
//! wrong for a multi-replica Cloud Run service, where the replica answering
//! `tasks/get` need not be the one that minted the handle. This adapter
//! persists every task in the `mcp_tasks` table through
//! [`McpTaskRepository`], keyed by `(tenant_id, user_id)` so the store itself
//! enforces owner isolation: a foreign task id reads as absent, never as
//! forbidden.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dravr_tronc::mcp::tasks::{
    DetailedTask, Task, TaskError, TaskId, TaskOwner, TaskPayload, TaskStatus, TaskStore,
};
use pierre_database::repositories::{McpTaskRepository, McpTaskRow};
use serde_json::{Map, Value};

/// Retention for a task handle: 30 minutes.
///
/// Long enough for the slowest conversion target (a full-season sciotte
/// scrape measured at ~4.5 minutes) plus generous client polling margin,
/// short enough that abandoned handles don't accrete.
pub const MCP_TASK_TTL_MS: u64 = 1_800_000;

/// Polling interval advertised to clients. The converted operations run tens
/// of seconds to minutes, so a 2s cadence keeps polls cheap without adding
/// meaningful latency to completion delivery.
pub const MCP_TASK_POLL_INTERVAL_MS: u64 = 2_000;

/// Wire string for a [`TaskStatus`], matching the extension's `snake_case`
/// vocabulary and the `mcp_tasks.status` CHECK constraint.
const fn status_to_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Working => "working",
        TaskStatus::InputRequired => "input_required",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

/// Parse a stored status string back into a [`TaskStatus`].
fn status_from_str(status: &str) -> Result<TaskStatus, TaskError> {
    match status {
        "working" => Ok(TaskStatus::Working),
        "input_required" => Ok(TaskStatus::InputRequired),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(TaskError::Store(format!(
            "mcp_tasks row holds unknown status '{other}'"
        ))),
    }
}

/// The `(input_requests, result, error)` columns of one row — at most one is
/// populated, selected by the row's status.
type PayloadColumns = (Option<String>, Option<String>, Option<String>);

/// Serialize a payload's JSON object column, or `None` for payload-free states.
fn payload_columns(payload: &TaskPayload) -> Result<PayloadColumns, TaskError> {
    let encode = |map: &Map<String, Value>| {
        serde_json::to_string(map)
            .map_err(|e| TaskError::Store(format!("failed to encode task payload: {e}")))
    };
    Ok(match payload {
        TaskPayload::Working | TaskPayload::Cancelled => (None, None, None),
        TaskPayload::InputRequired { input_requests } => {
            (Some(encode(input_requests)?), None, None)
        }
        TaskPayload::Completed { result } => (None, Some(encode(result)?), None),
        TaskPayload::Failed { error } => (None, None, Some(encode(error)?)),
    })
}

/// Decode a stored JSON object column that the row's status requires.
fn decode_column(column: Option<&str>, what: &str) -> Result<Map<String, Value>, TaskError> {
    let raw = column
        .ok_or_else(|| TaskError::Store(format!("mcp_tasks row is missing its {what} payload")))?;
    serde_json::from_str(raw)
        .map_err(|e| TaskError::Store(format!("mcp_tasks row holds invalid {what} JSON: {e}")))
}

/// Resolve the owner's identifiers, refusing an unauthenticated owner.
///
/// Every pierre tool call is authenticated before dispatch, so a missing
/// identity here is a wiring bug, not a legitimate anonymous task.
fn owner_ids(owner: &TaskOwner) -> Result<(&str, &str), TaskError> {
    match (owner.tenant_id.as_deref(), owner.user_id.as_deref()) {
        (Some(tenant_id), Some(user_id)) => Ok((tenant_id, user_id)),
        _ => Err(TaskError::Store(
            "MCP task owner must carry both tenant and user identity".to_owned(),
        )),
    }
}

/// Precompute the unix-millisecond expiry from the task's creation time + TTL.
fn expires_at_ms(task: &Task) -> Option<i64> {
    let ttl_ms = i64::try_from(task.ttl_ms?).ok()?;
    let created = DateTime::parse_from_rfc3339(&task.created_at).ok()?;
    created
        .with_timezone(&Utc)
        .timestamp_millis()
        .checked_add(ttl_ms)
}

/// Map a [`DetailedTask`] onto its `mcp_tasks` row.
fn task_to_row(
    owner_tenant: &str,
    owner_user: &str,
    task: &DetailedTask,
) -> Result<McpTaskRow, TaskError> {
    let (input_requests, result, error) = payload_columns(&task.payload)?;
    Ok(McpTaskRow {
        task_id: task.task.task_id.as_str().to_owned(),
        tenant_id: owner_tenant.to_owned(),
        user_id: owner_user.to_owned(),
        status: status_to_str(task.status()).to_owned(),
        status_message: task.task.status_message.clone(),
        created_at: task.task.created_at.clone(),
        last_updated_at: task.task.last_updated_at.clone(),
        ttl_ms: task.task.ttl_ms.and_then(|v| i64::try_from(v).ok()),
        poll_interval_ms: task
            .task
            .poll_interval_ms
            .and_then(|v| i64::try_from(v).ok()),
        expires_at_ms: expires_at_ms(&task.task),
        input_requests,
        result,
        error,
    })
}

/// Rehydrate a [`DetailedTask`] from its `mcp_tasks` row.
fn row_to_task(row: &McpTaskRow) -> Result<DetailedTask, TaskError> {
    let status = status_from_str(&row.status)?;
    let payload = match status {
        TaskStatus::Working => TaskPayload::Working,
        TaskStatus::Cancelled => TaskPayload::Cancelled,
        TaskStatus::InputRequired => TaskPayload::InputRequired {
            input_requests: decode_column(row.input_requests.as_deref(), "input_requests")?,
        },
        TaskStatus::Completed => TaskPayload::Completed {
            result: decode_column(row.result.as_deref(), "result")?,
        },
        TaskStatus::Failed => TaskPayload::Failed {
            error: decode_column(row.error.as_deref(), "error")?,
        },
    };
    let task = Task {
        task_id: TaskId::new(row.task_id.clone()),
        status,
        status_message: row.status_message.clone(),
        created_at: row.created_at.clone(),
        last_updated_at: row.last_updated_at.clone(),
        ttl_ms: row.ttl_ms.and_then(|v| u64::try_from(v).ok()),
        poll_interval_ms: row.poll_interval_ms.and_then(|v| u64::try_from(v).ok()),
    };
    Ok(DetailedTask::new(task, payload))
}

/// Durable, owner-scoped [`TaskStore`] over the `mcp_tasks` table.
pub struct PierreTaskStore {
    /// Shared task repository. Arc because the store itself is shared behind
    /// `Arc<dyn TaskStore>` across the async engine and completion followers.
    repo: Arc<dyn McpTaskRepository>,
}

impl PierreTaskStore {
    /// Build the store over the platform's task repository.
    #[must_use]
    pub fn new(repo: Arc<dyn McpTaskRepository>) -> Self {
        Self { repo }
    }

    async fn write(&self, owner: &TaskOwner, task: &DetailedTask) -> Result<(), TaskError> {
        let (tenant_id, user_id) = owner_ids(owner)?;
        let row = task_to_row(tenant_id, user_id, task)?;
        self.repo
            .upsert_task(&row)
            .await
            .map_err(|e| TaskError::Store(e.to_string()))
    }
}

#[async_trait]
impl TaskStore for PierreTaskStore {
    async fn create(&self, owner: &TaskOwner, task: DetailedTask) -> Result<(), TaskError> {
        self.write(owner, &task).await
    }

    async fn get(&self, owner: &TaskOwner, id: &TaskId) -> Result<Option<DetailedTask>, TaskError> {
        // An unauthenticated owner can own no tasks, so every id reads as
        // absent for it — same shape as a foreign task id.
        let Ok((tenant_id, user_id)) = owner_ids(owner) else {
            return Ok(None);
        };
        let row = self
            .repo
            .get_task(
                tenant_id,
                user_id,
                id.as_str(),
                Utc::now().timestamp_millis(),
            )
            .await
            .map_err(|e| TaskError::Store(e.to_string()))?;
        row.as_ref().map(row_to_task).transpose()
    }

    async fn put(&self, owner: &TaskOwner, task: DetailedTask) -> Result<(), TaskError> {
        self.write(owner, &task).await
    }

    async fn sweep_expired(&self) -> Result<usize, TaskError> {
        let removed = self
            .repo
            .delete_expired_tasks(Utc::now().timestamp_millis())
            .await
            .map_err(|e| TaskError::Store(e.to_string()))?;
        Ok(usize::try_from(removed).unwrap_or(usize::MAX))
    }

    /// Every non-terminal task this owner can see.
    ///
    /// Without this override the trait's default returns an empty list and
    /// `subscriptions/listen` opens successfully but never emits — a silent
    /// success no `is_ok()` assertion can catch. Terminality is read from
    /// [`TaskStatus::is_terminal`] rather than a status list written into SQL,
    /// so a new variant in the engine cannot leave a stale predicate behind:
    /// the watcher chases ids that leave this set to their terminal state
    /// through `get`, which is how a completion reaches a subscriber at all.
    async fn active_tasks(&self, owner: &TaskOwner) -> Result<Vec<DetailedTask>, TaskError> {
        // An unauthenticated owner owns nothing, so it watches nothing.
        let Ok((tenant_id, user_id)) = owner_ids(owner) else {
            return Ok(Vec::new());
        };
        let rows = self
            .repo
            .active_tasks(tenant_id, user_id, Utc::now().timestamp_millis())
            .await
            .map_err(|e| TaskError::Store(e.to_string()))?;

        // Decode first and filter second: filtering on `is_ok_and` would drop
        // an undecodable row as if it were terminal, turning stored corruption
        // into a silently shorter list.
        let tasks = rows
            .iter()
            .map(row_to_task)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tasks
            .into_iter()
            .filter(|task| !task.task.status.is_terminal())
            .collect())
    }
}
