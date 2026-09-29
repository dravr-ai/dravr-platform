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

use pierre_core::models::periodization::{
    DaySubstitution, ReadinessLevel, SpacingCheck, TrainingAlert, WorkoutPurpose,
};
use pierre_core::models::CalendarEventSource;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use pierre_services::plan_calendar_push::PushPreview;
use pierre_services::plan_fueling::WithheldFueling;
use pierre_services::ramp_check::RampVerdict;
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
    /// Whose plan this is — an agent may be reading a consenting athlete's.
    pub athlete: Option<String>,
    /// Present only when there is no plan: what to do instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The weeks, day by day. Absent when there is no plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weeks: Option<Vec<PlanWeek>>,
    /// Present when a medical/PAR-Q flag is on file: the athlete's clinician
    /// sets their fuelling amounts, so no day in `weeks` carries `fueling`
    /// rates, `dates` names the days whose stored rates were withheld, and a
    /// save carrying `fueling` is refused. Describe fuelling in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fueling_withheld: Option<WithheldFueling>,
    /// The agents that wrote this season: the one that laid it, and each one
    /// that wrote a returned week, with which of them is the reader. Absent
    /// when there is no plan or no agent wrote any of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<PlanAuthorView>,
    /// Whether the goal the plan snapshotted has since expired, so the agent
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

/// One agent that wrote part of the athlete's season.
///
/// The plan is the athlete's one season, shared by every agent they use, so
/// the reply says who laid it and who wrote which weeks — and which of them
/// is the agent reading it. An agent laying out a taper over a season another
/// agent laid reads here that the outline is not its to re-lay.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PlanAuthorView {
    /// The agent's id, as the outline and weeks record it.
    pub agent_id: String,
    /// The agent's title, or null when the reader cannot see that agent.
    pub name: Option<String>,
    /// Whether this author is the agent reading the plan.
    pub is_you: bool,
    /// Whether this author laid the season's outline.
    pub laid_the_season: bool,
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

/// The kernel's readiness verdict for one week, with the week named and each
/// moved day answered from the athlete's template bank.
///
/// A pair would serialise positionally — `["2026-09-16", {...}]` — and an
/// agent reading that has to know which slot is which. Each substitution is
/// the kernel's, flattened rather than copied field by field, so the kernel
/// stays the one definition of what a substitution is and why it happened;
/// the platform adds only what the kernel cannot see — which templates the
/// athlete's package and the catalogue hold.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeekReadiness {
    /// Monday, `YYYY-MM-DD`.
    pub week_start: String,
    /// The level the athlete's signals cleared.
    pub level: ReadinessLevel,
    /// Days the level no longer allows, in the order the week gives them.
    pub substitutions: Vec<ReadinessSubstitution>,
    /// Hard sessions the week asks for.
    pub hard_sessions: u8,
    /// Hard sessions the level allows.
    pub hard_sessions_allowed: u8,
    /// Days no template resolved for. Reported, never substituted.
    pub unclassified_days: u8,
}

impl WeekReadiness {
    /// Nothing in the week needs changing — the kernel's own rule.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.substitutions.is_empty() && self.hard_sessions <= self.hard_sessions_allowed
    }
}

/// One day the readiness ladder moved, and what to run in its place.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ReadinessSubstitution {
    /// The kernel's reading of the day: its `date`, the purpose it was `from`,
    /// the purpose to run instead (`to`), and the `reason` —
    /// `purpose_closed` when the level no longer opens the day's purpose (then
    /// `to` is the phase's heaviest purpose the level opens, or null), or
    /// `template_above_level` when the purpose is open but the day's template
    /// needs a higher level than today's (then `to` is the same purpose).
    #[serde(flatten)]
    pub day: DaySubstitution,
    /// On a `template_above_level` day, what the athlete's bank offers in its
    /// place. Absent on a `purpose_closed` day, whose `to` already names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<DayReplacement>,
}

