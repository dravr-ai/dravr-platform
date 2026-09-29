// ABOUTME: TrainingPlanRepository trait — persistence for the athlete's one training plan, authored by agents
// ABOUTME: One implementation, emitted per backend by impl_training_plan_repository!. Tenant-scoped throughout.
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
/// active outline (whole-row supersession, never mutation), so there is no
/// separate "update" call.
pub struct SaveTrainingPlanParams<'a> {
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// Athlete the plan is for.
    pub user_id: &'a str,
    /// The agent laying this outline.
    pub author: PlanAuthor<'a>,
    /// Pillar `Goal` user-fact this plan serves, when linked.
    pub goal_fact_id: Option<&'a str>,
    /// Snapshot of the goal race at plan time.
    pub goal_race: &'a GoalRace,
    /// The rest of the race calendar, or `None` to keep the one the plan
    /// being superseded already carried.
    ///
    /// An outline save writes a *new* row, so a calendar the payload does not
    /// restate has to be carried across or it is gone. `Some(&[])` clears it,
    /// which is how a cancelled race is expressed; `None` is what an
    /// adjustment that says nothing about racing means.
    pub races: Option<&'a [GoalRace]>,
    /// The agent's strategy in prose.
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
/// already carries — the bundle applies one tenant, user and author to outline
/// and weeks alike.
pub struct PlanOutlineInput<'a> {
    /// Snapshot of the goal race at plan time.
    pub goal_race: &'a GoalRace,
    /// The rest of the race calendar, or `None` to keep the one the plan
    /// being superseded already carried.
    ///
    /// An outline save writes a *new* row, so a calendar the payload does not
    /// restate has to be carried across or it is gone. `Some(&[])` clears it,
    /// which is how a cancelled race is expressed; `None` is what an
    /// adjustment that says nothing about racing means.
    pub races: Option<&'a [GoalRace]>,
    /// The agent's strategy in prose.
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
    /// The week's intent in agent voice.
    pub focus: &'a str,
    /// The day rows, in date order (at most seven).
    pub days: &'a [PlannedDay],
    /// Why the agent re-saved this week; empty on first save.
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
    /// The agent writing this save: recorded on the outline it lays and on
    /// every week it writes.
    pub author: PlanAuthor<'a>,
    /// Pillar `Goal` user-fact this plan serves, when linked.
    pub goal_fact_id: Option<&'a str>,
    /// New outline to create (superseding the current active one), or `None`
    /// to attach the weeks to the athlete's existing active plan.
    pub outline: Option<PlanOutlineInput<'a>>,
    /// Whether an outline may supersede a season another agent laid.
    ///
    /// Without it an outline supersedes only a season this author laid or an
    /// agent-agnostic one (`SUPERSEDE_OWN_ACTIVE_PLAN_SQL`); over anyone
    /// else's the insert meets the one-season index and the save is refused.
    /// Read only with an outline.
    pub replace_season: bool,
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

/// The stored `author_agent_id` of an outline or week no agent wrote.
///
/// One place, deliberately: it is what [`PlanAuthor::stored`] binds for a
/// writer with no agent and what `SUPERSEDE_OWN_ACTIVE_PLAN_SQL` reads as a
/// season anyone may re-lay. Spelling it into a second query is how the two
/// halves stop agreeing.
pub const NO_AUTHOR_AGENT: &str = "";

/// The agent writing to the athlete's plan, if the write comes from one.
///
/// Authorship never selects a plan — the athlete has one active season,
/// whoever laid it. It records who laid the outline and who wrote each week,
/// and decides whether an outline save may replace the season unasked
/// ([`Self::may_resave_outline`]). [`Self::stored`] is the only place the
/// stored form of "no agent", [`NO_AUTHOR_AGENT`], is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlanAuthor<'a>(Option<&'a str>);

impl<'a> PlanAuthor<'a> {
    /// A write by this agent.
    #[must_use]
    pub const fn agent(agent_id: &'a str) -> Self {
        Self(Some(agent_id))
    }

    /// A write no agent made: a direct call with no agent named.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    /// From a resolved agent id, which is the shape every call site has. An
    /// empty id is no agent: it is what "no agent" is stored as.
    #[must_use]
    pub const fn from_agent(agent_id: Option<&'a str>) -> Self {
        match agent_id {
            Some(id) if !id.is_empty() => Self(Some(id)),
            _ => Self(None),
        }
    }

