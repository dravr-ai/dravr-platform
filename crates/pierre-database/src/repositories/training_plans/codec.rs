// ABOUTME: Row and insert codecs for training plans — raw rows to domain types, params to serialized insert values
// ABOUTME: Shared by both backends so JSON encoding, enum parsing and epoch conversion live in exactly one place
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Conversions between the training-plan tables and the domain types.
//!
//! Decoding maps a raw [`TrainingPlanRow`] / [`PlanWeekRow`] to its domain
//! type; encoding serializes save parameters into the column values an insert
//! binds, and builds the domain value an insert returns. The parent module
//! re-exports everything the backend shells use.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::transport::TransportPolicy;
use pierre_memory::training_plans::{
    FlavourSelection, GoalRace, PlanPhase, PlanStatus, PlanWeek, PlannedDay, TrainingPlan,
    WeekStatus,
};

use super::{PlanWeekRow, SaveTrainingPlanParams, TrainingPlanRow};

/// Convert epoch seconds to `DateTime<Utc>`, treating an out-of-range value
/// as corruption rather than silently clamping.
fn epoch_to_datetime(epoch: i64, column: &str) -> AppResult<DateTime<Utc>> {
    DateTime::from_timestamp(epoch, 0)
        .ok_or_else(|| AppError::database(format!("training plan {column} out of range: {epoch}")))
}

/// Map a raw outline row to the domain type. Shared by both backends so JSON
/// and enum parsing live in exactly one place.
pub fn training_plan_from_row(row: TrainingPlanRow) -> AppResult<TrainingPlan> {
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
        author_agent_id: (!row.author_agent_id.is_empty()).then_some(row.author_agent_id),
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
        transport_policy: TransportPolicy::from_first_party_only(row.first_party_only),
    })
}

/// Map a raw week row to the domain type. Shared by both backends.
pub fn plan_week_from_row(row: PlanWeekRow) -> AppResult<PlanWeek> {
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
        author_agent_id: (!row.author_agent_id.is_empty()).then_some(row.author_agent_id),
        created_at: epoch_to_datetime(row.created_at, "created_at")?,
        updated_at: epoch_to_datetime(row.updated_at, "updated_at")?,
    })
}

/// Serialized column values for an outline insert, shared by both backends
/// so the JSON encoding happens once and identically.
pub struct PlanInsertValues {
    /// New row id.
    pub id: String,
    /// The outline's author as stored (`''` for no agent).
    pub author_agent_id: String,
    /// Serialized goal-race snapshot.
    pub goal_race_json: String,
    /// Serialized race calendar, or `None` to carry the superseded row's
    /// calendar across verbatim rather than re-encode it.
    pub races_json: Option<String>,
    /// Serialized phases.
    pub phases_json: String,
    /// Serialized flavour selection, when one was chosen.
    pub flavour_json: Option<String>,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// Build the serialized insert values for [`SaveTrainingPlanParams`].
pub fn plan_insert_values(params: &SaveTrainingPlanParams<'_>) -> AppResult<PlanInsertValues> {
    let goal_race_json = serde_json::to_string(params.goal_race)
        .map_err(|e| AppError::internal(format!("serialize goal race: {e}")))?;
    let races_json = params
        .races
        .map(serde_json::to_string)
        .transpose()
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
        author_agent_id: params.author.stored().to_owned(),
        goal_race_json,
        races_json,
        phases_json,
        flavour_json,
        now: Utc::now().timestamp(),
    })
}

/// Serialized column values for a week insert, shared by both backends.
pub struct WeekInsertValues {
    /// New row id.
    pub id: String,
    /// Serialized day rows.
    pub days_json: String,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// The one guard on a caller-supplied `phase_index`: the column is `int4` on
/// Postgres, so an index past `i32::MAX` names no phase any outline could
/// hold and is refused as invalid input before either engine sees it — the
/// same refusal on both, rather than `SQLite` storing what Postgres would
/// reject with a driver error. An index read back from a stored row never
/// passes through here: [`PlanWeekRow::phase_index`] is already the column's
/// width, and a row past it fails its decode as a database error.
pub fn phase_index_column(index: Option<u32>) -> AppResult<Option<i32>> {
    index.map(i32::try_from).transpose().map_err(|_| {
        AppError::invalid_input(format!(
            "phase_index out of range: {index:?} exceeds the column's width"
        ))
    })
}

/// Build the serialized insert values for one
/// [`PlanWeekInput`](super::PlanWeekInput)'s day rows.
pub fn week_insert_values(days: &[PlannedDay]) -> AppResult<WeekInsertValues> {
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
pub struct BuiltPlan<'a> {
    /// New row id.
    pub id: String,
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the plan is for.
    pub user_id: &'a str,
    /// The agent that laid the outline (`None` = no agent).
    pub author_agent_id: Option<&'a str>,
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
    /// The stamp the row was written with.
    pub transport_policy: TransportPolicy,
}

/// Construct the [`TrainingPlan`] returned after an insert. Shared by both
/// backends and by the bundle path so the mapping lives in one place.
pub fn built_training_plan(b: BuiltPlan<'_>) -> AppResult<TrainingPlan> {
    let created = epoch_to_datetime(b.now, "created_at")?;
    Ok(TrainingPlan {
        id: b.id,
        tenant_id: b.tenant_id.to_owned(),
        user_id: b.user_id.to_owned(),
        author_agent_id: b.author_agent_id.map(str::to_owned),
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
        transport_policy: b.transport_policy,
    })
}

/// Fields of a freshly persisted week row, shared across backends.
pub struct BuiltWeek<'a> {
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
    /// The agent that wrote the week (`None` = no agent).
    pub author_agent_id: Option<&'a str>,
    /// Insert timestamp (epoch seconds).
    pub now: i64,
}

/// Construct the [`PlanWeek`] returned after an insert. Shared by both backends.
pub fn built_plan_week(b: BuiltWeek<'_>) -> AppResult<PlanWeek> {
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
        author_agent_id: b.author_agent_id.map(str::to_owned),
        created_at: created,
        updated_at: created,
    })
}
