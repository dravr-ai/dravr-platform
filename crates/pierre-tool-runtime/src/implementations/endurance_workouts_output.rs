// ABOUTME: The shapes the endurance workout tools answer with, and their derived schemas
// ABOUTME: Split from endurance_workouts.rs, which is past its size ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the three endurance workout tools.
//!
//! `list_workout_templates` answers with either a summary row or the whole
//! template, decided by the caller's `detail` argument, so its row type is an
//! untagged enum over the two. Both come from `dravr_cageux`, which derives
//! `JsonSchema` as of v0.11.0 — the full template carries the phases, the
//! target zones and the step list, and mirroring that here would have been a
//! second copy of the workout bank's shape.

use pierre_core::models::periodization::{
    EvidenceTier, IntensityDistribution, PhaseKind, ProgressionLever, ReadinessLevel,
    WorkoutPurpose,
};
use pierre_core::models::{SportType, WorkoutTemplate};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// How a phase fits into a plan.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TemplateFit {
    /// Which training phases the session belongs in.
    pub phases: Vec<PhaseKind>,
    /// The readiness band below which it should not be prescribed.
    pub readiness_min: ReadinessLevel,
    /// How many times a week it can be repeated.
    pub max_per_week: u8,
    /// The rest required between repeats, in hours.
    pub min_spacing_hours: u16,
}

/// The fields an agent picks a session by, without its steps.
///
/// The default `detail` mode. A full template carries the whole step list,
/// which is long, and an agent choosing between sessions does not need it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WorkoutTemplateSummary {
    /// Stable identifier `prescribe_workout` takes.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// What the session is for.
    pub purpose: WorkoutPurpose,
    /// The sport it is written for.
    pub sport: SportType,
    /// Other sports it transfers to.
    pub sport_variants: Vec<SportType>,
    /// How long it runs.
    pub duration_minutes: u32,
    /// How the time splits across intensities.
    pub intensity_distribution: IntensityDistribution,
    /// How well evidenced the session is.
    pub evidence_tier: EvidenceTier,
    /// What to watch for when prescribing it; absent when there is nothing.
    pub caveat: Option<String>,
    /// The parameters that shape it, as the bank defines them.
    pub params: Value,
    /// Where it fits in a plan.
    pub fit: TemplateFit,
    /// How the session grows week to week.
    pub progression: TemplateProgression,
    /// Whether it ships with the platform rather than being athlete-authored.
    pub is_compiled_in: bool,
}

/// How a session is made harder from one week to the next.
///
/// Every template in the bank authors this and nothing read it, so an agent
/// writing week 2 of a fortnight invented the step — a rep here, a longer
/// interval there — with no source of truth to invent it from. The levers are
/// ordered: pull the first before the second.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TemplateProgression {
    /// The levers to pull, in the order the bank says to pull them.
    pub order: Vec<ProgressionLever>,
    /// How many levers may move in a single week.
    pub max_weekly_step: u8,
}

/// One row of the workout bank, at the detail the caller asked for.
///
/// Untagged, and the arms are NOT distinguishable from a row alone: the full
/// template carries `fit` and `is_compiled_in` too, so a summary's keys are a
/// subset of a full row's and a full row validates against both arms. Read
/// `filters.detail` on the parent, which always says which detail level was
/// served. Stated here because the obvious reading — that a row's own keys
/// tell you — is wrong.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum WorkoutTemplateRow {
    /// The picking fields only.
    Summary(Box<WorkoutTemplateSummary>),
    /// The whole session, steps included.
    Full(Box<WorkoutTemplate>),
}

/// The filters a listing was taken under, echoed back.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WorkoutTemplateFilters {
    /// The purpose filter, when one was given.
    pub purpose: Option<String>,
    /// The phase filter, when one was given.
    pub phase: Option<String>,
    /// The sport filter, when one was given.
    pub sport: Option<String>,
    /// The detail level in force: `summary` or `full`.
    pub detail: String,
}

/// What `list_workout_templates` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WorkoutTemplatesResult {
    /// How many rows came back.
    pub count: usize,
    /// What the listing was filtered by, so a caller can tell an empty result
    /// from a narrow filter.
    pub filters: WorkoutTemplateFilters,
    /// The bank, then the athlete's own saved sessions.
    pub templates: Vec<WorkoutTemplateRow>,
}

/// What `prescribe_workout` answers with.
///
/// Carries the provider's event id because the workout is on the athlete's
/// real calendar by the time this returns. An agent told only "it worked"
/// cannot undo it; `withdraw_prescribed_workout` takes the prescription id.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PrescribeWorkoutResult {
    /// The ledger row, which is what withdrawing takes.
    pub prescription_id: Uuid,
    /// The calendar the entry was pushed to.
    pub provider: String,
    /// The provider's own id for the entry.
    pub provider_event_id: String,
    /// The prescription this replaced, when it replaced one. Absent
    /// otherwise — a replacement and a first prescription are different acts.
    pub replaced_prescription_id: Option<Uuid>,
    /// The template that was prescribed.
    pub template_slug: String,
    /// Its display name.
    pub name: String,
    /// How long the session runs.
    pub duration_minutes: u32,
    /// The day it was put on, as `YYYY-MM-DD`.
    pub scheduled_for: String,
    /// Always `pushed`: the tool errors rather than reporting a failed push.
    pub status: String,
}

/// What `withdraw_prescribed_workout` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WithdrawWorkoutResult {
    /// The ledger row that was withdrawn.
    pub prescription_id: Uuid,
    /// The calendar the entry was removed from.
    pub provider: String,
    /// The provider's id for the entry that is now gone.
    pub provider_event_id: String,
    /// The template it was. Absent for a prescription whose ledger row
    /// predates the slug column.
    pub name: Option<String>,
    /// The day it had been on.
    pub scheduled_for: String,
    /// Always `withdrawn`.
    pub status: String,
}
