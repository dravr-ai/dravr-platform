// ABOUTME: SQLite CRUD for per-user rate-limit overrides (exemption pattern)
// ABOUTME: Row presence wins over UserTier::monthly_limit() in compute_user_rate_limits
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::{UserRateLimitOverride, UserRateLimitOverrideRepository};
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

fn parse_opt_uuid(row: &SqliteRow, col: &str) -> AppResult<Option<Uuid>> {
    let raw: Option<String> = row.get(col);
    raw.map(|s| {
        Uuid::parse_str(&s)
            .map_err(|e| AppError::database(format!("Invalid {col} uuid '{s}': {e}")))
    })
    .transpose()
}

fn parse_opt_u32(row: &SqliteRow, col: &str) -> Option<u32> {
    let raw: Option<i64> = row.get(col);
    raw.and_then(|v| u32::try_from(v).ok())
}

#[async_trait]
impl UserRateLimitOverrideRepository for Database {
    async fn get(&self, user_id: Uuid) -> AppResult<Option<UserRateLimitOverride>> {
        let row = sqlx::query(
            r"
            SELECT user_id, daily_limit, monthly_limit, note, set_by, set_at, updated_at
            FROM user_rate_limit_overrides
            WHERE user_id = ?1
            ",
        )
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch rate-limit override: {e}")))?;

        let Some(row) = row else { return Ok(None) };

        let user_id_raw: String = row.get("user_id");
        let user_id = Uuid::parse_str(&user_id_raw).map_err(|e| {
            AppError::database(format!("Invalid user_id uuid '{user_id_raw}': {e}"))
        })?;

        Ok(Some(UserRateLimitOverride {
            user_id,
            daily_limit: parse_opt_u32(&row, "daily_limit"),
            monthly_limit: parse_opt_u32(&row, "monthly_limit"),
            note: row.get("note"),
            set_by: parse_opt_uuid(&row, "set_by")?,
            set_at: parse_ts(&row, "set_at")?,
            updated_at: parse_ts(&row, "updated_at")?,
        }))
    }

    async fn upsert(&self, row: &UserRateLimitOverride) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let daily_limit = row.daily_limit.map(i64::from);
        let monthly_limit = row.monthly_limit.map(i64::from);
        let set_by = row.set_by.map(|u| u.to_string());

        // INSERT OR REPLACE preserves the original set_at on conflict by
        // pulling the existing value back in via COALESCE.
        sqlx::query(
            r"
            INSERT INTO user_rate_limit_overrides
                (user_id, daily_limit, monthly_limit, note, set_by, set_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, COALESCE(
                (SELECT set_at FROM user_rate_limit_overrides WHERE user_id = ?1),
                ?6
            ), ?6)
            ON CONFLICT(user_id) DO UPDATE SET
                daily_limit = excluded.daily_limit,
                monthly_limit = excluded.monthly_limit,
                note = excluded.note,
                set_by = excluded.set_by,
                updated_at = excluded.updated_at
            ",
        )
        .bind(row.user_id.to_string())
        .bind(daily_limit)
        .bind(monthly_limit)
        .bind(row.note.as_deref())
        .bind(set_by)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert rate-limit override: {e}")))?;

        Ok(())
    }

    async fn delete(&self, user_id: Uuid) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM user_rate_limit_overrides WHERE user_id = ?1")
            .bind(user_id.to_string())
            .execute(self.pool())
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to delete rate-limit override: {e}"))
            })?;
        Ok(res.rows_affected() > 0)
    }
}
