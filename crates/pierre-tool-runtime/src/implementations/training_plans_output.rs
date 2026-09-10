// ABOUTME: The declared shapes of get_training_plan and save_training_plan, and the calendar block both carry
// ABOUTME: The calendar block is Dravr's record of what Dravr pushed — never a view of the athlete's own calendar
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the training-plan tools answer with.
//!
//! Both carry a calendar block, and it is the one part of these replies that
//! has already misled an athlete: shown `entries: []` under a key called
//! `calendar`, a model reported "nothing on your calendar" to someone asking
//! whether he had a race (2026-08-28). The block is Dravr's ledger of what
//! Dravr pushed, and for an athlete with no calendar provider connected it
//! is empty permanently. `scope` says so in words next to the emptiness, and
//! it is part of the declared shape for that reason rather than a nicety.

use std::collections::BTreeSet;

use pierre_core::models::periodization::{ReadinessLevel, SubstitutionVerdict, TrainingAlert};
use pierre_core::models::CalendarEventSource;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use pierre_services::plan_calendar_push::PushPreview;
use serde::Serialize;
use uuid::Uuid;

/// What `get_training_plan` answers with.
///
/// `plan` is null when there is no active plan, and `message` says what to do
/// about it. The calendar block is present either way, because prescriptions
/// outlive the plan that made them.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetTrainingPlanResult {
    /// The active plan, or null when there is none.
    pub plan: Option<TrainingPlan>,
    /// Whose plan this is — a coach may be reading a consenting athlete's.
    pub athlete: Option<String>,
    /// Present only when there is no plan: what to do instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The weeks, day by day. Absent when there is no plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weeks: Option<Vec<PlanWeek>>,
    /// Whether the goal the plan snapshotted has since expired, so the coach
    /// re-confirms it rather than planning toward a race that moved. Absent
    /// when there is no plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_stale: Option<bool>,
    /// What Dravr has on the athlete's calendar provider.
    pub calendar: CalendarBlock,
    /// What the two rails make of the plan as it stands. Present only when
    /// the caller asked for it — reading it costs three history queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PlanStateBlock>,
}

/// What the readiness and compliance rails say about the plan as it stands.
///
/// Both rails have measured every save since they shipped and reported into
/// a log line no agent reads. This is the same verdict, handed to the agent
/// that has to act on it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PlanStateBlock {
    /// The readiness level the athlete's signals clear: `p0` block, `p1`
    /// caution, `p2` maintain, `p3` build. Absent when the plan carries no
    /// flavour, since the ladder is per-flavour data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<ReadinessLevel>,
    /// The alert labels the athlete's series raised, from the taxonomy the
    /// agent bodies already describe. Empty means none raised, which covers
    /// both measured-and-fine and not-measurable.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub alerts: BTreeSet<TrainingAlert>,
    /// Per week, what the readiness level no longer allows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readiness_weeks: Vec<WeekReadiness>,
    /// Per week, how the week measures against its phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compliance_weeks: Vec<WeekComplianceBlock>,
    /// Where the stored weeks stop covering what the outline promised:
    /// `uncovered_today` when no week spans the athlete's today,
    /// `short_of_outline` when the weeks stop before the last phase ends.
    /// Empty means the plan covers what it said it would — this is the signal
    /// that a fortnight needs writing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage_gaps: Vec<String>,
}

/// The kernel's readiness verdict for one week, with the week named.
///
/// A pair would serialise positionally — `["2026-09-16", {...}]` — and an
/// agent reading that has to know which slot is which. The verdict is
/// flattened rather than copied field by field, so the kernel stays the one
/// definition of what a substitution is.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeekReadiness {
    /// Monday, `YYYY-MM-DD`.
    pub week_start: String,
    /// What the level allows and what it refuses, from the kernel.
    #[serde(flatten)]
    pub verdict: SubstitutionVerdict,
}

/// One week against its phase's targets.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeekComplianceBlock {
    /// Monday, `YYYY-MM-DD`.
    pub week_start: String,
    /// Time in zone against the phase's target: `within`, `off` or
    /// `unmeasured`.
    pub tid: String,
    /// Hard-session count against the cap.
    pub hard_sessions: String,
    /// Hours between hard sessions.
    pub spacing: String,
    /// Hours against the phase's volume target.
    pub volume: String,
    /// Whether the week is a load week, a recovery week, or a recovery week
    /// measured against the full target because no cut was on file.
    pub week_loading: String,
    /// Days the grammar could place no time for.
    pub unclassified_days: u8,
}

/// What `save_training_plan` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SaveTrainingPlanResult {
    /// The plan that was written.
    pub plan_id: String,
    /// Whose plan it is.
    pub athlete: Option<String>,
    /// The goal race, as name and date, for the reply to quote back.
    pub goal_race: String,
    /// The plan this one superseded, when it replaced an active plan.
    pub superseded_plan_id: Option<String>,
    /// The living goal fact the plan is anchored to, when one was recorded.
    pub goal_fact_id: Option<String>,
    /// How many weeks were written.
    pub weeks_saved: usize,
    /// Each week, and whether it replaced an earlier version of itself.
    pub weeks: Vec<SavedWeek>,
    /// What a push would change now, when the calendar already carries this
    /// plan. Absent when the athlete has never pushed — an athlete who does
    /// not use the calendar is not nagged about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarPreview>,
}

/// One week as it was saved.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SavedWeek {
    /// The Monday the week starts on.
    pub week_start: String,
    /// The stored week's id.
    pub week_id: String,
    /// Whether it replaced an earlier version of the same week.
    pub superseded: bool,
}

/// Dravr's record of what Dravr pushed to the athlete's calendar provider.
///
/// Not a view of their calendar. Nothing here reads what the athlete entered
/// themselves, so an empty `entries` says nothing about whether they have a
/// race — which is exactly what `scope` exists to say out loud.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CalendarBlock {
    /// The calendar provider the entries were pushed to.
    pub provider: String,
    /// The live entries, always present — an empty list is a real answer and
    /// dropping the key would make "nothing pushed" indistinguishable from
    /// "not asked".
    pub entries: Vec<CalendarEntry>,
    /// What a push would change.
    pub pending: PushPreview,
    /// Whether a push would change anything at all.
    pub stale: bool,
    /// Present only when `entries` is empty: what that emptiness does and
    /// does not mean. A client without it reports "nothing on your calendar".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// One entry Dravr put on the calendar.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CalendarEntry {
    /// The id `prescribe_workout`'s `replaces` and
    /// `withdraw_prescribed_workout` take.
    pub prescription_id: Uuid,
    /// The day it is on, `YYYY-MM-DD`.
    pub date: String,
    /// Whether a plan week or a single prescription put it there.
    pub source: CalendarEventSource,
    /// The session's name, from the pushed payload or the template it came
    /// from. `None` when neither carried one.
    pub name: Option<String>,
    /// The provider's own id for the event, once it has been pushed.
    pub provider_event_id: Option<String>,
    /// When the ledger row last changed.
    pub pushed_at: chrono::DateTime<chrono::Utc>,
}

/// What a push would change, without the entry list.
///
/// The save reply reports the counts only: the athlete has just written the
/// plan and needs to know whether the calendar is behind, not to re-read
/// every entry they already have.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CalendarPreview {
    /// The calendar provider.
    pub provider: String,
    /// What a push would change.
    pub pending: PushPreview,
    /// Whether it would change anything.
    pub stale: bool,
}
