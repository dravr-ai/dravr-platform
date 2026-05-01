// ABOUTME: PostgreSQL implementation of WorkoutTemplateRepository (Endurance Phase 5)
// ABOUTME: Mirrors crates/pierre-database/src/database/workout_templates.rs for the Postgres tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    IntensityDistribution, SportType, TenantId, WorkoutStep, WorkoutTargetZones, WorkoutTemplate,
};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::WorkoutTemplateRepository;

#[async_trait]
impl WorkoutTemplateRepository for PostgresDatabase {
    async fn upsert_workout_template(&self, template: &WorkoutTemplate) -> AppResult<()> {
        let tenant_id = template.tenant_id.ok_or_else(|| {
            AppError::invalid_input(
                "user-authored workout templates must carry a tenant_id (cornerstones live in TOML)",
            )
        })?;
        let user_id = template.user_id.ok_or_else(|| {
            AppError::invalid_input(
                "user-authored workout templates must carry a user_id (cornerstones live in TOML)",
            )
        })?;
        let sport = serde_json::to_string(&template.sport)
            .map_err(|e| AppError::database(format!("serialize sport: {e}")))?;
        let intensity_distribution = serde_json::to_string(&template.intensity_distribution)
            .map_err(|e| AppError::database(format!("serialize intensity_distribution: {e}")))?;
        let structure_value = serde_json::to_value(&template.structure)
            .map_err(|e| AppError::database(format!("serialize structure: {e}")))?;
        let target_zones_value = serde_json::to_value(&template.target_zones)
            .map_err(|e| AppError::database(format!("serialize target_zones: {e}")))?;
        let duration_minutes = i32::try_from(template.duration_minutes)
            .map_err(|e| AppError::database(format!("duration_minutes out of i32 range: {e}")))?;

        sqlx::query(
            r"
            INSERT INTO workout_templates (
                id, tenant_id, user_id, slug, name, sport,
                duration_minutes, intensity_distribution,
                structure_json, target_zones_json,
                is_compiled_in, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, FALSE, $11, $11)
            ON CONFLICT (id) DO UPDATE SET
                slug = EXCLUDED.slug,
                name = EXCLUDED.name,
                sport = EXCLUDED.sport,
                duration_minutes = EXCLUDED.duration_minutes,
                intensity_distribution = EXCLUDED.intensity_distribution,
                structure_json = EXCLUDED.structure_json,
                target_zones_json = EXCLUDED.target_zones_json,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(template.id)
        .bind(tenant_id)
        .bind(user_id)
        .bind(&template.slug)
        .bind(&template.name)
        .bind(&sport)
        .bind(duration_minutes)
        .bind(&intensity_distribution)
        .bind(structure_value)
        .bind(target_zones_value)
        .bind(template.updated_at)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("upsert_workout_template: {e}")))?;
        Ok(())
    }

    async fn list_user_workout_templates(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<WorkoutTemplate>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, slug, name, sport,
                   duration_minutes, intensity_distribution,
                   structure_json, target_zones_json,
                   is_compiled_in, updated_at
            FROM workout_templates
            WHERE tenant_id = $1 AND user_id = $2 AND is_compiled_in = FALSE
            ORDER BY updated_at DESC
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list_user_workout_templates: {e}")))?;

        rows.iter().map(row_to_template).collect()
    }

    async fn get_user_workout_template(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        slug: &str,
    ) -> AppResult<Option<WorkoutTemplate>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, slug, name, sport,
                   duration_minutes, intensity_distribution,
                   structure_json, target_zones_json,
                   is_compiled_in, updated_at
            FROM workout_templates
            WHERE tenant_id = $1 AND user_id = $2 AND slug = $3 AND is_compiled_in = FALSE
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(slug)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("get_user_workout_template: {e}")))?;

        row.as_ref().map(row_to_template).transpose()
    }
}

fn row_to_template(row: &PgRow) -> AppResult<WorkoutTemplate> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("read id: {e}")))?;
    let tenant_id: Option<Uuid> = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("read tenant_id: {e}")))?;
    let user_id: Option<Uuid> = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("read user_id: {e}")))?;
    let slug: String = row
        .try_get("slug")
        .map_err(|e| AppError::database(format!("read slug: {e}")))?;
    let name: String = row
        .try_get("name")
        .map_err(|e| AppError::database(format!("read name: {e}")))?;
    let sport_str: String = row
        .try_get("sport")
        .map_err(|e| AppError::database(format!("read sport: {e}")))?;
    let sport: SportType = serde_json::from_str(&sport_str)
        .map_err(|e| AppError::database(format!("parse sport {sport_str}: {e}")))?;
    let duration_minutes: i32 = row
        .try_get("duration_minutes")
        .map_err(|e| AppError::database(format!("read duration_minutes: {e}")))?;
    let duration_minutes = u32::try_from(duration_minutes)
        .map_err(|e| AppError::database(format!("duration_minutes out of u32 range: {e}")))?;
    let intensity_str: String = row
        .try_get("intensity_distribution")
        .map_err(|e| AppError::database(format!("read intensity_distribution: {e}")))?;
    let intensity_distribution: IntensityDistribution = serde_json::from_str(&intensity_str)
        .map_err(|e| AppError::database(format!("parse intensity_distribution: {e}")))?;
    let structure_value: Value = row
        .try_get("structure_json")
        .map_err(|e| AppError::database(format!("read structure_json: {e}")))?;
    let structure: Vec<WorkoutStep> = serde_json::from_value(structure_value)
        .map_err(|e| AppError::database(format!("parse structure_json: {e}")))?;
    let target_zones_value: Value = row
        .try_get("target_zones_json")
        .map_err(|e| AppError::database(format!("read target_zones_json: {e}")))?;
    let target_zones: WorkoutTargetZones = serde_json::from_value(target_zones_value)
        .map_err(|e| AppError::database(format!("parse target_zones_json: {e}")))?;
    let is_compiled_in: bool = row
        .try_get("is_compiled_in")
        .map_err(|e| AppError::database(format!("read is_compiled_in: {e}")))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("read updated_at: {e}")))?;

    Ok(WorkoutTemplate {
        id,
        tenant_id,
        user_id,
        slug,
        name,
        sport,
        duration_minutes,
        intensity_distribution,
        structure,
        target_zones,
        is_compiled_in,
        updated_at,
    })
}
