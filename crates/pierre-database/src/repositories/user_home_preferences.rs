// ABOUTME: Repository trait, statements and shared body for the per-user choices Home's layout honours on every device
// ABOUTME: One row per user, absent until a choice is made; emitted per backend by macro with that backend's uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Home preferences (carnet#820).
//!
//! What an athlete chose about Home's layout, stored server-side so the web
//! and the phone agree: today, whether they set aside the suggestion to build
//! a training plan. A user with no row has made no choice, and reads as
//! [`HomePreferences::default`].

use std::fmt::Display;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

/// The choices one athlete made about Home.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HomePreferences {
    /// The athlete set aside the suggestion to build a training plan, so
    /// Home stops offering it while they have none. Settings brings it back.
    pub plan_suggestion_hidden: bool,
}

/// Read and write an athlete's Home preferences.
///
/// The table carries no `tenant_id`; every statement is scoped by `user_id`:
/// the choice belongs to the person, whichever tenant they are signed in to.
#[async_trait]
pub trait HomePreferencesRepository: Send + Sync {
    /// The athlete's choices, or the defaults when they have made none.
    async fn get_home_preferences(&self, user_id: Uuid) -> AppResult<HomePreferences>;

    /// Store the athlete's choices, replacing what was there.
    async fn set_home_preferences(&self, user_id: Uuid, prefs: HomePreferences) -> AppResult<()>;
}

/// The athlete's row.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The id binds through the
/// backend's uuid codec (see [`super::uuid_columns`]); the flag binds as a
/// `bool`, which each driver encodes for its own column type.
pub(crate) const GET_HOME_PREFERENCES_SQL: &str = r"
            SELECT plan_suggestion_hidden
            FROM user_home_preferences
            WHERE user_id = $1
            ";

/// Insert the athlete's row, or replace its choices.
pub(crate) const UPSERT_HOME_PREFERENCES_SQL: &str = r"
            INSERT INTO user_home_preferences (user_id, plan_suggestion_hidden, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id) DO UPDATE SET
                plan_suggestion_hidden = EXCLUDED.plan_suggestion_hidden,
                updated_at = EXCLUDED.updated_at
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn home_preferences_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} home preferences: {e}"))
}

/// Emit the whole [`HomePreferencesRepository`] implementation for one
/// backend type. `$ids` is the codec in [`super::uuid_columns`] for how that
/// backend's `user_id` column binds.
macro_rules! impl_home_preferences_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl HomePreferencesRepository for $ty {
            async fn get_home_preferences(&self, user_id: Uuid) -> AppResult<HomePreferences> {
                let row = sqlx::query(GET_HOME_PREFERENCES_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| home_preferences_error("read", e))?;
                let Some(row) = row else {
                    return Ok(HomePreferences::default());
                };
                Ok(HomePreferences {
                    plan_suggestion_hidden: row
                        .try_get("plan_suggestion_hidden")
                        .map_err(|e| home_preferences_error("decode", e))?,
                })
            }

            async fn set_home_preferences(
                &self,
                user_id: Uuid,
                prefs: HomePreferences,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_HOME_PREFERENCES_SQL)
                    .bind($ids::bind(user_id))
                    .bind(prefs.plan_suggestion_hidden)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| home_preferences_error("store", e))?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_home_preferences_repository;
