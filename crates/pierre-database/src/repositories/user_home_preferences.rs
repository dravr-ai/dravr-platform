// ABOUTME: Repository trait, statements and shared body for the per-user choices Home's layout honours on every device
// ABOUTME: One row per user, absent until a choice is made; emitted per backend by macro with that backend's uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Home preferences (carnet#820).
//!
//! What an athlete chose about Home's layout, stored server-side so the web
//! and the phone agree: whether they set aside the suggestion to build a
//! training plan, and whether they hid Getting started (carnet#1098). A user
//! with no row has made no choice, and reads as [`HomePreferences::default`].
//!
//! A write is a merge: a field the caller leaves `None` keeps what is stored,
//! so a client that predates a field cannot reset it by writing the others.

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
    /// The athlete hid Getting started, the first-steps list at the top of
    /// Today. Settings brings it back.
    pub getting_started_hidden: bool,
}

/// A write of an athlete's Home preferences: `Some` stores that choice,
/// `None` keeps what is stored (or the default when nothing is).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HomePreferencesUpdate {
    /// See [`HomePreferences::plan_suggestion_hidden`].
    pub plan_suggestion_hidden: Option<bool>,
    /// See [`HomePreferences::getting_started_hidden`].
    pub getting_started_hidden: Option<bool>,
}

/// Read and write an athlete's Home preferences.
///
/// The table carries no `tenant_id`; every statement is scoped by `user_id`:
/// the choice belongs to the person, whichever tenant they are signed in to.
#[async_trait]
pub trait HomePreferencesRepository: Send + Sync {
    /// The athlete's choices, or the defaults when they have made none.
    async fn get_home_preferences(&self, user_id: Uuid) -> AppResult<HomePreferences>;

    /// Merge the athlete's choices into what is stored: each `Some` field
    /// replaces its column, each `None` field keeps it.
    async fn set_home_preferences(
        &self,
        user_id: Uuid,
        update: HomePreferencesUpdate,
    ) -> AppResult<()>;
}

/// The athlete's row.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The id binds through the
/// backend's uuid codec (see [`super::uuid_columns`]); the flags bind as
/// `bool`, which each driver encodes for its own column type.
pub(crate) const GET_HOME_PREFERENCES_SQL: &str = r"
            SELECT plan_suggestion_hidden, getting_started_hidden
            FROM user_home_preferences
            WHERE user_id = $1
            ";

/// Insert the athlete's row, or merge into its choices.
///
/// `$2` and `$3` are nullable: NULL inserts the column default and, on
/// conflict, keeps the stored value, so a write naming one choice leaves the
/// other alone.
pub(crate) const UPSERT_HOME_PREFERENCES_SQL: &str = r"
            INSERT INTO user_home_preferences
                (user_id, plan_suggestion_hidden, getting_started_hidden, updated_at)
            VALUES ($1, COALESCE($2, FALSE), COALESCE($3, FALSE), $4)
            ON CONFLICT (user_id) DO UPDATE SET
                plan_suggestion_hidden =
                    COALESCE($2, user_home_preferences.plan_suggestion_hidden),
                getting_started_hidden =
                    COALESCE($3, user_home_preferences.getting_started_hidden),
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
                    getting_started_hidden: row
                        .try_get("getting_started_hidden")
                        .map_err(|e| home_preferences_error("decode", e))?,
                })
            }

            async fn set_home_preferences(
                &self,
                user_id: Uuid,
                update: HomePreferencesUpdate,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_HOME_PREFERENCES_SQL)
                    .bind($ids::bind(user_id))
                    .bind(update.plan_suggestion_hidden)
                    .bind(update.getting_started_hidden)
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
