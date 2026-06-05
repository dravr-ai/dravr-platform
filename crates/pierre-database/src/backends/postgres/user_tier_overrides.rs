// ABOUTME: PostgreSQL implementation of UserTierOverrideRepository
// ABOUTME: Native UUID + TIMESTAMPTZ; mirrors the SQLite path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserTier;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{UserTierOverride, UserTierOverrideRepository};

fn row_to_override(row: &PgRow) -> AppResult<UserTierOverride> {
    let tier_raw: String = row.get("tier");
    let tier = UserTier::from_str(&tier_raw)
        .map_err(|e| AppError::database(format!("Invalid stored tier '{tier_raw}': {e}")))?;
    Ok(UserTierOverride {
        user_id: row.get("user_id"),
        tier,
        note: row.get("note"),
        set_by: row.get("set_by"),
        set_at: row.get("set_at"),
        updated_at: row.get("updated_at"),
    })
}

#[async_trait]
impl UserTierOverrideRepository for PostgresDatabase {
    async fn get(&self, user_id: Uuid) -> AppResult<Option<UserTierOverride>> {
        let row = sqlx::query(
            r"
            SELECT user_id, tier, note, set_by, set_at, updated_at
            FROM user_tier_overrides
            WHERE user_id = $1
            ",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tier override: {e}")))?;

        row.as_ref().map(row_to_override).transpose()
    }

    async fn upsert(&self, row: &UserTierOverride) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO user_tier_overrides
                (user_id, tier, note, set_by, set_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $5)
            ON CONFLICT (user_id) DO UPDATE SET
                tier = EXCLUDED.tier,
                note = EXCLUDED.note,
                set_by = EXCLUDED.set_by,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(row.user_id)
        .bind(row.tier.as_str())
        .bind(row.note.as_deref())
        .bind(row.set_by)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tier override: {e}")))?;

        Ok(())
    }

    async fn delete(&self, user_id: Uuid) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM user_tier_overrides WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete tier override: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}
