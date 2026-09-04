// ABOUTME: SQLite implementation of WorkoutTemplateRepository (Endurance Phase 5)
// ABOUTME: Stores user-authored templates only; the catalogue bank lives in training_catalogue/workouts/*.toml
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{EvidenceTier, PhaseFit, Progression, WorkoutParams};
use pierre_core::models::{
    IntensityDistribution, SportType, TenantId, WorkoutStep, WorkoutTargetZones, WorkoutTemplate,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::WorkoutTemplateRepository;

#[async_trait]
impl WorkoutTemplateRepository for Database {
    async fn upsert_workout_template(&self, template: &WorkoutTemplate) -> AppResult<()> {
        let tenant_id = template.tenant_id.ok_or_else(|| {
            AppError::invalid_input(
                "user-authored workout templates must carry a tenant_id (catalogue templates live in TOML)",
            )
        })?;
        let user_id = template.user_id.ok_or_else(|| {
            AppError::invalid_input(
                "user-authored workout templates must carry a user_id (catalogue templates live in TOML)",
            )
        })?;
        let sport = serde_json::to_string(&template.sport)
            .map_err(|e| AppError::database(format!("serialize sport: {e}")))?;
        let intensity_distribution = serde_json::to_string(&template.intensity_distribution)
            .map_err(|e| AppError::database(format!("serialize intensity_distribution: {e}")))?;
        let structure_json = json_column("structure", &template.structure)?;
        let target_zones_json = json_column("target_zones", &template.target_zones)?;
        let sport_variants_json = json_column("sport_variants", &template.sport_variants)?;
        let params_json = json_column("params", &template.params)?;
        let progression_json = json_column("progression", &template.progression)?;
        let fit_json = json_column("fit", &template.fit)?;
        let evidence_refs_json = json_column("evidence_refs", &template.evidence_refs)?;
        let duration_minutes = i64::from(template.duration_minutes);
        let updated_at = template.updated_at.to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO workout_templates (
                id, tenant_id, user_id, slug, name, sport,
                duration_minutes, intensity_distribution,
                structure_json, target_zones_json,
                purpose, sport_variants_json, evidence_tier, caveat,
                params_json, progression_json, fit_json, evidence_refs_json,
                is_compiled_in, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                slug = excluded.slug,
                name = excluded.name,
                sport = excluded.sport,
                duration_minutes = excluded.duration_minutes,
                intensity_distribution = excluded.intensity_distribution,
                structure_json = excluded.structure_json,
                target_zones_json = excluded.target_zones_json,
                purpose = excluded.purpose,
                sport_variants_json = excluded.sport_variants_json,
                evidence_tier = excluded.evidence_tier,
                caveat = excluded.caveat,
                params_json = excluded.params_json,
                progression_json = excluded.progression_json,
                fit_json = excluded.fit_json,
                evidence_refs_json = excluded.evidence_refs_json,
                updated_at = excluded.updated_at
            ",
        )
        .bind(template.id.to_string())
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(&template.slug)
        .bind(&template.name)
        .bind(&sport)
        .bind(duration_minutes)
        .bind(&intensity_distribution)
        .bind(&structure_json)
        .bind(&target_zones_json)
        .bind(template.purpose.as_str())
        .bind(&sport_variants_json)
        .bind(template.evidence_tier.as_str())
        .bind(template.caveat.as_deref())
        .bind(&params_json)
        .bind(&progression_json)
        .bind(&fit_json)
        .bind(&evidence_refs_json)
        .bind(&updated_at)
        .bind(&updated_at)
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
                   purpose, sport_variants_json, evidence_tier, caveat,
                   params_json, progression_json, fit_json, evidence_refs_json,
                   is_compiled_in, updated_at
            FROM workout_templates
            WHERE tenant_id = ? AND user_id = ? AND is_compiled_in = 0
            ORDER BY updated_at DESC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
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
                   purpose, sport_variants_json, evidence_tier, caveat,
                   params_json, progression_json, fit_json, evidence_refs_json,
                   is_compiled_in, updated_at
            FROM workout_templates
            WHERE tenant_id = ? AND user_id = ? AND slug = ? AND is_compiled_in = 0
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(slug)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("get_user_workout_template: {e}")))?;

        row.as_ref().map(row_to_template).transpose()
    }
}