    /// The value to bind: the agent's id, or [`NO_AUTHOR_AGENT`].
    #[must_use]
    pub const fn stored(self) -> &'a str {
        match self.0 {
            Some(agent_id) => agent_id,
            None => NO_AUTHOR_AGENT,
        }
    }

    /// The agent's id, or `None` for a write no agent made.
    #[must_use]
    pub const fn agent_id(self) -> Option<&'a str> {
        self.0
    }

    /// Whether this writer may lay a new outline over a season laid by
    /// `season_author` without being told to replace it.
    ///
    /// Yes over an agent-agnostic season (`None`) and over a season this same
    /// agent laid; no over another agent's, and no for a writer with no agent
    /// over any agent's season. `SUPERSEDE_OWN_ACTIVE_PLAN_SQL` spells the
    /// same rule in SQL, inside the save's transaction.
    #[must_use]
    pub fn may_resave_outline(self, season_author: Option<&str>) -> bool {
        season_author.is_none_or(|author| Some(author) == self.agent_id())
    }
}

/// Persistence for the athlete's training plan.
///
/// Plans are **tenant-scoped**: every query carries `tenant_id` in its
/// `WHERE` clause. An athlete holds at most one active outline per tenant —
/// the season — which a unique index enforces; every agent reads that one
/// and adds its weeks to it. [`PlanAuthor`] records who laid the outline and
/// who wrote each week, and [`NO_AUTHOR_AGENT`] is the stored value when no
/// agent did.
#[async_trait]
pub trait TrainingPlanRepository: Send + Sync {
    /// Atomically persist an optional outline plus zero or more weeks in a
    /// **single transaction**, so a mid-payload failure can never leave the
    /// athlete with a superseded old plan and a half-populated new one. This
    /// is the only write path: one week is a bundle of one ("move Tuesday to
    /// Wednesday" is a whole-week re-save superseding that `week_start`).
    ///
    /// With an outline: supersedes the current active plan — any author's
    /// when [`SavePlanBundleParams::replace_season`] is set, otherwise only
    /// this author's or an agnostic one — inserts the new one, carries the
    /// superseded outline's surviving weeks onto it with their authors, then
    /// attaches every week in the payload. Without an outline: claims the
    /// athlete's existing active plan with a row-locking write (erroring if
    /// none) and attaches the weeks. Either way the whole set commits or none
    /// of it does.
    ///
    /// An outline that finds a season it may not supersede — another
    /// author's, or one a concurrent save laid while this one ran — meets the
    /// one-season index and fails as `ResourceAlreadyExists`, writing nothing.
    async fn save_plan_bundle(
        &self,
        params: &SavePlanBundleParams<'_>,
    ) -> AppResult<SavedPlanBundle>;

    /// Fetch the athlete's active outline — their one season, whichever
    /// agent laid it. Returns `None` when they have no active plan.
    async fn get_active_plan(
        &self,
        tenant_id: &str,
        user_id: &str,
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
    /// `author_agent_id` column: the agent that laid the outline (`''` = no
    /// agent).
    pub author_agent_id: String,
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
    /// `author_agent_id` column: the agent that wrote the week (`''` = no
    /// agent).
    pub author_agent_id: String,
    /// `phase_index` column; `None` when the week names no phase. Read as
    /// the `i32` the column is declared as (`INTEGER` on `SQLite`, `int4` on
    /// Postgres), so a stored value past that width fails the read as a
    /// database error on either engine instead of decoding on one only.
    pub phase_index: Option<i32>,
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
        author_agent_id: (!row.author_agent_id.is_empty()).then_some(row.author_agent_id),
        created_at: epoch_to_datetime(row.created_at, "created_at")?,
        updated_at: epoch_to_datetime(row.updated_at, "updated_at")?,
    })
}

