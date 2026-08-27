// ABOUTME: Repository trait definitions for the prescribed workouts, templates, routes, training history domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::AppResult;

use pierre_core::models::PrescribedWorkout;
use pierre_core::models::TenantId;
use pierre_core::models::{DailyTrainingState, WorkoutTemplate};
use uuid::Uuid;

/// CRUD for the `prescribed_workouts` table — the ledger of every calendar
/// entry Dravr wrote to a provider.
///
/// Each row records one write attempt: a single prescription, or one entry of
/// a plan push. Rows are never edited into a different entry; a re-push of the
/// same key writes a new `pushed` row and moves the old one to `replaced`, so
/// the partial unique index on (`tenant_id`, `user_id`, `provider`,
/// `external_id`) `WHERE status = 'pushed'` holds one live row per key.
#[async_trait]
pub trait PrescribedWorkoutRepository: Send + Sync {
    /// Insert a ledger row, or refresh an existing row's outcome fields
    /// (`provider_event_id`, `status`, payload, hash, `updated_at`) by id.
    async fn upsert_prescribed_workout(&self, prescribed: &PrescribedWorkout) -> AppResult<()>;

    /// List the most recent `limit` rows for a (`tenant_id`, `user_id`),
    /// newest first.
    async fn list_prescribed_workouts(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<PrescribedWorkout>>;

    /// Fetch one row by id. Returns `None` when no row matches the
    /// (`tenant_id`, `user_id`, `id`) tuple — a row of another athlete is
    /// indistinguishable from a missing one, by design.
    async fn get_prescribed_workout(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
    ) -> AppResult<Option<PrescribedWorkout>>;

    /// List the rows whose entry is live on `provider` (`status = pushed`),
    /// on or after `from` when given, in calendar order.
    async fn list_live_calendar_events(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        provider: &str,
        from: Option<NaiveDate>,
    ) -> AppResult<Vec<PrescribedWorkout>>;

    /// Move a row to `status` (`replaced` or `withdrawn`), stamping
    /// `updated_at`. Errors when no row matches the (`tenant_id`, `id`) pair.
    async fn set_prescribed_workout_status(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        status: &str,
    ) -> AppResult<()>;
}

/// CRUD for user-authored Endurance workout templates.
///
/// The six compiled-in cornerstones live in `workout_templates/*.toml` and
/// are loaded by `pierre_server::services::workout_library`. This repo only
/// owns rows the user authored at runtime: every persisted row therefore
/// carries `tenant_id` and `user_id` (the migration tolerates `NULL` for
/// historical reasons but the trait rejects either as `None` so the table
/// never duplicates the read-only cornerstone library).
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

/// CRUD for the `route_summaries` cache table.
///
/// Stores parsed-GPX terrain + climbs JSON keyed by `(tenant_id, user_id,
/// activity_id)` so the route endpoint can skip re-parsing when the
/// underlying GPX hash matches. Cache freshness check is the
/// caller's responsibility — `get_route_summary` returns `None` when the
/// row is missing OR when the supplied `expected_hash` does not match.
#[async_trait]
pub trait RouteSummaryRepository: Send + Sync {
    /// Insert or update the cached terrain + climbs JSON for an activity.
    async fn upsert_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        gpx_hash: &str,
        terrain_summary_json: &str,
        climbs_json: &str,
    ) -> AppResult<()>;

    /// Fetch the cached entry. Returns `None` when the row is missing or
    /// when `expected_hash` does not match the stored `gpx_hash`.
    async fn get_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        expected_hash: &str,
    ) -> AppResult<Option<(String, String)>>;
}

/// Daily training-state rollup CRUD backing the `training_history` table.
///
/// Each row captures one day of derived training metrics (CTL/ATL/TSB/ACWR/
/// monotony/strain/`ramp_rate`/`daily_load`) for a single (`tenant_id`, `user_id`).
/// Computation is the responsibility of
/// [`pierre_fitness_compute::training_history_compute`]; this repo only persists.
#[async_trait]
pub trait TrainingHistoryRepository: Send + Sync {
    /// Insert or update a single day's row.
    async fn upsert_training_history_day(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        state: &DailyTrainingState,
    ) -> AppResult<()>;

    /// Insert or update many days at once. Implementations may batch the
    /// underlying writes; callers must not assume atomicity across rows.
    async fn upsert_training_history_batch(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        states: &[DailyTrainingState],
    ) -> AppResult<()>;

    /// Fetch all rows in `[from, to]` (inclusive on both ends) in
    /// chronological order (oldest first).
    async fn get_training_history(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> AppResult<Vec<DailyTrainingState>>;

    /// Fetch the most recent row for the user, if any.
    async fn latest_training_history(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<DailyTrainingState>>;
}