/// The JSON text a `*_json` column stores for `value`.
fn json_column<T: Serialize>(name: &str, value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|e| AppError::database(format!("serialize {name}: {e}")))
}

/// Read a `*_json` column back into its type.
fn read_json<T: DeserializeOwned>(row: &SqliteRow, col: &str) -> AppResult<T> {
    let text: String = row
        .try_get(col)
        .map_err(|e| AppError::database(format!("read {col}: {e}")))?;
    serde_json::from_str(&text).map_err(|e| AppError::database(format!("parse {col}: {e}")))
}

/// Read a vocabulary column — the `snake_case` name a `vocab_enum` writes —
/// back into its enum through serde, the one place the names are defined.
fn read_vocab<T: DeserializeOwned>(row: &SqliteRow, col: &str) -> AppResult<T> {
    let text: String = row
        .try_get(col)
        .map_err(|e| AppError::database(format!("read {col}: {e}")))?;
    serde_json::from_value(Value::String(text))
        .map_err(|e| AppError::database(format!("parse {col}: {e}")))
}

fn row_to_template(row: &SqliteRow) -> AppResult<WorkoutTemplate> {
    let id_str: String = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("read id: {e}")))?;
    let id = Uuid::parse_str(&id_str)
        .map_err(|e| AppError::database(format!("parse id {id_str}: {e}")))?;
    let tenant_id = optional_uuid(row, "tenant_id")?;
    let user_id = optional_uuid(row, "user_id")?;
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
    let duration_minutes: i64 = row
        .try_get("duration_minutes")
        .map_err(|e| AppError::database(format!("read duration_minutes: {e}")))?;
    let duration_minutes = u32::try_from(duration_minutes)
        .map_err(|e| AppError::database(format!("duration_minutes out of range: {e}")))?;
    let intensity_str: String = row
        .try_get("intensity_distribution")
        .map_err(|e| AppError::database(format!("read intensity_distribution: {e}")))?;
    let intensity_distribution: IntensityDistribution = serde_json::from_str(&intensity_str)
        .map_err(|e| AppError::database(format!("parse intensity_distribution: {e}")))?;
    let structure: Vec<WorkoutStep> = read_json(row, "structure_json")?;
    let target_zones: WorkoutTargetZones = read_json(row, "target_zones_json")?;
    let purpose = read_vocab(row, "purpose")?;
    let sport_variants: Vec<SportType> = read_json(row, "sport_variants_json")?;
    let evidence_tier: EvidenceTier = read_vocab(row, "evidence_tier")?;
    let caveat: Option<String> = row
        .try_get("caveat")
        .map_err(|e| AppError::database(format!("read caveat: {e}")))?;
    let params: WorkoutParams = read_json(row, "params_json")?;
    let progression: Progression = read_json(row, "progression_json")?;
    let fit: PhaseFit = read_json(row, "fit_json")?;
    let evidence_refs: Vec<String> = read_json(row, "evidence_refs_json")?;
    let is_compiled_in_int: i64 = row
        .try_get("is_compiled_in")
        .map_err(|e| AppError::database(format!("read is_compiled_in: {e}")))?;
    let is_compiled_in = is_compiled_in_int != 0;
    let updated_at_str: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("read updated_at: {e}")))?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("parse updated_at: {e}")))?;

    Ok(WorkoutTemplate {
        id,
        tenant_id,
        user_id,
        slug,
        name,
        sport,
        duration_minutes,
        intensity_distribution,
        purpose,
        sport_variants,
        evidence_tier,
        caveat,
        structure,
        target_zones,
        params,
        progression,
        fit,
        evidence_refs,
        is_compiled_in,
        updated_at,
    })
}

fn optional_uuid(row: &SqliteRow, col: &str) -> AppResult<Option<Uuid>> {
    let raw: Option<String> = row
        .try_get(col)
        .map_err(|e| AppError::database(format!("read {col}: {e}")))?;
    raw.map(|s| {
        Uuid::parse_str(&s).map_err(|e| AppError::database(format!("parse {col} {s}: {e}")))
    })
    .transpose()
}
