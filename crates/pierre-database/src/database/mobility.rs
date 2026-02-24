// ABOUTME: Database operations for Mobility features (stretching exercises and yoga poses)
// ABOUTME: Handles CRUD operations and activity-based recommendations for recovery training
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};
use std::collections::HashMap;

pub use pierre_core::models::mobility::{
    ActivityMuscleMapping, DifficultyLevel, ListStretchingFilter, ListYogaFilter,
    StretchingCategory, StretchingExercise, YogaCategory, YogaPose, YogaPoseType,
};

// ============================================================================
// Mobility Manager
// ============================================================================

/// Database manager for mobility operations
pub struct MobilityManager {
    pool: SqlitePool,
}

impl MobilityManager {
    /// Create a new mobility manager
    #[must_use]
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ========================================================================
    // Stretching Exercise Operations
    // ========================================================================

    /// Get a stretching exercise by ID
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>> {
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
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get stretching exercise: {e}")))?;

        row.map(|r| row_to_stretching_exercise(&r)).transpose()
    }

    /// List stretching exercises with optional filtering
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_stretching_exercises(
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
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list stretching exercises: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    /// Search stretching exercises by name or description
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn search_stretching_exercises(
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
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to search stretching exercises: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    /// Get stretches recommended for a specific activity type
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_stretches_for_activity(
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
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get stretches for activity: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    // ========================================================================
    // Yoga Pose Operations
    // ========================================================================

    /// Get a yoga pose by ID
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>> {
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
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get yoga pose: {e}")))?;

        row.map(|r| row_to_yoga_pose(&r)).transpose()
    }

    /// List yoga poses with optional filtering
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>> {
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
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list yoga poses: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    /// Search yoga poses by name
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn search_yoga_poses(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>> {
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
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to search yoga poses: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    /// Get yoga poses recommended for a recovery context
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_poses_for_recovery(
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
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get poses for recovery: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    // ========================================================================
    // Activity-Muscle Mapping Operations
    // ========================================================================

    /// Get muscle mapping for an activity type
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_activity_muscle_mapping(
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
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get activity muscle mapping: {e}")))?;

        row.map(|r| row_to_activity_muscle_mapping(&r)).transpose()
    }

    /// List all activity-muscle mappings
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>> {
        let rows = sqlx::query(
            r"
            SELECT id, activity_type, primary_muscles, secondary_muscles,
                   recommended_stretch_categories, recommended_yoga_categories,
                   created_at, updated_at
            FROM activity_muscle_mapping
            ORDER BY activity_type ASC
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list activity muscle mappings: {e}")))?;

        rows.iter().map(row_to_activity_muscle_mapping).collect()
    }
}

// ============================================================================
// Row Conversion Functions
// ============================================================================

/// Convert a database row to a `StretchingExercise`
fn row_to_stretching_exercise(row: &SqliteRow) -> AppResult<StretchingExercise> {
    let category_str: String = row.get("category");
    let difficulty_str: String = row.get("difficulty");
    let primary_muscles_json: String = row.get("primary_muscles");
    let secondary_muscles_json: Option<String> = row.get("secondary_muscles");
    let recommended_activities_json: Option<String> = row.get("recommended_for_activities");
    let contraindications_json: Option<String> = row.get("contraindications");
    let instructions_json: String = row.get("instructions");
    let cues_json: Option<String> = row.get("cues");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");
    let duration_seconds: i64 = row.get("duration_seconds");
    let repetitions: Option<i64> = row.get("repetitions");
    let sets: i64 = row.get("sets");

    let primary_muscles: Vec<String> = serde_json::from_str(&primary_muscles_json)?;
    let secondary_muscles: Vec<String> = secondary_muscles_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let recommended_for_activities: Vec<String> = recommended_activities_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let contraindications: Vec<String> = contraindications_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let instructions: Vec<String> = serde_json::from_str(&instructions_json)?;
    let cues: Vec<String> = cues_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(StretchingExercise {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        category: StretchingCategory::parse(&category_str),
        difficulty: DifficultyLevel::parse(&difficulty_str),
        primary_muscles,
        secondary_muscles,
        duration_seconds: duration_seconds as u32,
        repetitions: repetitions.map(|r| r as u32),
        sets: sets as u32,
        recommended_for_activities,
        contraindications,
        instructions,
        cues,
        image_url: row.get("image_url"),
        video_url: row.get("video_url"),
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
    })
}

/// Convert a database row to a `YogaPose`
fn row_to_yoga_pose(row: &SqliteRow) -> AppResult<YogaPose> {
    let category_str: String = row.get("category");
    let difficulty_str: String = row.get("difficulty");
    let pose_type_str: String = row.get("pose_type");
    let benefits_json: String = row.get("benefits");
    let primary_muscles_json: String = row.get("primary_muscles");
    let secondary_muscles_json: Option<String> = row.get("secondary_muscles");
    let chakras_json: Option<String> = row.get("chakras");
    let recommended_activities_json: Option<String> = row.get("recommended_for_activities");
    let recommended_recovery_json: Option<String> = row.get("recommended_for_recovery");
    let contraindications_json: Option<String> = row.get("contraindications");
    let instructions_json: String = row.get("instructions");
    let modifications_json: Option<String> = row.get("modifications");
    let progressions_json: Option<String> = row.get("progressions");
    let cues_json: Option<String> = row.get("cues");
    let warmup_poses_json: Option<String> = row.get("warmup_poses");
    let followup_poses_json: Option<String> = row.get("followup_poses");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");
    let hold_duration_seconds: i64 = row.get("hold_duration_seconds");

    let benefits: Vec<String> = serde_json::from_str(&benefits_json)?;
    let primary_muscles: Vec<String> = serde_json::from_str(&primary_muscles_json)?;
    let secondary_muscles: Vec<String> = secondary_muscles_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let chakras: Vec<String> = chakras_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let recommended_for_activities: Vec<String> = recommended_activities_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let recommended_for_recovery: Vec<String> = recommended_recovery_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let contraindications: Vec<String> = contraindications_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let instructions: Vec<String> = serde_json::from_str(&instructions_json)?;
    let modifications: Vec<String> = modifications_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let progressions: Vec<String> = progressions_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let cues: Vec<String> = cues_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let warmup_poses: Vec<String> = warmup_poses_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let followup_poses: Vec<String> = followup_poses_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(YogaPose {
        id: row.get("id"),
        english_name: row.get("english_name"),
        sanskrit_name: row.get("sanskrit_name"),
        description: row.get("description"),
        benefits,
        category: YogaCategory::parse(&category_str),
        difficulty: DifficultyLevel::parse(&difficulty_str),
        pose_type: YogaPoseType::parse(&pose_type_str),
        primary_muscles,
        secondary_muscles,
        chakras,
        hold_duration_seconds: hold_duration_seconds as u32,
        breath_guidance: row.get("breath_guidance"),
        recommended_for_activities,
        recommended_for_recovery,
        contraindications,
        instructions,
        modifications,
        progressions,
        cues,
        warmup_poses,
        followup_poses,
        image_url: row.get("image_url"),
        video_url: row.get("video_url"),
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
    })
}

/// Convert a database row to an `ActivityMuscleMapping`
fn row_to_activity_muscle_mapping(row: &SqliteRow) -> AppResult<ActivityMuscleMapping> {
    let primary_muscles_json: String = row.get("primary_muscles");
    let secondary_muscles_json: Option<String> = row.get("secondary_muscles");
    let stretch_categories_json: Option<String> = row.get("recommended_stretch_categories");
    let yoga_categories_json: Option<String> = row.get("recommended_yoga_categories");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");

    let primary_muscles: HashMap<String, u8> = serde_json::from_str(&primary_muscles_json)?;
    let secondary_muscles: HashMap<String, u8> = secondary_muscles_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let recommended_stretch_categories: Vec<String> = stretch_categories_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    let recommended_yoga_categories: Vec<String> = yoga_categories_json
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();

    Ok(ActivityMuscleMapping {
        id: row.get("id"),
        activity_type: row.get("activity_type"),
        primary_muscles,
        secondary_muscles,
        recommended_stretch_categories,
        recommended_yoga_categories,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
    })
}
