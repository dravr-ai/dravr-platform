// ABOUTME: Repository trait, statements and shared body for the profile document, goals and the user configuration
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id columns need
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The per-user documents, written once.
//!
//! Three small tables keyed by the user: `user_profiles` (one JSON document),
//! `goals` (one JSON document per goal, carrying its own `goal_id`) and
//! `user_configurations` (one JSON text). `user_id` and `goals.id` are `uuid`
//! columns on Postgres and `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec. The JSON documents bind and
//! read as one [`Value`] on both drivers — `jsonb` on Postgres, `TEXT` on
//! `SQLite` — so both hand back a compact re-rendering rather than the
//! caller's bytes (carnet#436's decision: the stored JSON is an opaque blob,
//! and every reader parses it). The configuration stays the caller's text,
//! as both backends always kept it. Timestamps bind as
//! [`Timestamp`](dravr_tronc::dates::Timestamp).

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use serde_json::Value;
use uuid::Uuid;

/// User profiles, goals, and configuration repository
#[async_trait]
pub trait ProfileRepository: Send + Sync {
    /// Upsert user profile data
    async fn upsert_profile(&self, user_id: Uuid, profile_data: Value) -> AppResult<()>;
    /// Get user profile data
    async fn get_profile(&self, user_id: Uuid) -> AppResult<Option<Value>>;
    /// Create a new goal for a user, returning the generated goal id.
    ///
    /// The id is also embedded into the stored `goal_data` under `goal_id`,
    /// so every goal a [`Self::get_goals`] read returns identifies itself —
    /// progress tracking looks goals up by that key.
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> AppResult<String>;
    /// Get all goals for a user
    async fn get_goals(&self, user_id: Uuid) -> AppResult<Vec<Value>>;
    /// Get user configuration data
    async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>>;
    /// Save user configuration data
    async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()>;
}

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

/// Emit the whole [`ProfileRepository`] implementation for one backend type.
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
                    .bind(Timestamp::now())
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
                let goal_uuid = Uuid::new_v4();
                let goal_id = goal_uuid.to_string();
                // The stored JSON carries its own row id under `goal_id`: goal reads
                // return `goal_data` exactly as stored, and progress tracking finds a
                // goal by that key, so the id must live inside the JSON itself.
                if let Some(goal_object) = goal_data.as_object_mut() {
                    goal_object.insert("goal_id".to_owned(), Value::String(goal_id.clone()));
                }

                // `goals.id` is a uuid on Postgres and TEXT on SQLite: it binds
                // through the codec like `user_id`, never as a bare string.
                sqlx::query(CREATE_GOAL_SQL)
                    .bind($ids::bind(goal_uuid))
                    .bind($ids::bind(user_id))
                    .bind(&goal_data)
                    .bind(Timestamp::now())
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
                    .bind(Timestamp::now())
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
