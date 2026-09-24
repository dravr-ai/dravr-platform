// ABOUTME: A TrainingPeaks planned workout, as sciotte scrapes it, in the periodization step grammar
// ABOUTME: Blocks become numbered sets, each step's target a RelativeIntensity label, or none when the grammar cannot hold it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte's `PlannedWorkout` → cageux's [`PlannedWorkout`].
//!
//! Sciotte keeps a `TrainingPeaks` structure as the provider states it: blocks
//! of steps, each block with its repeat count, and every target a number in
//! the structure's one intensity metric. The platform's shape is the
//! periodization kernel's flat step list, whose sets are runs of steps that
//! share a `repeat` and a `repeat_group`, and whose targets are labels in the
//! [`RelativeIntensity`] grammar. This module is the one place the first
//! becomes the second.
//!
//! - **Sets.** Every step of block *n* (counted from 1, in order) carries the
//!   block's `repeat` and `repeat_group = n`. The group number is what keeps
//!   two adjacent blocks with the same count — 3 × (1 min on, 1 min off)
//!   straight into 3 × (30 s on, 30 s off) — from reading as one set.
//! - **Length.** A step's seconds, or `0` with its metres for a step measured
//!   only by distance: the lower bound the kernel documents for a
//!   distance-only step.
//! - **Target.** `percentOfFtp`, `percentOfThresholdPace` and
//!   `percentOfThresholdHr` become a percent band of FTP, threshold pace and
//!   threshold heart rate; `rpe` becomes an RPE band, rounded to the whole
//!   ratings the scale holds. A target the grammar cannot state — a percent
//!   outside the band it accepts (a swim rest's 0 %), an RPE outside 1–10, a
//!   bound the step does not carry, a metric or target kind sciotte has not
//!   named — leaves `target_zone` empty. The kernel reads an empty or
//!   unparseable label as prose, so the step stays timed and untargeted
//!   rather than carrying a target the coach never set.
//!
//! Titles, descriptions and step names cross verbatim: they are untrusted
//! third-party text, and each consumer neutralizes them for its own
//! destination (the planned-workouts tool fences them before a model reads
//! them).

use dravr_sciotte::models::{
    IntensityClass, IntensityMetric, PlannedBlock, PlannedStep, PlannedStructure,
    PlannedWorkout as SciottePlannedWorkout, TargetKind,
};

use crate::constants::oauth::providers::TRAININGPEAKS;
use crate::models::periodization::{RpeRange, ThresholdBasis};
use crate::models::{PlannedWorkout, PlannedWorkoutBuilder, RelativeIntensity, WorkoutStep};
use crate::sciotte_provider::convert_sport_type;

/// The spelling `TrainingPeaks` gives a structure measured in percent of
/// threshold heart rate. Sciotte has no variant for it and hands it through
/// as `IntensityMetric::Other`, the provider's own spelling.
const PERCENT_OF_THRESHOLD_HR: &str = "percentOfThresholdHr";

/// Convert one planned workout sciotte read from a `TrainingPeaks` calendar.
///
/// The provider is `trainingpeaks`, the name the athlete knows it by, and
/// the workout keeps sciotte's id (`athleteId:workoutId`), the same id the
/// activity list gives the workout once it is done — which is also what
/// `completed_activity_id` names.
#[must_use]
pub fn planned_workout_from_trainingpeaks(sciotte: &SciottePlannedWorkout) -> PlannedWorkout {
    PlannedWorkoutBuilder::new(
        TRAININGPEAKS,
        sciotte.id.clone(),
        sciotte.date,
        convert_sport_type(&sciotte.sport_type),
        sciotte.title.clone(),
    )
    .start_time_opt(sciotte.start_time)
    .description_opt(sciotte.description.clone())
    .planned_duration_seconds_opt(sciotte.planned_duration_seconds)
    .planned_distance_meters_opt(sciotte.planned_distance_meters)
    .planned_training_stress_score_opt(sciotte.planned_training_stress_score)
    .planned_intensity_factor_opt(sciotte.planned_intensity_factor)
    .steps(
        sciotte
            .structure
            .as_ref()
            .map_or_else(Vec::new, steps_from_structure),
    )
    .completed_activity_id_opt(sciotte.completed_activity_id.clone())
    .build()
}

/// The structure's blocks flattened into steps, in the order they are
/// performed, each numbered with the block it came from.
fn steps_from_structure(structure: &PlannedStructure) -> Vec<WorkoutStep> {
    (1_u32..)
        .zip(&structure.blocks)
        .flat_map(|(ordinal, block)| block_steps(structure, block, ordinal))
        .collect()
}

