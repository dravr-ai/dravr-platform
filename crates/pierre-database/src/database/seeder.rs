// ABOUTME: SQLite implementation of SeederRepository for seed-only database operations
// ABOUTME: Handles upserts, resets, and bulk inserts using SQLite-specific syntax
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};
use pierre_core::models::User;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::{SeedTable, SeederRepository};
use crate::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedAdaptedInsight, SeedApiKey, SeedApiKeyUsage, SeedCoach,
    SeedCoachAuthor, SeedCoachRelation, SeedDemoUser, SeedFriendConnection, SeedInsightReaction,
    SeedLlmUsageRecord, SeedProviderConnection, SeedSharedInsight, SeedSocialSettings,
    SeedStoreListing, SeedSyntheticActivity, SeedTenant,
};

#[async_trait]
impl SeederRepository for Database {
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

        sqlx::query(
            "INSERT OR REPLACE INTO stretching_exercises \
             (id, name, description, category, difficulty, primary_muscles, secondary_muscles, \
              duration_seconds, repetitions, sets, recommended_for_activities, contraindications, \
              instructions, cues, image_url, video_url, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
        )
        .bind(&exercise.id)
        .bind(&exercise.name)
        .bind(&exercise.description)
        .bind(exercise.category.as_str())
        .bind(exercise.difficulty.as_str())
        .bind(&primary_muscles_json)
        .bind(&secondary_muscles_json)
        .bind(exercise.duration_seconds)
        .bind(exercise.repetitions)
        .bind(exercise.sets)
        .bind(&recommended_json)
        .bind(&contraindications_json)
        .bind(&instructions_json)
        .bind(&cues_json)
        .bind(&exercise.image_url)
        .bind(&exercise.video_url)
        .bind(exercise.created_at.to_rfc3339())
        .bind(exercise.updated_at.to_rfc3339())
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

        sqlx::query(
            "INSERT OR REPLACE INTO yoga_poses \
             (id, english_name, sanskrit_name, description, benefits, \
              category, difficulty, pose_type, primary_muscles, secondary_muscles, \
              chakras, hold_duration_seconds, breath_guidance, \
              recommended_for_activities, recommended_for_recovery, contraindications, \
              instructions, modifications, progressions, cues, \
              warmup_poses, followup_poses, image_url, video_url, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                     $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, \
                     $21, $22, $23, $24, $25, $26)",
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
        .bind(pose.hold_duration_seconds)
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
        .bind(pose.created_at.to_rfc3339())
        .bind(pose.updated_at.to_rfc3339())
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
            "INSERT OR REPLACE INTO activity_muscle_mapping \
             (id, activity_type, primary_muscles, secondary_muscles, \
              recommended_stretch_categories, recommended_yoga_categories, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&mapping.id)
        .bind(&mapping.activity_type)
        .bind(&primary_muscles_json)
        .bind(&secondary_muscles_json)
        .bind(&stretch_categories_json)
        .bind(&yoga_categories_json)
        .bind(mapping.created_at.to_rfc3339())
        .bind(mapping.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert activity mapping: {e}")))?;
        Ok(())
    }

    async fn seed_get_admin_user(&self) -> AppResult<Option<User>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT email FROM users WHERE is_admin = 1 ORDER BY created_at ASC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to find admin user: {e}")))?;

        match row {
            Some((email,)) => {
                // Use the existing UserRepository to get the full User
                use crate::repositories::UserRepository;
                self.get_by_email(&email).await
            }
            None => Ok(None),
        }
    }

    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>> {
        let row = sqlx::query("SELECT tenant_id FROM users WHERE id = $1")
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenant: {e}")))?;

        Ok(row.and_then(|r| r.get::<Option<String>, _>("tenant_id")))
    }

    async fn seed_find_user_by_email(&self, email: &str) -> AppResult<Option<User>> {
        use crate::repositories::UserRepository;
        self.get_by_email(email).await
    }

