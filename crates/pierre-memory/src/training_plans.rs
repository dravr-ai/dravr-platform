// ABOUTME: Training-plan domain model — the athlete's vision (flavour, season phases) + weekly microcycles
// ABOUTME: Two-level periodization (outline phases / day-by-day weeks); supersession, never mutation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Training plans
//!
//! A training plan is the agent's strategic answer to a dated goal, split in
//! two levels that mirror endurance periodization:
//!
//! - [`TrainingPlan`] — the **vision** (macrocycle): a goal-race snapshot,
//!   the season window, the [`FlavourSelection`] with its provenance, the
//!   ordered [`PlanPhase`]s (prep/base/build/specialty/peak/taper/race/
//!   transition/recovery — the kernel's [`PhaseKind`]) each carrying the
//!   purpose, volume and intensity targets the fortnight is written against,
//!   and a prose strategy. One `active` outline per (tenant, user): the
//!   athlete's season, which every agent they use reads and adds weeks to.
//! - [`PlanWeek`] — one **microcycle**: the [`PlannedDay`] rows of one
//!   calendar week, two sessions on a date being two rows, with intensity
//!   expressed *relative to thresholds* (zones, %FTP) so an FTP retest
//!   never invalidates a stored plan, and — when the agent states it — the
//!   session's structure as [`WorkoutStep`]s.
//!
//! Plans are captured by explicit agent tool calls, never post-hoc
//! extraction — Tier-2 extraction minting agent prescriptions as `user_facts`
//! is the failure this entity replaces. Adjustments are **prospective-only
//! whole-row supersession**: a re-save creates a new row pointing at the old
//! one via `supersedes_id`; nothing is edited in place, past weeks stay
//! immutable, and the chain is the audit trail.
//!
//! Boundary versus the sibling agent-memory surfaces: a plan is a
//! forward-looking structured prescription; a playbook
//! ([`crate::playbooks::Playbook`]) is a learned trigger→intervention pattern.
//! A plan is never a playbook (not evidence-scored).

use pierre_core::transport::TransportPolicy;
use std::collections::{BTreeMap, BTreeSet};

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::models::periodization::serde_num::{whole_u32_opt, whole_u8};
use pierre_core::models::periodization::{
    AnchorChoice, EnvelopeFinding, EnvelopeReport, EvidenceBadge, FlavourFamily, FlavourInputs,
    FlavourVerdict, LoadingPattern, Modifier, Pacing, PhaseKind, PresetParams, Sequencing, Share,
    TidTarget, WorkoutPurpose,
};
use pierre_core::models::{FuelingProtocol, TargetAnchor, WorkoutStep};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle of a [`TrainingPlan`] outline.
///
/// The first four describe the athlete's live season and its history. The
/// last three describe a human coach's draft of it (carnet#1069): a plan the
/// athlete never sees until the coach publishes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// The plan currently guiding this athlete: their one season.
    Active,
    /// Replaced by a newer outline (`supersedes_id` chain).
    Superseded,
    /// The goal race happened; kept for history.
    Completed,
    /// Dropped without completing (goal abandoned, athlete left).
    Abandoned,
    /// A human coach's open draft of the athlete's plan. Only that coach
    /// reads it; no surface of the athlete's does.
    Draft,
    /// A draft the coach published: its outline and weeks now live in the
    /// athlete's season, and the row is kept as the record of what was sent.
    Published,
    /// A draft the coach threw away without publishing.
    Discarded,
}

impl PlanStatus {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Discarded => "discarded",
        }
    }

    /// Parse the DB representation. Unknown strings return `None` so a
    /// migration gap surfaces as an error, not a silent default.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "completed" => Some(Self::Completed),
            "abandoned" => Some(Self::Abandoned),
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            "discarded" => Some(Self::Discarded),
            _ => None,
        }
    }
}

/// Lifecycle of a [`PlanWeek`]. Past-ness is derived from `week_start` at
/// read time — a week that elapsed is simply in the past, not a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeekStatus {
    /// The current prescription for its calendar week.
    Active,
    /// Replaced by an adjusted re-save of the same week.
    Superseded,
}

impl WeekStatus {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
        }
    }

    /// Parse the DB representation.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// Race priority in the athlete's calendar, borrowed from standard coaching
/// nomenclature: A = the goal, B = tune-up raced for a result, C = training
/// race / no taper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum RacePriority {
    /// Goal race the plan builds toward.
    A,
    /// Secondary race worth a mini-taper.
    B,
    /// Training race ridden through.
    C,
}

impl RacePriority {
    /// Stable string identifier — byte-for-byte what serde emits for this
    /// variant, so renderers can label a race without a JSON round-trip.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
        }
    }
}

