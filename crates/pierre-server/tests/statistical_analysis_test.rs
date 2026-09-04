// ABOUTME: Unit tests for statistical analysis functionality
// ABOUTME: Validates statistical analysis behavior, edge cases, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Statistical analysis module tests

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::cast_possible_wrap)] // Test helper uses small indices

use chrono::Utc;
use pierre_intelligence::{StatisticalAnalyzer, TrendDataPoint, TrendDirection};

/// Slope threshold the `analyze_performance_trends` tool passes to `determine_trend_direction`.
const SLOPE_THRESHOLD: f64 = 0.01;

fn create_test_data_points(values: Vec<f64>) -> Vec<TrendDataPoint> {
    values
        .into_iter()
        .enumerate()
        .map(|(i, value)| TrendDataPoint {
            date: Utc::now() + chrono::Duration::days(i as i64),
            value,
            smoothed_value: None,
        })
        .collect()
}

#[test]
fn test_linear_regression_perfect_positive_correlation() {
    let data_points = create_test_data_points(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let result = StatisticalAnalyzer::linear_regression(&data_points).unwrap();

    assert!((result.slope - 1.0).abs() < 0.001);
    assert!((result.correlation - 1.0).abs() < 0.001);
    assert!((result.r_squared - 1.0).abs() < 0.001);
}

#[test]
fn test_linear_regression_perfect_negative_correlation() {
    let data_points = create_test_data_points(vec![5.0, 4.0, 3.0, 2.0, 1.0]);
    let result = StatisticalAnalyzer::linear_regression(&data_points).unwrap();

    assert!((result.slope - (-1.0)).abs() < 0.001);
    assert!((result.correlation - (-1.0)).abs() < 0.001);
    assert!((result.r_squared - 1.0).abs() < 0.001);
}

#[test]
fn test_trend_strength_calculation() {
    let data_points = create_test_data_points(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let trend_strength = StatisticalAnalyzer::calculate_trend_strength(&data_points).unwrap();

    assert!((trend_strength - 1.0).abs() < 0.001);
}

#[test]
fn test_insufficient_data_points() {
    let data_points = create_test_data_points(vec![1.0]);
    let result = StatisticalAnalyzer::linear_regression(&data_points);

    assert!(result.is_err());
}

#[test]
fn test_perfect_fit_has_no_p_value_and_reads_stable() {
    // SS_res = 0 leaves the residual variance undefined: no p-value, so the
    // direction gate cannot pass even though the slope is unmistakable.
    let data_points = create_test_data_points(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let result = StatisticalAnalyzer::linear_regression(&data_points).unwrap();

    assert_eq!(result.degrees_of_freedom, 3);
    assert!(result.p_value.is_none());
    assert_eq!(
        StatisticalAnalyzer::determine_trend_direction(&result, false, SLOPE_THRESHOLD),
        TrendDirection::Stable
    );
}

#[test]
fn test_slope_significance_is_students_t_not_a_normal_approximation() {
    // Seven weekly blocks, slope 2.5, t = 2.549 on ν = 5 against the critical
    // 2.571: the Student's t two-tailed p is 0.0514, so the block stays stable.
    // A normal read of the same statistic gives 0.011 (0.027 with the
    // z-equivalent correction) and would call it improving. Pins the
    // dravr-cageux fix for carnet#16 through the platform's re-export.
    let data_points = create_test_data_points(vec![42.0, 44.0, 40.0, 48.0, 60.0, 54.0, 52.0]);
    let result = StatisticalAnalyzer::linear_regression(&data_points).unwrap();

    assert_eq!(result.degrees_of_freedom, 5);
    assert!((result.slope - 2.5).abs() < 1e-9, "slope={}", result.slope);
    let p = result.p_value.unwrap();
    assert!((p - 0.051_36).abs() < 1e-4, "p={p}");
    assert!(p > 0.05);
    assert_eq!(
        StatisticalAnalyzer::determine_trend_direction(&result, false, SLOPE_THRESHOLD),
        TrendDirection::Stable
    );

    // The critical value itself lands on the table entry.
    let at_critical = StatisticalAnalyzer::student_t_two_tailed_p_value(2.571, 5);
    assert!((at_critical - 0.05).abs() < 5e-4, "p={at_critical}");
}