/// Serialized column values for an outline insert, shared by both backends
/// so the JSON encoding happens once and identically.
pub(crate) struct PlanInsertValues {
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
pub(crate) fn plan_insert_values(
    params: &SaveTrainingPlanParams<'_>,
) -> AppResult<PlanInsertValues> {
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
pub(crate) struct WeekInsertValues {
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
pub(crate) fn phase_index_column(index: Option<u32>) -> AppResult<Option<i32>> {
    index.map(i32::try_from).transpose().map_err(|_| {
        AppError::invalid_input(format!(
            "phase_index out of range: {index:?} exceeds the column's width"
        ))
    })
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
}

/// Construct the [`TrainingPlan`] returned after an insert. Shared by both
/// backends and by the bundle path so the mapping lives in one place.
pub(crate) fn built_training_plan(b: BuiltPlan<'_>) -> AppResult<TrainingPlan> {
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
    /// The agent that wrote the week (`None` = no agent).
    pub author_agent_id: Option<&'a str>,
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
        author_agent_id: b.author_agent_id.map(str::to_owned),
        created_at: created,
        updated_at: created,
    })
}

/// The seventeen columns every outline read returns, in the order
/// [`plan_row`] reads them. One list after `SELECT`, inside `INSERT (...)`
/// and after `RETURNING`, so a column added to [`TrainingPlanRow`] reaches
/// every statement at once.
macro_rules! plan_columns {
    () => {
        "id, tenant_id, user_id, author_agent_id, goal_fact_id, goal_race_json, \
         races_json, strategy, phases_json, status, supersedes_id, source_conversation_id, \
         created_at, updated_at, flavour_json, season_start, season_end"
    };
}

/// The fourteen columns every week read returns, in the order [`week_row`]
/// reads them.
macro_rules! week_columns {
    () => {
        "id, tenant_id, user_id, plan_id, week_start, focus, days_json, \
         status, supersedes_id, adjustment_reason, created_at, updated_at, phase_index, \
         author_agent_id"
    };
}

/// Mark the athlete's active outline superseded, whoever laid it, returning
/// its id and the calendar the replacement inherits. The statement for a save
/// that set [`SavePlanBundleParams::replace_season`].
///
/// `$n` placeholders throughout this module: sqlx accepts them on `SQLite` as
/// well as Postgres, and every bind on these two tables is a plain
/// `&str`/`String`/`Option<&str>`/`i64`/`Option<i32>`, so one statement
/// serves both backends and cannot drift between them.
pub(crate) const SUPERSEDE_ACTIVE_PLAN_SQL: &str = "UPDATE training_plans \
     SET status = 'superseded', updated_at = $1 \
     WHERE tenant_id = $2 AND user_id = $3 AND status = 'active' \
     RETURNING id, races_json";

/// [`SUPERSEDE_ACTIVE_PLAN_SQL`] restricted to a season the writer (`$4`, as
/// [`PlanAuthor::stored`] binds it) may re-lay unasked: one it laid, or one no
/// agent laid ([`NO_AUTHOR_AGENT`]). The SQL half of
/// [`PlanAuthor::may_resave_outline`], evaluated inside the save's
/// transaction: over another author's season it supersedes nothing, and the
/// insert that follows meets the one-season index.
pub(crate) const SUPERSEDE_OWN_ACTIVE_PLAN_SQL: &str = "UPDATE training_plans \
     SET status = 'superseded', updated_at = $1 \
     WHERE tenant_id = $2 AND user_id = $3 AND status = 'active' \
     AND author_agent_id IN ($4, '') \
     RETURNING id, races_json";

/// Insert a new active outline.
pub(crate) const INSERT_PLAN_SQL: &str = concat!(
    "INSERT INTO training_plans (",
    plan_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10, $11, $12, $12, $13, $14, $15)"
);

/// Mark the active row for one `week_start` superseded, returning its id.
pub(crate) const SUPERSEDE_ACTIVE_WEEK_SQL: &str = "UPDATE training_plan_weeks \
     SET status = 'superseded', updated_at = $1 \
     WHERE tenant_id = $2 AND user_id = $3 AND plan_id = $4 AND week_start = $5 \
     AND status = 'active' \
     RETURNING id";

/// Insert a new active week.
pub(crate) const INSERT_WEEK_SQL: &str = concat!(
    "INSERT INTO training_plan_weeks (",
    week_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', $8, $9, $10, $10, $11, $12)"
);

/// Flip every active week of a superseded outline to `'superseded'`,
/// returning the rows so they can be re-inserted against the new outline.
pub(crate) const SUPERSEDE_CARRIED_WEEKS_SQL: &str = concat!(
    "UPDATE training_plan_weeks SET status = 'superseded', updated_at = $1 \
     WHERE tenant_id = $2 AND user_id = $3 AND plan_id = $4 AND status = 'active' \
     RETURNING ",
    week_columns!()
);

