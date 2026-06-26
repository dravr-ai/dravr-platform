// ABOUTME: Repository trait definitions for the prescribed workouts, templates, routes, training history domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::PrescribedWorkout;
use pierre_core::models::TenantId;
use pierre_core::models::{DailyTrainingState, WorkoutTemplate};
use uuid::Uuid;

/// Audit-trail CRUD for the `prescribed_workouts` table.
///
/// Each row records a single prescription pushed to a provider calendar
/// (Endurance Phase 5). The table is append-only from the API path; the
/// repo exposes upsert by id (so retries can update the
/// `provider_event_id`) plus a tenant-scoped recent-list read.
#[async_trait]
pub trait PrescribedWorkoutRepository: Send + Sync {
    /// Insert or update a prescription audit row.
    async fn upsert_prescribed_workout(&self, prescribed: &PrescribedWorkout) -> AppResult<()>;

    /// List the most recent `limit` prescriptions for a (`tenant_id`, `user_id`).
    async fn list_prescribed_workouts(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<PrescribedWorkout>>;
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
