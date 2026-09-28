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
use dravr_cageux::training_load::TrainingLoadCalculator;
use pierre_core::models::{DailyTrainingState, FormBand};
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
    let rows = compute_training_history(&[], AthleteInputs::default(), from, to, &algos(), None)
        .expect("training-load series");
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
    )
    .expect("training-load series");
    assert!(rows.is_empty());
}

#[test]
fn oversize_window_returns_no_rows() {
    let from = day(0);
    let to = from + Duration::days(MAX_BACKFILL_DAYS + 1);
    let rows = compute_training_history(&[], AthleteInputs::default(), from, to, &algos(), None)
        .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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
    )
    .expect("training-load series");
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

/// The history series and the current training load are one computation.
///
/// Ten days of training then five of rest: the history's last row and
/// cageux's `TrainingLoadCalculator` as of that same day must report the same
/// CTL, ATL and TSB, both decayed over the rest days. The platform used to
/// walk its own EMA here while the calculator stopped at the last activity,
/// so after rest days the two disagreed.
#[test]
fn history_and_training_load_agree_after_rest_days() {
    let activities: Vec<Activity> = (0..10).map(|i| run_at(i, 3600, 165)).collect();
    let inputs = athlete_with_lthr();
    let rows = compute_training_history(&activities, inputs, day(0), day(14), &algos(), None)
        .expect("training-load series");
    let last = rows.last().expect("a row for the as-of day");
    assert_eq!(last.date, day(14));

    let load = TrainingLoadCalculator::from_config(algos(), day(14))
        .calculate_training_load(
            &activities,
            inputs.ftp_watts,
            inputs.lthr,
            inputs.max_hr,
            inputs.resting_hr,
            inputs.weight_kg,
        )
        .expect("training load");
    assert!(load.ctl > 0.0 && load.atl > 0.0, "{load:?}");
    assert!(
        (last.ctl - load.ctl).abs() < 1e-9,
        "ctl {} vs {}",
        last.ctl,
        load.ctl
    );
    assert!(
        (last.atl - load.atl).abs() < 1e-9,
        "atl {} vs {}",
        last.atl,
        load.atl
    );
    assert!(
        (last.tsb - load.tsb).abs() < 1e-9,
        "tsb {} vs {}",
        last.tsb,
        load.tsb
    );
    // Five rest days pull ATL below the last training day's.
    assert!(last.atl < rows[9].atl, "{} vs {}", last.atl, rows[9].atl);
}

/// The operator's training-load selection reaches the history series.
///
/// Under a simple moving average one session's TSS counts as `tss / window`
/// on every day of the window; the EMA the platform used to hard-code would
/// weigh it `2 / (window + 1)` on its own day and decay it after.
#[test]
fn history_honours_the_configured_training_load_algorithm() {
    let mut config = algos();
    config.training_load = "sma".to_owned();
    let activities = vec![run_at(0, 3600, 165)];
    let rows = compute_training_history(
        &activities,
        athlete_with_lthr(),
        day(0),
        day(3),
        &config,
        None,
    )
    .expect("training-load series");
    let tss = rows[0].daily_load;
    assert!(tss > 0.0);
    for row in &rows {
        assert!(
            (row.ctl - tss / 42.0).abs() < 1e-9,
            "ctl on {}: {}",
            row.date,
            row.ctl
        );
        assert!(
            (row.atl - tss / 7.0).abs() < 1e-9,
            "atl on {}: {}",
            row.date,
            row.atl
        );
    }
}

// =============================================================================
// Form on a day is the day before's CTL minus ATL (Coggan / TrainingPeaks,
// carnet#601). A daily rider read at noon, before today's ride, must not be
// reported fresh by a rest day that has not happened, and the ride landing
// must move fitness and fatigue but not the day's form.
// =============================================================================

/// A ride worth exactly `tss` at 06:00 UTC on `date_idx`. The explicit score
/// is what cageux sums, so the worked example's numbers are exact.
fn ride_worth(date_idx: i64, tss: f32) -> Activity {
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 6, 0, 0).unwrap() + Duration::days(date_idx);
    ActivityBuilder::new(
        format!("ride{date_idx}"),
        format!("daily ride {date_idx}"),
        SportType::Ride,
        start,
        3_600,
        "synthetic".to_owned(),
    )
    .training_stress_score(tss)
    .build()
}

