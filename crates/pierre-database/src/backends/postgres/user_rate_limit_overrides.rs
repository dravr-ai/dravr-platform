// ABOUTME: PostgreSQL implementation of UserRateLimitOverrideRepository
// ABOUTME: Native UUID + TIMESTAMPTZ; mirrors the SQLite path
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
use crate::repositories::{UserRateLimitOverride, UserRateLimitOverrideRepository};

fn row_to_override(row: &PgRow) -> UserRateLimitOverride {
    let daily_raw: Option<i32> = row.get("daily_limit");
    let monthly_raw: Option<i32> = row.get("monthly_limit");
    UserRateLimitOverride {
        user_id: row.get("user_id"),
        daily_limit: daily_raw.and_then(|v| u32::try_from(v).ok()),
        monthly_limit: monthly_raw.and_then(|v| u32::try_from(v).ok()),
        note: row.get("note"),
        set_by: row.get("set_by"),
        set_at: row.get("set_at"),
        updated_at: row.get("updated_at"),
    }
}

#[async_trait]
impl UserRateLimitOverrideRepository for PostgresDatabase {
    async fn get(&self, user_id: Uuid) -> AppResult<Option<UserRateLimitOverride>> {
        let row = sqlx::query(
            r"
            SELECT user_id, daily_limit, monthly_limit, note, set_by, set_at, updated_at
            FROM user_rate_limit_overrides
            WHERE user_id = $1
            ",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch rate-limit override: {e}")))?;

        Ok(row.as_ref().map(row_to_override))
    }

    async fn upsert(&self, row: &UserRateLimitOverride) -> AppResult<()> {
        let now = Utc::now();
        let daily_limit = row
            .daily_limit
            .map(|v| i32::try_from(v).unwrap_or(i32::MAX));
        let monthly_limit = row
            .monthly_limit
            .map(|v| i32::try_from(v).unwrap_or(i32::MAX));

        sqlx::query(
            r"
            INSERT INTO user_rate_limit_overrides
                (user_id, daily_limit, monthly_limit, note, set_by, set_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            ON CONFLICT (user_id) DO UPDATE SET
                daily_limit = EXCLUDED.daily_limit,
                monthly_limit = EXCLUDED.monthly_limit,
                note = EXCLUDED.note,
                set_by = EXCLUDED.set_by,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(row.user_id)
        .bind(daily_limit)
        .bind(monthly_limit)
        .bind(row.note.as_deref())
        .bind(row.set_by)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert rate-limit override: {e}")))?;

        Ok(())
    }

    async fn delete(&self, user_id: Uuid) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM user_rate_limit_overrides WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to delete rate-limit override: {e}"))
            })?;
        Ok(res.rows_affected() > 0)
    }
}
