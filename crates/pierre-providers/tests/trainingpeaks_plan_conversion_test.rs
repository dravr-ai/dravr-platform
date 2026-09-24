// ABOUTME: Pins how a scraped TrainingPeaks plan becomes a cageux PlannedWorkout, step by step and target by target
// ABOUTME: The three captured structure shapes with exact values, plus every target the grammar cannot hold left empty
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte TrainingPeaks plan → cageux `PlannedWorkout` conversion contract.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The three structures are the shapes sciotte's planned extract reads off
//! the captured TrainingPeaks calendar (`tests/fixtures/trainingpeaks/
//! workouts-v7.json` in dravr-sciotte, pinned there by
//! `trainingpeaks_planned_extract.rs`): a ride by duration in percent of FTP
//! with one-step blocks, a run by duration in RPE ranges with a 4× repetition
//! block, and a swim by distance in percent of threshold pace whose rests are
//! timed and target 0 %. Each is built here with the exact values that test
//! asserts, and converted with the function production calls.

use chrono::{NaiveDate, TimeZone, Utc};
use dravr_sciotte::models::{
    IntensityClass, IntensityMetric, LengthMetric, PlannedBlock, PlannedStep, PlannedStructure,
    PlannedWorkout as SciottePlannedWorkout, SportType as SciotteSportType, TargetKind,
};
use pierre_providers::models::periodization::ThresholdBasis;
use pierre_providers::models::{PlannedWorkout, RelativeIntensity, SportType, WorkoutStep};
use pierre_providers::trainingpeaks_plan::planned_workout_from_trainingpeaks;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date literal") // Safe: literal calendar date
}

/// A sciotte step: its name, role, length and target bounds.
fn tp_step(
    name: &str,
    intensity_class: IntensityClass,
    duration_seconds: Option<u32>,
    distance_meters: Option<f64>,
    target: (Option<f64>, Option<f64>),
) -> PlannedStep {
    PlannedStep {
        name: name.to_owned(),
        intensity_class,
        duration_seconds,
        distance_meters,
        target_min: target.0,
        target_max: target.1,
    }
}

fn block(repeat: u32, steps: Vec<PlannedStep>) -> PlannedBlock {
    PlannedBlock { repeat, steps }
}

/// A planned workout with nothing but its identity, day, sport and title.
fn tp_workout(
    id: &str,
    date: NaiveDate,
    sport: SciotteSportType,
    title: &str,
) -> SciottePlannedWorkout {
    SciottePlannedWorkout {
        id: id.to_owned(),
        date,
        start_time: None,
        sport_type: sport,
        title: title.to_owned(),
        description: None,
        planned_duration_seconds: None,
        planned_distance_meters: None,
        planned_training_stress_score: None,
        planned_intensity_factor: None,
        structure: None,
        completed_activity_id: None,
    }
}

/// A cageux step as the conversion should produce it.
fn step(
    label: &str,
    duration_seconds: u32,
    distance_meters: Option<f64>,
    target_zone: &str,
    repeat: u32,
    repeat_group: u32,
) -> WorkoutStep {
    WorkoutStep {
        label: label.to_owned(),
        duration_seconds,
        distance_meters,
        target_zone: target_zone.to_owned(),
        repeat,
        repeat_group: Some(repeat_group),
        note: None,
    }
}

/// 910004: "Threshold 3x10 (next week)", a ride by duration in percent of
/// FTP, every target a single value, eight one-step blocks.
fn threshold_ride() -> SciottePlannedWorkout {
    let work = |name: &str, class: IntensityClass, seconds: u32, pct: f64| {
        block(
            1,
            vec![tp_step(name, class, Some(seconds), None, (Some(pct), None))],
        )
    };
    SciottePlannedWorkout {
        description: Some("Same session as last week; aim for 5 more watts per block.".to_owned()),
        planned_duration_seconds: Some(3600),
        planned_distance_meters: Some(32_000.0),
        planned_training_stress_score: Some(78.3),
        planned_intensity_factor: Some(0.85),
        structure: Some(PlannedStructure {
            intensity_metric: IntensityMetric::PercentOfFtp,
            target_kind: Some(TargetKind::Target),
            length_metric: LengthMetric::Duration,
            blocks: vec![
                work("Warm up", IntensityClass::WarmUp, 600, 45.0),
                work("Active", IntensityClass::Active, 600, 120.0),
                work("Recovery", IntensityClass::Rest, 300, 55.0),
                work("Active", IntensityClass::Active, 600, 120.0),
                work("Recovery", IntensityClass::Rest, 300, 55.0),
                work("Active", IntensityClass::Active, 600, 120.0),
                work("Recovery", IntensityClass::Rest, 300, 55.0),
                work("Cool down", IntensityClass::CoolDown, 300, 45.0),
            ],
        }),
        ..tp_workout(
            "900001:910004",
            day(2026, 9, 28),
            SciotteSportType::Ride,
            "Threshold 3x10 (next week)",
        )
    }
}

