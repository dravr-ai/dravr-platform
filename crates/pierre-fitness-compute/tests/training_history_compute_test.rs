// ABOUTME: Endurance Phase 2 unit tests for compute_training_history — CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate
// ABOUTME: Pure-function tests against synthetic activity series; locks in framework formulas
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::models::activity::{Activity, ActivityBuilder};
use dravr_cageux::models::sport::SportType;
use pierre_fitness_compute::training_history_compute::{
    compute_training_history, AthleteInputs, MAX_BACKFILL_DAYS,
};
use std::slice::from_ref;

/// Default algorithm config (EMA 42/7) for the dense training-history rollup.
fn algos() -> AlgorithmConfig {
    AlgorithmConfig::default()
}

fn day(d: i64) -> NaiveDate {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .unwrap()
        .date_naive()
        + Duration::days(d)
}

fn run_at(date_idx: i64, duration_seconds: u64, avg_hr: u32) -> Activity {
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap() + Duration::days(date_idx);
    ActivityBuilder::new(
        format!("a{date_idx}"),
        format!("synthetic {date_idx}"),
        SportType::Run,
        start,
        duration_seconds,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(avg_hr)
    .build()
}

fn athlete_with_lthr() -> AthleteInputs {
    AthleteInputs {
        ftp_watts: None,
        lthr: Some(170.0),
        max_hr: Some(190.0),
        resting_hr: Some(50.0),
        weight_kg: Some(70.0),
    }
}

#[test]
fn empty_window_returns_dense_zero_rows() {
    let from = day(0);
    let to = day(6);
    let rows = compute_training_history(&[], AthleteInputs::default(), from, to, &algos(), None);
    assert_eq!(rows.len(), 7);
    for row in &rows {
        assert!(row.daily_load.abs() < f64::EPSILON);
        assert!(row.ctl.abs() < f64::EPSILON);
        assert!(row.atl.abs() < f64::EPSILON);
        assert!(row.tsb.abs() < f64::EPSILON);
    }
}

#[test]
fn inverted_window_returns_no_rows() {
    let rows = compute_training_history(
        &[],
        AthleteInputs::default(),
        day(10),
        day(1),
        &algos(),
        None,
    );
    assert!(rows.is_empty());
}

#[test]
fn oversize_window_returns_no_rows() {
    let from = day(0);
    let to = from + Duration::days(MAX_BACKFILL_DAYS + 1);
    let rows = compute_training_history(&[], AthleteInputs::default(), from, to, &algos(), None);
    assert!(rows.is_empty());
}

#[test]
fn ctl_atl_tsb_monotonically_track_load() {
    // 30 consecutive days of identical hard effort.
    let activities: Vec<Activity> = (0..30).map(|i| run_at(i, 3600, 165)).collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(29),
        &algos(),
        None,
    );
    assert_eq!(rows.len(), 30);
    // CTL should be strictly increasing while we're loading consistently.
    let mut prev_ctl = -1.0_f64;
    for row in &rows {
        assert!(
            row.ctl >= prev_ctl,
            "CTL should be non-decreasing under sustained load: prev={prev_ctl}, today={}",
            row.ctl
        );
        prev_ctl = row.ctl;
    }
    // ATL must move faster than CTL initially → TSB negative early on.
    assert!(rows[10].tsb < 0.0);
    // Daily load is positive when activities are present.
    assert!(rows[15].daily_load > 0.0);
}

#[test]
fn rest_days_drop_atl_and_lift_tsb() {
    // 14 days of effort, then 14 days of rest.
    let mut activities: Vec<Activity> = (0..14).map(|i| run_at(i, 3600, 165)).collect();
    activities.extend((14..28).map(|i| run_at(i, 0, 0))); // zero-duration acts ignored
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(27),
        &algos(),
        None,
    );
    let last_load_day = &rows[13];
    let last_rest_day = &rows[27];
    assert!(
        last_rest_day.atl < last_load_day.atl,
        "ATL should drop after 14 rest days: load_day={}, rest_day={}",
        last_load_day.atl,
        last_rest_day.atl
    );
    assert!(
        last_rest_day.tsb >= last_load_day.tsb,
        "TSB should rise after 14 rest days: load_day={}, rest_day={}",
        last_load_day.tsb,
        last_rest_day.tsb
    );
}

#[test]
fn acwr_unset_for_short_history() {
    let activities: Vec<Activity> = (0..14).map(|i| run_at(i, 3600, 165)).collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(13),
        &algos(),
        None,
    );
    // First 27 days must have ACWR == None; we have 14 days here, so all None.
    for row in &rows {
        assert!(row.acwr.is_none());
    }
}

