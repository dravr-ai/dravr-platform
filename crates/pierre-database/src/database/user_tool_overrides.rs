// ABOUTME: SQLite CRUD for per-user admin tool overrides (allow/deny one tool for one user)
// ABOUTME: Overlay consulted by ToolSelectionService above tenant plan + tenant overrides
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::{UserToolOverride, UserToolOverrideRepository};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

fn parse_ts(row: &SqliteRow, col: &str) -> AppResult<DateTime<Utc>> {
    let raw: String = row.get(col);
    DateTime::parse_from_rfc3339(&raw)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("Invalid {col} timestamp '{raw}': {e}")))
}

fn row_to_override(row: &SqliteRow) -> AppResult<UserToolOverride> {
    let user_id_raw: String = row.get("user_id");
    let user_id = Uuid::parse_str(&user_id_raw)
        .map_err(|e| AppError::database(format!("Invalid user_id uuid '{user_id_raw}': {e}")))?;

    let set_by = match row.get::<Option<String>, _>("set_by") {
        Some(s) => Some(
            Uuid::parse_str(&s)
                .map_err(|e| AppError::database(format!("Invalid set_by uuid '{s}': {e}")))?,
        ),
        None => None,
    };

    Ok(UserToolOverride {
        user_id,
        tool_name: row.get("tool_name"),
        is_enabled: row.get("is_enabled"),
        set_by,
        reason: row.get("reason"),
        created_at: parse_ts(row, "created_at")?,
        updated_at: parse_ts(row, "updated_at")?,
    })
}

#[async_trait]
impl UserToolOverrideRepository for Database {
    async fn get(&self, user_id: Uuid, tool_name: &str) -> AppResult<Option<UserToolOverride>> {
        let row = sqlx::query(
            r"
            SELECT user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at
            FROM user_tool_overrides
            WHERE user_id = ?1 AND tool_name = ?2
            ",
        )
        .bind(user_id.to_string())
        .bind(tool_name)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tool override: {e}")))?;

        row.as_ref().map(row_to_override).transpose()
    }

    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<UserToolOverride>> {
        let rows = sqlx::query(
            r"
            SELECT user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at
            FROM user_tool_overrides
            WHERE user_id = ?1
            ",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list tool overrides: {e}")))?;

        rows.iter().map(row_to_override).collect()
    }

    async fn upsert(&self, row: &UserToolOverride) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let set_by = row.set_by.map(|u| u.to_string());

        // INSERT preserves the original created_at on conflict via COALESCE;
        // updated_at always advances to the call time.
        sqlx::query(
            r"
            INSERT INTO user_tool_overrides
                (user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, COALESCE(
                (SELECT created_at FROM user_tool_overrides
                 WHERE user_id = ?1 AND tool_name = ?2),
                ?6
            ), ?6)
            ON CONFLICT(user_id, tool_name) DO UPDATE SET
                is_enabled = excluded.is_enabled,
                set_by = excluded.set_by,
                reason = excluded.reason,
                updated_at = excluded.updated_at
            ",
        )
        .bind(row.user_id.to_string())
        .bind(&row.tool_name)
        .bind(row.is_enabled)
        .bind(set_by)
        .bind(row.reason.as_deref())
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tool override: {e}")))?;

        Ok(())
    }

    async fn delete(&self, user_id: Uuid, tool_name: &str) -> AppResult<bool> {
        let res =
            sqlx::query("DELETE FROM user_tool_overrides WHERE user_id = ?1 AND tool_name = ?2")
                .bind(user_id.to_string())
                .bind(tool_name)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to delete tool override: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}