/// 100 TSS a day on days 0..=99, then today is day 100.
fn a_hundred_days_of_daily_rides() -> Vec<Activity> {
    (0..100).map(|d| ride_worth(d, 100.0)).collect()
}

/// The series with today (day 100) as its last row. The window starts early
/// enough that its 72-day warm-up reaches back to the first ride.
fn series_through_today(activities: &[Activity]) -> Vec<DailyTrainingState> {
    compute_training_history(
        activities,
        AthleteInputs::default(),
        day(72),
        day(100),
        &algos(),
        None,
    )
    .expect("training-load series")
}

#[test]
fn a_daily_rider_read_before_todays_ride_has_last_nights_form() {
    let rows = series_through_today(&a_hundred_days_of_daily_rides());
    let yesterday = &rows[rows.len() - 2];
    let today = &rows[rows.len() - 1];
    assert_eq!(today.date, day(100));

    // Form today is exactly yesterday evening's balance, read against
    // yesterday evening's CTL.
    assert!(
        (today.tsb - (yesterday.ctl - yesterday.atl)).abs() < 1e-9,
        "today's tsb {} vs yesterday's ctl {} - atl {}",
        today.tsb,
        yesterday.ctl,
        yesterday.atl
    );
    assert!((today.form_ctl - yesterday.ctl).abs() < 1e-9);

    // Reading today's own CTL and ATL, which have decayed through a rest day
    // that has not happened, reported +19.5 (+20.7% of CTL): detraining.
    let reading = today.form_reading();
    assert_eq!(reading.band, FormBand::Balanced);
    assert!(
        today.ctl - today.atl > 19.0,
        "the same-day balance this replaced"
    );
}

#[test]
fn todays_ride_moves_fitness_and_fatigue_but_not_todays_form() {
    let before = series_through_today(&a_hundred_days_of_daily_rides());
    let mut with_ride = a_hundred_days_of_daily_rides();
    with_ride.push(ride_worth(100, 100.0));
    let after = series_through_today(&with_ride);

    let noon = before.last().unwrap();
    let evening = after.last().unwrap();
    assert!(
        evening.ctl > noon.ctl && evening.atl > noon.atl,
        "the ride lands in today's CTL {} -> {} and ATL {} -> {}",
        noon.ctl,
        evening.ctl,
        noon.atl,
        evening.atl
    );
    assert!(
        (evening.tsb - noon.tsb).abs() < 1e-9,
        "noon and evening agree on today's form: {} vs {}",
        noon.tsb,
        evening.tsb
    );
    assert!((evening.form_ctl - noon.form_ctl).abs() < 1e-9);
    assert_eq!(evening.form_reading().band, noon.form_reading().band);

    // Every day before today is untouched by today's ride.
    assert_eq!(&before[..before.len() - 1], &after[..after.len() - 1]);
}

/// The worked example, EMA 42/7 from a zero seed: 100 TSS a day for 100 days.
/// Yesterday ended at CTL 99.15 / ATL 100.00, so form today is -0.85, -0.86%
/// of CTL: balanced, before and after today's ride.
#[test]
fn the_worked_example_bands_balanced_as_trainingpeaks_does() {
    let rows = series_through_today(&a_hundred_days_of_daily_rides());
    let noon = rows.last().unwrap();
    let close = |got: f64, want: f64| (got - want).abs() < 0.005;

    assert!(close(noon.ctl, 94.5344), "noon ctl {}", noon.ctl);
    assert!(close(noon.atl, 75.0), "noon atl {}", noon.atl);
    assert!(close(noon.form_ctl, 99.1458), "form ctl {}", noon.form_ctl);
    assert!(close(noon.tsb, -0.8542), "tsb {}", noon.tsb);
    let reading = noon.form_reading();
    assert!(
        reading.form_pct.is_some_and(|pct| close(pct, -0.8615)),
        "form {:?}",
        reading.form_pct
    );
    assert_eq!(reading.band, FormBand::Balanced);

    let mut with_ride = a_hundred_days_of_daily_rides();
    with_ride.push(ride_worth(100, 100.0));
    let rows = series_through_today(&with_ride);
    let evening = rows.last().unwrap();
    assert!(close(evening.ctl, 99.1856), "evening ctl {}", evening.ctl);
    assert!(close(evening.atl, 100.0), "evening atl {}", evening.atl);
    assert!(close(evening.tsb, -0.8542), "evening tsb {}", evening.tsb);
    assert_eq!(evening.form_reading().band, FormBand::Balanced);
}
