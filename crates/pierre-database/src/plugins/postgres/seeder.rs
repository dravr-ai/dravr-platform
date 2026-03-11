// ABOUTME: PostgreSQL implementation of SeederRepository for seed-only database operations
// ABOUTME: Handles upserts, resets, and bulk inserts using PostgreSQL ON CONFLICT syntax
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};
use pierre_core::models::User;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{SeedTable, SeederRepository};

#[async_trait]
impl SeederRepository for PostgresDatabase {
    async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64> {
        let sql = format!("DELETE FROM {}", table.table_name());
        let result = sqlx::query(&sql).execute(&self.pool).await.map_err(|e| {
            AppError::database(format!("Failed to reset table {}: {e}", table.table_name()))
        })?;
        Ok(result.rows_affected())
    }

    async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64> {
        let sql = format!("SELECT COUNT(*) as cnt FROM {}", table.table_name());
        let row = sqlx::query(&sql).fetch_one(&self.pool).await.map_err(|e| {
            AppError::database(format!("Failed to count table {}: {e}", table.table_name()))
        })?;
        Ok(row.get("cnt"))
    }

    async fn seed_upsert_stretching_exercise(
        &self,
        exercise: &StretchingExercise,
    ) -> AppResult<()> {
        let primary_muscles_json = serde_json::to_string(&exercise.primary_muscles)
            .map_err(|e| AppError::internal(format!("Failed to serialize primary_muscles: {e}")))?;
        let secondary_muscles_json =
            serde_json::to_string(&exercise.secondary_muscles).map_err(|e| {
                AppError::internal(format!("Failed to serialize secondary_muscles: {e}"))
            })?;
        let recommended_json = serde_json::to_string(&exercise.recommended_for_activities)
            .map_err(|e| {
                AppError::internal(format!(
                    "Failed to serialize recommended_for_activities: {e}"
                ))
            })?;
        let contraindications_json =
            serde_json::to_string(&exercise.contraindications).map_err(|e| {
                AppError::internal(format!("Failed to serialize contraindications: {e}"))
            })?;
        let instructions_json = serde_json::to_string(&exercise.instructions)
            .map_err(|e| AppError::internal(format!("Failed to serialize instructions: {e}")))?;
        let cues_json = serde_json::to_string(&exercise.cues)
            .map_err(|e| AppError::internal(format!("Failed to serialize cues: {e}")))?;

        #[allow(clippy::cast_possible_wrap)]
        let duration_seconds = exercise.duration_seconds as i32;
        #[allow(clippy::cast_possible_wrap)]
        let repetitions = exercise.repetitions.map(|r| r as i32);
        #[allow(clippy::cast_possible_wrap)]
        let sets = exercise.sets as i32;

        sqlx::query(
            "INSERT INTO stretching_exercises \
             (id, name, description, category, difficulty, primary_muscles, secondary_muscles, \
              duration_seconds, repetitions, sets, recommended_for_activities, contraindications, \
              instructions, cues, image_url, video_url, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) \
             ON CONFLICT (id) DO UPDATE SET \
              name = EXCLUDED.name, description = EXCLUDED.description, \
              category = EXCLUDED.category, difficulty = EXCLUDED.difficulty, \
              primary_muscles = EXCLUDED.primary_muscles, secondary_muscles = EXCLUDED.secondary_muscles, \
              duration_seconds = EXCLUDED.duration_seconds, repetitions = EXCLUDED.repetitions, \
              sets = EXCLUDED.sets, recommended_for_activities = EXCLUDED.recommended_for_activities, \
              contraindications = EXCLUDED.contraindications, instructions = EXCLUDED.instructions, \
              cues = EXCLUDED.cues, image_url = EXCLUDED.image_url, video_url = EXCLUDED.video_url, \
              updated_at = EXCLUDED.updated_at",
        )
        .bind(&exercise.id)
        .bind(&exercise.name)
        .bind(&exercise.description)
        .bind(exercise.category.as_str())
        .bind(exercise.difficulty.as_str())
        .bind(&primary_muscles_json)
        .bind(&secondary_muscles_json)
        .bind(duration_seconds)
        .bind(repetitions)
        .bind(sets)
        .bind(&recommended_json)
        .bind(&contraindications_json)
        .bind(&instructions_json)
        .bind(&cues_json)
        .bind(&exercise.image_url)
        .bind(&exercise.video_url)
        .bind(exercise.created_at)
        .bind(exercise.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert stretching exercise: {e}")))?;
        Ok(())
    }

    async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()> {
        let benefits_json = serde_json::to_string(&pose.benefits)
            .map_err(|e| AppError::internal(format!("Failed to serialize benefits: {e}")))?;
        let primary_muscles_json = serde_json::to_string(&pose.primary_muscles)
            .map_err(|e| AppError::internal(format!("Failed to serialize primary_muscles: {e}")))?;
        let secondary_muscles_json =
            serde_json::to_string(&pose.secondary_muscles).map_err(|e| {
                AppError::internal(format!("Failed to serialize secondary_muscles: {e}"))
            })?;
        let chakras_json = serde_json::to_string(&pose.chakras)
            .map_err(|e| AppError::internal(format!("Failed to serialize chakras: {e}")))?;
        let recommended_activities_json = serde_json::to_string(&pose.recommended_for_activities)
            .map_err(|e| {
            AppError::internal(format!(
                "Failed to serialize recommended_for_activities: {e}"
            ))
        })?;
        let recommended_recovery_json = serde_json::to_string(&pose.recommended_for_recovery)
            .map_err(|e| {
                AppError::internal(format!("Failed to serialize recommended_for_recovery: {e}"))
            })?;
        let contraindications_json =
            serde_json::to_string(&pose.contraindications).map_err(|e| {
                AppError::internal(format!("Failed to serialize contraindications: {e}"))
            })?;
        let instructions_json = serde_json::to_string(&pose.instructions)
            .map_err(|e| AppError::internal(format!("Failed to serialize instructions: {e}")))?;
        let modifications_json = serde_json::to_string(&pose.modifications)
            .map_err(|e| AppError::internal(format!("Failed to serialize modifications: {e}")))?;
        let progressions_json = serde_json::to_string(&pose.progressions)
            .map_err(|e| AppError::internal(format!("Failed to serialize progressions: {e}")))?;
        let cues_json = serde_json::to_string(&pose.cues)
            .map_err(|e| AppError::internal(format!("Failed to serialize cues: {e}")))?;
        let warmup_poses_json = serde_json::to_string(&pose.warmup_poses)
            .map_err(|e| AppError::internal(format!("Failed to serialize warmup_poses: {e}")))?;
        let followup_poses_json = serde_json::to_string(&pose.followup_poses)
            .map_err(|e| AppError::internal(format!("Failed to serialize followup_poses: {e}")))?;

        #[allow(clippy::cast_possible_wrap)]
        let hold_duration_seconds = pose.hold_duration_seconds as i32;

        sqlx::query(
            "INSERT INTO yoga_poses \
             (id, english_name, sanskrit_name, description, benefits, \
              category, difficulty, pose_type, primary_muscles, secondary_muscles, \
              chakras, hold_duration_seconds, breath_guidance, \
              recommended_for_activities, recommended_for_recovery, contraindications, \
              instructions, modifications, progressions, cues, \
              warmup_poses, followup_poses, image_url, video_url, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                     $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, \
                     $21, $22, $23, $24, $25, $26) \
             ON CONFLICT (id) DO UPDATE SET \
              english_name = EXCLUDED.english_name, sanskrit_name = EXCLUDED.sanskrit_name, \
              description = EXCLUDED.description, benefits = EXCLUDED.benefits, \
              category = EXCLUDED.category, difficulty = EXCLUDED.difficulty, \
              pose_type = EXCLUDED.pose_type, primary_muscles = EXCLUDED.primary_muscles, \
              secondary_muscles = EXCLUDED.secondary_muscles, chakras = EXCLUDED.chakras, \
              hold_duration_seconds = EXCLUDED.hold_duration_seconds, \
              breath_guidance = EXCLUDED.breath_guidance, \
              recommended_for_activities = EXCLUDED.recommended_for_activities, \
              recommended_for_recovery = EXCLUDED.recommended_for_recovery, \
              contraindications = EXCLUDED.contraindications, \
              instructions = EXCLUDED.instructions, modifications = EXCLUDED.modifications, \
              progressions = EXCLUDED.progressions, cues = EXCLUDED.cues, \
              warmup_poses = EXCLUDED.warmup_poses, followup_poses = EXCLUDED.followup_poses, \
              image_url = EXCLUDED.image_url, video_url = EXCLUDED.video_url, \
              updated_at = EXCLUDED.updated_at",
        )
        .bind(&pose.id)
        .bind(&pose.english_name)
        .bind(&pose.sanskrit_name)
        .bind(&pose.description)
        .bind(&benefits_json)
        .bind(pose.category.as_str())
        .bind(pose.difficulty.as_str())
        .bind(pose.pose_type.as_str())
        .bind(&primary_muscles_json)
        .bind(&secondary_muscles_json)
        .bind(&chakras_json)
        .bind(hold_duration_seconds)
        .bind(&pose.breath_guidance)
        .bind(&recommended_activities_json)
        .bind(&recommended_recovery_json)
        .bind(&contraindications_json)
        .bind(&instructions_json)
        .bind(&modifications_json)
        .bind(&progressions_json)
        .bind(&cues_json)
        .bind(&warmup_poses_json)
        .bind(&followup_poses_json)
        .bind(&pose.image_url)
        .bind(&pose.video_url)
        .bind(pose.created_at)
        .bind(pose.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert yoga pose: {e}")))?;
        Ok(())
    }

    async fn seed_upsert_activity_mapping(&self, mapping: &ActivityMuscleMapping) -> AppResult<()> {
        let primary_muscles_json = serde_json::to_string(&mapping.primary_muscles)
            .map_err(|e| AppError::internal(format!("Failed to serialize primary_muscles: {e}")))?;
        let secondary_muscles_json =
            serde_json::to_string(&mapping.secondary_muscles).map_err(|e| {
                AppError::internal(format!("Failed to serialize secondary_muscles: {e}"))
            })?;
        let stretch_categories_json =
            serde_json::to_string(&mapping.recommended_stretch_categories).map_err(|e| {
                AppError::internal(format!(
                    "Failed to serialize recommended_stretch_categories: {e}"
                ))
            })?;
        let yoga_categories_json = serde_json::to_string(&mapping.recommended_yoga_categories)
            .map_err(|e| {
                AppError::internal(format!(
                    "Failed to serialize recommended_yoga_categories: {e}"
                ))
            })?;

        sqlx::query(
            "INSERT INTO activity_muscle_mapping \
             (id, activity_type, primary_muscles, secondary_muscles, \
              recommended_stretch_categories, recommended_yoga_categories, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (activity_type) DO UPDATE SET \
              primary_muscles = EXCLUDED.primary_muscles, \
              secondary_muscles = EXCLUDED.secondary_muscles, \
              recommended_stretch_categories = EXCLUDED.recommended_stretch_categories, \
              recommended_yoga_categories = EXCLUDED.recommended_yoga_categories, \
              updated_at = EXCLUDED.updated_at",
        )
        .bind(&mapping.id)
        .bind(&mapping.activity_type)
        .bind(&primary_muscles_json)
        .bind(&secondary_muscles_json)
        .bind(&stretch_categories_json)
        .bind(&yoga_categories_json)
        .bind(mapping.created_at)
        .bind(mapping.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert activity mapping: {e}")))?;
        Ok(())
    }

    async fn seed_get_admin_user(&self) -> AppResult<Option<User>> {
        let row = sqlx::query("SELECT * FROM users WHERE role IN ('super_admin', 'admin') LIMIT 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to find admin user: {e}")))?;

        row.map(|r| Self::parse_user_from_row(&r)).transpose()
    }

    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>> {
        let row = sqlx::query("SELECT tenant_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenant: {e}")))?;

        Ok(row.and_then(|r| r.get::<Option<String>, _>("tenant_id")))
    }
}