/// A race in the plan's calendar.
///
/// The outline's `goal_race` is a **snapshot** taken at plan time; the living
/// source of truth for what the athlete wants is the pillar `Goal` user fact
/// linked via `goal_fact_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GoalRace {
    /// Race name as the athlete calls it ("Big Red").
    pub name: String,
    /// Civil race date, `YYYY-MM-DD` (validated by [`parse_plan_date`]).
    pub date: String,
    /// Discipline ("gravel", "xco", "road", "trail run", …). Freeform.
    pub discipline: String,
    /// Priority in the athlete's calendar.
    pub priority: RacePriority,
}

/// Who chose the plan's flavour — the provenance that makes the choice
/// measurable (D4: the rule proposes, a human may overrule, the override
/// rate is the falsification test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SelectedBy {
    /// The selection rule's top-ranked flavour, taken as proposed.
    Rule,
    /// A human coach chose or overrode, with `override_reason` recorded.
    Coach,
    /// The athlete chose or overrode, with `override_reason` recorded.
    Athlete,
}

impl SelectedBy {
    /// Stable string identifier — byte-for-byte what serde emits.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Coach => "coach",
            Self::Athlete => "athlete",
        }
    }
}

/// The flavour a vision runs on, with its provenance.
///
/// `id` names a catalogue flavour (`training/flavours/<id>.yaml`); `family`,
/// `sequencing` and `modifiers` are copied from that flavour at save time so a
/// stored plan still says what it was built on after the catalogue moves.
///
/// `PartialEq` only: the inputs snapshot carries the weekly hours as a float.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FlavourSelection {
    /// Catalogue flavour id (`polarized-classic`, `hvlit-foundation`, …).
    pub id: String,
    /// Intensity architecture of the flavour at save time.
    pub family: FlavourFamily,
    /// Phase sequencing of the flavour at save time.
    pub sequencing: Sequencing,
    /// Orthogonal add-ons of the flavour at save time.
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    /// Who chose it.
    pub selected_by: SelectedBy,
    /// Why a human overrode the rule, when they did. Empty for a rule
    /// selection; required prose for an agent or athlete choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
    /// What the rule proposed at save time — `recommend_plan_flavour`'s
    /// verdict, passed through by the agent — so an override can be measured
    /// against it later. Absent on a plan saved without running the rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict_snapshot: Option<FlavourVerdict>,
    /// The inputs that verdict was computed from, for the same reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs_snapshot: Option<FlavourInputs>,
}

/// One phase of the vision (mesocycle): a kind with a start date, a length
/// in weeks, its purpose, and the targets the fortnight is written against.
///
/// `purpose` is the catalogue's text for what the phase is for (the skeleton
/// that produced it, or the agent's own words when authored by hand);
/// `intent` stays the agent's voice for this athlete. The targets are optional
/// because an agent may lay out a season by hand without stating them — the
/// save-time rails only measure what a phase states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanPhase {
    /// What kind of phase this is.
    pub kind: PhaseKind,
    /// Civil start date of the phase, `YYYY-MM-DD` (a Monday by convention,
    /// not enforced — athletes' weeks start where their lives allow).
    pub start: String,
    /// Phase length in weeks.
    #[serde(deserialize_with = "whole_u8")]
    pub weeks: u8,
    /// What the phase is for, one sentence.
    #[serde(default)]
    pub purpose: String,
    /// The agent's intent, in agent voice ("rebuild volume, one moderate
    /// day/week").
    pub intent: String,
    /// Target weekly volume in hours, when the agent prescribes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_hours: Option<f32>,
    /// Weekly volume as a share of the season's peak week.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_share_of_peak: Option<Share>,
    /// Share of time below LT1 / between / above LT2 the phase aims for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tid_target: Option<TidTarget>,
    /// Most hard sessions a week in this phase.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub hard_sessions_max: Option<u32>,
    /// Weights over workout purposes the phase draws its sessions from;
    /// empty when the phase states none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub session_mix: BTreeMap<WorkoutPurpose, u8>,
    /// The intensity architecture this phase runs on when it differs from the
    /// plan's flavour (a pyramidal base under a polarized plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavour_override: Option<FlavourFamily>,
    /// Load-to-recovery week pattern inside the phase ("3:1").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_pattern: Option<LoadingPattern>,
    /// The catalogue skeleton this phase was laid out from, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_id: Option<String>,
}

impl PlanPhase {
    /// Civil start date, when it parses.
    #[must_use]
    pub fn start_date(&self) -> Option<CivilDate> {
        parse_plan_date(&self.start)
    }

    /// First day after the phase — `start + 7 × weeks` — when it exists on
    /// the calendar. A phase whose span runs past `CivilDate::MAX` holds no
    /// end date, the same way an unparseable start holds no start.
    #[must_use]
    pub fn end_exclusive(&self) -> Option<CivilDate> {
        let days = i64::from(self.weeks).checked_mul(7)?;
        self.start_date()?.checked_add_days(days)
    }

