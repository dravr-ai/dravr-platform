// ABOUTME: TrainingPlanRepository trait — persistence for coach-authored training plans
// ABOUTME: Dual SQLite/Postgres impls live in database/ and backends/postgres/. Tenant-scoped throughout.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::{
    FlavourSelection, GoalRace, PlanPhase, PlanStatus, PlanWeek, PlannedDay, TrainingPlan,
    WeekStatus,
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
    /// The flavour the season runs on, when one was chosen.
    pub flavour: Option<&'a FlavourSelection>,
    /// First day of the season, when the plan states one.
    pub season_start: Option<&'a str>,
    /// Last day of the season, when the plan states one.
    pub season_end: Option<&'a str>,
    /// Ordered season phases.
    pub phases: &'a [PlanPhase],
    /// Conversation the plan was agreed in, for provenance.
    pub source_conversation_id: Option<&'a str>,
}

/// The outline half of a [`SavePlanBundleParams`].
///
/// Mirrors [`SaveTrainingPlanParams`] minus the identity fields the bundle
/// already carries — the bundle applies one tenant/user/coach to outline and
/// weeks alike.
pub struct PlanOutlineInput<'a> {
    /// Snapshot of the goal race at plan time.
    pub goal_race: &'a GoalRace,
    /// Secondary races on the calendar.
    pub races: &'a [GoalRace],
    /// The coach's strategy in prose.
    pub strategy: &'a str,
    /// The flavour the season runs on, when one was chosen.
    pub flavour: Option<&'a FlavourSelection>,
    /// First day of the season, when the plan states one.
    pub season_start: Option<&'a str>,
    /// Last day of the season, when the plan states one.
    pub season_end: Option<&'a str>,
    /// Ordered season phases.
    pub phases: &'a [PlanPhase],
    /// Conversation the plan was agreed in, for provenance.
    pub source_conversation_id: Option<&'a str>,
}

/// One microcycle of a [`SavePlanBundleParams`] — a new week, or an adjusted
/// re-save of one, which supersedes the plan's current active row for the same
/// `week_start`.
///
/// Carries no `plan_id`: the bundle resolves the plan (fresh outline insert or
/// existing active plan) once and attaches every week to it inside the same
/// transaction, so a week can only ever land on a plan this tenant + user owns.
pub struct PlanWeekInput<'a> {
    /// Civil date of the week's first day, `YYYY-MM-DD`.
    pub week_start: &'a str,
    /// The week's intent in coach voice.
    pub focus: &'a str,
    /// The day rows, in date order (at most seven).
    pub days: &'a [PlannedDay],
    /// Why the coach re-saved this week; empty on first save.
    pub adjustment_reason: &'a str,
    /// Index of the outline phase this week instantiates, when stated.
    pub phase_index: Option<u32>,
}

/// A whole atomic save: an optional outline plus zero or more weeks, all
/// committed in a single transaction (see
/// [`TrainingPlanRepository::save_plan_bundle`]).
pub struct SavePlanBundleParams<'a> {
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the plan is for.
    pub user_id: &'a str,
    /// Coach persona slug, or `None` for a coach-agnostic plan.
    pub coach_slug: Option<&'a str>,
    /// Pillar `Goal` user-fact this plan serves, when linked.
    pub goal_fact_id: Option<&'a str>,
    /// New outline to create (superseding the current active one), or `None`
    /// to attach the weeks to the athlete's existing active plan.
    pub outline: Option<PlanOutlineInput<'a>>,
    /// Weeks to save, each superseding the plan's current active row for its
    /// `week_start`.
    pub weeks: &'a [PlanWeekInput<'a>],
}

/// Result of [`TrainingPlanRepository::save_plan_bundle`]: the active plan the
/// weeks attached to, the weeks saved, and the outline this save superseded
/// (when an outline was created).
pub struct SavedPlanBundle {
    /// The active plan (freshly created, or the existing one weeks attached to).
    pub plan: TrainingPlan,
    /// The weeks persisted by this save, in input order.
    pub weeks: Vec<PlanWeek>,
    /// The outline this save superseded, if any.
    pub superseded_plan_id: Option<String>,
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
    /// `supersedes_id` points at the replaced outline (audit chain), and the
    /// replaced outline's still-active weeks are carried onto the new plan id
    /// so the athlete's day-by-day schedule follows it. Returns the stored plan.
    async fn save_training_plan(
        &self,
        params: &SaveTrainingPlanParams<'_>,
    ) -> AppResult<TrainingPlan>;

