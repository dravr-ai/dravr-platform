// ABOUTME: The declared shape of recommend_plan_flavour — the verdict, the season laid out, and the inputs it used
// ABOUTME: The verdict is cageux's own type; only the projections this tool adds are modelled here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a flavour recommendation answers with.
//!
//! The verdict is [`FlavourVerdict`] itself rather than a copy of it. Its
//! vocabulary enums are generated with `#[serde(rename = …)]` matching their
//! `as_str()` byte for byte, so serializing the type produces exactly what
//! the hand-written projection produced — and a client now gets the allowed
//! values in the schema instead of a free-text string.
//!
//! The season and the inputs echo are this tool's own projections: they add
//! a `status` discriminator and a per-input provenance list that cageux does
//! not carry, so they are modelled here.
//!
//! The verdict is a near-mirror rather than the kernel type itself, for one
//! field: every ranked and excluded flavour carries a `label`, the flavour in
//! the athlete's own words and locale. That is what the coach says out loud
//! while the id stays the coach's name for it, and it is resolved from the
//! messaging registry, so it cannot live on the kernel type.

use std::collections::BTreeSet;

use pierre_core::models::periodization::{
    Confidence, EventClass, InjuryLoad, InputDimension, IntervalExperience, Measurement, Reason,
    RecoverySpeed, SeasonPhase, SportMix, TrainingAge,
};
use serde::Serialize;

/// What `recommend_plan_flavour` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PlanFlavourResult {
    /// The athlete this is for — a coach may be acting for someone else,
    /// and `None` means the caller's own plan.
    pub athlete: Option<String>,
    /// What they can run, what they cannot, and how sure the rule is.
    pub verdict: LabelledVerdict,
    /// The season laid backward from the goal, or why it could not be.
    pub season: SeasonReport,
    /// The inputs the verdict was reached on, and where each came from, so
    /// the athlete can correct one rather than argue with the answer.
    pub inputs: FlavourInputsEcho,
}

/// The verdict, with each flavour named in the athlete's language.
///
/// Every field but the labels is [`pierre_core::models::periodization::FlavourVerdict`]'s
/// own, and an unknown field is ignored on the way back in, so a coach can
/// pass this straight to `save_training_plan.flavour.verdict` and the stored
/// snapshot still deserializes into the kernel type.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LabelledVerdict {
    /// Eligible flavours, highest score first.
    pub ranked: Vec<LabelledFlavour>,
    /// Everything ruled out, with its reasons.
    pub excluded: Vec<LabelledExclusion>,
    /// How much to lean on this.
    pub confidence: Confidence,
    /// Dimensions the profile could not answer — the ones to ask about.
    pub missing_inputs: Vec<InputDimension>,
    /// Set when a coach package pinned a flavour and it survived eligibility.
    pub coach_pinned: Option<String>,
}

/// A flavour the athlete can run.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LabelledFlavour {
    /// The flavour id — the coach's name for it, stable across locales.
    pub id: String,
    /// The same flavour in the athlete's language. Falls back to the id when
    /// the registry has no string for it, so this is never empty.
    pub label: String,
    /// Summed `prefer` weights.
    pub score: u32,
    /// The rows that argued for it, heaviest first.
    pub reasons: Vec<Reason>,
}

/// A flavour the athlete cannot run.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LabelledExclusion {
    /// The flavour id.
    pub id: String,
    /// The same flavour in the athlete's language.
    pub label: String,
    /// Every reason it is out, in the order they were found.
    pub reasons: Vec<String>,
}

/// The season, or the reason there is none.
///
/// `status` says which: `laid` fills `phases` and `shrunk`,
/// `not_enough_runway` fills `needs_weeks` and `has_weeks`, and `no_goal`
/// fills neither. One shape with a discriminator rather than untagged arms,
/// because `no_goal` carries a strict subset of the others' keys.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SeasonReport {
    /// `laid`, `not_enough_runway` or `no_goal`. Read this first.
    pub status: String,
    /// The skeleton the layout was built on, when one was chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_id: Option<String>,
    /// What to do about it, when there is nothing to lay out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Weeks the skeleton needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_weeks: Option<u8>,
    /// Weeks there actually are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_weeks: Option<u32>,
    /// The phases, earliest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<LaidPhaseReport>,
    /// Phases that had to be shortened to fit the runway. Named rather than
    /// counted, because which phase was cut is the athlete's decision to
    /// disagree with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shrunk: Vec<String>,
}

/// One phase of a laid season.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LaidPhaseReport {
    /// Base, build, peak, taper — what the weeks are for.
    pub kind: String,
    /// The first day, `YYYY-MM-DD`.
    pub start: String,
    /// How many weeks it runs.
    pub weeks: u8,
    /// What the phase is meant to achieve, in the athlete's terms.
    pub purpose: String,
    /// The volume band, as a share of the season's peak week.
    pub volume_share_of_peak: ShareRange,
    /// A flavour this phase runs instead of the season's, when it differs.
    pub flavour_override: Option<String>,
    /// The sessions that define the phase.
    pub key_sessions: Vec<String>,
    /// Load weeks to recovery weeks, written `"3:1"`.
    pub loading_pattern: String,
    /// The peak week's date, `YYYY-MM-DD`.
    pub peak: String,
}

/// A share band, both ends inclusive.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ShareRange {
    /// The low end, as a fraction of the peak week.
    pub min: f32,
    /// The high end.
    pub max: f32,
}

/// The inputs echoed back, with their provenance.
///
/// The vocabulary fields are cageux's enums, so the schema lists what each
/// one accepts. An athlete correcting "recreational" to "trained" can see
/// the whole ladder rather than guessing at the spelling.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FlavourInputsEcho {
    /// Weekly training hours.
    pub hours_per_week: f32,
    /// Which hours band that falls in.
    pub hours_tier: String,
    /// Sessions per week.
    pub sessions_per_week: u8,
    /// The training-age band.
    pub training_age: TrainingAge,
    /// Years of structured training behind it.
    pub training_age_years: f32,
    /// The goal event's class, when a goal is known.
    pub event_class: Option<EventClass>,
    /// Weeks until that goal.
    pub weeks_to_goal: Option<u32>,
    /// What the athlete can actually measure — the devices they have
    /// thresholds for, which is what makes a flavour runnable.
    pub measurements: BTreeSet<Measurement>,
    /// How fast they come back from hard work.
    pub recovery_speed: RecoverySpeed,
    /// How much injury history to plan around.
    pub injury_load: InjuryLoad,
    /// How much interval work they have done before.
    pub interval_experience: IntervalExperience,
    /// Single sport or multi.
    pub sport_mix: SportMix,
    /// Where in the season they are, when it is known.
    pub season_phase: Option<SeasonPhase>,
    /// A flavour the coach asked for, when one was named.
    pub coach_preference: Option<String>,
    /// Where each input came from — what the athlete answered, what the
    /// profile held, what was assumed. An assumed input is the one to
    /// correct first.
    pub sources: Vec<InputSource>,
}

/// Where one input came from.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct InputSource {
    /// The input's name.
    pub input: String,
    /// Its provenance.
    pub from: String,
}