    /// Whether `date` falls inside the phase's span.
    #[must_use]
    pub fn covers(&self, date: CivilDate) -> bool {
        match (self.start_date(), self.end_exclusive()) {
            (Some(start), Some(end)) => (start..end).contains(&date),
            _ => false,
        }
    }
}

/// The parameters a planned day fills in on its template, inside the
/// template's ranges — what makes template use measurable at save time.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct TemplateParams {
    /// Sets, when the template has them.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub sets: Option<u32>,
    /// Repetitions per set.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub reps: Option<u32>,
    /// Work interval in seconds.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub work_seconds: Option<u32>,
    /// Recovery interval in seconds.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub rest_seconds: Option<u32>,
    /// Session duration in minutes, for continuous templates.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub duration_minutes: Option<u32>,
    /// Rest between sets in seconds, for a family preset with sets.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub set_rest_seconds: Option<u32>,
    /// How the main set is paced: `best_sustainable` (the trialled dose for
    /// short reps) or `fixed` on the written target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing: Option<Pacing>,
    /// The anchor the athlete wants targets written against (`critical_power`,
    /// `max_heart_rate`, `rpe`, …); the preset's own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<AnchorChoice>,
}

impl TemplateParams {
    /// `true` when no parameter is stated.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.sets.is_none()
            && self.reps.is_none()
            && self.work_seconds.is_none()
            && self.rest_seconds.is_none()
            && self.duration_minutes.is_none()
            && self.set_rest_seconds.is_none()
            && self.pacing.is_none()
            && self.anchor.is_none()
    }

    /// The values as the kernel's family generator reads them.
    #[must_use]
    pub const fn preset_params(&self) -> PresetParams {
        PresetParams {
            sets: self.sets,
            reps: self.reps,
            work_seconds: self.work_seconds,
            rest_seconds: self.rest_seconds,
            set_rest_seconds: self.set_rest_seconds,
            duration_minutes: self.duration_minutes,
            pacing: self.pacing,
            anchor: self.anchor,
        }
    }
}

/// What the envelope check found on one planned day — the session measured
/// against its family's evidence envelope, stamped by the save.
///
/// Every finding warns; a relabel also names the purpose the session now
/// trains, which compliance and readiness read in place of the template's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DayEnvelope {
    /// The preset the session was measured against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The purpose the session trains when an edit moved it to another
    /// family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relabelled_purpose: Option<WorkoutPurpose>,
    /// The preset's evidence badge in the day's sport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<EvidenceBadge>,
    /// Whether the save generated the steps from the preset.
    #[serde(default)]
    pub generated: bool,
    /// What the check found.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<EnvelopeFinding>,
    /// Why the save could not build the day's steps from its preset, when it
    /// could not: the generated session left the bounds a written one keeps.
    /// The day then carries no steps and the agent is told so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_generated: Option<String>,
    /// The athlete's anchor the day's targets are written on, as it stood
    /// when the day was saved: what a notice about the day names ("FTP 285 W
    /// from intervals.icu"), with how far to trust it (carnet#1496). Absent
    /// when the targets name no anchor the athlete holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<TargetAnchor>,
}

impl DayEnvelope {
    /// The stored part of a kernel report, with the anchor the measured
    /// steps are written on.
    #[must_use]
    pub fn from_report(
        report: EnvelopeReport,
        generated: bool,
        anchor: Option<TargetAnchor>,
    ) -> Self {
        Self {
            preset: report.preset,
            relabelled_purpose: report.relabelled_purpose,
            badge: report.badge,
            generated,
            findings: report.findings,
            not_generated: None,
            anchor,
        }
    }
}