/// 910005: "Run by feel", RPE ranges by duration, with a 4× block of two
/// steps between lone ones.
fn run_by_feel() -> SciottePlannedWorkout {
    let easy = (Some(1.0), Some(5.0));
    SciottePlannedWorkout {
        planned_training_stress_score: Some(82.0),
        planned_intensity_factor: Some(0.92),
        structure: Some(PlannedStructure {
            intensity_metric: IntensityMetric::Rpe,
            target_kind: Some(TargetKind::Range),
            length_metric: LengthMetric::Duration,
            blocks: vec![
                block(
                    1,
                    vec![tp_step(
                        "Warm up",
                        IntensityClass::WarmUp,
                        Some(300),
                        None,
                        easy,
                    )],
                ),
                block(
                    4,
                    vec![
                        tp_step(
                            "Hard",
                            IntensityClass::Active,
                            Some(60),
                            None,
                            (Some(7.0), Some(9.0)),
                        ),
                        tp_step("Easy", IntensityClass::Rest, Some(60), None, easy),
                    ],
                ),
                block(
                    1,
                    vec![tp_step(
                        "Recovery",
                        IntensityClass::Rest,
                        Some(300),
                        None,
                        easy,
                    )],
                ),
                block(
                    1,
                    vec![tp_step(
                        "Active",
                        IntensityClass::Active,
                        Some(1800),
                        None,
                        (Some(8.0), Some(10.0)),
                    )],
                ),
                block(
                    1,
                    vec![tp_step(
                        "Cool Down",
                        IntensityClass::CoolDown,
                        Some(600),
                        None,
                        easy,
                    )],
                ),
            ],
        }),
        ..tp_workout(
            "900001:910005",
            day(2026, 9, 29),
            SciotteSportType::Run,
            "Run by feel",
        )
    }
}

/// 910006: "Pool 100s", percent of threshold pace by distance, no target
/// kind stated, rests measured in seconds at 0 %.
fn pool_hundreds() -> SciottePlannedWorkout {
    let meters = |name: &str, class: IntensityClass, m: f64, pct: f64| {
        tp_step(name, class, None, Some(m), (Some(pct), None))
    };
    let rest = |name: &str| {
        tp_step(
            name,
            IntensityClass::Rest,
            Some(10),
            None,
            (Some(0.0), None),
        )
    };
    let drill = || {
        block(
            1,
            vec![
                meters("Pull", IntensityClass::Active, 50.0, 75.0),
                meters("Kick", IntensityClass::Active, 50.0, 75.0),
                meters("Swim", IntensityClass::Rest, 50.0, 75.0),
            ],
        )
    };
    SciottePlannedWorkout {
        planned_distance_meters: Some(3225.0),
        structure: Some(PlannedStructure {
            intensity_metric: IntensityMetric::PercentOfThresholdPace,
            target_kind: None,
            length_metric: LengthMetric::Distance,
            blocks: vec![
                drill(),
                block(1, vec![rest("Recovery")]),
                drill(),
                block(1, vec![meters("Kick", IntensityClass::Rest, 200.0, 75.0)]),
                block(
                    2,
                    vec![
                        meters("Hard", IntensityClass::Active, 50.0, 90.0),
                        rest("Rest"),
                    ],
                ),
                block(
                    4,
                    vec![
                        meters("Hard", IntensityClass::Active, 100.0, 100.0),
                        rest("Rest"),
                    ],
                ),
                block(
                    1,
                    vec![meters("Time Trial", IntensityClass::Active, 300.0, 110.0)],
                ),
                block(
                    1,
                    vec![meters("Cool Down", IntensityClass::CoolDown, 200.0, 75.0)],
                ),
            ],
        }),
        ..tp_workout(
            "900001:910006",
            day(2026, 9, 30),
            SciotteSportType::Swim,
            "Pool 100s",
        )
    }
}

