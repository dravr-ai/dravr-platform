// ABOUTME: Unit tests for training_load module
// ABOUTME: Tests training load calculations and TSB analysis with comprehensive coverage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, SportType};
use pierre_intelligence::{FormBand, RiskLevel, TrainingLoad, TrainingLoadCalculator};

fn create_test_activity(
    date: DateTime<Utc>,
    duration_seconds: u32,
    avg_power: Option<u32>,
    avg_hr: Option<u32>,
) -> Activity {
    use pierre_core::models::ActivityBuilder;

    let mut builder = ActivityBuilder::new(
        format!("test_{}", date.timestamp()),
        "Test Activity",
        SportType::Run,
        date,
        u64::from(duration_seconds),
        "test",
    )
    .distance_meters(10000.0);

    if let Some(power) = avg_power {
        builder = builder.average_power(power);
    }
    if let Some(hr) = avg_hr {
        builder = builder.average_heart_rate(hr);
    }

    builder.build()
}

/// A training load whose CTL did not move since yesterday, so the CTL form is
/// a share of (`form_ctl`) equals the end-of-day CTL.
fn steady(tsb: f64, form_ctl: f64) -> TrainingLoad {
    TrainingLoad {
        ctl: form_ctl,
        atl: form_ctl - tsb,
        tsb,
        form_ctl,
        tss_history: Vec::new(),
    }
}

#[test]
fn test_form_band_is_relative_to_ctl() {
    let band = |tsb, form_ctl| FormBand::from_training_load(&steady(tsb, form_ctl));
    // Same TSB, different athletes: -25 on a CTL-100 elite is the deep end of
    // a normal block; -25 on a CTL-40 athlete is the deepest fatigue band.
    assert_eq!(band(-25.0, 100.0), FormBand::HeavyBlock);
    assert_eq!(band(-25.0, 40.0), FormBand::DeepFatigue);
    // Band edges on form as % of CTL
    assert_eq!(band(-35.0, 100.0), FormBand::DeepFatigue);
    assert_eq!(band(-15.0, 100.0), FormBand::Productive);
    assert_eq!(band(10.0, 100.0), FormBand::Fresh);
    assert_eq!(band(25.0, 100.0), FormBand::Detraining);
    // No chronic base: the honest answer is that form cannot be judged, not
    // a band read off the absolute number.
    assert_eq!(band(-35.0, 0.0), FormBand::InsufficientHistory);
}

#[test]
fn test_recommend_recovery_days_is_relative_to_ctl() {
    let days =
        |tsb, form_ctl| TrainingLoadCalculator::recommend_recovery_days(&steady(tsb, form_ctl));
    // Elite (CTL 100): -25% form is a normal block, no rest prescription
    assert_eq!(days(-25.0, 100.0), 0);
    assert_eq!(days(-35.0, 100.0), 1);
    assert_eq!(days(-45.0, 100.0), 2);
    assert_eq!(days(-55.0, 100.0), 3);
    // Low chronic base (CTL 40): the same -25 TSB is -62.5% form → 3 days
    assert_eq!(days(-25.0, 40.0), 3);
    assert_eq!(days(5.0, 100.0), 0);
    // No chronic base: no prescription derived from an uninterpretable number
    assert_eq!(days(-35.0, 0.0), 0);
}

#[test]
fn test_empty_activities() {
    let calculator = TrainingLoadCalculator::new(Utc::now().date_naive());
    let result = calculator
        .calculate_training_load(&[], Some(250.0), None, Some(180.0), Some(60.0), Some(70.0))
        .unwrap();

    assert!(result.ctl.abs() < f64::EPSILON, "CTL should be 0.0");
    assert!(result.atl.abs() < f64::EPSILON, "ATL should be 0.0");
    assert!(result.tsb.abs() < f64::EPSILON, "TSB should be 0.0");
}

#[test]
fn test_training_load_with_power() {
    let calculator = TrainingLoadCalculator::new(Utc::now().date_naive());
    let now = Utc::now();

    let activities = vec![
        create_test_activity(now - Duration::days(2), 3600, Some(200), None),
        create_test_activity(now - Duration::days(1), 3600, Some(220), None),
        create_test_activity(now, 3600, Some(210), None),
    ];

    let result = calculator
        .calculate_training_load(
            &activities,
            Some(250.0), // FTP
            None,
            None,
            None,
            Some(70.0),
        )
        .unwrap();

    // Should have calculated CTL and ATL
    assert!(result.ctl > 0.0);
    assert!(result.atl > 0.0);
    assert_eq!(result.tss_history.len(), 3);
}

