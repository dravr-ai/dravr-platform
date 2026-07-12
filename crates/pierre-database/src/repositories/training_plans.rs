// ABOUTME: TrainingPlanRepository trait — persistence for coach-authored training plans
// ABOUTME: Dual SQLite/Postgres impls live in database/ and backends/postgres/. Tenant-scoped throughout.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::{
    GoalRace, PlanBlock, PlanStatus, PlanWeek, PlannedDay, TrainingPlan, WeekStatus,
};

/// A new plan outline to persist. Saving supersedes the athlete's current
/// active outline for the same coach (whole-row supersession, never
/// mutation), so there is no separate "update" call.
pub struct SaveTrainingPlanParams<'a> {
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the plan is for.
    pub user_id: &'a str,
    /// Coach persona slug, or `None` for a coach-agnostic plan.
    pub coach_slug: Option<&'a str>,
    /// Pillar `Goal` user-fact this plan serves, when linked.
    pub goal_fact_id: Option<&'a str>,
    /// Snapshot of the goal race at plan time.
    pub goal_race: &'a GoalRace,
    /// Secondary races on the calendar.
    pub races: &'a [GoalRace],
    /// The coach's strategy in prose.
    pub strategy: &'a str,
    /// Ordered mesocycle blocks.
    pub blocks: &'a [PlanBlock],
    /// Conversation the plan was agreed in, for provenance.
    pub source_conversation_id: Option<&'a str>,
}

/// A new microcycle (or adjusted re-save of one) to persist. Saving
/// supersedes the plan's current active row for the same `week_start`.
pub struct SavePlanWeekParams<'a> {
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the week is for.
    pub user_id: &'a str,
    /// Outline this week belongs to.
    pub plan_id: &'a str,
    /// Civil date of the week's first day, `YYYY-MM-DD`.
    pub week_start: &'a str,
    /// The week's intent in coach voice.
    pub focus: &'a str,
    /// The day rows, in date order (at most seven).
    pub days: &'a [PlannedDay],
    /// Why the coach re-saved this week; empty on first save.
    pub adjustment_reason: &'a str,
}

/// Persistence for coach-authored training plans.
///
/// Plans are **tenant-scoped**: every query carries `tenant_id` in its
/// `WHERE` clause. `coach_slug` is stored as `''` for coach-agnostic rows —
/// the repository maps `Option<&str>` <-> `''` at the boundary so the
/// one-active-per-coach uniqueness key constrains those rows too (mirrors
/// [`super::playbooks::PlaybookRepository`]).
#[async_trait]
pub trait TrainingPlanRepository: Send + Sync {
    /// Persist a new plan outline, superseding the athlete's current active
    /// outline for the same coach in the same transaction. The new row's
    /// `supersedes_id` points at the replaced outline (audit chain). Returns
    /// the stored plan.
    async fn save_training_plan(
        &self,
        params: &SaveTrainingPlanParams<'_>,
    ) -> AppResult<TrainingPlan>;