#[test]
fn acwr_present_after_28_days_of_history() {
    let activities: Vec<Activity> = (0..40).map(|i| run_at(i, 3600, 160)).collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(39),
        &algos(),
        None,
    );
    // Index 27 is the first day where 28 days of history are available.
    assert!(
        rows[27].acwr.is_some(),
        "ACWR must be set on day 27 (idx 27 = day 28)"
    );
    let acwr = rows[35].acwr.unwrap();
    // With perfectly steady load, ACWR should be approximately 1.0.
    assert!(
        (acwr - 1.0).abs() < 0.2,
        "ACWR ~= 1.0 under steady load, got {acwr}"
    );
}

#[test]
fn monotony_and_strain_unset_until_seven_days_of_load() {
    let activities: Vec<Activity> = (0..6).map(|i| run_at(i, 3600, 165)).collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(5),
        &algos(),
        None,
    );
    for row in &rows {
        assert!(row.monotony.is_none());
        assert!(row.strain.is_none());
    }
}

#[test]
fn monotony_strain_present_after_one_full_week() {
    // Vary the load across the week so std-dev > 0 (Foster monotony is
    // undefined when every day has identical load).
    let activities: Vec<Activity> = (0..14)
        .map(|i| {
            let dur = 1800 + (i % 4) as u64 * 600; // 30, 40, 50, 60 min cycle
            run_at(i, dur, 165)
        })
        .collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(13),
        &algos(),
        None,
    );
    assert!(rows[6].monotony.is_some());
    assert!(rows[6].strain.is_some());
}

#[test]
fn ramp_rate_unset_for_short_history() {
    let activities: Vec<Activity> = (0..6).map(|i| run_at(i, 3600, 165)).collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(5),
        &algos(),
        None,
    );
    for row in &rows {
        assert!(row.ramp_rate.is_none());
    }
}

#[test]
fn ramp_rate_positive_under_progressive_load() {
    // 21 days of escalating load (warmup + buildup so CTL has room to climb).
    let activities: Vec<Activity> = (0..21)
        .map(|i| {
            let dur = 1800 + u64::try_from(i).unwrap_or(0) * 120;
            run_at(i, dur, 165)
        })
        .collect();
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(20),
        &algos(),
        None,
    );
    let ramp = rows[15].ramp_rate.expect("ramp_rate after warmup");
    assert!(
        ramp >= 0.0,
        "ramp_rate should be non-negative under progressive load, got {ramp}"
    );
}

/// The window's end must be the athlete's civil day, not the server's.
///
/// The rollup buckets each activity on the athlete's own date, and then drops
/// anything past `to`. So a caller that bounds the window with
/// `Utc::now().date_naive()` silently loses the current day for every athlete
/// ahead of UTC — the session they just finished never reaches CTL, ATL, TSB,
/// or any answer built on them (registre#260).
///
/// This is the coupling, stated as a test: the same activity, the same zone,
/// two window ends one day apart.
#[test]
fn a_utc_window_end_drops_the_athletes_current_civil_day() {
    // 23:00 UTC on the 3rd is 09:00 on the 4th in Sydney.
    let start = Utc.with_ymd_and_hms(2026, 9, 3, 23, 0, 0).unwrap();
    let morning_ride = ActivityBuilder::new(
        "syd-1".to_owned(),
        "this morning".to_owned(),
        SportType::Run,
        start,
        3_600,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(165)
    .build();

    let from = NaiveDate::from_ymd_opt(2026, 8, 25).unwrap();
    let server_today = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
    let athlete_today = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let zone = Some("Australia/Sydney");

    let bounded_by_the_server = compute_training_history(
        from_ref(&morning_ride),
        athlete_with_lthr(),
        from,
        server_today,
        &algos(),
        zone,
    );
    assert!(
        bounded_by_the_server
            .iter()
            .all(|r| r.daily_load.abs() < f64::EPSILON),
        "the ride buckets onto the athlete's 4th, which is past a window \
         ending on the server's 3rd — so every row is empty and the day is \
         simply gone"
    );

    let bounded_by_the_athlete = compute_training_history(
        from_ref(&morning_ride),
        athlete_with_lthr(),
        from,
        athlete_today,
        &algos(),
        zone,
    );
    let carried = bounded_by_the_athlete
        .iter()
        .find(|r| r.date == athlete_today)
        .expect("the athlete's own day must be a row in their own window");
    assert!(
        carried.daily_load > 0.0,
        "bounded on the athlete's clock the ride is counted: {}",
        carried.daily_load
    );
}
