// ABOUTME: WorkoutTemplateRepository trait plus the one shared implementation both backends emit (Endurance Phase 5)
// ABOUTME: Stores user-authored templates only; the seven *_json columns are opaque blobs bound as serde_json::Value
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{EvidenceTier, PhaseFit, Progression, WorkoutParams};
use pierre_core::models::{
    IntensityDistribution, SportType, TenantId, UserId, WorkoutStep, WorkoutTargetZones,
    WorkoutTemplate,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::uuid_column::UuidColumn;

/// CRUD for user-authored Endurance workout templates.
///
/// The catalogue bank lives in `training_catalogue/workouts/*.toml` and is
/// held by `pierre_contremaitre::TrainingCatalogueRegistry`. This repo only
/// owns rows the user authored at runtime: every persisted row therefore
/// carries `tenant_id` and `user_id` (the migration tolerates `NULL` for
/// historical reasons but the trait rejects either as `None` so the table
/// never duplicates the read-only catalogue bank).
#[async_trait]
pub trait WorkoutTemplateRepository: Send + Sync {
    /// Insert or update a user-authored workout template.
    ///
    /// `template.tenant_id` and `template.user_id` MUST both be `Some` — the
    /// implementation returns an [`AppError::invalid_input`] otherwise.
    async fn upsert_workout_template(&self, template: &WorkoutTemplate) -> AppResult<()>;

    /// List all user-authored templates for (`tenant_id`, `user_id`),
    /// ordered by `updated_at` descending.
    async fn list_user_workout_templates(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<WorkoutTemplate>>;

    /// Look up a single user-authored template by slug. Returns `None`
    /// when no row matches the (`tenant_id`, `user_id`, `slug`) tuple.
    async fn get_user_workout_template(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        slug: &str,
    ) -> AppResult<Option<WorkoutTemplate>>;
}

/// The column list every read projects, in the order [`template_from_row`]
/// names them.
macro_rules! template_columns {
    () => {
        "id, tenant_id, user_id, slug, name, sport, \
         duration_minutes, intensity_distribution, \
         structure_json, target_zones_json, \
         purpose, sport_variants_json, evidence_tier, caveat, \
         params_json, progression_json, fit_json, evidence_refs_json, \
         is_compiled_in, updated_at"
    };
}

/// Insert a user-authored template, or refresh the row sharing its id.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and `$19` serves both timestamp columns on a fresh row. `id`
/// binds as [`UuidColumn`], `tenant_id` as [`TenantId`] and `user_id` as
/// [`UserId`], each of which encodes as hyphenated text on `SQLite` and as
/// a native `uuid` on Postgres. `duration_minutes` binds as `i32` (Postgres
/// `INTEGER` is four bytes). `is_compiled_in` is the literal `FALSE`, which
/// `SQLite` reads as 0 in its `INTEGER` column. `updated_at` binds as
/// `DateTime<Utc>`: RFC3339 text on `SQLite`, `TIMESTAMPTZ` on Postgres.
/// The seven `*_json` columns bind as [`Value`], which sqlx stores as
/// `TEXT` on `SQLite` and `jsonb` on Postgres: the stored JSON is an opaque
/// blob whose only reader is [`template_from_row`], which parses each back
/// into its typed field.
pub(crate) const UPSERT_WORKOUT_TEMPLATE_SQL: &str = r"
            INSERT INTO workout_templates (
                id, tenant_id, user_id, slug, name, sport,
                duration_minutes, intensity_distribution,
                structure_json, target_zones_json,
                purpose, sport_variants_json, evidence_tier, caveat,
                params_json, progression_json, fit_json, evidence_refs_json,
                is_compiled_in, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                    $11, $12, $13, $14, $15, $16, $17, $18,
                    FALSE, $19, $19)
            ON CONFLICT (id) DO UPDATE SET
                slug = EXCLUDED.slug,
                name = EXCLUDED.name,
                sport = EXCLUDED.sport,
                duration_minutes = EXCLUDED.duration_minutes,
                intensity_distribution = EXCLUDED.intensity_distribution,
                structure_json = EXCLUDED.structure_json,
                target_zones_json = EXCLUDED.target_zones_json,
                purpose = EXCLUDED.purpose,
                sport_variants_json = EXCLUDED.sport_variants_json,
                evidence_tier = EXCLUDED.evidence_tier,
                caveat = EXCLUDED.caveat,
                params_json = EXCLUDED.params_json,
                progression_json = EXCLUDED.progression_json,
                fit_json = EXCLUDED.fit_json,
                evidence_refs_json = EXCLUDED.evidence_refs_json,
                updated_at = EXCLUDED.updated_at
            ";

/// Every user-authored template of one athlete, most recently updated first.
pub(crate) const LIST_USER_WORKOUT_TEMPLATES_SQL: &str = concat!(
    "
            SELECT ",
    template_columns!(),
    "
            FROM workout_templates
            WHERE tenant_id = $1 AND user_id = $2 AND is_compiled_in = FALSE
            ORDER BY updated_at DESC
            "
);

/// One user-authored template by slug.
pub(crate) const GET_USER_WORKOUT_TEMPLATE_SQL: &str = concat!(
    "
            SELECT ",
    template_columns!(),
    "
            FROM workout_templates
            WHERE tenant_id = $1 AND user_id = $2 AND slug = $3 AND is_compiled_in = FALSE
            "
);

/// The JSON value a `*_json` column stores for `value`.
///
/// # Errors
/// Returns a database error naming `name` when `value` cannot be serialized.
pub(crate) fn json_column<T: Serialize>(name: &str, value: &T) -> AppResult<Value> {
    serde_json::to_value(value).map_err(|e| AppError::database(format!("serialize {name}: {e}")))
}

/// The JSON text an enum-valued `TEXT` column stores for `value`.
///
/// # Errors
/// Returns a database error naming `name` when `value` cannot be serialized.
pub(crate) fn json_text<T: Serialize>(name: &str, value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|e| AppError::database(format!("serialize {name}: {e}")))
}

/// Read a `*_json` column back into its type.
fn read_json<R, T>(row: &R, col: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Value: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    T: DeserializeOwned,
{
    let value: Value = row
        .try_get(col)
        .map_err(|e| AppError::database(format!("read {col}: {e}")))?;
    serde_json::from_value(value).map_err(|e| AppError::database(format!("parse {col}: {e}")))
}

/// Read a vocabulary column — the `snake_case` name a `vocab_enum` writes —
/// back into its enum through serde, the one place the names are defined.
fn read_vocab<R, T>(row: &R, col: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    T: DeserializeOwned,
{
    let text: String = row
        .try_get(col)
        .map_err(|e| AppError::database(format!("read {col}: {e}")))?;
    serde_json::from_value(Value::String(text))
        .map_err(|e| AppError::database(format!("parse {col}: {e}")))
}

/// Rebuild a [`WorkoutTemplate`] from one row of [`template_columns`].
///
/// # Errors
/// Returns a database error when a column cannot be decoded or a stored
/// blob no longer parses into its typed field.
pub(crate) fn template_from_row<R>(row: &R) -> AppResult<WorkoutTemplate>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Value: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UuidColumn: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UserId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let id: UuidColumn = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("read id: {e}")))?;
    let tenant_id: Option<TenantId> = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("read tenant_id: {e}")))?;
    let user_id: Option<UserId> = row
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
    let is_compiled_in: bool = row
        .try_get("is_compiled_in")
        .map_err(|e| AppError::database(format!("read is_compiled_in: {e}")))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("read updated_at: {e}")))?;

    Ok(WorkoutTemplate {
        id: id.into(),
        tenant_id: tenant_id.map(|t| t.as_uuid()),
        user_id: user_id.map(|u| u.as_uuid()),
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

/// Emit the whole [`WorkoutTemplateRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion.
macro_rules! impl_workout_template_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl WorkoutTemplateRepository for $ty {
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
                let sport = json_text("sport", &template.sport)?;
                let intensity_distribution =
                    json_text("intensity_distribution", &template.intensity_distribution)?;
                let structure_value = json_column("structure", &template.structure)?;
                let target_zones_value = json_column("target_zones", &template.target_zones)?;
                let sport_variants_value =
                    json_column("sport_variants", &template.sport_variants)?;
                let params_value = json_column("params", &template.params)?;
                let progression_value = json_column("progression", &template.progression)?;
                let fit_value = json_column("fit", &template.fit)?;
                let evidence_refs_value = json_column("evidence_refs", &template.evidence_refs)?;
                let duration_minutes = i32::try_from(template.duration_minutes).map_err(|e| {
                    AppError::database(format!("duration_minutes out of i32 range: {e}"))
                })?;

                sqlx::query(UPSERT_WORKOUT_TEMPLATE_SQL)
                    .bind(UuidColumn(template.id))
                    .bind(TenantId::from_uuid(tenant_id))
                    .bind(UserId::from_uuid(user_id))
                    .bind(&template.slug)
                    .bind(&template.name)
                    .bind(&sport)
                    .bind(duration_minutes)
                    .bind(&intensity_distribution)
                    .bind(structure_value)
                    .bind(target_zones_value)
                    .bind(template.purpose.as_str())
                    .bind(sport_variants_value)
                    .bind(template.evidence_tier.as_str())
                    .bind(template.caveat.as_deref())
                    .bind(params_value)
                    .bind(progression_value)
                    .bind(fit_value)
                    .bind(evidence_refs_value)
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
                let rows = sqlx::query(LIST_USER_WORKOUT_TEMPLATES_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("list_user_workout_templates: {e}"))
                    })?;
                rows.iter().map(template_from_row).collect()
            }

            async fn get_user_workout_template(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                slug: &str,
            ) -> AppResult<Option<WorkoutTemplate>> {
                let row = sqlx::query(GET_USER_WORKOUT_TEMPLATE_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .bind(slug)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_user_workout_template: {e}")))?;
                row.as_ref().map(template_from_row).transpose()
            }
        }
    };
}
pub(crate) use impl_workout_template_repository;