/// The steps of a structure with one block per `(metric, kind, step)` case,
/// converted, each block's lone step's target.
fn targets(
    metric: IntensityMetric,
    kind: Option<TargetKind>,
    cases: Vec<PlannedStep>,
) -> Vec<String> {
    let workout = SciottePlannedWorkout {
        structure: Some(PlannedStructure {
            intensity_metric: metric,
            target_kind: kind,
            length_metric: LengthMetric::Duration,
            blocks: cases.into_iter().map(|s| block(1, vec![s])).collect(),
        }),
        ..tp_workout("900001:1", day(2026, 10, 5), SciotteSportType::Run, "Cases")
    };
    planned_workout_from_trainingpeaks(&workout)
        .steps()
        .iter()
        .map(|s| s.target_zone.clone())
        .collect()
}

fn bounded(min: f64, max: Option<f64>) -> PlannedStep {
    tp_step(
        "Step",
        IntensityClass::Active,
        Some(60),
        None,
        (Some(min), max),
    )
}

#[test]
fn a_power_structure_by_duration_becomes_ftp_percent_steps_numbered_by_block() {
    let workout = planned_workout_from_trainingpeaks(&threshold_ride());

    assert_eq!(workout.provider(), "trainingpeaks");
    assert_eq!(workout.provider_workout_id(), "900001:910004");
    assert_eq!(workout.date(), day(2026, 9, 28));
    assert_eq!(workout.sport_type(), &SportType::Ride);
    assert_eq!(workout.title(), "Threshold 3x10 (next week)");
    assert_eq!(
        workout.description(),
        Some("Same session as last week; aim for 5 more watts per block.")
    );
    assert_eq!(workout.planned_duration_seconds(), Some(3600));
    assert_eq!(workout.planned_distance_meters(), Some(32_000.0));
    assert_eq!(workout.planned_training_stress_score(), Some(78.3));
    assert_eq!(workout.planned_intensity_factor(), Some(0.85));
    assert_eq!(workout.completed_activity_id(), None);
    assert_eq!(
        workout.steps(),
        &[
            step("Warm up", 600, None, "45% FTP", 1, 1),
            step("Active", 600, None, "120% FTP", 1, 2),
            step("Recovery", 300, None, "55% FTP", 1, 3),
            step("Active", 600, None, "120% FTP", 1, 4),
            step("Recovery", 300, None, "55% FTP", 1, 5),
            step("Active", 600, None, "120% FTP", 1, 6),
            step("Recovery", 300, None, "55% FTP", 1, 7),
            step("Cool down", 300, None, "45% FTP", 1, 8),
        ][..]
    );
    // The label is the grammar's, so the kernel reads it back as the band.
    assert_eq!(
        RelativeIntensity::parse(&workout.steps()[1].target_zone),
        Some(RelativeIntensity::Percent {
            low: 120,
            high: 120,
            threshold: Some(ThresholdBasis::Ftp),
        })
    );
}

#[test]
fn an_rpe_range_structure_keeps_its_four_times_block_as_one_set() {
    let workout = planned_workout_from_trainingpeaks(&run_by_feel());

    assert_eq!(workout.sport_type(), &SportType::Run);
    assert_eq!(workout.planned_duration_seconds(), None);
    assert_eq!(workout.planned_training_stress_score(), Some(82.0));
    assert_eq!(workout.planned_intensity_factor(), Some(0.92));
    assert_eq!(
        workout.steps(),
        &[
            step("Warm up", 300, None, "RPE 1-5", 1, 1),
            step("Hard", 60, None, "RPE 7-9", 4, 2),
            step("Easy", 60, None, "RPE 1-5", 4, 2),
            step("Recovery", 300, None, "RPE 1-5", 1, 3),
            step("Active", 1800, None, "RPE 8-10", 1, 4),
            step("Cool Down", 600, None, "RPE 1-5", 1, 5),
        ][..]
    );
    // 4 × (1 min hard, 1 min easy) is eight minutes of the session, not two.
    assert_eq!(WorkoutStep::total_seconds(workout.steps()), 3480);
}

