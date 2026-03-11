// ABOUTME: Seeder repository dispatch for the database factory
// ABOUTME: Delegates SeederRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::{SeedTable, SeederRepository};
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};
use pierre_core::models::User;
use uuid::Uuid;

#[async_trait]
impl SeederRepository for Database {
    async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.seed_reset_table(table).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_reset_table(table).await,
        }
    }

    async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => db.seed_count_table(table).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_count_table(table).await,
        }
    }

    async fn seed_upsert_stretching_exercise(
        &self,
        exercise: &StretchingExercise,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.seed_upsert_stretching_exercise(exercise).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_upsert_stretching_exercise(exercise).await,
        }
    }

    async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.seed_upsert_yoga_pose(pose).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_upsert_yoga_pose(pose).await,
        }
    }

    async fn seed_upsert_activity_mapping(&self, mapping: &ActivityMuscleMapping) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.seed_upsert_activity_mapping(mapping).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_upsert_activity_mapping(mapping).await,
        }
    }

    async fn seed_get_admin_user(&self) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.seed_get_admin_user().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_get_admin_user().await,
        }
    }

    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => db.seed_get_user_tenant(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.seed_get_user_tenant(user_id).await,
        }
    }
}
