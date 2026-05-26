// ABOUTME: Repository trait definitions for the mobility (yoga / stretching) persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};

/// Mobility (stretching exercises and yoga poses) read-only repository
#[async_trait]
pub trait MobilityRepository: Send + Sync {
    /// Get a stretching exercise by ID
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>>;
    /// List stretching exercises with optional filtering
    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Search stretching exercises by text query
    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Get stretches recommended for a specific activity type
    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Get a yoga pose by ID
    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>>;
    /// List yoga poses with optional filtering
    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>>;
    /// Search yoga poses by text query
    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>>;
    /// Get yoga poses recommended for a recovery context
    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>>;
    /// Get muscle mapping for a specific activity type
    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>>;
    /// List all activity-to-muscle mappings
    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>>;
}
