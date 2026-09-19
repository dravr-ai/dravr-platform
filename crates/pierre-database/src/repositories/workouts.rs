// ABOUTME: Repository trait definitions for the training history domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::DailyTrainingState;
use pierre_core::models::TenantId;
use uuid::Uuid;

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

    /// Delete this user's rows in `[from, to]` inclusive, returning the count.
    ///
    /// The rollup is upsert-only everywhere else, because a recompute overwrites
    /// the days it covers. That leaves a gap the writer must close itself: when
    /// a compute determines it cannot stand behind a span — too little stored
    /// history to warm the CTL EMA — declining to write leaves whatever was
    /// there before, and `ctl`/`atl`/`tsb` carry no marker distinguishing a row
    /// vouched for from one an earlier, less careful path fabricated. The reader
    /// then serves the stale value as current.
    ///
    /// So the writer clears the span it just proved it cannot vouch for. Scoped
    /// to one user, one tenant and an explicit date range — never a bulk purge.
    async fn delete_training_history_range(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> AppResult<u64>;
}