#[test]
fn a_pace_structure_by_distance_keeps_its_metres_and_leaves_zero_percent_rests_untargeted() {
    let workout = planned_workout_from_trainingpeaks(&pool_hundreds());

    assert_eq!(workout.sport_type(), &SportType::Swim);
    assert_eq!(workout.planned_distance_meters(), Some(3225.0));
    let drill = |group: u32| {
        [
            step("Pull", 0, Some(50.0), "75% threshold pace", 1, group),
            step("Kick", 0, Some(50.0), "75% threshold pace", 1, group),
            step("Swim", 0, Some(50.0), "75% threshold pace", 1, group),
        ]
    };
    let mut expected = Vec::new();
    expected.extend(drill(1));
    // A 0 % rest is outside the band the grammar accepts: timed, untargeted.
    expected.push(step("Recovery", 10, None, "", 1, 2));
    expected.extend(drill(3));
    expected.push(step("Kick", 0, Some(200.0), "75% threshold pace", 1, 4));
    expected.push(step("Hard", 0, Some(50.0), "90% threshold pace", 2, 5));
    expected.push(step("Rest", 10, None, "", 2, 5));
    expected.push(step("Hard", 0, Some(100.0), "100% threshold pace", 4, 6));
    expected.push(step("Rest", 10, None, "", 4, 6));
    expected.push(step(
        "Time Trial",
        0,
        Some(300.0),
        "110% threshold pace",
        1,
        7,
    ));
    expected.push(step(
        "Cool Down",
        0,
        Some(200.0),
        "75% threshold pace",
        1,
        8,
    ));
    assert_eq!(workout.steps(), &expected[..]);
    assert_eq!(
        RelativeIntensity::parse(&workout.steps()[0].target_zone),
        Some(RelativeIntensity::Percent {
            low: 75,
            high: 75,
            threshold: Some(ThresholdBasis::Pace),
        })
    );
}

#[test]
fn adjacent_blocks_with_the_same_count_stay_two_sets() {
    let on_off = |on: u32, off: u32| {
        block(
            3,
            vec![
                tp_step(
                    "On",
                    IntensityClass::Active,
                    Some(on),
                    None,
                    (Some(8.0), None),
                ),
                tp_step(
                    "Off",
                    IntensityClass::Rest,
                    Some(off),
                    None,
                    (Some(3.0), None),
                ),
            ],
        )
    };
    let workout = SciottePlannedWorkout {
        structure: Some(PlannedStructure {
            intensity_metric: IntensityMetric::Rpe,
            target_kind: Some(TargetKind::Target),
            length_metric: LengthMetric::Duration,
            blocks: vec![on_off(60, 60), on_off(30, 30)],
        }),
        ..tp_workout(
            "900001:2",
            day(2026, 10, 6),
            SciotteSportType::Run,
            "Two sets",
        )
    };
    assert_eq!(
        planned_workout_from_trainingpeaks(&workout).steps(),
        &[
            step("On", 60, None, "RPE 8", 3, 1),
            step("Off", 60, None, "RPE 3", 3, 1),
            step("On", 30, None, "RPE 8", 3, 2),
            step("Off", 30, None, "RPE 3", 3, 2),
        ][..]
    );
}

#[test]
fn a_threshold_heart_rate_structure_is_a_band_of_lthr() {
    assert_eq!(
        targets(
            IntensityMetric::Other("percentOfThresholdHr".to_owned()),
            Some(TargetKind::Range),
            vec![bounded(95.0, Some(100.0)), bounded(65.0, Some(75.0))],
        ),
        vec!["95-100% threshold HR", "65-75% threshold HR"]
    );
}

#[test]
fn fractional_bounds_round_to_the_whole_numbers_the_grammar_holds() {
    assert_eq!(
        targets(
            IntensityMetric::PercentOfFtp,
            Some(TargetKind::Range),
            vec![bounded(87.6, Some(93.4))],
        ),
        vec!["88-93% FTP"]
    );
    assert_eq!(
        targets(
            IntensityMetric::Rpe,
            Some(TargetKind::Target),
            vec![bounded(6.4, None)],
        ),
        vec!["RPE 6"]
    );
}

