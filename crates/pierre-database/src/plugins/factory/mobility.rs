// ABOUTME: Mobility repository dispatch for the database factory
// ABOUTME: Delegates MobilityRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::plugins::MobilityRepository;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};

#[async_trait]
impl MobilityRepository for Database {
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>> {
        match self {
            Self::SQLite(db) => db.get_stretching_exercise(id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_stretching_exercise(id).await,
        }
    }

    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>> {
        match self {
            Self::SQLite(db) => db.list_stretching_exercises(filter).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_stretching_exercises(filter).await,
        }
    }

    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        match self {
            Self::SQLite(db) => db.search_stretching_exercises(query, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.search_stretching_exercises(query, limit).await,
        }
    }

    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        match self {
            Self::SQLite(db) => db.get_stretches_for_activity(activity_type, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_stretches_for_activity(activity_type, limit).await,
        }
    }

    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>> {
        match self {
            Self::SQLite(db) => db.get_yoga_pose(id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_yoga_pose(id).await,
        }
    }

    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>> {
        match self {
            Self::SQLite(db) => db.list_yoga_poses(filter).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_yoga_poses(filter).await,
        }
    }

    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>> {
        match self {
            Self::SQLite(db) => db.search_yoga_poses(query, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.search_yoga_poses(query, limit).await,
        }
    }

    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>> {
        match self {
            Self::SQLite(db) => db.get_poses_for_recovery(recovery_context, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_poses_for_recovery(recovery_context, limit).await,
        }
    }

    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>> {
        match self {
            Self::SQLite(db) => db.get_activity_muscle_mapping(activity_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_activity_muscle_mapping(activity_type).await,
        }
    }

    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>> {
        match self {
            Self::SQLite(db) => db.list_activity_muscle_mappings().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_activity_muscle_mappings().await,
        }
    }
}
