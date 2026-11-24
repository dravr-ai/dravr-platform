// ABOUTME: The analytics tools read the training load as of the athlete's today in their zone
// ABOUTME: analyze_training_load and calculate_fitness_score agree with cageux's zone-aware reading (carnet#1663)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `analyze_training_load` and `calculate_fitness_score` read the current load
//! as of the athlete's own civil day, the day `get_training_history` and the
//! coach roster read it on. Near the athlete's midnight that day and the UTC
//! date differ, and a reading on the UTC date decays one rest day more or
//! fewer than the history and the roster, a different CTL for the same
//! morning.
//!
//! The tests pick a zone whose civil day differs from UTC's at the moment
//! they run: Kiritimati (UTC+14) is on the next day from 10:00 UTC, the
//! western side of the date line (UTC-12) on the previous day before 12:00
//! UTC; switching at 11:00 keeps an hour from either edge.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::training_load::TrainingLoadCalculator;
use dravr_tronc::dates::Elapsed;
use dravr_tronc::dates::{Timestamp, Zone};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_fitness_compute::AthleteInputs;
use pierre_tool_runtime::implementations::analytics::output::{
    FitnessScoreResult, ProvidersUsed, TrainingLoadResult,
};
use pierre_tool_runtime::implementations::analytics::{
    analyze_detailed_training_load, calculate_fitness_metrics,
};

fn providers() -> ProvidersUsed {
    ProvidersUsed {
        activity_provider: "synthetic".to_owned(),
        sleep_provider: None,
    }
}

/// A zone whose civil day is not UTC's at `now`.
fn zone_off_the_utc_day(now: Timestamp) -> Zone {
    let zone: Zone = if now.utc_time().hour() >= 11 {
        "Pacific/Kiritimati"
    } else {
        "Etc/GMT+12"
    }
    .parse()
    .unwrap();
    assert_ne!(zone.date_at(now), now.utc_date(), "{zone} at {now}");
    zone
}

/// Ninety days of rides that build, 40 TSS a day rising to 130, each at an
/// hour of the UTC day that falls on the same date in `zone`. The two
/// readings then count the same sessions on the same days and differ only in
/// the day they are read as of: a day's decay more or fewer.
fn building_block(now: Timestamp, zone: &Zone) -> Vec<Activity> {
    let hour = if *zone == Zone::from_name("Pacific/Kiritimati").unwrap() {
        5
    } else {
        18
    };
    (1..=90_i64)
        .map(|days_ago| {
            let tss = 130.0 - f32::from(u8::try_from(days_ago).unwrap());
            let start = (now.utc_date() - Elapsed::days(days_ago))
                .and_hms(hour, 0, 0)
                .unwrap()
                .and_utc()
                .unwrap();
            assert_eq!(
                zone.date_at(start),
                start.utc_date(),
                "fixture bug: {start} falls on another date in {zone}"
            );
            ActivityBuilder::new(
                format!("ride-{days_ago}"),
                "Sortie",
                SportType::Ride,
                start,
                3_600,
                "synthetic",
            )
            .training_stress_score(tss)
            .build()
        })
        .collect()
}

/// CTL by cageux's own reading on the athlete's today in `zone`.
fn reading(activities: &[Activity], zone: &Zone, now: Timestamp) -> f64 {
    TrainingLoadCalculator::as_of_today(AlgorithmConfig::default(), zone.clone(), now)
        .calculate_training_load(activities, None, None, None, None, None)
        .unwrap()
        .ctl
}

#[test]
fn analyze_training_load_reads_the_athletes_today() {
    let now = Timestamp::now();
    let zone = zone_off_the_utc_day(now);
    let activities = building_block(now, &zone);
    let TrainingLoadResult::Analyzed(detail) = analyze_detailed_training_load(
        &activities,
        &AthleteInputs::default(),
        &AlgorithmConfig::default(),
        &zone,
        providers(),
    ) else {
        panic!("ninety rides are analysed");
    };
    let local = reading(&activities, &zone, now);
    let utc = reading(&activities, &Zone::utc(), now);
    // The payload rounds CTL to a whole number.
    assert_eq!(detail.load_metrics.ctl, local.round(), "{local}");
    assert!(
        (local.round() - utc.round()).abs() >= 1.0,
        "the two days must read apart: {local} vs {utc}"
    );
}

#[test]
fn calculate_fitness_score_reads_the_athletes_today() {
    let now = Timestamp::now();
    let zone = zone_off_the_utc_day(now);
    let activities = building_block(now, &zone);
    let FitnessScoreResult::Scored(detail) = calculate_fitness_metrics(
        &activities,
        "all_time",
        &AthleteInputs::default(),
        &AlgorithmConfig::default(),
        &zone,
        providers(),
    ) else {
        panic!("ninety rides score");
    };
    let local = reading(&activities, &zone, now);
    // The payload rounds CTL to a whole number.
    assert_eq!(detail.metrics.ctl, local.round(), "{local}");
}
