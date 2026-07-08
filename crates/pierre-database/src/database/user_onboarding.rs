// ABOUTME: SQLite-backed UserOnboardingRepository — durable per-user onboarding step completion state
// ABOUTME: All-TEXT columns bound as strings; upsert on (user_id, step_id), read scoped by user_id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::{OnboardingStepRecord, UserOnboardingRepository};
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

/// Decode a row via `try_get` (never `Row::get`, which is `try_get().unwrap()` and
/// panics on a type/NULL surprise) so a corrupt row is a recoverable error.
fn sqlite_step_record(row: &SqliteRow) -> AppResult<OnboardingStepRecord> {
    Ok(OnboardingStepRecord {
        step_id: row
            .try_get::<String, _>("step_id")
            .map_err(|e| AppError::database(format!("user_onboarding step_id: {e}")))?,
        status: row
            .try_get::<String, _>("status")
            .map_err(|e| AppError::database(format!("user_onboarding status: {e}")))?,
        chosen_channel: row
            .try_get::<Option<String>, _>("chosen_channel")
            .map_err(|e| AppError::database(format!("user_onboarding chosen_channel: {e}")))?,
    })
}

#[async_trait]
impl UserOnboardingRepository for Database {
    async fn set_onboarding_step(
        &self,
        user_id: &str,
        step_id: &str,
        status: &str,
        chosen_channel: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO user_onboarding (user_id, step_id, status, chosen_channel, tenant_id, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
            ON CONFLICT(user_id, step_id) DO UPDATE SET
                status = excluded.status,
                chosen_channel = excluded.chosen_channel,
                tenant_id = excluded.tenant_id,
                updated_at = datetime('now')
            ",
        )
        .bind(user_id)
        .bind(step_id)
        .bind(status)
        .bind(chosen_channel)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert user_onboarding: {e}")))?;

        Ok(())
    }

    async fn get_onboarding_steps(&self, user_id: &str) -> AppResult<Vec<OnboardingStepRecord>> {
        let rows = sqlx::query(
            r"
            SELECT step_id, status, chosen_channel
            FROM user_onboarding
            WHERE user_id = ?1
            ",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to read user_onboarding: {e}")))?;

        rows.iter().map(sqlite_step_record).collect()
    }
}
