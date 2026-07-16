// ABOUTME: PostgreSQL implementation of UserToolOverrideRepository
// ABOUTME: Native UUID + BOOLEAN + TIMESTAMPTZ; mirrors the SQLite path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{UserToolOverride, UserToolOverrideRepository};

fn row_to_override(row: &PgRow) -> UserToolOverride {
    UserToolOverride {
        user_id: row.get("user_id"),
        tool_name: row.get("tool_name"),
        is_enabled: row.get("is_enabled"),
        set_by: row.get("set_by"),
        reason: row.get("reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[async_trait]
impl UserToolOverrideRepository for PostgresDatabase {
    async fn get(&self, user_id: Uuid, tool_name: &str) -> AppResult<Option<UserToolOverride>> {
        let row = sqlx::query(
            r"
            SELECT user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at
            FROM user_tool_overrides
            WHERE user_id = $1 AND tool_name = $2
            ",
        )
        .bind(user_id)
        .bind(tool_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tool override: {e}")))?;

        Ok(row.as_ref().map(row_to_override))
    }

    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<UserToolOverride>> {
        let rows = sqlx::query(
            r"
            SELECT user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at
            FROM user_tool_overrides
            WHERE user_id = $1
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list tool overrides: {e}")))?;

        Ok(rows.iter().map(row_to_override).collect())
    }

    async fn upsert(&self, row: &UserToolOverride) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO user_tool_overrides
                (user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            ON CONFLICT (user_id, tool_name) DO UPDATE SET
                is_enabled = EXCLUDED.is_enabled,
                set_by = EXCLUDED.set_by,
                reason = EXCLUDED.reason,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(row.user_id)
        .bind(&row.tool_name)
        .bind(row.is_enabled)
        .bind(row.set_by)
        .bind(row.reason.as_deref())
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tool override: {e}")))?;

        Ok(())
    }

    async fn delete(&self, user_id: Uuid, tool_name: &str) -> AppResult<bool> {
        let res =
            sqlx::query("DELETE FROM user_tool_overrides WHERE user_id = $1 AND tool_name = $2")
                .bind(user_id)
                .bind(tool_name)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to delete tool override: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}