#[test]
fn test_overtraining_risk_detection() {
    let high_risk = TrainingLoad {
        ctl: 80.0,
        atl: 150.0, // Very high ATL
        tsb: -70.0, // Deep fatigue
        form_ctl: 80.0,
        tss_history: Vec::new(),
    };

    let risk = TrainingLoadCalculator::check_overtraining_risk(&high_risk);
    assert_eq!(risk.risk_level, RiskLevel::High);
    // One observation yields one factor. This used to assert `>= 2`, which the
    // old scheme satisfied by restating a single inequality: because
    // form is the previous day's CTL minus ATL, "ATL 30% above CTL" and form
    // below -30% are the same condition, so severity was decided by counting
    // it twice.
    assert_eq!(
        risk.risk_factors.len(),
        1,
        "one axis must yield one factor, got {:?}",
        risk.risk_factors
    );

    // Moderate is reachable again — it was unreachable for any athlete with a
    // chronic base while the count decided severity.
    let heavy_block = TrainingLoad {
        ctl: 100.0,
        atl: 125.0,
        tsb: -25.0, // form -25%: the deep end of a productive block
        form_ctl: 100.0,
        tss_history: Vec::new(),
    };
    assert_eq!(
        TrainingLoadCalculator::check_overtraining_risk(&heavy_block).risk_level,
        RiskLevel::Moderate
    );

    let low_risk = TrainingLoad {
        ctl: 90.0,
        atl: 80.0,
        tsb: 10.0,
        form_ctl: 90.0,
        tss_history: Vec::new(),
    };

    let risk = TrainingLoadCalculator::check_overtraining_risk(&low_risk);
    assert_eq!(risk.risk_level, RiskLevel::Low);
}

// =============================================================================
// cageux sums TSS per calendar day and walks every day through the as-of day,
// so input order does not change the load, and rest days after the last
// activity decay it.
// =============================================================================

#[test]
fn test_training_load_is_independent_of_input_order() {
    let calculator = TrainingLoadCalculator::new(Utc::now().date_naive());
    let now = Utc::now();

    // Newest first, the way Strava returns activities.
    let newest_first = vec![
        create_test_activity(now, 3600, Some(210), None),
        create_test_activity(now - Duration::days(1), 3600, Some(220), None),
        create_test_activity(now - Duration::days(2), 3600, Some(200), None),
    ];
    let mut oldest_first = newest_first.clone();
    oldest_first.reverse();

    let unsorted = calculator
        .calculate_training_load(&newest_first, Some(250.0), None, None, None, Some(70.0))
        .unwrap();
    let sorted = calculator
        .calculate_training_load(&oldest_first, Some(250.0), None, None, None, Some(70.0))
        .unwrap();

    assert!(unsorted.ctl > 0.0 && unsorted.atl > 0.0, "{unsorted:?}");
    assert!((unsorted.ctl - sorted.ctl).abs() < 1e-9);
    assert!((unsorted.atl - sorted.atl).abs() < 1e-9);
}

#[test]
fn test_training_load_decays_over_rest_days_to_the_as_of_day() {
    let now = Utc::now();
    let activities = vec![
        create_test_activity(now - Duration::days(2), 3600, Some(200), None),
        create_test_activity(now - Duration::days(1), 3600, Some(220), None),
        create_test_activity(now, 3600, Some(210), None),
    ];
    let today = now.date_naive();
    let load = |as_of| {
        TrainingLoadCalculator::new(as_of)
            .calculate_training_load(&activities, Some(250.0), None, None, None, Some(70.0))
            .unwrap()
    };
    let on_the_day = load(today);
    let after_rest = load(today + Duration::days(5));

    // Five empty days multiply the 7-day EMA by (6/8)^5 and the 42-day one by
    // (41/43)^5.
    let atl_decay = (6.0_f64 / 8.0).powi(5);
    let ctl_decay = (41.0_f64 / 43.0).powi(5);
    assert!(on_the_day.atl.mul_add(-atl_decay, after_rest.atl).abs() < 1e-9);
    assert!(on_the_day.ctl.mul_add(-ctl_decay, after_rest.ctl).abs() < 1e-9);
    assert!(after_rest.tsb > on_the_day.tsb);
}

#[test]
fn test_training_load_sorted_chronological_produces_nonzero() {
    let calculator = TrainingLoadCalculator::new(Utc::now().date_naive());
    let now = Utc::now();

    // Oldest first (correct order for EMA)
    let activities = vec![
        create_test_activity(now - Duration::days(2), 3600, Some(200), None),
        create_test_activity(now - Duration::days(1), 3600, Some(220), None),
        create_test_activity(now, 3600, Some(210), None),
    ];

    let result = calculator
        .calculate_training_load(&activities, Some(250.0), None, None, None, Some(70.0))
        .unwrap();

    assert!(
        result.ctl > 0.0,
        "CTL must be positive when sorted oldest-first"
    );
    assert!(
        result.atl > 0.0,
        "ATL must be positive when sorted oldest-first"
    );
}

#[test]
fn test_training_load_pace_fallback_no_physiological_params() {
    let calculator = TrainingLoadCalculator::new(Utc::now().date_naive());
    let now = Utc::now();

    // Activities with 10km distance but no power/HR — pace fallback should work
    let activities = vec![
        create_test_activity(now - Duration::days(5), 3600, None, None),
        create_test_activity(now - Duration::days(3), 3600, None, None),
        create_test_activity(now - Duration::days(1), 3600, None, None),
        create_test_activity(now, 3600, None, None),
    ];

    let result = calculator
        .calculate_training_load(&activities, None, None, None, None, None)
        .unwrap();

    assert!(
        !result.tss_history.is_empty(),
        "Pace fallback should produce TSS values"
    );
    assert!(
        result.ctl > 0.0,
        "CTL should be positive with pace-based estimation"
    );
}
