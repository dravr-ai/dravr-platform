// ABOUTME: SQLite implementation of the McpTaskRepository trait
// ABOUTME: Persists MCP Tasks extension handles with owner-scoped lookups and TTL expiry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Database;
use crate::repositories::{McpTaskRepository, McpTaskRow};

fn row_to_task(row: &SqliteRow) -> McpTaskRow {
    McpTaskRow {
        task_id: row.get("task_id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        status: row.get("status"),
        status_message: row.get("status_message"),
        created_at: row.get("created_at"),
        last_updated_at: row.get("last_updated_at"),
        ttl_ms: row.get("ttl_ms"),
        poll_interval_ms: row.get("poll_interval_ms"),
        expires_at_ms: row.get("expires_at_ms"),
        input_requests: row.get("input_requests"),
        result: row.get("result"),
        error: row.get("error"),
    }
}

#[async_trait]
impl McpTaskRepository for Database {
    async fn upsert_task(&self, row: &McpTaskRow) -> AppResult<()> {
        // The conflict update is owner-guarded: a row owned by a different
        // (tenant, user) is left untouched and the zero-row result surfaces as
        // an error, so an id collision can never hand one owner's task state
        // to another.
        let outcome = sqlx::query(
            r"
            INSERT INTO mcp_tasks (
                task_id, tenant_id, user_id, status, status_message,
                created_at, last_updated_at, ttl_ms, poll_interval_ms,
                expires_at_ms, input_requests, result, error
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT(task_id) DO UPDATE SET
                status = excluded.status,
                status_message = excluded.status_message,
                last_updated_at = excluded.last_updated_at,
                ttl_ms = excluded.ttl_ms,
                poll_interval_ms = excluded.poll_interval_ms,
                expires_at_ms = excluded.expires_at_ms,
                input_requests = excluded.input_requests,
                result = excluded.result,
                error = excluded.error
            WHERE mcp_tasks.tenant_id = excluded.tenant_id
              AND mcp_tasks.user_id = excluded.user_id
            ",
        )
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
        .execute(&self.pool)
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
        let row = sqlx::query(
            r"
            SELECT task_id, tenant_id, user_id, status, status_message,
                   created_at, last_updated_at, ttl_ms, poll_interval_ms,
                   expires_at_ms, input_requests, result, error
            FROM mcp_tasks
            WHERE task_id = $1 AND tenant_id = $2 AND user_id = $3
              AND (expires_at_ms IS NULL OR expires_at_ms >= $4)
            ",
        )
        .bind(task_id)
        .bind(tenant_id)
        .bind(user_id)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch MCP task: {e}")))?;

        Ok(row.as_ref().map(row_to_task))
    }

    async fn delete_expired_tasks(&self, now_ms: i64) -> AppResult<u64> {
        let outcome = sqlx::query(
            "DELETE FROM mcp_tasks WHERE expires_at_ms IS NOT NULL AND expires_at_ms < $1",
        )
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to sweep expired MCP tasks: {e}")))?;
        Ok(outcome.rows_affected())
    }
}
