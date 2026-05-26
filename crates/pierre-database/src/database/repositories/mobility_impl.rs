// ABOUTME: Direct MobilityRepository impl on Database (SQLite stretching & yoga catalogue)
// ABOUTME: Split out of repositories/direct_impls.rs to mirror per-domain PG backend shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! #[async_trait] impl MobilityRepository for Database — stretching exercises, yoga poses, activity-muscle mappings.

use super::MobilityRepository;
use crate::database::mobility::{
    row_to_activity_muscle_mapping, row_to_stretching_exercise, row_to_yoga_pose,
};
use crate::database::Database;
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};

// ============================================================================
// MobilityRepository
// ============================================================================

#[async_trait]
impl MobilityRepository for Database {
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>> {
        let row = sqlx::query(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get stretching exercise: {e}")))?;

        row.map(|r| row_to_stretching_exercise(&r)).transpose()
    }

    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);

        // Build dynamic query with parameterized conditions to prevent SQL injection
        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(ref cat) = filter.category {
            conditions.push("category = ?".to_owned());
            bind_values.push(cat.as_str().to_owned());
        }
        if let Some(ref diff) = filter.difficulty {
            conditions.push("difficulty = ?".to_owned());
            bind_values.push(diff.as_str().to_owned());
        }
        if let Some(ref muscle) = filter.muscle_group {
            conditions.push("(primary_muscles LIKE ? OR secondary_muscles LIKE ?)".to_owned());
            let pattern = format!("%\"{muscle}\"");
            bind_values.push(pattern.clone());
            bind_values.push(pattern);
        }
        if let Some(ref activity) = filter.activity_type {
            conditions.push("recommended_for_activities LIKE ?".to_owned());
            bind_values.push(format!("%\"{activity}\""));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            {where_clause}
            ORDER BY name ASC
            LIMIT ? OFFSET ?
            "
        );

        let mut sql_query = sqlx::query(&query);
        for value in &bind_values {
            sql_query = sql_query.bind(value);
        }
        sql_query = sql_query.bind(limit_val).bind(offset_val);

        let rows = sql_query
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list stretching exercises: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            WHERE name LIKE $1 OR description LIKE $1
            ORDER BY name ASC
            LIMIT $2
            ",
        )
        .bind(&search_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search stretching exercises: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        let limit_val = i32::try_from(limit.unwrap_or(10)).unwrap_or(10);
        let activity_pattern = format!("%\"{activity_type}\"%");

        let rows = sqlx::query(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            WHERE recommended_for_activities LIKE $1
            ORDER BY category, name ASC
            LIMIT $2
            ",
        )
        .bind(&activity_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get stretches for activity: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>> {
        let row = sqlx::query(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get yoga pose: {e}")))?;

        row.map(|r| row_to_yoga_pose(&r)).transpose()
    }

    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);

        // Build dynamic query with parameterized conditions to prevent SQL injection
        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(ref cat) = filter.category {
            conditions.push("category = ?".to_owned());
            bind_values.push(cat.as_str().to_owned());
        }
        if let Some(ref diff) = filter.difficulty {
            conditions.push("difficulty = ?".to_owned());
            bind_values.push(diff.as_str().to_owned());
        }
        if let Some(ref pt) = filter.pose_type {
            conditions.push("pose_type = ?".to_owned());
            bind_values.push(pt.as_str().to_owned());
        }
        if let Some(ref muscle) = filter.muscle_group {
            conditions.push("(primary_muscles LIKE ? OR secondary_muscles LIKE ?)".to_owned());
            let pattern = format!("%\"{muscle}\"");
            bind_values.push(pattern.clone());
            bind_values.push(pattern);
        }
        if let Some(ref activity) = filter.activity_type {
            conditions.push("recommended_for_activities LIKE ?".to_owned());
            bind_values.push(format!("%\"{activity}\""));
        }
        if let Some(ref recovery) = filter.recovery_context {
            conditions.push("recommended_for_recovery LIKE ?".to_owned());
            bind_values.push(format!("%\"{recovery}\""));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            {where_clause}
            ORDER BY english_name ASC
            LIMIT ? OFFSET ?
            "
        );

        let mut sql_query = sqlx::query(&query);
        for value in &bind_values {
            sql_query = sql_query.bind(value);
        }
        sql_query = sql_query.bind(limit_val).bind(offset_val);

        let rows = sql_query
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list yoga poses: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            WHERE english_name LIKE $1 OR sanskrit_name LIKE $1 OR description LIKE $1
            ORDER BY english_name ASC
            LIMIT $2
            ",
        )
        .bind(&search_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search yoga poses: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>> {
        let limit_val = i32::try_from(limit.unwrap_or(10)).unwrap_or(10);
        let recovery_pattern = format!("%\"{recovery_context}\"%");

        let rows = sqlx::query(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            WHERE recommended_for_recovery LIKE $1
            ORDER BY category, english_name ASC
            LIMIT $2
            ",
        )
        .bind(&recovery_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get poses for recovery: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>> {
        let row = sqlx::query(
            r"
            SELECT id, activity_type, primary_muscles, secondary_muscles,
                   recommended_stretch_categories, recommended_yoga_categories,
                   created_at, updated_at
            FROM activity_muscle_mapping
            WHERE activity_type = $1
            ",
        )
        .bind(activity_type)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get activity muscle mapping: {e}")))?;

        row.map(|r| row_to_activity_muscle_mapping(&r)).transpose()
    }

    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>> {
        let rows = sqlx::query(
            r"
            SELECT id, activity_type, primary_muscles, secondary_muscles,
                   recommended_stretch_categories, recommended_yoga_categories,
                   created_at, updated_at
            FROM activity_muscle_mapping
            ORDER BY activity_type ASC
            ",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list activity muscle mappings: {e}")))?;

        rows.iter().map(row_to_activity_muscle_mapping).collect()
    }
}