/// The athlete's active outline: their one season, whichever agent laid it.
/// The one-season index guarantees at most one row.
pub(crate) const ACTIVE_PLAN_SQL: &str = concat!(
    "SELECT ",
    plan_columns!(),
    " FROM training_plans \
     WHERE tenant_id = $1 AND user_id = $2 AND status = 'active'"
);

/// The athlete's active outline, read by writing it: the status is set to the
/// value it already holds, so the "rows change only by a status flip" rule
/// holds while the statement takes the row lock on Postgres and the write lock
/// on `SQLite`. A weeks-only save claims the season this way so an outline
/// save cannot supersede it between the read and the week inserts — the weeks
/// land on the season that is active when the save commits.
pub(crate) const CLAIM_ACTIVE_PLAN_SQL: &str = concat!(
    "UPDATE training_plans SET status = 'active' \
     WHERE tenant_id = $1 AND user_id = $2 AND status = 'active' \
     RETURNING ",
    plan_columns!()
);

/// Every week of one outline, superseded rows included, in supersession order.
pub(crate) const LIST_ALL_PLAN_WEEKS_SQL: &str = concat!(
    "SELECT ",
    week_columns!(),
    " FROM training_plan_weeks \
     WHERE tenant_id = $1 AND user_id = $2 AND plan_id = $3 \
     ORDER BY week_start ASC, created_at ASC"
);

/// The active weeks of one outline.
pub(crate) const LIST_ACTIVE_PLAN_WEEKS_SQL: &str = concat!(
    "SELECT ",
    week_columns!(),
    " FROM training_plan_weeks \
     WHERE tenant_id = $1 AND user_id = $2 AND plan_id = $3 AND status = 'active' \
     ORDER BY week_start ASC"
);

fn plan_column_error(column: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("training plan column {column}: {e}"))
}

/// Extract a [`TrainingPlanRow`] from a row of either backend via `try_get`
/// only — `Row::get` is `try_get().unwrap()` and panics the whole read path
/// on a width or NULL surprise, so a corrupt row surfaces as a recoverable
/// error the caller can act on, never as a crash.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn plan_row<R>(row: &R) -> AppResult<TrainingPlanRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name).map_err(|e| plan_column_error(name, &e))
    };
    let opt = |name: &str| -> AppResult<Option<String>> {
        row.try_get(name).map_err(|e| plan_column_error(name, &e))
    };
    let epoch = |name: &str| -> AppResult<i64> {
        row.try_get(name).map_err(|e| plan_column_error(name, &e))
    };
    Ok(TrainingPlanRow {
        id: col("id")?,
        tenant_id: col("tenant_id")?,
        user_id: col("user_id")?,
        author_agent_id: col("author_agent_id")?,
        goal_fact_id: opt("goal_fact_id")?,
        goal_race_json: col("goal_race_json")?,
        races_json: col("races_json")?,
        strategy: col("strategy")?,
        phases_json: col("phases_json")?,
        flavour_json: opt("flavour_json")?,
        season_start: opt("season_start")?,
        season_end: opt("season_end")?,
        status: col("status")?,
        supersedes_id: opt("supersedes_id")?,
        source_conversation_id: opt("source_conversation_id")?,
        created_at: epoch("created_at")?,
        updated_at: epoch("updated_at")?,
    })
}

/// Extract a [`PlanWeekRow`] from a row of either backend, via `try_get`
/// only for the reason [`plan_row`] gives.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn week_row<R>(row: &R) -> AppResult<PlanWeekRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name).map_err(|e| plan_column_error(name, &e))
    };
    let opt = |name: &str| -> AppResult<Option<String>> {
        row.try_get(name).map_err(|e| plan_column_error(name, &e))
    };
    let epoch = |name: &str| -> AppResult<i64> {
        row.try_get(name).map_err(|e| plan_column_error(name, &e))
    };
    Ok(PlanWeekRow {
        id: col("id")?,
        tenant_id: col("tenant_id")?,
        user_id: col("user_id")?,
        plan_id: col("plan_id")?,
        week_start: col("week_start")?,
        focus: col("focus")?,
        days_json: col("days_json")?,
        status: col("status")?,
        supersedes_id: opt("supersedes_id")?,
        adjustment_reason: col("adjustment_reason")?,
        author_agent_id: col("author_agent_id")?,
        created_at: epoch("created_at")?,
        updated_at: epoch("updated_at")?,
        phase_index: row
            .try_get("phase_index")
            .map_err(|e| plan_column_error("phase_index", &e))?,
    })
}