/// One prescribed day inside a [`PlanWeek`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlannedDay {
    /// Civil date, `YYYY-MM-DD`.
    pub date: String,
    /// Sport ("mtb", "gravel", "run", …) or `"rest"` for a rest day.
    pub sport: String,
    /// What to do, in agent voice ("2h endurance, low HR on climbs").
    pub workout: String,
    /// Planned duration in minutes; `None` for rest days.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub duration_min: Option<u32>,
    /// Intensity relative to the athlete's thresholds ("Z2", "3x8min @
    /// 88-93% FTP"). Empty for rest days. Stored relative — never absolute
    /// watts — so the plan survives an FTP retest. With `steps`, the day's
    /// summary label.
    #[serde(default)]
    pub intensity: String,
    /// The session's structure, when the agent states it as steps rather
    /// than prose: the same [`WorkoutStep`] vocabulary a single prescription
    /// carries, so a plan day reaches a provider calendar as workout-builder
    /// steps with a computable planned load. Empty for a steady day, a rest
    /// day, or a day the agent described in words only — `intensity` is then
    /// the only structure the day has.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkoutStep>,
    /// What to take in during the session, when the agent prescribes it.
    ///
    /// The agents prescribe fuelling on every long session; without a field
    /// to land in, that prescription was discarded on the way to storage and
    /// the athlete never saw it. Absent for a session short enough not to
    /// need fuelling, and for a rest day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fueling: Option<FuelingProtocol>,
    /// The catalogue template this day instantiates, when it does — the
    /// slug `list_workout_templates` lists. Absent for a rest day and for a
    /// session the agent wrote without a template, which the compliance
    /// verdict then counts as unclassified rather than guesses at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_slug: Option<String>,
    /// The values this day fills the template's ranges with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_params: Option<TemplateParams>,
    /// Which tier `template_slug` resolved in when the day was saved —
    /// stamped by the save, never taken from the payload. Absent when the
    /// day names no template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_source: Option<TemplateSource>,
    /// The family preset this day instantiates (`30_15`, `end_spinups`),
    /// when `template_slug` names a parametric family. A legacy slug the
    /// family absorbed already names its preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_preset: Option<String>,
    /// What the envelope check found on the session — stamped by the save,
    /// never taken from the payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<DayEnvelope>,
    /// The day as it stood before the coach's first edit in this draft, when
    /// the coach edited it from their plan thread (carnet#1714). Stamped by the
    /// coach's day saves only, never taken from an agent payload, never shown
    /// to the agent, and dropped when the draft is published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub coach_original: Option<Box<Self>>,
    /// Whether the coach added this session to its date from their plan
    /// thread (carnet#1716): restoring it removes it. Set by the coach's "add
    /// a session" only, never taken from an agent payload, never shown to the
    /// agent, and dropped when the draft is published.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(skip)]
    pub coach_added: bool,
    /// The coach's call on this session (carnet#1814), « C'est mon choix »:
    /// the coach saw where its evidence stands — a convention, or outside its
    /// family's evidence — and keeps the session as their choice. Set or
    /// cleared by the coach's own day write only, never taken from an agent
    /// payload.
    ///
    /// Unlike the editor marks it describes the day itself, so publishing
    /// keeps it and the agent reads it. A changed session drops it: an
    /// agent's save keeps it only on a session it saves unchanged
    /// ([`carry_coach_calls`]), and the coach's own edit of the session
    /// writes it without one, so the pre-publish check asks again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub coach_call: Option<CoachCall>,
}

/// Who made the coach's call on a plan day, and when (carnet#1814).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoachCall {
    /// The coach who made the call.
    pub by: Uuid,
    /// When they made it.
    pub at: Timestamp,
}

/// The authorship tier a day's template was found in at save time.
///
/// The resolution order is package, then catalogue, then the athlete's own
/// rows, and the saved day records which one answered so a plan still says
/// where its sessions came from after a package is unpublished or a
/// catalogue file moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TemplateSource {
    /// The agent package's own `workouts/<slug>.toml`.
    Package,
    /// The Dravr catalogue — contremaitre's `training/workouts/`, or the
    /// compiled-in mirror of it.
    Catalogue,
    /// The athlete's own saved session (`workout_templates` row).
    Athlete,
}

impl TemplateSource {
    /// Stable string identifier — byte-for-byte what serde emits.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Catalogue => "catalogue",
            Self::Athlete => "athlete",
        }
    }
}

impl PlannedDay {
    /// `true` when the day prescribes no training.
    #[must_use]
    pub fn is_rest(&self) -> bool {
        self.sport.eq_ignore_ascii_case("rest")
    }

    /// Whether `other` prescribes the same session as this day: the same
    /// sport, name, length, intensity and steps, from the same template and
    /// preset with the same values. The date is not compared: the caller
    /// pairs the two days by date and position.
    ///
    /// What a save stamps on a day (its template's tier, its envelope), its
    /// fuelling — which an agent may never have been shown, so it may not
    /// write it back — and the coach's marks are not the session. Generated
    /// steps are compared like written ones: a family day whose steps came
    /// out otherwise on the athlete's current anchors is a changed session.
    #[must_use]
    pub(crate) fn is_same_session(&self, other: &Self) -> bool {
        self.sport == other.sport
            && self.workout == other.workout
            && self.duration_min == other.duration_min
            && self.intensity == other.intensity
            && self.steps == other.steps
            && self.template_slug == other.template_slug
            && self.template_preset == other.template_preset
            && self.template_params == other.template_params
    }
}

