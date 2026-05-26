// ABOUTME: Repository trait definitions for the coach-athlete roster persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::CoachAthleteAssignment;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// 1:N coach → athlete roster assignment repository.
///
/// Backed by the `coach_athlete_assignments` table. All queries are
/// tenant-scoped — the route layer is responsible for verifying both
/// the coach and the athlete belong to the same tenant before calling
/// `assign_athlete`. Active assignments have `revoked_at IS NULL`.
#[async_trait]
pub trait RosterRepository: Send + Sync {
    /// List the active assignments where `coach_user_id` is coaching
    /// other users in `tenant_id`. Ordered by `assigned_at DESC` so the
    /// most recent assignments surface first.
    async fn list_athletes_for_coach(
        &self,
        coach_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAthleteAssignment>>;

    /// List the active assignments where `athlete_user_id` is being
    /// coached by other users in `tenant_id`. The inverse of
    /// `list_athletes_for_coach`, used by the athlete-side roster view.
    async fn list_coaches_for_athlete(
        &self,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAthleteAssignment>>;

    /// Insert a new assignment row. Returns `Ok(None)` when an active
    /// assignment for the same `(coach, athlete, tenant)` already exists
    /// (the unique partial index guards against duplicates). Caller
    /// MUST verify both users belong to `tenant_id` before invoking.
    async fn assign_athlete(
        &self,
        assignment: &CoachAthleteAssignment,
    ) -> AppResult<Option<CoachAthleteAssignment>>;

    /// Mark the active assignment for `(coach, athlete, tenant)` as
    /// revoked. Returns `true` when a row was updated, `false` when no
    /// active assignment matched. The audit row stays.
    async fn revoke_assignment(
        &self,
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
        revoked_by: Option<Uuid>,
    ) -> AppResult<bool>;

    /// `true` when an active assignment exists for the given triple.
    /// Used by route handlers to gate athlete-data reads.
    async fn is_athlete_managed_by(
        &self,
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
}
