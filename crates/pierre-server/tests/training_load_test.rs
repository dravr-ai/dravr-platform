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

#[test]
fn test_calculate_tsb() {
    let ctl = 100.0;
    let atl = 80.0;
    let tsb = TrainingLoadCalculator::calculate_tsb(ctl, atl);
    assert!((tsb - 20.0).abs() < f64::EPSILON, "TSB should be 20.0");
}

#[test]
fn test_form_band_is_relative_to_ctl() {
    // Same TSB, different athletes: -25 on a CTL-100 elite is the deep end of
    // a normal block; -25 on a CTL-40 athlete is the deepest fatigue band.
    assert_eq!(FormBand::from_tsb(-25.0, 100.0), FormBand::HeavyBlock);
    assert_eq!(FormBand::from_tsb(-25.0, 40.0), FormBand::DeepFatigue);
    // Band edges on form as % of CTL
    assert_eq!(FormBand::from_tsb(-35.0, 100.0), FormBand::DeepFatigue);
    assert_eq!(FormBand::from_tsb(-15.0, 100.0), FormBand::Productive);
    assert_eq!(FormBand::from_tsb(10.0, 100.0), FormBand::Fresh);
    assert_eq!(FormBand::from_tsb(25.0, 100.0), FormBand::Detraining);
    // No chronic base: the honest answer is that form cannot be judged, not
    // a band read off the absolute number.
    assert_eq!(
        FormBand::from_tsb(-35.0, 0.0),
        FormBand::InsufficientHistory
    );
}

#[test]
fn test_recommend_recovery_days_is_relative_to_ctl() {
    // Elite (CTL 100): -25% form is a normal block, no rest prescription
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(-25.0, 100.0),
        0
    );
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(-35.0, 100.0),
        1
    );
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(-45.0, 100.0),
        2
    );
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(-55.0, 100.0),
        3
    );
    // Low chronic base (CTL 40): the same -25 TSB is -62.5% form → 3 days
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(-25.0, 40.0),
        3
    );
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(5.0, 100.0),
        0
    );
    // No chronic base: no prescription derived from an uninterpretable number
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(-35.0, 0.0),
        0
    );
}

#[test]
fn test_empty_activities() {
    let calculator = TrainingLoadCalculator::new();
    let result = calculator
        .calculate_training_load(&[], Some(250.0), None, Some(180.0), Some(60.0), Some(70.0))
        .unwrap();

    assert!(result.ctl.abs() < f64::EPSILON, "CTL should be 0.0");
    assert!(result.atl.abs() < f64::EPSILON, "ATL should be 0.0");
    assert!(result.tsb.abs() < f64::EPSILON, "TSB should be 0.0");
}

#[test]
fn test_training_load_with_power() {
    let calculator = TrainingLoadCalculator::new();
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
        tss_history: Vec::new(),
    };

    let risk = TrainingLoadCalculator::check_overtraining_risk(&high_risk);
    assert_eq!(risk.risk_level, RiskLevel::High);
    // One observation yields one factor. This used to assert `>= 2`, which the
    // old scheme satisfied by restating a single inequality: because
    // tsb == ctl - atl, "ATL 30% above CTL" and form below -30% are the same
    // condition, so severity was decided by counting it twice.
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
        tss_history: Vec::new(),
    };

    let risk = TrainingLoadCalculator::check_overtraining_risk(&low_risk);
    assert_eq!(risk.risk_level, RiskLevel::Low);
}

// =============================================================================
// Issue #1 regression: reverse-chronological input is rejected, not silently
// zeroed. cageux validates TSS ordering (EMA needs days_span >= 0) and returns
// an error instead of a misleading zero load; every production caller sorts
// oldest-first before calling.
// =============================================================================

#[test]
fn test_training_load_reverse_chronological_order_is_rejected() {
    let calculator = TrainingLoadCalculator::new();
    let now = Utc::now();

    // Newest first (like Strava returns) — unsorted, so cageux rejects it.
    let activities = vec![
        create_test_activity(now, 3600, Some(210), None),
        create_test_activity(now - Duration::days(1), 3600, Some(220), None),
        create_test_activity(now - Duration::days(2), 3600, Some(200), None),
    ];

    let result =
        calculator.calculate_training_load(&activities, Some(250.0), None, None, None, Some(70.0));

    assert!(
        result.is_err(),
        "reverse-chronological (unsorted) activities must be rejected, not silently zeroed"
    );
}

#[test]
fn test_training_load_sorted_chronological_produces_nonzero() {
    let calculator = TrainingLoadCalculator::new();
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
    let calculator = TrainingLoadCalculator::new();
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