/// The plan outline (macrocycle): goal-race snapshot, block structure, and
/// strategy.
///
/// Tenant-scoped; at most one `active` per (tenant, user) — the athlete's
/// season, which every agent they use reads and adds weeks to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrainingPlan {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the plan.
    pub tenant_id: String,
    /// Athlete the plan is for.
    pub user_id: String,
    /// The agent that laid this outline, or `None` when no agent did. It
    /// records authorship only: which plan an agent reads never depends on it.
    pub author_agent_id: Option<String>,
    /// Pillar `Goal` user-fact this plan serves, when linked. The fact is the
    /// living goal; this plan snapshots it. A superseded fact flags the plan
    /// stale on read.
    pub goal_fact_id: Option<String>,
    /// Snapshot of the goal race taken at plan time; `None` for a season the
    /// athlete started by adding a session, before any race was set
    /// (carnet#1841). Stored as JSON `null`.
    pub goal_race: Option<GoalRace>,
    /// The rest of the race calendar beside [`Self::goal_race`]. An entry
    /// at [`RacePriority::A`] is a second peak the season is laid on, with
    /// a transition between the two blocks; B and C entries are context.
    pub races: Vec<GoalRace>,
    /// The agent's strategy in prose — what the athlete sees as "what the
    /// coach has in mind".
    pub strategy: String,
    /// The flavour the season runs on, with who chose it. `None` for a plan
    /// laid out without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavour: Option<FlavourSelection>,
    /// First day of the season the phases lay out, `YYYY-MM-DD`; `None`
    /// when the plan runs from its first phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_start: Option<String>,
    /// Last day of the season, `YYYY-MM-DD`; `None` when the plan ends with
    /// its goal race.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_end: Option<String>,
    /// Ordered phases of the season, multi-peak allowed.
    pub phases: Vec<PlanPhase>,
    /// Lifecycle status.
    pub status: PlanStatus,
    /// Outline this one replaced, if any (audit chain).
    pub supersedes_id: Option<String>,
    /// Conversation the plan was agreed in, for provenance.
    pub source_conversation_id: Option<String>,
    /// When this outline row was created.
    pub created_at: Timestamp,
    /// When this outline row last changed status.
    pub updated_at: Timestamp,
    /// Where this plan may be served (carnet#769): first-party-only when what it
    /// was derived from was. Plans written before the stamp read as unstamped.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub transport_policy: TransportPolicy,
}

/// One microcycle: the day-by-day prescription for a single calendar week of
/// a [`TrainingPlan`]. At most one `active` per (plan, `week_start`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanWeek {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the week.
    pub tenant_id: String,
    /// Athlete the week is for (denormalized for tenant-scoped reads).
    pub user_id: String,
    /// Outline this week belongs to.
    pub plan_id: String,
    /// Civil date of the week's first day, `YYYY-MM-DD`.
    pub week_start: String,
    /// The week's intent in agent voice ("volume back up, one moderate day").
    pub focus: String,
    /// Index into the outline's `phases` of the phase this week instantiates,
    /// when the agent states it; the week is then measured against that
    /// phase's targets.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub phase_index: Option<u32>,
    /// The day rows, in date order. Two sessions on one date are two entries
    /// with that date, in the order they are done; at most 5 per date.
    pub days: Vec<PlannedDay>,
    /// Lifecycle status.
    pub status: WeekStatus,
    /// Week row this one replaced, if any (adjustment audit chain).
    pub supersedes_id: Option<String>,
    /// Why the agent re-saved this week ("legs heavy after Buckland, moved
    /// tempo to Wednesday"). Empty on first save.
    pub adjustment_reason: String,
    /// The agent that wrote this version of the week, or `None` when no agent
    /// did. A week carried onto a new outline keeps its author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,
    /// When this week row was created.
    pub created_at: Timestamp,
    /// When this week row last changed status.
    pub updated_at: Timestamp,
}

impl PlanWeek {
    /// Drop the coach's editor marks from every day — the pre-edit snapshot
    /// (carnet#1714) and the added-session flag (carnet#1716): a week that
    /// goes live, or that an agent reads, carries neither. The coach's call
    /// on a day (carnet#1814) is no editor mark and stays.
    pub fn drop_coach_marks(&mut self) {
        for day in &mut self.days {
            day.coach_original = None;
            day.coach_added = false;
        }
    }
}

/// Most entries one date holds, a rest entry included (carnet#1716). A week
/// is bounded by its seven dates, so it holds at most 35.
pub const MAX_SESSIONS_PER_DATE: usize = 5;

/// The position of each of `days` among the entries on its date, in the
/// order given: 0-based, a rest entry included. With its date, this is how a
/// session is known (carnet#1716); a session has no stable id.
#[must_use]
pub fn session_positions<'a>(days: impl IntoIterator<Item = &'a PlannedDay>) -> Vec<u32> {
    let mut seen: BTreeMap<&'a str, u32> = BTreeMap::new();
    days.into_iter()
        .map(|day| {
            let next = seen.entry(day.date.as_str()).or_insert(0);
            let position = *next;
            *next = next.saturating_add(1);
            position
        })
        .collect()
}

