// ABOUTME: Shared statements and body for ProfileRepository — the profile document, goals and the user configuration
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id columns need
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The per-user documents, written once.
//!
//! Three small tables keyed by the user: `user_profiles` (one JSON document),
//! `goals` (one JSON document per goal, carrying its own `goal_id`) and
//! `user_configurations` (one JSON text). `user_id` is a `uuid` column on
//! Postgres and `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec. The JSON documents bind and
//! read as one [`Value`] on both drivers — `jsonb` on Postgres, `TEXT` on
//! `SQLite` — so both hand back a compact re-rendering rather than the
//! caller's bytes (carnet#436's decision: the stored JSON is an opaque blob,
//! and every reader parses it). The configuration stays the caller's text,
//! as both backends always kept it. Timestamps bind as
//! [`DateTime<Utc>`](chrono::DateTime).

use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// Store or replace the user's profile document.
pub(crate) const UPSERT_USER_PROFILE_SQL: &str = r"
            INSERT INTO user_profiles (user_id, profile_data, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT (user_id)
            DO UPDATE SET profile_data = $2, updated_at = $3
            ";

/// The user's profile document.
pub(crate) const GET_USER_PROFILE_SQL: &str =
    "SELECT profile_data FROM user_profiles WHERE user_id = $1";

/// Record a goal.
pub(crate) const CREATE_GOAL_SQL: &str = r"
            INSERT INTO goals (id, user_id, goal_data, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $4)
            ";

/// Every goal of the user, newest first.
pub(crate) const GET_USER_GOALS_SQL: &str = r"
            SELECT goal_data
            FROM goals
            WHERE user_id = $1
            ORDER BY created_at DESC
            ";

/// One of the user's goals, for a read-modify-write of its progress.
pub(crate) const GET_USER_GOAL_SQL: &str =
    "SELECT goal_data FROM goals WHERE id = $1 AND user_id = $2";

/// Write the goal document back after its progress moved.
pub(crate) const UPDATE_USER_GOAL_SQL: &str = r"
            UPDATE goals
            SET goal_data = $1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $2 AND user_id = $3
            ";

/// The user's configuration text.
pub(crate) const GET_USER_CONFIGURATION_SQL: &str =
    "SELECT config_data FROM user_configurations WHERE user_id = $1";

/// Store or replace the user's configuration text.
pub(crate) const SAVE_USER_CONFIGURATION_SQL: &str = r"
            INSERT INTO user_configurations (user_id, config_data, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT(user_id) DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = $3
            ";

/// Write `current_value`, `last_updated`, and — when the goal carries a
/// positive `target_value` — a clamped `progress_percentage` into the goal
/// document: the derived fields goal reads hand back beside what the caller
/// stored.
///
/// # Errors
/// Returns an internal error when a value is not representable as JSON.
pub(crate) fn apply_progress_fields(goal_data: &mut Value, current_value: f64) -> AppResult<()> {
    let Some(obj) = goal_data.as_object_mut() else {
        return Ok(());
    };
    obj.insert(
        "current_value".into(),
        Value::Number(serde_json::Number::from_f64(current_value).ok_or_else(|| {
            AppError::internal(format!("Invalid current_value: {current_value}"))
        })?),
    );
    obj.insert(
        "last_updated".into(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    if let Some(target) = obj.get("target_value").and_then(Value::as_f64) {
        if target > 0.0 {
            let progress_percentage = (current_value / target * 100.0).clamp(0.0, 100.0);
            obj.insert(
                "progress_percentage".into(),
                Value::Number(
                    serde_json::Number::from_f64(progress_percentage).ok_or_else(|| {
                        AppError::internal(format!(
                            "Invalid progress_percentage: {progress_percentage}"
                        ))
                    })?,
                ),
            );
        }
    }
    Ok(())
}

/// Emit the whole [`ProfileRepository`](super::ProfileRepository)
/// implementation for one backend type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec,
/// which spells how the `user_id` columns bind. The body is written once
/// here; each backend's shell invokes it with its own type, and sqlx resolves
/// the driver from `self.pool()` per expansion.
macro_rules! impl_profile_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl ProfileRepository for $ty {
            async fn upsert_profile(&self, user_id: Uuid, profile_data: Value) -> AppResult<()> {
                sqlx::query(UPSERT_USER_PROFILE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(&profile_data)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert user profile: {e}"))
                    })?;
                Ok(())
            }

            async fn get_profile(&self, user_id: Uuid) -> AppResult<Option<Value>> {
                let row = sqlx::query(GET_USER_PROFILE_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user profile: {e}")))?;
                row.map(|r| {
                    r.try_get::<Value, _>("profile_data")
                        .map_err(|e| AppError::database(format!("Failed to get profile_data: {e}")))
                })
                .transpose()
            }

            async fn create_goal(&self, user_id: Uuid, mut goal_data: Value) -> AppResult<String> {
                let goal_id = Uuid::new_v4().to_string();
                // The stored JSON carries its own row id under `goal_id`: goal reads
                // return `goal_data` exactly as stored, and progress tracking finds a
                // goal by that key, so the id must live inside the JSON itself.
                if let Some(goal_object) = goal_data.as_object_mut() {
                    goal_object.insert("goal_id".to_owned(), Value::String(goal_id.clone()));
                }

                sqlx::query(CREATE_GOAL_SQL)
                    .bind(&goal_id)
                    .bind($ids::bind(user_id))
                    .bind(&goal_data)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create goal: {e}")))?;

                Ok(goal_id)
            }

            async fn get_goals(&self, user_id: Uuid) -> AppResult<Vec<Value>> {
                let rows = sqlx::query(GET_USER_GOALS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user goals: {e}")))?;
                rows.iter()
                    .map(|row| {
                        row.try_get::<Value, _>("goal_data").map_err(|e| {
                            AppError::database(format!("Failed to get goal_data: {e}"))
                        })
                    })
                    .collect()
            }

            async fn update_goal_progress(
                &self,
                goal_id: &str,
                user_id: Uuid,
                current_value: f64,
            ) -> AppResult<()> {
                let row = sqlx::query(GET_USER_GOAL_SQL)
                    .bind(goal_id)
                    .bind($ids::bind(user_id))
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get goal data for update: {e}"))
                    })?;
                let mut goal_data: Value = row
                    .try_get("goal_data")
                    .map_err(|e| AppError::database(format!("Failed to get goal_data: {e}")))?;
                apply_progress_fields(&mut goal_data, current_value)?;

                sqlx::query(UPDATE_USER_GOAL_SQL)
                    .bind(&goal_data)
                    .bind(goal_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update goal progress: {e}"))
                    })?;

                Ok(())
            }

            async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>> {
                let row = sqlx::query(GET_USER_CONFIGURATION_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get user configuration: {e}"))
                    })?;
                row.map(|r| {
                    r.try_get::<String, _>("config_data")
                        .map_err(|e| AppError::database(format!("Failed to get config_data: {e}")))
                })
                .transpose()
            }

            async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()> {
                sqlx::query(SAVE_USER_CONFIGURATION_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(config_json)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to save user configuration: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_profile_repository;