#[test]
fn a_target_the_grammar_cannot_hold_leaves_the_step_untargeted() {
    // Percent: outside 1–300, inverted.
    assert_eq!(
        targets(
            IntensityMetric::PercentOfFtp,
            Some(TargetKind::Range),
            vec![
                bounded(0.0, Some(0.0)),
                bounded(250.0, Some(320.0)),
                bounded(95.0, Some(90.0)),
                bounded(-5.0, Some(10.0)),
            ],
        ),
        vec!["", "", "", ""]
    );
    // RPE: outside 1–10.
    assert_eq!(
        targets(
            IntensityMetric::Rpe,
            Some(TargetKind::Range),
            vec![bounded(0.0, Some(3.0)), bounded(9.0, Some(11.0))],
        ),
        vec!["", ""]
    );
    // A range with no upper bound, and a step with no target at all.
    assert_eq!(
        targets(
            IntensityMetric::PercentOfFtp,
            Some(TargetKind::Range),
            vec![
                bounded(90.0, None),
                tp_step("Open", IntensityClass::Active, Some(60), None, (None, None)),
            ],
        ),
        vec!["", ""]
    );
    // A metric and a target kind sciotte has not named.
    assert_eq!(
        targets(
            IntensityMetric::Other("percentOfMaxHr".to_owned()),
            Some(TargetKind::Target),
            vec![bounded(80.0, None)],
        ),
        vec![""]
    );
    assert_eq!(
        targets(
            IntensityMetric::PercentOfFtp,
            Some(TargetKind::Other("ramp".to_owned())),
            vec![bounded(80.0, Some(90.0))],
        ),
        vec![""]
    );
}

#[test]
fn a_structure_with_no_target_kind_reads_the_step_bounds() {
    assert_eq!(
        targets(
            IntensityMetric::PercentOfThresholdPace,
            None,
            vec![bounded(90.0, Some(95.0)), bounded(100.0, None)],
        ),
        vec!["90-95% threshold pace", "100% threshold pace"]
    );
}

#[test]
fn a_blank_step_name_falls_back_to_the_role_the_step_plays() {
    let named = |class: IntensityClass| tp_step("  ", class, Some(60), None, (None, None));
    let workout = SciottePlannedWorkout {
        structure: Some(PlannedStructure {
            intensity_metric: IntensityMetric::PercentOfFtp,
            target_kind: Some(TargetKind::Target),
            length_metric: LengthMetric::Duration,
            blocks: vec![block(
                1,
                vec![
                    named(IntensityClass::WarmUp),
                    named(IntensityClass::Active),
                    named(IntensityClass::Rest),
                    named(IntensityClass::CoolDown),
                    named(IntensityClass::Other("recover".to_owned())),
                ],
            )],
        }),
        ..tp_workout(
            "900001:3",
            day(2026, 10, 7),
            SciotteSportType::Ride,
            "Roles",
        )
    };
    let labels: Vec<String> = planned_workout_from_trainingpeaks(&workout)
        .steps()
        .iter()
        .map(|s| s.label.clone())
        .collect();
    assert_eq!(
        labels,
        vec!["Warm up", "Active", "Rest", "Cool down", "recover"]
    );
}

#[test]
fn a_day_off_is_a_plan_with_no_steps() {
    let workout = planned_workout_from_trainingpeaks(&SciottePlannedWorkout {
        description: Some("Rest. Sleep in if you can.".to_owned()),
        ..tp_workout(
            "900001:910007",
            day(2026, 10, 1),
            SciotteSportType::Other("Day Off".to_owned()),
            "Day off",
        )
    });
    assert_eq!(
        workout.sport_type(),
        &SportType::Other("Day Off".to_owned())
    );
    assert!(workout.steps().is_empty());
    assert_eq!(workout.description(), Some("Rest. Sleep in if you can."));
    assert_eq!(workout.planned_duration_seconds(), None);
}

#[test]
fn a_done_workout_names_the_activity_that_completed_it_and_keeps_its_start() {
    let start = Utc.with_ymd_and_hms(2026, 9, 14, 7, 15, 0).unwrap(); // Safe: literal instant
    let converted: PlannedWorkout = planned_workout_from_trainingpeaks(&SciottePlannedWorkout {
        start_time: Some(start),
        planned_duration_seconds: Some(3600),
        planned_training_stress_score: Some(70.0),
        completed_activity_id: Some("900001:910001".to_owned()),
        ..tp_workout(
            "900001:910001",
            day(2026, 9, 14),
            SciotteSportType::Ride,
            "Threshold 3x10",
        )
    });
    assert_eq!(converted.completed_activity_id(), Some("900001:910001"));
    assert_eq!(converted.start_time(), Some(start));
    assert_eq!(converted.planned_training_stress_score(), Some(70.0));
}

#[test]
fn an_absolute_pace_is_not_turned_into_a_band_of_threshold() {
    // 4:41-4:49 /km, as COROS states it: seconds per kilometre, faster first.
    // Read as a percent it would be a nonsense band of 281-289%.
    assert_eq!(
        targets(
            IntensityMetric::Pace,
            Some(TargetKind::Range),
            vec![bounded(281.0, Some(289.0))],
        ),
        vec![""]
    );
}