/// Carry the coach's calls in `prior` onto the unchanged sessions of `days`.
///
/// A call moves to the entry of `days` at the same date and position when it
/// is still the same session: the same sport, name, length, intensity and
/// steps, from the same template and preset with the same values. Every other
/// entry of `days` is left with no call, whatever it carried (carnet#1814).
///
/// `prior` is the week a save writes `days` over. A session saved unchanged
/// keeps the coach's call; a changed, moved or new one has none, so the
/// coach's pre-publish check asks about it again.
pub fn carry_coach_calls(days: &mut [PlannedDay], prior: &[PlannedDay]) {
    let called: BTreeMap<(&str, u32), &PlannedDay> = prior
        .iter()
        .zip(session_positions(prior))
        .filter(|(day, _)| day.coach_call.is_some())
        .map(|(day, position)| ((day.date.as_str(), position), day))
        .collect();
    let positions = session_positions(&*days);
    for (day, position) in days.iter_mut().zip(positions) {
        let carried = called
            .get(&(day.date.as_str(), position))
            .filter(|before| before.is_same_session(day))
            .and_then(|before| before.coach_call);
        day.coach_call = carried;
    }
}

/// The dates whose entries `after` changes over `before`, in date order.
///
/// Each week of `after` is compared with the week of `before` that has the
/// same `week_start`, or with no entries when `before` holds none: weeks that
/// are not in `after` are not compared. A date counts as changed when its
/// entries differ, compared in order and in every field — a session
/// rewritten, added, taken away, or moved within the date (carnet#1716).
/// What is compared is the session the athlete trains on: the coach's call
/// (carnet#1814) and the coach's editor marks (carnet#1714, carnet#1716) are
/// not part of it, so a session that only gained or lost one of them is no
/// change.
#[must_use]
pub fn changed_dates<'a>(before: &'a [PlanWeek], after: &'a [PlanWeek]) -> BTreeSet<&'a str> {
    let mut changed = BTreeSet::new();
    for week in after {
        let prior = before
            .iter()
            .find(|held| held.week_start == week.week_start)
            .map_or(&[][..], |held| held.days.as_slice());
        let dates: BTreeSet<&str> = prior
            .iter()
            .chain(&week.days)
            .map(|day| day.date.as_str())
            .collect();
        for date in dates {
            let was = prior
                .iter()
                .filter(|day| day.date == date)
                .map(training_content);
            let is = week
                .days
                .iter()
                .filter(|day| day.date == date)
                .map(training_content);
            if !was.eq(is) {
                changed.insert(date);
            }
        }
    }
    changed
}

/// `day` as [`changed_dates`] compares it: without the coach's call and the
/// coach's editor marks. Two entries whose training content is equal are the
/// same session, whoever marked either.
#[must_use]
pub fn training_content(day: &PlannedDay) -> PlannedDay {
    PlannedDay {
        coach_original: None,
        coach_added: false,
        coach_call: None,
        ..day.clone()
    }
}