/// Emit the whole [`TrainingPlanRepository`] implementation for one backend
/// type, together with the in-transaction helpers it runs on. The body is
/// written once here; each backend's shell invokes it with its own type and
/// the sqlx driver its pool is parameterised on, which is what the helpers'
/// connection type is derived from, and sqlx resolves the driver from
/// `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_training_plan_repository {
    ($ty:ty, $db:ty) => {
        /// Re-parent a superseded outline's surviving weeks onto the outline that
        /// replaced it: every active week is flipped to `'superseded'` and re-inserted
        /// against the new plan id with `supersedes_id` pointing back at it. Weeks are
        /// keyed by `plan_id` and every read surface lists the *active* plan's weeks,
        /// so a week left behind is a schedule the athlete can no longer see. Row
        /// content is carried verbatim — `days_json` is never re-parsed, and the week
        /// keeps the author that wrote it — so only identity changes and the
        /// migration's "new row with `supersedes_id` set, old row
        /// `status='superseded'`" model holds across the plan boundary too.
        async fn carry_forward_active_weeks(
            conn: &mut <$db as sqlx::Database>::Connection,
            tenant_id: &str,
            user_id: &str,
            old_plan_id: &str,
            new_plan_id: &str,
            now: i64,
        ) -> AppResult<()> {
            let rows = sqlx::query(SUPERSEDE_CARRIED_WEEKS_SQL)
                .bind(now)
                .bind(tenant_id)
                .bind(user_id)
                .bind(old_plan_id)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("supersede carried weeks: {e}")))?;
            for row in &rows {
                let old = week_row(row)?;
                sqlx::query(INSERT_WEEK_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(new_plan_id)
                    .bind(&old.week_start)
                    .bind(&old.focus)
                    .bind(&old.days_json)
                    .bind(&old.id)
                    .bind(&old.adjustment_reason)
                    .bind(now)
                    .bind(old.phase_index)
                    .bind(&old.author_agent_id)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| AppError::database(format!("carry forward plan week: {e}")))?;
            }
            Ok(())
        }

        /// Supersede the current active outline and insert the new one, on an
        /// in-transaction connection. The caller owns the transaction envelope.
        ///
        /// `replace_season` picks the supersede: any author's season, or only one
        /// this author may re-lay unasked. When the supersede matched nothing while
        /// a season is active — another author's without the flag, or one a
        /// concurrent save laid — the insert meets the one-season index and the save
        /// is refused as `ResourceAlreadyExists`.
        async fn supersede_and_insert_plan(
            conn: &mut <$db as sqlx::Database>::Connection,
            params: &SaveTrainingPlanParams<'_>,
            replace_season: bool,
        ) -> AppResult<TrainingPlan> {
            let v = plan_insert_values(params)?;
            let supersede = if replace_season {
                sqlx::query(SUPERSEDE_ACTIVE_PLAN_SQL)
                    .bind(v.now)
                    .bind(params.tenant_id)
                    .bind(params.user_id)
            } else {
                sqlx::query(SUPERSEDE_OWN_ACTIVE_PLAN_SQL)
                    .bind(v.now)
                    .bind(params.tenant_id)
                    .bind(params.user_id)
                    .bind(&v.author_agent_id)
            };
            let replaced = supersede
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("supersede active plan: {e}")))?;
            // Split the superseded row into the id the weeks are re-parented onto and
            // the calendar the new row inherits when this save states none.
            let superseded: Option<String> = replaced
                .as_ref()
                .map(|row| row.try_get("id"))
                .transpose()
                .map_err(|e| AppError::database(format!("superseded plan id: {e}")))?;
            // The calendar crosses the supersede the way the weeks do, and for the
            // same reason: the new row is a different row, so anything the payload
            // does not restate has to be carried or it is lost. Carried verbatim as
            // JSON — it was validated when it was first stated, and re-encoding it
            // would only invent a way for the two rows to disagree.
            let carried_races: Option<String> = replaced
                .as_ref()
                .map(|row| row.try_get("races_json"))
                .transpose()
                .map_err(|e| AppError::database(format!("superseded plan races_json: {e}")))?;
            let races_json = v
                .races_json
                .clone()
                .or(carried_races)
                .unwrap_or_else(|| "[]".to_owned());
            // What the returned plan says it holds must be what the row now holds,
            // carried calendar included — a caller that read the payload back would
            // otherwise be told the calendar is empty on the very save that kept it.
            let written_races: Vec<GoalRace> = match params.races {
                Some(races) => races.to_vec(),
                None => serde_json::from_str(&races_json)
                    .map_err(|e| AppError::database(format!("carried races_json: {e}")))?,
            };
            sqlx::query(INSERT_PLAN_SQL)
                .bind(&v.id)
                .bind(params.tenant_id)
                .bind(params.user_id)
                .bind(&v.author_agent_id)
                .bind(params.goal_fact_id)
                .bind(&v.goal_race_json)
                .bind(&races_json)
                .bind(params.strategy)
                .bind(&v.phases_json)
                .bind(superseded.as_deref())
                .bind(params.source_conversation_id)
                .bind(v.now)
                .bind(v.flavour_json.as_deref())
                .bind(params.season_start)
                .bind(params.season_end)
                .execute(&mut *conn)
                .await
                .map_err(|e| {
                    // The one-season index: a season this save may not supersede
                    // is still active. The tool checks authorship before writing,
                    // so reaching this means the season changed while the save
                    // ran — the transaction rolls back and nothing is written.
                    if e.as_database_error()
                        .is_some_and(|db| db.is_unique_violation())
                    {
                        AppError::new(
                            ErrorCode::ResourceAlreadyExists,
                            "The athlete's training plan changed while this save ran, so \
                             nothing was saved — read it again with get_training_plan and \
                             save again.",
                        )
                    } else {
                        AppError::database(format!("insert training plan: {e}"))
                    }
                })?;
            // The new outline carries a new id, so the replaced outline's weeks move
            // with it in this same transaction — otherwise the athlete's schedule
            // stays on a plan no read surface fetches.
            if let Some(old_plan_id) = superseded.as_deref() {
                carry_forward_active_weeks(
                    &mut *conn,
                    params.tenant_id,
                    params.user_id,
                    old_plan_id,
                    &v.id,
                    v.now,
                )
                .await?;
            }
            built_training_plan(BuiltPlan {
                id: v.id,
                tenant_id: params.tenant_id,
                user_id: params.user_id,
                author_agent_id: params.author.agent_id(),
                goal_fact_id: params.goal_fact_id,
                goal_race: params.goal_race,
                races: &written_races,
                strategy: params.strategy,
                flavour: params.flavour,
                season_start: params.season_start,
                season_end: params.season_end,
                phases: params.phases,
                superseded,
                source_conversation_id: params.source_conversation_id,
                now: v.now,
            })
        }

        /// Supersede the active row for this `week_start` and insert the new week, on
        /// an in-transaction connection. The caller resolves `plan_id` from a plan it
        /// created or claimed for this tenant + user in the same transaction, so the
        /// week can never attach to another athlete's (or tenant's) plan. The week is
        /// authored by this save's writer; the row it supersedes keeps its own author
        /// in the history.
        async fn supersede_and_insert_week(
            conn: &mut <$db as sqlx::Database>::Connection,
            params: &SavePlanBundleParams<'_>,
            plan_id: &str,
            week: &PlanWeekInput<'_>,
        ) -> AppResult<PlanWeek> {
            let tenant_id = params.tenant_id;
            let user_id = params.user_id;
            let v = week_insert_values(week.days)?;
            let superseded: Option<String> = sqlx::query_scalar(SUPERSEDE_ACTIVE_WEEK_SQL)
                .bind(v.now)
                .bind(tenant_id)
                .bind(user_id)
                .bind(plan_id)
                .bind(week.week_start)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("supersede active week: {e}")))?;
            sqlx::query(INSERT_WEEK_SQL)
                .bind(&v.id)
                .bind(tenant_id)
                .bind(user_id)
                .bind(plan_id)
                .bind(week.week_start)
                .bind(week.focus)
                .bind(&v.days_json)
                .bind(superseded.as_deref())
                .bind(week.adjustment_reason)
                .bind(v.now)
                .bind(phase_index_column(week.phase_index)?)
                .bind(params.author.stored())
                .execute(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("insert plan week: {e}")))?;
            built_plan_week(BuiltWeek {
                id: v.id,
                tenant_id,
                user_id,
                plan_id,
                week_start: week.week_start,
                focus: week.focus,
                days: week.days,
                superseded,
                adjustment_reason: week.adjustment_reason,
                phase_index: week.phase_index,
                author_agent_id: params.author.agent_id(),
                now: v.now,
            })
        }

        /// Claim the athlete's active outline on an in-transaction connection
        /// ([`CLAIM_ACTIVE_PLAN_SQL`]), a second time when the first finds nothing:
        /// on Postgres (READ COMMITTED) a claim that waited on an outline save's
        /// lock re-checks its row, finds it superseded and matches nothing, and the
        /// second statement's fresh snapshot claims the outline that save
        /// committed. Nothing both times means the athlete has no season.
        async fn claim_active_plan(
            conn: &mut <$db as sqlx::Database>::Connection,
            tenant_id: &str,
            user_id: &str,
        ) -> AppResult<Option<TrainingPlan>> {
            for _ in 0..2 {
                let row = sqlx::query(CLAIM_ACTIVE_PLAN_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .fetch_optional(&mut *conn)
                    .await
                    .map_err(|e| AppError::database(format!("claim active plan: {e}")))?;
                if let Some(row) = row {
                    return plan_row(&row).and_then(training_plan_from_row).map(Some);
                }
            }
            Ok(None)
        }

        #[async_trait::async_trait]
        impl TrainingPlanRepository for $ty {
            async fn save_plan_bundle(
                &self,
                params: &SavePlanBundleParams<'_>,
            ) -> AppResult<SavedPlanBundle> {
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("begin bundle tx: {e}")))?;

                // Resolve the plan the weeks attach to — a fresh outline (superseding
                // the current active one) or the existing active plan, claimed — inside
                // the transaction so the outline supersession and every week either all
                // commit or all roll back.
                let (plan, superseded_plan_id) = if let Some(o) = &params.outline {
                    let stp = SaveTrainingPlanParams {
                        tenant_id: params.tenant_id,
                        user_id: params.user_id,
                        author: params.author,
                        goal_fact_id: params.goal_fact_id,
                        goal_race: o.goal_race,
                        races: o.races,
                        strategy: o.strategy,
                        flavour: o.flavour,
                        season_start: o.season_start,
                        season_end: o.season_end,
                        phases: o.phases,
                        source_conversation_id: o.source_conversation_id,
                    };
                    let plan =
                        supersede_and_insert_plan(&mut tx, &stp, params.replace_season).await?;
                    let superseded = plan.supersedes_id.clone();
                    (plan, superseded)
                } else {
                    let plan = claim_active_plan(&mut tx, params.tenant_id, params.user_id)
                        .await?
                        .ok_or_else(|| {
                            AppError::invalid_input(
                                "no active plan to attach weeks to — save an outline first",
                            )
                        })?;
                    (plan, None)
                };

                let mut weeks = Vec::with_capacity(params.weeks.len());
                for w in params.weeks {
                    weeks.push(supersede_and_insert_week(&mut tx, params, &plan.id, w).await?);
                }

                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("commit bundle tx: {e}")))?;
                Ok(SavedPlanBundle {
                    plan,
                    weeks,
                    superseded_plan_id,
                })
            }

            async fn get_active_plan(
                &self,
                tenant_id: &str,
                user_id: &str,
            ) -> AppResult<Option<TrainingPlan>> {
                let row = sqlx::query(ACTIVE_PLAN_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get active plan: {e}")))?;
                row.map(|r| plan_row(&r).and_then(training_plan_from_row))
                    .transpose()
            }

            async fn list_plan_weeks(
                &self,
                tenant_id: &str,
                user_id: &str,
                plan_id: &str,
                include_superseded: bool,
            ) -> AppResult<Vec<PlanWeek>> {
                let sql = if include_superseded {
                    LIST_ALL_PLAN_WEEKS_SQL
                } else {
                    LIST_ACTIVE_PLAN_WEEKS_SQL
                };
                let rows = sqlx::query(sql)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(plan_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list plan weeks: {e}")))?;
                rows.iter()
                    .map(|r| week_row(r).and_then(plan_week_from_row))
                    .collect()
            }
        }
    };
}
pub(crate) use impl_training_plan_repository;
