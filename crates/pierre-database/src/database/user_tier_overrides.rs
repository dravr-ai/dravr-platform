// ABOUTME: SQLite CRUD for per-user admin tier overrides (anti-clobber marker)
// ABOUTME: Row presence makes the billing webhook skip set_tier/set_plan for that user
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use crate::database::Database;
use crate::repositories::{UserTierOverride, UserTierOverrideRepository};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserTier;
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

#[async_trait]
impl UserTierOverrideRepository for Database {
    async fn get(&self, user_id: Uuid) -> AppResult<Option<UserTierOverride>> {
        let row = sqlx::query(
            r"
            SELECT user_id, tier, note, set_by, set_at, updated_at
            FROM user_tier_overrides
            WHERE user_id = ?1
            ",
        )
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tier override: {e}")))?;

        let Some(row) = row else { return Ok(None) };

        let user_id_raw: String = row.get("user_id");
        let user_id = Uuid::parse_str(&user_id_raw).map_err(|e| {
            AppError::database(format!("Invalid user_id uuid '{user_id_raw}': {e}"))
        })?;

        let tier_raw: String = row.get("tier");
        let tier = UserTier::from_str(&tier_raw)
            .map_err(|e| AppError::database(format!("Invalid stored tier '{tier_raw}': {e}")))?;

        Ok(Some(UserTierOverride {
            user_id,
            tier,
            note: row.get("note"),
            set_by: parse_opt_uuid(&row, "set_by")?,
            set_at: parse_ts(&row, "set_at")?,
            updated_at: parse_ts(&row, "updated_at")?,
        }))
    }

    async fn upsert(&self, row: &UserTierOverride) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let set_by = row.set_by.map(|u| u.to_string());

        // INSERT preserves the original set_at on conflict by pulling the
        // existing value back in via COALESCE; updated_at always advances.
        sqlx::query(
            r"
            INSERT INTO user_tier_overrides
                (user_id, tier, note, set_by, set_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, COALESCE(
                (SELECT set_at FROM user_tier_overrides WHERE user_id = ?1),
                ?5
            ), ?5)
            ON CONFLICT(user_id) DO UPDATE SET
                tier = excluded.tier,
                note = excluded.note,
                set_by = excluded.set_by,
                updated_at = excluded.updated_at
            ",
        )
        .bind(row.user_id.to_string())
        .bind(row.tier.as_str())
        .bind(row.note.as_deref())
        .bind(set_by)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tier override: {e}")))?;

        Ok(())
    }

    async fn delete(&self, user_id: Uuid) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM user_tier_overrides WHERE user_id = ?1")
            .bind(user_id.to_string())
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to delete tier override: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}
