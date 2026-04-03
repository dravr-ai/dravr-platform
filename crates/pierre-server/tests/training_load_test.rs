// ABOUTME: Unit tests for training_load module
// ABOUTME: Tests training load calculations and TSB analysis with comprehensive coverage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use pierre_mcp_server::intelligence::{
    RiskLevel, TrainingLoad, TrainingLoadCalculator, TrainingStatus,
};
use pierre_mcp_server::models::{Activity, SportType};

fn create_test_activity(
    date: DateTime<Utc>,
    duration_seconds: u32,
    avg_power: Option<u32>,
    avg_hr: Option<u32>,
) -> Activity {
    use pierre_mcp_server::models::ActivityBuilder;

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
fn test_interpret_tsb() {
    assert_eq!(
        TrainingLoadCalculator::interpret_tsb(-15.0),
        TrainingStatus::Overreaching
    );
    assert_eq!(
        TrainingLoadCalculator::interpret_tsb(-5.0),
        TrainingStatus::Productive
    );
    assert_eq!(
        TrainingLoadCalculator::interpret_tsb(5.0),
        TrainingStatus::Fresh
    );
    assert_eq!(
        TrainingLoadCalculator::interpret_tsb(15.0),
        TrainingStatus::Detraining
    );
}

#[test]
fn test_recommend_recovery_days() {
    assert_eq!(TrainingLoadCalculator::recommend_recovery_days(-25.0), 5);
    assert_eq!(TrainingLoadCalculator::recommend_recovery_days(-18.0), 3);
    assert_eq!(TrainingLoadCalculator::recommend_recovery_days(-12.0), 2);
    assert_eq!(TrainingLoadCalculator::recommend_recovery_days(-5.0), 1);
    assert_eq!(TrainingLoadCalculator::recommend_recovery_days(5.0), 0);
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
    assert!(risk.risk_factors.len() >= 2);

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
// Issue #1 regression: CTL/ATL/TSB = 0 when activities are reverse-chronological
// =============================================================================

#[test]
fn test_training_load_reverse_chronological_order_produces_zero() {
    let calculator = TrainingLoadCalculator::new();
    let now = Utc::now();

    // Newest first (like Strava returns) — EMA returns 0 because days_span < 0
    let activities = vec![
        create_test_activity(now, 3600, Some(210), None),
        create_test_activity(now - Duration::days(1), 3600, Some(220), None),
        create_test_activity(now - Duration::days(2), 3600, Some(200), None),
    ];

    let result = calculator
        .calculate_training_load(&activities, Some(250.0), None, None, None, Some(70.0))
        .unwrap();

    assert!(
        result.ctl.abs() < f64::EPSILON,
        "CTL should be 0 when activities are newest-first (unsorted)"
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