    /// Persist one microcycle, superseding the plan's current active row for
    /// the same `week_start` in the same transaction ("move Tuesday to
    /// Wednesday" is a whole-week re-save). Fails with `invalid_input` when
    /// `plan_id` is not an existing plan of this tenant + user — a week must
    /// never attach to another athlete's plan.
    async fn save_plan_week(&self, params: &SavePlanWeekParams<'_>) -> AppResult<PlanWeek>;

    /// Fetch the athlete's active outline. When `coach_slug` is `Some`,
    /// prefers that coach's plan and falls back to a coach-agnostic (`''`)
    /// one; when `None`, only coach-agnostic. Returns `None` when the athlete
    /// has no active plan.
    async fn get_active_plan(
        &self,
        tenant_id: &str,
        user_id: &str,
        coach_slug: Option<&str>,
    ) -> AppResult<Option<TrainingPlan>>;

    /// List a plan's weeks in calendar order (`week_start` ascending).
    /// `include_superseded` adds the adjustment history; otherwise only
    /// active rows are returned.
    async fn list_plan_weeks(
        &self,
        tenant_id: &str,
        user_id: &str,
        plan_id: &str,
        include_superseded: bool,
    ) -> AppResult<Vec<PlanWeek>>;
}

// ============================================================================
// Shared row shapes + mapping (single parse path for both backends)
// ============================================================================

/// Raw `training_plans` row as read from either backend.
pub struct TrainingPlanRow {
    /// `id` column.
    pub id: String,
    /// `tenant_id` column.
    pub tenant_id: String,
    /// `user_id` column.
    pub user_id: String,
    /// `coach_slug` column (`''` = coach-agnostic).
    pub coach_slug: String,
    /// `goal_fact_id` column.
    pub goal_fact_id: Option<String>,
    /// `goal_race_json` column.
    pub goal_race_json: String,
    /// `races_json` column.
    pub races_json: String,
    /// `strategy` column.
    pub strategy: String,
    /// `blocks_json` column.
    pub blocks_json: String,
    /// `status` column.
    pub status: String,
    /// `supersedes_id` column.
    pub supersedes_id: Option<String>,
    /// `source_conversation_id` column.
    pub source_conversation_id: Option<String>,
    /// `created_at` epoch seconds.
    pub created_at: i64,
    /// `updated_at` epoch seconds.
    pub updated_at: i64,
}

/// Raw `training_plan_weeks` row as read from either backend.
pub struct PlanWeekRow {
    /// `id` column.
    pub id: String,
    /// `tenant_id` column.
    pub tenant_id: String,
    /// `user_id` column.
    pub user_id: String,
    /// `plan_id` column.
    pub plan_id: String,
    /// `week_start` column (`YYYY-MM-DD`).
    pub week_start: String,
    /// `focus` column.
    pub focus: String,
    /// `days_json` column.
    pub days_json: String,
    /// `status` column.
    pub status: String,
    /// `supersedes_id` column.
    pub supersedes_id: Option<String>,
    /// `adjustment_reason` column.
    pub adjustment_reason: String,
    /// `created_at` epoch seconds.
    pub created_at: i64,
    /// `updated_at` epoch seconds.
    pub updated_at: i64,
}

/// Convert epoch seconds to `DateTime<Utc>`, treating an out-of-range value
/// as corruption rather than silently clamping.
fn epoch_to_datetime(epoch: i64, column: &str) -> AppResult<DateTime<Utc>> {
    DateTime::from_timestamp(epoch, 0)
        .ok_or_else(|| AppError::database(format!("training plan {column} out of range: {epoch}")))
}

/// Map a raw outline row to the domain type. Shared by both backends so JSON
/// and enum parsing live in exactly one place.
pub(crate) fn training_plan_from_row(row: TrainingPlanRow) -> AppResult<TrainingPlan> {
    let goal_race: GoalRace = serde_json::from_str(&row.goal_race_json)
        .map_err(|e| AppError::database(format!("training plan goal_race_json: {e}")))?;
    let races: Vec<GoalRace> = serde_json::from_str(&row.races_json)
        .map_err(|e| AppError::database(format!("training plan races_json: {e}")))?;
    let blocks: Vec<PlanBlock> = serde_json::from_str(&row.blocks_json)
        .map_err(|e| AppError::database(format!("training plan blocks_json: {e}")))?;
    let status = PlanStatus::parse(&row.status).ok_or_else(|| {
        AppError::database(format!("unknown training plan status: {}", row.status))
    })?;
    Ok(TrainingPlan {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        coach_slug: (!row.coach_slug.is_empty()).then_some(row.coach_slug),
        goal_fact_id: row.goal_fact_id,
        goal_race,
        races,
        strategy: row.strategy,
        blocks,
        status,
        supersedes_id: row.supersedes_id,
        source_conversation_id: row.source_conversation_id,
        created_at: epoch_to_datetime(row.created_at, "created_at")?,
        updated_at: epoch_to_datetime(row.updated_at, "updated_at")?,
    })
}

/// Map a raw week row to the domain type. Shared by both backends.
pub(crate) fn plan_week_from_row(row: PlanWeekRow) -> AppResult<PlanWeek> {
    let days: Vec<PlannedDay> = serde_json::from_str(&row.days_json)
        .map_err(|e| AppError::database(format!("plan week days_json: {e}")))?;
    let status = WeekStatus::parse(&row.status)
        .ok_or_else(|| AppError::database(format!("unknown plan week status: {}", row.status)))?;
    Ok(PlanWeek {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        plan_id: row.plan_id,
        week_start: row.week_start,
        focus: row.focus,
        days,
        status,
        supersedes_id: row.supersedes_id,
        adjustment_reason: row.adjustment_reason,
        created_at: epoch_to_datetime(row.created_at, "created_at")?,
        updated_at: epoch_to_datetime(row.updated_at, "updated_at")?,
    })
}

/// Serialized column values for an outline insert, shared by both backends
/// so the JSON encoding happens once and identically.
pub(crate) struct PlanInsertValues {
    /// New row id.
    pub id: String,
    /// `''`-normalized coach slug.
    pub coach_slug: String,
    /// Serialized goal-race snapshot.
    pub goal_race_json: String,
    /// Serialized secondary races.
    pub races_json: String,
    /// Serialized blocks.
    pub blocks_json: String,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// Build the serialized insert values for [`SaveTrainingPlanParams`].
pub(crate) fn plan_insert_values(
    params: &SaveTrainingPlanParams<'_>,
) -> AppResult<PlanInsertValues> {
    let goal_race_json = serde_json::to_string(params.goal_race)
        .map_err(|e| AppError::internal(format!("serialize goal race: {e}")))?;
    let races_json = serde_json::to_string(params.races)
        .map_err(|e| AppError::internal(format!("serialize races: {e}")))?;
    let blocks_json = serde_json::to_string(params.blocks)
        .map_err(|e| AppError::internal(format!("serialize blocks: {e}")))?;
    Ok(PlanInsertValues {
        id: uuid::Uuid::new_v4().to_string(),
        coach_slug: params.coach_slug.unwrap_or_default().to_owned(),
        goal_race_json,
        races_json,
        blocks_json,
        now: Utc::now().timestamp(),
    })
}

/// Serialized column values for a week insert, shared by both backends.
pub(crate) struct WeekInsertValues {
    /// New row id.
    pub id: String,
    /// Serialized day rows.
    pub days_json: String,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// Build the serialized insert values for [`SavePlanWeekParams`].
pub(crate) fn week_insert_values(params: &SavePlanWeekParams<'_>) -> AppResult<WeekInsertValues> {
    let days_json = serde_json::to_string(params.days)
        .map_err(|e| AppError::internal(format!("serialize plan days: {e}")))?;
    Ok(WeekInsertValues {
        id: uuid::Uuid::new_v4().to_string(),
        days_json,
        now: Utc::now().timestamp(),
    })
}