    /// Atomically persist an optional outline plus zero or more weeks in a
    /// **single transaction**, so a mid-payload failure can never leave the
    /// athlete with a superseded old plan and a half-populated new one. This
    /// is the only write path: one week is a bundle of one ("move Tuesday to
    /// Wednesday" is a whole-week re-save superseding that `week_start`).
    ///
    /// With an outline: supersedes the current active plan, inserts the new
    /// one, carries the superseded outline's surviving weeks onto it, then
    /// attaches every week in the payload. Without an outline: resolves the
    /// athlete's existing active plan (erroring if none) and attaches the
    /// weeks. Either way the whole set commits or none of it does.
    async fn save_plan_bundle(
        &self,
        params: &SavePlanBundleParams<'_>,
    ) -> AppResult<SavedPlanBundle>;

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
    /// `phases_json` column.
    pub phases_json: String,
    /// `flavour_json` column; `None` for a plan without a flavour.
    pub flavour_json: Option<String>,
    /// `season_start` column.
    pub season_start: Option<String>,
    /// `season_end` column.
    pub season_end: Option<String>,
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
    /// `phase_index` column; `None` when the week names no phase.
    pub phase_index: Option<i64>,
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
    let phases: Vec<PlanPhase> = serde_json::from_str(&row.phases_json)
        .map_err(|e| AppError::database(format!("training plan phases_json: {e}")))?;
    let flavour: Option<FlavourSelection> = row
        .flavour_json
        .as_deref()
        .filter(|json| !json.trim().is_empty())
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| AppError::database(format!("training plan flavour_json: {e}")))?;
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
        flavour,
        season_start: row.season_start,
        season_end: row.season_end,
        phases,
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
    let phase_index = row
        .phase_index
        .map(u32::try_from)
        .transpose()
        .map_err(|_| {
            AppError::database(format!(
                "plan week phase_index out of range: {:?}",
                row.phase_index
            ))
        })?;
    Ok(PlanWeek {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        plan_id: row.plan_id,
        week_start: row.week_start,
        focus: row.focus,
        phase_index,
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
    /// Serialized phases.
    pub phases_json: String,
    /// Serialized flavour selection, when one was chosen.
    pub flavour_json: Option<String>,
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
    let phases_json = serde_json::to_string(params.phases)
        .map_err(|e| AppError::internal(format!("serialize phases: {e}")))?;
    let flavour_json = params
        .flavour
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::internal(format!("serialize flavour: {e}")))?;
    Ok(PlanInsertValues {
        id: uuid::Uuid::new_v4().to_string(),
        coach_slug: params.coach_slug.unwrap_or_default().to_owned(),
        goal_race_json,
        races_json,
        phases_json,
        flavour_json,
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

/// Build the serialized insert values for one [`PlanWeekInput`]'s day rows.
pub(crate) fn week_insert_values(days: &[PlannedDay]) -> AppResult<WeekInsertValues> {
    let days_json = serde_json::to_string(days)
        .map_err(|e| AppError::internal(format!("serialize plan days: {e}")))?;
    Ok(WeekInsertValues {
        id: uuid::Uuid::new_v4().to_string(),
        days_json,
        now: Utc::now().timestamp(),
    })
}

/// Fields of a freshly persisted outline row, shared so both backends
/// construct the returned [`TrainingPlan`] identically.
pub(crate) struct BuiltPlan<'a> {
    /// New row id.
    pub id: String,
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the plan is for.
    pub user_id: &'a str,
    /// Coach slug (`None` = coach-agnostic).
    pub coach_slug: Option<&'a str>,
    /// Linked pillar Goal fact, if any.
    pub goal_fact_id: Option<&'a str>,
    /// Goal-race snapshot.
    pub goal_race: &'a GoalRace,
    /// Secondary races.
    pub races: &'a [GoalRace],
    /// Strategy prose.
    pub strategy: &'a str,
    /// Flavour selection, when one was chosen.
    pub flavour: Option<&'a FlavourSelection>,
    /// Season window start.
    pub season_start: Option<&'a str>,
    /// Season window end.
    pub season_end: Option<&'a str>,
    /// Season phases.
    pub phases: &'a [PlanPhase],
    /// Outline this row superseded, if any.
    pub superseded: Option<String>,
    /// Provenance conversation.
    pub source_conversation_id: Option<&'a str>,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// Construct the [`TrainingPlan`] returned after an insert. Shared by both
/// backends and by the bundle path so the mapping lives in one place.
pub(crate) fn built_training_plan(b: BuiltPlan<'_>) -> AppResult<TrainingPlan> {
    let created = epoch_to_datetime(b.now, "created_at")?;
    Ok(TrainingPlan {
        id: b.id,
        tenant_id: b.tenant_id.to_owned(),
        user_id: b.user_id.to_owned(),
        coach_slug: b.coach_slug.map(str::to_owned),
        goal_fact_id: b.goal_fact_id.map(str::to_owned),
        goal_race: b.goal_race.clone(),
        races: b.races.to_vec(),
        strategy: b.strategy.to_owned(),
        flavour: b.flavour.cloned(),
        season_start: b.season_start.map(str::to_owned),
        season_end: b.season_end.map(str::to_owned),
        phases: b.phases.to_vec(),
        status: PlanStatus::Active,
        supersedes_id: b.superseded,
        source_conversation_id: b.source_conversation_id.map(str::to_owned),
        created_at: created,
        updated_at: created,
    })
}

/// Fields of a freshly persisted week row, shared across backends.
pub(crate) struct BuiltWeek<'a> {
    /// New row id.
    pub id: String,
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the week is for.
    pub user_id: &'a str,
    /// Plan the week belongs to.
    pub plan_id: &'a str,
    /// Civil week start.
    pub week_start: &'a str,
    /// Week focus.
    pub focus: &'a str,
    /// Day rows.
    pub days: &'a [PlannedDay],
    /// Week row this one superseded, if any.
    pub superseded: Option<String>,
    /// Adjustment reason.
    pub adjustment_reason: &'a str,
    /// Phase the week instantiates, when stated.
    pub phase_index: Option<u32>,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// Construct the [`PlanWeek`] returned after an insert. Shared by both backends.
pub(crate) fn built_plan_week(b: BuiltWeek<'_>) -> AppResult<PlanWeek> {
    let created = epoch_to_datetime(b.now, "created_at")?;
    Ok(PlanWeek {
        id: b.id,
        tenant_id: b.tenant_id.to_owned(),
        user_id: b.user_id.to_owned(),
        plan_id: b.plan_id.to_owned(),
        week_start: b.week_start.to_owned(),
        focus: b.focus.to_owned(),
        phase_index: b.phase_index,
        days: b.days.to_vec(),
        status: WeekStatus::Active,
        supersedes_id: b.superseded,
        adjustment_reason: b.adjustment_reason.to_owned(),
        created_at: created,
        updated_at: created,
    })
}