/// One block's steps, carrying its repeat count and its ordinal as the set.
fn block_steps<'a>(
    structure: &'a PlannedStructure,
    block: &'a PlannedBlock,
    ordinal: u32,
) -> impl Iterator<Item = WorkoutStep> + 'a {
    let repeat = block.repeat;
    block.steps.iter().map(move |step| WorkoutStep {
        label: step_label(step),
        duration_seconds: step.duration_seconds.unwrap_or(0),
        distance_meters: step.distance_meters,
        target_zone: target_zone(
            &structure.intensity_metric,
            structure.target_kind.as_ref(),
            step,
        ),
        repeat,
        repeat_group: Some(ordinal),
        note: None,
    })
}

/// The step's name, or the role it plays when the plan left the name blank.
fn step_label(step: &PlannedStep) -> String {
    if !step.name.trim().is_empty() {
        return step.name.clone();
    }
    match &step.intensity_class {
        IntensityClass::WarmUp => "Warm up".to_owned(),
        IntensityClass::Active => "Active".to_owned(),
        IntensityClass::Rest => "Rest".to_owned(),
        IntensityClass::CoolDown => "Cool down".to_owned(),
        IntensityClass::Other(spelling) => spelling.clone(),
    }
}

/// The step's target as a [`RelativeIntensity`] label, or empty when the
/// grammar cannot state it.
fn target_zone(metric: &IntensityMetric, kind: Option<&TargetKind>, step: &PlannedStep) -> String {
    let Some((low, high)) = target_bounds(kind, step) else {
        return String::new();
    };
    let intensity = match metric {
        IntensityMetric::PercentOfFtp => percent_band(low, high, ThresholdBasis::Ftp),
        IntensityMetric::PercentOfThresholdPace => percent_band(low, high, ThresholdBasis::Pace),
        IntensityMetric::Other(spelling) if spelling == PERCENT_OF_THRESHOLD_HR => {
            percent_band(low, high, ThresholdBasis::HeartRate)
        }
        IntensityMetric::Rpe => rpe_band(low, high),
        // An absolute pace (COROS, seconds per kilometre) has no place in a
        // grammar of bands relative to the athlete's threshold, and turning it
        // into one would need a threshold the plan does not carry: the step
        // goes out untargeted, as an unknown metric does.
        IntensityMetric::Pace | IntensityMetric::Other(_) => None,
    };
    intensity.map_or_else(String::new, |intensity| intensity.to_string())
}

/// The target's `(low, high)` bounds, by the structure's target kind.
///
/// A single value carries only `target_min`. A range needs both bounds, so a
/// range step missing its upper one has no target. `TrainingPeaks` leaves the
/// kind off some structures (the captured swim set states none), and then the
/// step's own bounds say which it is: two bounds are a range, one is a value.
/// A kind sciotte has not named says nothing about how to read the bounds.
fn target_bounds(kind: Option<&TargetKind>, step: &PlannedStep) -> Option<(f64, f64)> {
    let low = step.target_min?;
    match kind {
        Some(TargetKind::Target) => Some((low, low)),
        Some(TargetKind::Range) => Some((low, step.target_max?)),
        None => Some((low, step.target_max.unwrap_or(low))),
        Some(TargetKind::Other(_)) => None,
    }
}

/// A percent band of `basis`, when the grammar accepts it.
fn percent_band(low: f64, high: f64, basis: ThresholdBasis) -> Option<RelativeIntensity> {
    accepted(RelativeIntensity::Percent {
        low: whole(low)?,
        high: whole(high)?,
        threshold: Some(basis),
    })
}

/// An RPE band on the 1–10 scale, when the grammar accepts it.
fn rpe_band(low: f64, high: f64) -> Option<RelativeIntensity> {
    accepted(RelativeIntensity::Rpe(RpeRange {
        min: u8::try_from(whole(low)?).ok()?,
        max: u8::try_from(whole(high)?).ok()?,
    }))
}

/// `intensity` when the grammar reads its own label back to it, else `None`.
///
/// The grammar's parser is the one statement of which bands it accepts (a
/// percent inside 1–300 with `low <= high`, an RPE inside 1–10 with
/// `min <= max`), so a band is checked against it rather than against a
/// second copy of those bounds that could drift.
fn accepted(intensity: RelativeIntensity) -> Option<RelativeIntensity> {
    (RelativeIntensity::parse(&intensity.to_string()) == Some(intensity)).then_some(intensity)
}

/// `value` rounded to a whole number, when it is one a `u16` holds.
fn whole(value: f64) -> Option<u16> {
    let rounded = value.round();
    if !(0.0..=f64::from(u16::MAX)).contains(&rounded) {
        return None;
    }
    // Safe: `rounded` is a whole number inside 0..=u16::MAX, checked above,
    // so the cast neither truncates a fraction nor wraps a sign.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = rounded as u16;
    Some(whole)
}