/// Parse a civil plan date, accepting only canonical zero-padded
/// `YYYY-MM-DD`.
///
/// Stored dates sort lexically (`ORDER BY week_start`), so a non-padded
/// `2026-8-8` — which `CivilDate::parse_from_str` would happily parse — must
/// be rejected or it would sort after every August entry. Plan dates are athlete-visible and
/// must never be guessed.
#[must_use]
pub fn parse_plan_date(s: &str) -> Option<CivilDate> {
    let parsed = CivilDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    (parsed.to_string() == s).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited_day(date: &str) -> PlannedDay {
        serde_json::from_value(serde_json::json!({
            "date": date, "sport": "run", "workout": "Easy 50 min", "intensity": "Z2",
            "coach_original": { "date": date, "sport": "rest", "workout": "Rest" }
        }))
        .unwrap()
    }

    #[test]
    fn a_coachs_marks_are_stored_with_their_day_and_dropped_from_a_week_that_goes_live() {
        let day = edited_day("2026-11-18");
        let stored = serde_json::to_value(&day).unwrap();
        assert_eq!(
            stored["coach_original"]["workout"], "Rest",
            "bug: the day's JSON column keeps the coach's snapshot: {stored}"
        );
        let unedited = PlannedDay {
            coach_original: None,
            ..day.clone()
        };
        let unmarked = serde_json::to_value(&unedited).unwrap();
        assert!(
            unmarked.get("coach_original").is_none() && unmarked.get("coach_added").is_none(),
            "bug: a day with no snapshot and not added writes neither key: {unmarked}"
        );
        let added = PlannedDay {
            coach_added: true,
            ..unedited
        };
        let stored_added = serde_json::to_value(&added).unwrap();
        assert_eq!(
            stored_added["coach_added"], true,
            "bug: the day's JSON column keeps the added-session flag: {stored_added}"
        );
        assert_eq!(
            serde_json::from_value::<PlannedDay>(stored_added).unwrap(),
            added,
            "bug: an added session reads back as written"
        );

        let mut week = week_of("2026-11-16", vec![edited_day("2026-11-17"), day, added]);
        week.drop_coach_marks();
        assert!(
            week.days
                .iter()
                .all(|day| day.coach_original.is_none() && !day.coach_added),
            "bug: a week that goes live keeps a coach's mark: {:?}",
            week.days
        );
        assert!(
            week.days.iter().all(|day| day.workout == "Easy 50 min"),
            "bug: dropping the marks changed the days themselves"
        );
    }

    #[test]
    fn a_session_is_placed_among_its_dates_entries_rest_included() {
        let on = |date: &str, sport: &str| -> PlannedDay {
            serde_json::from_value(
                serde_json::json!({ "date": date, "sport": sport, "workout": sport }),
            )
            .unwrap()
        };
        let days = [
            on("2026-11-16", "ride"),
            on("2026-11-16", "rest"),
            on("2026-11-16", "strength"),
            on("2026-11-17", "run"),
            on("2026-11-18", "swim"),
            on("2026-11-18", "run"),
        ];
        assert_eq!(
            session_positions(&days),
            [0, 1, 2, 0, 0, 1],
            "bug: each entry's position counts the entries before it on its date"
        );
        assert!(
            session_positions(&[]).is_empty(),
            "bug: no entries, no positions"
        );
    }

    fn call(by: u128) -> CoachCall {
        CoachCall {
            by: Uuid::from_u128(by),
            at: Timestamp::now(),
        }
    }

    fn session(date: &str, workout: &str) -> PlannedDay {
        serde_json::from_value(serde_json::json!({
            "date": date, "sport": "ride", "workout": workout, "duration_min": 61,
            "intensity": "100-110% MAP", "template_slug": "vo2max_short",
            "template_preset": "30_15", "template_params": { "sets": 3 },
            "steps": [{ "label": "Work", "duration_seconds": 30, "target_zone": "Z5", "repeat": 13 }]
        }))
        .unwrap()
    }

    fn called(date: &str, workout: &str, by: u128) -> PlannedDay {
        PlannedDay {
            coach_call: Some(call(by)),
            ..session(date, workout)
        }
    }

    #[test]
    fn a_coachs_call_is_stored_with_its_day_and_kept_by_a_week_that_goes_live() {
        let day = called("2026-11-18", "VO2max — 30/15", 7);
        let stored = serde_json::to_value(&day).unwrap();
        assert_eq!(
            stored["coach_call"]["by"],
            Uuid::from_u128(7).to_string(),
            "bug: the day's JSON column keeps who made the call: {stored}"
        );
        assert_eq!(
            serde_json::from_value::<PlannedDay>(stored).unwrap(),
            day,
            "bug: a called day reads back as written"
        );
        assert!(
            serde_json::to_value(session("2026-11-18", "VO2max — 30/15"))
                .unwrap()
                .get("coach_call")
                .is_none(),
            "bug: a day with no call writes no key"
        );

        let mut week = PlanWeek {
            id: "week".to_owned(),
            tenant_id: "tenant".to_owned(),
            user_id: "user".to_owned(),
            plan_id: "draft".to_owned(),
            week_start: "2026-11-16".to_owned(),
            focus: String::new(),
            phase_index: None,
            days: vec![PlannedDay {
                coach_added: true,
                ..day.clone()
            }],
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        };
        week.drop_coach_marks();
        assert_eq!(
            week.days[0].coach_call, day.coach_call,
            "bug: a week that goes live dropped the coach's call on its day"
        );
        assert!(
            !week.days[0].coach_added,
            "bug: the editor marks are still dropped beside the call"
        );
    }

    #[test]
    fn a_session_is_the_same_whatever_its_save_stamped_fuelling_or_marks() {
        let day = session("2026-11-18", "VO2max — 30/15");
        let restamped: PlannedDay = serde_json::from_value(serde_json::json!({
            "date": "2026-11-25", "sport": "ride", "workout": "VO2max — 30/15",
            "duration_min": 61, "intensity": "100-110% MAP", "template_slug": "vo2max_short",
            "template_preset": "30_15", "template_params": { "sets": 3 },
            "template_source": "catalogue", "envelope": { "generated": true },
            "fueling": { "carbs_g_per_h": 60.0, "fluid_ml_per_h": 500.0 },
            "steps": [{ "label": "Work", "duration_seconds": 30, "target_zone": "Z5", "repeat": 13 }],
            "coach_added": true
        }))
        .unwrap();
        assert!(
            day.is_same_session(&restamped),
            "bug: the date, the save's stamps, the fuelling and the marks are not the session"
        );

        let edits: [fn(&mut PlannedDay); 8] = [
            |d| d.sport = "gravel".to_owned(),
            |d| d.workout = "VO2max — 40/20".to_owned(),
            |d| d.duration_min = Some(62),
            |d| d.intensity = "Z5".to_owned(),
            |d| d.steps[0].repeat = 12,
            |d| d.template_slug = Some("vo2max_long".to_owned()),
            |d| d.template_preset = Some("40_20".to_owned()),
            |d| d.template_params = None,
        ];
        for (at, edit) in edits.iter().enumerate() {
            let mut changed = day.clone();
            edit(&mut changed);
            assert!(
                !day.is_same_session(&changed),
                "bug: change {at} of the session's own fields leaves it the same session"
            );
        }
    }

    #[test]
    fn an_agents_save_keeps_a_coachs_call_only_on_a_session_it_saves_unchanged() {
        let prior = vec![
            called("2026-11-17", "Tuesday intervals", 1),
            called("2026-11-18", "Morning ride", 2),
            called("2026-11-18", "Evening ride", 3),
            session("2026-11-19", "Thursday ride"),
            called("2026-11-20", "Friday ride", 4),
        ];
        let mut saved = vec![
            // Unchanged, but the payload forged another coach's call.
            called("2026-11-17", "Tuesday intervals", 99),
            // The morning ride is rewritten; the evening ride moves to
            // position 0 of its date, unchanged.
            session("2026-11-18", "Evening ride"),
            // Thursday never had a call, and the payload cannot give it one.
            called("2026-11-19", "Thursday ride", 99),
            // Friday's session comes back with one rep fewer.
            {
                let mut fewer = session("2026-11-20", "Friday ride");
                fewer.steps[0].repeat = 12;
                fewer
            },
        ];
        carry_coach_calls(&mut saved, &prior);
        let calls: Vec<Option<Uuid>> = saved
            .iter()
            .map(|day| day.coach_call.map(|call| call.by))
            .collect();
        assert_eq!(
            calls,
            [Some(Uuid::from_u128(1)), None, None, None],
            "bug: only the session saved unchanged at its date and position keeps the coach's call, \
             and never the payload's own: {saved:?}"
        );
        assert_eq!(
            saved[0].coach_call.map(|call| call.at),
            prior[0].coach_call.map(|call| call.at),
            "bug: the call carried is the prior one, when it was made included"
        );

        let mut over_nothing = vec![called("2026-11-17", "Tuesday intervals", 99)];
        carry_coach_calls(&mut over_nothing, &[]);
        assert!(
            over_nothing[0].coach_call.is_none(),
            "bug: a week saved over no prior week carries no call"
        );
    }

    fn week_of(week_start: &str, days: Vec<PlannedDay>) -> PlanWeek {
        PlanWeek {
            id: format!("week-{week_start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "user".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: week_start.to_owned(),
            focus: String::new(),
            phase_index: None,
            days,
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    #[test]
    fn the_changed_dates_are_those_whose_sessions_differ_and_never_a_mark_alone() {
        let before = vec![
            week_of(
                "2026-11-16",
                vec![
                    session("2026-11-16", "Monday ride"),
                    session("2026-11-17", "Morning ride"),
                    session("2026-11-17", "Evening ride"),
                    session("2026-11-18", "Wednesday ride"),
                    session("2026-11-19", "Thursday ride"),
                    session("2026-11-20", "Friday ride"),
                ],
            ),
            week_of("2026-11-23", vec![session("2026-11-24", "Untouched week")]),
        ];
        let marked = PlannedDay {
            coach_original: Some(Box::new(session("2026-11-19", "Older"))),
            coach_added: true,
            ..called("2026-11-19", "Thursday ride", 7)
        };
        let after = vec![
            week_of(
                "2026-11-16",
                vec![
                    // Monday unchanged; Tuesday's two sessions swap order.
                    session("2026-11-16", "Monday ride"),
                    session("2026-11-17", "Evening ride"),
                    session("2026-11-17", "Morning ride"),
                    // Wednesday rewritten; Thursday the same session with a
                    // call and both editor marks; Friday taken away, and
                    // Saturday added.
                    session("2026-11-18", "Hill repeats"),
                    marked,
                    session("2026-11-21", "Long ride"),
                ],
            ),
            // A week with no counterpart before: every date it holds.
            week_of("2026-11-30", vec![session("2026-12-01", "New week")]),
        ];
        let changed: Vec<&str> = changed_dates(&before, &after).into_iter().collect();
        assert_eq!(
            changed,
            [
                "2026-11-17",
                "2026-11-18",
                "2026-11-20",
                "2026-11-21",
                "2026-12-01"
            ],
            "bug: a date counts when its sessions were rewritten, reordered, taken away or added, \
             never for a call or an editor mark alone, nor in a week the change does not hold"
        );
        assert!(
            changed_dates(&before, &before).is_empty(),
            "bug: weeks written over themselves change no date"
        );
    }
}