/// What the athlete's bank — the agent's package, then the catalogue — offers
/// for a day whose template sits above today's readiness level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct DayReplacement {
    /// How the replacement was found: `easier_template` when the bank holds a
    /// template of the day's own purpose that today's level clears, or
    /// `purpose_fallback` when it holds none and the day falls back to the
    /// phase's heaviest purpose the level opens.
    pub basis: ReplacementBasis,
    /// The purpose to run. Null only on a `purpose_fallback` whose level opens
    /// nothing else the phase asks for — the day does not fit today.
    pub purpose: Option<WorkoutPurpose>,
    /// The template to run: of that purpose, written for the day's sport,
    /// fitting the week's phase, and with a floor today's level clears. Null
    /// when the bank holds no such template.
    pub template: Option<String>,
}

/// How a [`DayReplacement`] was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplacementBasis {
    /// A template of the day's own purpose whose floor today's level clears.
    EasierTemplate,
    /// No template of the day's purpose fits today; the phase's heaviest
    /// purpose the level opens, other than the day's own.
    PurposeFallback,
}

/// One week whose hard sessions sit closer together than its flavour allows.
///
/// Named rather than paired for the reason [`WeekReadiness`] is: a tuple
/// serialises positionally and the reader has to know which slot is which.
/// The check is flattened, so the kernel stays the one definition of what
/// "too close" means — including the minimum it applied and which gaps were
/// short.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CrowdedWeek {
    /// Monday, `YYYY-MM-DD`.
    pub week_start: String,
    /// The spacing check, from the kernel.
    #[serde(flatten)]
    pub spacing: SpacingCheck,
}

/// What the two safety-shaped checks found in the weeks this save wrote.
///
/// The compliance rail has measured every saved week since it shipped and
/// reported into a log line no agent reads; the readiness rail the same. This
/// is the half that reaches the turn: the agent sees what the platform
/// measured on the save it just made, and can say so before the athlete acts
/// on a week that crowds its hard days or opens well above their recent load.
///
/// Only the two checks the ledger calls safety-shaped travel here. Time in
/// zone, volume against target and template parameter ranges are coaching
/// judgements the agent already owns, and a platform that volunteered those
/// would be second-guessing the plan rather than flagging a risk.
///
/// Structured, never prose. What the athlete is told is the agent's to write:
/// the framing rules on load ratios are CI-enforced and live in the prompt,
/// so a sentence composed here would be a second place they could be broken.
///
/// Absent when both checks read within, unmeasured, or had nothing to measure
/// — which is the common case and costs the turn nothing.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PlanSafetyReport {
    /// Weeks whose hard sessions are closer together than the minimum.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub crowded_weeks: Vec<CrowdedWeek>,
    /// The opening week against the athlete's recent weekly hours, when it
    /// came in above the threshold. `None` when it did not, or when there was
    /// no baseline to compare against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ramp: Option<RampVerdict>,
}

impl PlanSafetyReport {
    /// Whether anything was found worth the agent's attention.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.crowded_weeks.is_empty() && self.ramp.is_none()
    }
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
    /// Argument keys the caller supplied that this tool does not have.
    ///
    /// Serde drops an unknown key in silence, so a plan saved with
    /// `agent_id` instead of `agent_id` stores against no agent and the
    /// athlete is told "no active plan to extend yet" on some later turn,
    /// with nothing anywhere naming the cause. Reporting the dropped keys
    /// puts that on the turn it happened, where the model can fix it.
    ///
    /// Reported rather than refused on purpose: these payloads are
    /// LLM-written, and one provider wraps its arguments in an envelope of
    /// its own (Cohere's v1 `{"parameters":{...}}` leaking onto the v2
    /// surface), so rejecting an unknown key would turn a degraded turn into
    /// a failed one — intermittently, and only on that provider.
    ///
    /// Empty on every well-formed call, and then omitted from the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored_arguments: Vec<String>,
    /// What the two safety-shaped checks found in the weeks just written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety: Option<PlanSafetyReport>,
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