    async fn seed_get_non_admin_user_ids(&self) -> AppResult<Vec<Uuid>> {
        let rows = sqlx::query("SELECT id FROM users WHERE is_admin = 0 ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get non-admin user IDs: {e}")))?;

        rows.iter()
            .map(|r| {
                let id_str: String = r.get("id");
                id_str
                    .parse::<Uuid>()
                    .map_err(|e| AppError::database(format!("Failed to parse user ID: {e}")))
            })
            .collect()
    }

    async fn seed_count_non_admin_users(&self) -> AppResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM users WHERE is_admin = 0")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to count non-admin users: {e}")))?;
        Ok(row.get("cnt"))
    }

    async fn seed_delete_llm_usage_by_tenant(&self, tenant_id: Uuid) -> AppResult<u64> {
        let result = sqlx::query("DELETE FROM llm_usage WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to delete LLM usage by tenant: {e}"))
            })?;
        Ok(result.rows_affected())
    }

    async fn seed_insert_llm_usage(&self, record: &SeedLlmUsageRecord) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO llm_usage \
             (id, tenant_id, user_id, conversation_id, provider, model, \
              prompt_tokens, completion_tokens, total_tokens, call_type, \
              tool_calls_count, execution_time_ms, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(record.id.to_string())
        .bind(record.tenant_id.to_string())
        .bind(record.user_id.to_string())
        .bind(&record.conversation_id)
        .bind(&record.provider)
        .bind(&record.model)
        .bind(record.prompt_tokens)
        .bind(record.completion_tokens)
        .bind(record.total_tokens)
        .bind(&record.call_type)
        .bind(record.tool_calls_count)
        .bind(record.execution_time_ms)
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert LLM usage record: {e}")))?;
        Ok(())
    }

    async fn seed_delete_synthetic_by_user(&self, user_id: Uuid) -> AppResult<u64> {
        let result = sqlx::query("DELETE FROM synthetic_activities WHERE user_id = $1")
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to delete synthetic activities by user: {e}"
                ))
            })?;
        Ok(result.rows_affected())
    }

    async fn seed_insert_synthetic_activity(
        &self,
        activity: &SeedSyntheticActivity,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO synthetic_activities \
             (id, user_id, tenant_id, name, sport_type, start_date, duration_seconds, \
              distance_meters, elevation_gain, average_heart_rate, max_heart_rate, \
              average_speed, max_speed, calories, city, region, country, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)",
        )
        .bind(activity.id.to_string())
        .bind(activity.user_id.to_string())
        .bind(activity.tenant_id.to_string())
        .bind(&activity.name)
        .bind(&activity.sport_type)
        .bind(activity.start_date.to_rfc3339())
        .bind(activity.duration_seconds)
        .bind(activity.distance_meters)
        .bind(activity.elevation_gain)
        .bind(activity.average_heart_rate)
        .bind(activity.max_heart_rate)
        .bind(activity.average_speed)
        .bind(activity.max_speed)
        .bind(activity.calories)
        .bind(&activity.city)
        .bind(&activity.region)
        .bind(&activity.country)
        .bind(activity.created_at.to_rfc3339())
        .bind(activity.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert synthetic activity: {e}")))?;
        Ok(())
    }

    async fn seed_upsert_provider_connection(
        &self,
        conn: &SeedProviderConnection,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO provider_connections \
             (id, user_id, tenant_id, provider, connection_type, connected_at, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT(user_id, tenant_id, provider) DO UPDATE SET \
               connected_at = excluded.connected_at, \
               metadata = excluded.metadata",
        )
        .bind(conn.id.to_string())
        .bind(conn.user_id.to_string())
        .bind(conn.tenant_id.to_string())
        .bind(&conn.provider)
        .bind(&conn.connection_type)
        .bind(conn.connected_at.to_rfc3339())
        .bind(&conn.metadata)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert provider connection: {e}")))?;
        Ok(())
    }

    async fn seed_reset_social_data(&self) -> AppResult<()> {
        // Delete in foreign key order: dependent tables first
        for sql in &[
            "DELETE FROM adapted_insights",
            "DELETE FROM insight_reactions",
            "DELETE FROM shared_insights",
            "DELETE FROM friend_connections",
            "DELETE FROM user_social_settings",
        ] {
            sqlx::query(sql)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to reset social data: {e}")))?;
        }
        Ok(())
    }

    async fn seed_upsert_social_settings(&self, settings: &SeedSocialSettings) -> AppResult<bool> {
        // Check if settings already exist for this user
        let exists = sqlx::query("SELECT user_id FROM user_social_settings WHERE user_id = $1")
            .bind(settings.user_id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to check social settings existence: {e}"))
            })?;

        if exists.is_some() {
            return Ok(false);
        }

        sqlx::query(
            "INSERT INTO user_social_settings \
             (user_id, discoverable, default_visibility, share_activity_types, \
              notify_friend_requests, notify_insight_reactions, notify_adapted_insights, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, $5, $6)",
        )
        .bind(settings.user_id.to_string())
        .bind(i32::from(settings.discoverable))
        .bind(&settings.default_visibility)
        .bind(&settings.share_activity_types)
        .bind(settings.created_at.to_rfc3339())
        .bind(settings.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert social settings: {e}")))?;

        Ok(true)
    }

    async fn seed_insert_friend_connection_if_absent(
        &self,
        conn: &SeedFriendConnection,
    ) -> AppResult<bool> {
        // Check both directions for an existing connection
        let exists = sqlx::query(
            "SELECT id FROM friend_connections \
             WHERE (initiator_id = $1 AND receiver_id = $2) \
                OR (initiator_id = $2 AND receiver_id = $1)",
        )
        .bind(conn.initiator_id.to_string())
        .bind(conn.receiver_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to check friend connection existence: {e}"))
        })?;

        if exists.is_some() {
            return Ok(false);
        }

        let accepted_at_str = conn.accepted_at.map(|dt| dt.to_rfc3339());

        sqlx::query(
            "INSERT INTO friend_connections \
             (id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(conn.id.to_string())
        .bind(conn.initiator_id.to_string())
        .bind(conn.receiver_id.to_string())
        .bind(&conn.status)
        .bind(conn.created_at.to_rfc3339())
        .bind(conn.updated_at.to_rfc3339())
        .bind(accepted_at_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert friend connection: {e}")))?;

        Ok(true)
    }

    async fn seed_insert_shared_insight(&self, insight: &SeedSharedInsight) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO shared_insights \
             (id, user_id, visibility, insight_type, sport_type, content, title, \
              training_phase, reaction_count, adapt_count, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 0, $9, $10)",
        )
        .bind(insight.id.to_string())
        .bind(insight.user_id.to_string())
        .bind(&insight.visibility)
        .bind(&insight.insight_type)
        .bind(&insight.sport_type)
        .bind(&insight.content)
        .bind(&insight.title)
        .bind(&insight.training_phase)
        .bind(insight.created_at.to_rfc3339())
        .bind(insight.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert shared insight: {e}")))?;
        Ok(())
    }

    async fn seed_get_shared_insight_ids(&self) -> AppResult<Vec<Uuid>> {
        let rows = sqlx::query("SELECT id FROM shared_insights")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get shared insight IDs: {e}")))?;

        rows.iter()
            .map(|r| {
                let id_str: String = r.get("id");
                id_str
                    .parse::<Uuid>()
                    .map_err(|e| AppError::database(format!("Failed to parse insight ID: {e}")))
            })
            .collect()
    }

    async fn seed_insert_reaction_if_absent(
        &self,
        reaction: &SeedInsightReaction,
    ) -> AppResult<bool> {
        let exists =
            sqlx::query("SELECT id FROM insight_reactions WHERE insight_id = $1 AND user_id = $2")
                .bind(reaction.insight_id.to_string())
                .bind(reaction.user_id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to check reaction existence: {e}"))
                })?;

        if exists.is_some() {
            return Ok(false);
        }

        sqlx::query(
            "INSERT INTO insight_reactions (id, insight_id, user_id, reaction_type, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(reaction.id.to_string())
        .bind(reaction.insight_id.to_string())
        .bind(reaction.user_id.to_string())
        .bind(&reaction.reaction_type)
        .bind(reaction.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert insight reaction: {e}")))?;

        Ok(true)
    }

    async fn seed_get_shared_insights_with_authors(&self) -> AppResult<Vec<(Uuid, Uuid)>> {
        let rows = sqlx::query("SELECT id, user_id FROM shared_insights")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to get shared insights with authors: {e}"))
            })?;

        rows.iter()
            .map(|r| {
                let id_str: String = r.get("id");
                let user_id_str: String = r.get("user_id");
                let id = id_str
                    .parse::<Uuid>()
                    .map_err(|e| AppError::database(format!("Failed to parse insight ID: {e}")))?;
                let user_id = user_id_str
                    .parse::<Uuid>()
                    .map_err(|e| AppError::database(format!("Failed to parse user ID: {e}")))?;
                Ok((id, user_id))
            })
            .collect()
    }

    async fn seed_insert_adapted_insight_if_absent(
        &self,
        adapted: &SeedAdaptedInsight,
    ) -> AppResult<bool> {
        let exists = sqlx::query(
            "SELECT id FROM adapted_insights WHERE source_insight_id = $1 AND user_id = $2",
        )
        .bind(adapted.source_insight_id.to_string())
        .bind(adapted.user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to check adapted insight existence: {e}"))
        })?;

        if exists.is_some() {
            return Ok(false);
        }

        sqlx::query(
            "INSERT INTO adapted_insights \
             (id, source_insight_id, user_id, adapted_content, adaptation_context, \
              was_helpful, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(adapted.id.to_string())
        .bind(adapted.source_insight_id.to_string())
        .bind(adapted.user_id.to_string())
        .bind(&adapted.adapted_content)
        .bind(&adapted.adaptation_context)
        .bind(i32::from(adapted.was_helpful))
        .bind(adapted.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert adapted insight: {e}")))?;

        Ok(true)
    }

    async fn seed_get_shared_insights_not_by_user(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<Uuid>> {
        let rows = sqlx::query(
            "SELECT id FROM shared_insights WHERE user_id != $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(user_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get shared insights not by user: {e}"))
        })?;

        rows.iter()
            .map(|r| {
                let id_str: String = r.get("id");
                id_str
                    .parse::<Uuid>()
                    .map_err(|e| AppError::database(format!("Failed to parse insight ID: {e}")))
            })
            .collect()
    }

    async fn seed_check_user_exists(&self, email: &str) -> AppResult<Option<Uuid>> {
        let row = sqlx::query("SELECT id FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to check user existence: {e}")))?;

        row.map(|r| {
            let id_str: String = r.get("id");
            id_str
                .parse::<Uuid>()
                .map_err(|e| AppError::database(format!("Failed to parse user ID: {e}")))
        })
        .transpose()
    }

    async fn seed_insert_demo_user(&self, user: &SeedDemoUser) -> AppResult<()> {
        let is_active = i32::from(user.status != "suspended");
        let approved_at = if user.status == "active" {
            Some(user.created_at.to_rfc3339())
        } else {
            None
        };

        let role = if user.is_admin { "admin" } else { "user" };

        sqlx::query(
            "INSERT INTO users \
             (id, email, display_name, password_hash, tier, is_active, user_status, \
              is_admin, role, approved_at, created_at, last_active, auth_provider) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'email') \
             ON CONFLICT(email) DO UPDATE SET \
              password_hash = excluded.password_hash, \
              role = excluded.role, \
              is_admin = excluded.is_admin, \
              display_name = excluded.display_name",
        )
        .bind(user.id.to_string())
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(&user.tier)
        .bind(is_active)
        .bind(&user.status)
        .bind(i32::from(user.is_admin))
        .bind(role)
        .bind(approved_at)
        .bind(user.created_at.to_rfc3339())
        .bind(user.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert demo user: {e}")))?;
        Ok(())
    }

    async fn seed_insert_tenant(&self, tenant: &SeedTenant) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO tenants \
             (id, name, slug, plan, owner_user_id, is_active, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, 1, $6, $7)",
        )
        .bind(tenant.id.to_string())
        .bind(&tenant.name)
        .bind(&tenant.slug)
        .bind(&tenant.plan)
        .bind(tenant.owner_user_id.to_string())
        .bind(tenant.created_at.to_rfc3339())
        .bind(tenant.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert tenant: {e}")))?;
        Ok(())
    }

    async fn seed_insert_tenant_user(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let now_str = now.to_rfc3339();
        sqlx::query(
            "INSERT INTO tenant_users \
             (id, tenant_id, user_id, role, invited_at, joined_at, is_active) \
             VALUES ($1, $2, $3, 'owner', $4, $5, 1)",
        )
        .bind(id.to_string())
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(&now_str)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert tenant user: {e}")))?;
        Ok(())
    }

    async fn seed_update_user_tenant(&self, user_id: Uuid, tenant_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE users SET tenant_id = $1 WHERE id = $2")
            .bind(tenant_id.to_string())
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to update user tenant: {e}")))?;
        Ok(())
    }

    async fn seed_check_api_key_by_name(&self, name: &str) -> AppResult<Option<Uuid>> {
        let row = sqlx::query("SELECT id FROM api_keys WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to check API key by name: {e}")))?;

        row.map(|r| {
            let id_str: String = r.get("id");
            id_str
                .parse::<Uuid>()
                .map_err(|e| AppError::database(format!("Failed to parse API key ID: {e}")))
        })
        .transpose()
    }

    async fn seed_insert_api_key(&self, key: &SeedApiKey) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO api_keys \
             (id, user_id, name, description, key_hash, key_prefix, tier, \
              rate_limit_requests, rate_limit_window_seconds, is_active, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 3600, 1, $9, $10)",
        )
        .bind(key.id.to_string())
        .bind(key.user_id.to_string())
        .bind(&key.name)
        .bind(&key.description)
        .bind(&key.key_hash)
        .bind(&key.key_prefix)
        .bind(&key.tier)
        .bind(key.rate_limit)
        .bind(key.expires_at.map(|dt| dt.to_rfc3339()))
        .bind(key.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert API key: {e}")))?;
        Ok(())
    }

    async fn seed_check_a2a_client_by_name(&self, name: &str) -> AppResult<Option<Uuid>> {
        let row = sqlx::query("SELECT id FROM a2a_clients WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to check A2A client by name: {e}")))?;

        row.map(|r| {
            let id_str: String = r.get("id");
            id_str
                .parse::<Uuid>()
                .map_err(|e| AppError::database(format!("Failed to parse A2A client ID: {e}")))
        })
        .transpose()
    }

    async fn seed_insert_a2a_client(&self, client: &SeedA2AClient) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO a2a_clients \
             (id, user_id, name, description, public_key, client_secret, \
              permissions, capabilities, rate_limit_requests, rate_limit_window_seconds, \
              is_active, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1000, 3600, 1, $9, $10)",
        )
        .bind(client.id.to_string())
        .bind(client.user_id.to_string())
        .bind(&client.name)
        .bind(&client.description)
        .bind(&client.public_key)
        .bind(&client.client_secret)
        .bind(&client.permissions)
        .bind(&client.capabilities)
        .bind(client.created_at.to_rfc3339())
        .bind(client.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert A2A client: {e}")))?;
        Ok(())
    }

    async fn seed_insert_api_key_usage(&self, usage: &SeedApiKeyUsage) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO api_key_usage \
             (id, api_key_id, timestamp, tool_name, status_code, response_time_ms) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(usage.id.to_string())
        .bind(usage.api_key_id.to_string())
        .bind(usage.timestamp.to_rfc3339())
        .bind(&usage.tool_name)
        .bind(usage.status_code)
        .bind(usage.response_time_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert API key usage: {e}")))?;
        Ok(())
    }

    async fn seed_insert_a2a_usage(&self, usage: &SeedA2AUsage) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO a2a_usage \
             (id, client_id, timestamp, tool_name, status_code, response_time_ms, protocol_version) \
             VALUES ($1, $2, $3, $4, $5, $6, '1.0')",
        )
        .bind(usage.id.to_string())
        .bind(usage.client_id.to_string())
        .bind(usage.timestamp.to_rfc3339())
        .bind(&usage.tool_name)
        .bind(usage.status_code)
        .bind(usage.response_time_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert A2A usage: {e}")))?;
        Ok(())
    }

    async fn seed_find_coach_by_slug(
        &self,
        slug: &str,
        tenant_id: &str,
    ) -> AppResult<Option<(String, Option<String>)>> {
        let row =
            sqlx::query("SELECT id, content_hash FROM coaches WHERE slug = $1 AND tenant_id = $2")
                .bind(slug)
                .bind(tenant_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to find coach by slug: {e}")))?;

        Ok(row.map(|r| {
            let id: String = r.get("id");
            let content_hash: Option<String> = r.get("content_hash");
            (id, content_hash)
        }))
    }

    async fn seed_insert_coach(&self, coach: &SeedCoach) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO coaches \
             (id, user_id, tenant_id, title, description, system_prompt, category, tags, \
              sample_prompts, token_count, created_at, updated_at, is_system, visibility, \
              slug, purpose, when_to_use, instructions, example_inputs, example_outputs, \
              success_criteria, prerequisites, source_file, content_hash, startup_query, \
              data_requirements) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 1, $13, \
                     $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)",
        )
        .bind(&coach.id)
        .bind(coach.user_id.to_string())
        .bind(coach.tenant_id)
        .bind(&coach.title)
        .bind(&coach.description)
        .bind(&coach.system_prompt)
        .bind(&coach.category)
        .bind(&coach.tags_json)
        .bind(&coach.sample_prompts_json)
        .bind(coach.token_count)
        .bind(coach.created_at.to_rfc3339())
        .bind(coach.updated_at.to_rfc3339())
        .bind(&coach.visibility)
        .bind(&coach.slug)
        .bind(&coach.purpose)
        .bind(&coach.when_to_use)
        .bind(&coach.instructions)
        .bind(&coach.example_inputs)
        .bind(&coach.example_outputs)
        .bind(&coach.success_criteria)
        .bind(&coach.prerequisites_json)
        .bind(&coach.source_file)
        .bind(&coach.content_hash)
        .bind(&coach.startup_query)
        .bind(&coach.data_requirements)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert coach: {e}")))?;
        Ok(())
    }

    async fn seed_update_coach(&self, coach: &SeedCoach) -> AppResult<()> {
        sqlx::query(
            "UPDATE coaches SET \
               title = $1, description = $2, system_prompt = $3, category = $4, \
               tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8, \
               visibility = $9, purpose = $10, when_to_use = $11, instructions = $12, \
               example_inputs = $13, example_outputs = $14, success_criteria = $15, \
               prerequisites = $16, source_file = $17, content_hash = $18, startup_query = $19, \
               data_requirements = $20 \
             WHERE id = $21",
        )
        .bind(&coach.title)
        .bind(&coach.description)
        .bind(&coach.system_prompt)
        .bind(&coach.category)
        .bind(&coach.tags_json)
        .bind(&coach.sample_prompts_json)
        .bind(coach.token_count)
        .bind(coach.updated_at.to_rfc3339())
        .bind(&coach.visibility)
        .bind(&coach.purpose)
        .bind(&coach.when_to_use)
        .bind(&coach.instructions)
        .bind(&coach.example_inputs)
        .bind(&coach.example_outputs)
        .bind(&coach.success_criteria)
        .bind(&coach.prerequisites_json)
        .bind(&coach.source_file)
        .bind(&coach.content_hash)
        .bind(&coach.startup_query)
        .bind(&coach.data_requirements)
        .bind(&coach.id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;
        Ok(())
    }

    async fn seed_insert_coach_relation_if_absent(
        &self,
        relation: &SeedCoachRelation,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO coach_relations \
             (id, coach_id, related_coach_id, relation_type, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&relation.id)
        .bind(&relation.coach_id)
        .bind(&relation.related_coach_id)
        .bind(&relation.relation_type)
        .bind(relation.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert coach relation: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn seed_upsert_coach_author(&self, author: &SeedCoachAuthor) -> AppResult<String> {
        // Check if author already exists for this user+tenant
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM coach_authors WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(author.user_id.to_string())
        .bind(&author.tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check coach author: {e}")))?;

        if let Some(existing_id) = existing {
            return Ok(existing_id);
        }

        sqlx::query(
            "INSERT INTO coach_authors \
             (id, user_id, tenant_id, display_name, is_verified, \
              published_coach_count, total_install_count, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 0, 0, 0, $5, $6)",
        )
        .bind(&author.id)
        .bind(author.user_id.to_string())
        .bind(&author.tenant_id)
        .bind(&author.display_name)
        .bind(author.created_at.to_rfc3339())
        .bind(author.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert coach author: {e}")))?;

        Ok(author.id.clone())
    }

    async fn seed_insert_store_listing_if_absent(
        &self,
        listing: &SeedStoreListing,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO store_listings \
             (id, coach_id, tenant_id, publish_status, published_at, install_count, \
              author_id, created_at, updated_at) \
             VALUES ($1, $2, $3, 'published', $4, 0, $5, $6, $7)",
        )
        .bind(&listing.id)
        .bind(&listing.coach_id)
        .bind(listing.tenant_id)
        .bind(listing.created_at.to_rfc3339())
        .bind(&listing.author_id)
        .bind(listing.created_at.to_rfc3339())
        .bind(listing.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert store listing: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}
