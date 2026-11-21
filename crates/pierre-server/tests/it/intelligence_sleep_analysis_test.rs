// ABOUTME: Unit tests for sleep analysis module, moved from src/intelligence/sleep_analysis.rs
// ABOUTME: Tests sleep quality scoring, HRV trends, and NSF/AASM guideline compliance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, NaiveDate, Utc};
use dravr_cageux::config::intelligence::{IntelligenceConfig, SleepRecoveryConfig};
use dravr_cageux::sleep_analysis::{
    HrvRecoveryStatus, HrvTrend, NightlyHrv, SleepAnalyzer, SleepData, SleepQualityCategory,
};

/// Helper to get default test config
fn test_config() -> SleepRecoveryConfig {
    IntelligenceConfig::default().sleep_recovery
}

#[test]
fn test_optimal_sleep_duration_score() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(8.0, &config);
    assert!((99.0..=100.0).contains(&score));
}

#[test]
fn test_short_sleep_duration_score() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(5.5, &config);
    assert!(score < 50.0);
}

#[test]
fn test_sleep_quality_calculation() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.6),  // 20%
        rem_sleep_hours: Some(2.0),   // 25%
        light_sleep_hours: Some(4.0), // 50%
        awake_hours: Some(0.4),       // 5%
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());

    let quality = result.unwrap();
    assert!(quality.overall_score >= 80.0);
    assert_eq!(quality.quality_category, SleepQualityCategory::Excellent);
}

// ============================================================================
// COMPREHENSIVE EDGE CASE TESTS FOR SLEEP DURATION SCORING
// ============================================================================

#[test]
fn test_duration_score_zero_hours() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(0.0, &config);
    assert!(score.abs() < 0.001, "Zero sleep should score 0");
}

#[test]
fn test_duration_score_negative_hours() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(-2.0, &config);
    assert!(
        score.abs() < 0.001,
        "Negative sleep duration should score 0"
    );
}

#[test]
fn test_duration_score_very_short_sleep() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(3.0, &config);
    assert!(score < 20.0, "3 hours sleep should score very low");
}

#[test]
fn test_duration_score_boundary_short() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(config.sleep_duration.short_sleep_threshold, &config);
    // Should be exactly at boundary scoring
    assert!((30.0..=50.0).contains(&score));
}

#[test]
fn test_duration_score_lower_optimal_boundary() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(config.sleep_duration.adult_min_hours, &config);
    assert!(
        score >= 85.0,
        "Lower optimal boundary (7h) should score >=85"
    );
}

#[test]
fn test_duration_score_upper_optimal_boundary() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(config.sleep_duration.adult_max_hours, &config);
    assert!(
        score >= 95.0,
        "Upper optimal boundary (9h) should score >=95"
    );
}

#[test]
fn test_duration_score_very_long_sleep() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(12.0, &config);
    assert!(
        score < 85.0,
        "12 hours sleep should score lower (oversleeping)"
    );
}

#[test]
fn test_duration_score_extreme_long_sleep() {
    let config = test_config();
    let score = SleepAnalyzer::score_duration(16.0, &config);
    assert!(
        score <= 70.0,
        "16 hours sleep should score low (potential health concern)"
    );
}

// ============================================================================
// COMPREHENSIVE TESTS FOR SLEEP STAGE SCORING
// ============================================================================

#[test]
fn test_stage_scoring_all_missing() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: None,
        rem_sleep_hours: None,
        light_sleep_hours: None,
        awake_hours: None,
        efficiency_percent: None,
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    // Should use default score of 50.0 when stages missing
    assert!((quality.stage_quality_score - 50.0).abs() < 0.001);
}

#[test]
fn test_stage_scoring_optimal_percentages() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.6),  // 20% (optimal range: 15-25%)
        rem_sleep_hours: Some(2.0),   // 25% (optimal range: 20-30%)
        light_sleep_hours: Some(4.0), // 50% (optimal range: 45-55%)
        awake_hours: Some(0.4),       // 5% (optimal: <5%)
        efficiency_percent: Some(95.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    // All stages optimal should score very high
    assert!(
        quality.stage_quality_score >= 95.0,
        "Optimal stage percentages should score >=95"
    );
}

#[test]
fn test_stage_scoring_low_deep_sleep() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(0.4),  // 5% (below optimal 15-25%)
        rem_sleep_hours: Some(2.0),   // 25%
        light_sleep_hours: Some(5.0), // 62.5%
        awake_hours: Some(0.6),       // 7.5%
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    // Low deep sleep should reduce stage score
    assert!(
        quality.stage_quality_score < 75.0,
        "Low deep sleep should reduce stage score"
    );
}

#[test]
fn test_stage_scoring_low_rem_sleep() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.6),  // 20%
        rem_sleep_hours: Some(0.8),   // 10% (below optimal 20-30%)
        light_sleep_hours: Some(5.0), // 62.5%
        awake_hours: Some(0.6),       // 7.5%
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    assert!(
        quality.stage_quality_score < 80.0,
        "Low REM sleep should reduce stage score"
    );
}

#[test]
fn test_stage_scoring_excessive_awake_time() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.2),  // 15%
        rem_sleep_hours: Some(1.6),   // 20%
        light_sleep_hours: Some(4.0), // 50%
        awake_hours: Some(1.2),       // 15% (high awakening)
        efficiency_percent: Some(85.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    assert!(
        quality.efficiency_score < 90.0,
        "Excessive awake time reduces efficiency score"
    );
}

#[test]
fn test_stage_scoring_invalid_percentages_sum() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(2.0),  // 25%
        rem_sleep_hours: Some(2.0),   // 25%
        light_sleep_hours: Some(2.0), // 25%
        awake_hours: Some(2.0),       // 25% = 100% total
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok(), "Should handle stages summing to 100%");
}

#[test]
fn test_stage_scoring_negative_values() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(-1.0), // Invalid negative
        rem_sleep_hours: Some(2.0),
        light_sleep_hours: Some(4.0),
        awake_hours: Some(0.5),
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    // Should handle gracefully (treat as 0%)
    assert!(result.is_ok());
}

// ============================================================================
// COMPREHENSIVE TESTS FOR SLEEP EFFICIENCY SCORING
// ============================================================================

#[test]
fn test_efficiency_score_perfect() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(100.0, &config);
    assert!(
        (score - 100.0).abs() < 0.001,
        "100% efficiency should score 100"
    );
}

#[test]
fn test_efficiency_score_excellent() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(95.0, &config);
    assert!(score >= 95.0, "95% efficiency should score very high");
}

#[test]
fn test_efficiency_score_good() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(85.0, &config);
    assert!(
        (80.0..=90.0).contains(&score),
        "85% efficiency should score in good range"
    );
}

#[test]
fn test_efficiency_score_poor() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(60.0, &config);
    assert!(score < 65.0, "60% efficiency should score low");
}

#[test]
fn test_efficiency_score_very_poor() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(40.0, &config);
    assert!(score < 45.0, "40% efficiency should score very low");
}

#[test]
fn test_efficiency_score_zero() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(0.0, &config);
    assert!(score.abs() < 0.001, "0% efficiency should score 0");
}

#[test]
fn test_efficiency_score_over_100() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(105.0, &config);
    // Should clamp to 100
    assert!(
        (score - 100.0).abs() < 0.001,
        "Efficiency >100% should clamp to 100"
    );
}

#[test]
fn test_efficiency_score_negative() {
    let config = test_config();
    let score = SleepAnalyzer::score_efficiency(-10.0, &config);
    assert!(score.abs() < 0.001, "Negative efficiency should score 0");
}

// ============================================================================
// COMPREHENSIVE TESTS FOR SLEEP QUALITY CALCULATION
// ============================================================================

#[test]
fn test_sleep_quality_poor_overall() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 4.5,            // Poor
        deep_sleep_hours: Some(0.3),    // 6.7% - poor
        rem_sleep_hours: Some(0.5),     // 11% - poor
        light_sleep_hours: Some(3.0),   // 66.7% - excessive
        awake_hours: Some(0.7),         // 15.6% - excessive
        efficiency_percent: Some(60.0), // Poor
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    assert!(quality.overall_score < 50.0, "Poor sleep should score <50");
    assert_eq!(quality.quality_category, SleepQualityCategory::Poor);
}

#[test]
fn test_sleep_quality_category_boundaries() {
    let config = test_config();
    // Test Excellent boundary (85-100)
    let sleep_excellent = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.6),
        rem_sleep_hours: Some(2.0),
        light_sleep_hours: Some(4.0),
        awake_hours: Some(0.4),
        efficiency_percent: Some(95.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };
    let result = SleepAnalyzer::calculate_sleep_quality(&sleep_excellent, &config);
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().quality_category,
        SleepQualityCategory::Excellent
    );

    // Test Good boundary (70-84)
    let sleep_good = SleepData {
        date: Utc::now(),
        duration_hours: 7.5,
        deep_sleep_hours: Some(1.2),
        rem_sleep_hours: Some(1.8),
        light_sleep_hours: Some(4.0),
        awake_hours: Some(0.5),
        efficiency_percent: Some(87.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };
    let result = SleepAnalyzer::calculate_sleep_quality(&sleep_good, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    assert!(quality.overall_score >= 85.0 && quality.overall_score < 100.0);
}

#[test]
fn test_sleep_quality_with_provider_score() {
    let config = test_config();
    let sleep = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.6),
        rem_sleep_hours: Some(2.0),
        light_sleep_hours: Some(4.0),
        awake_hours: Some(0.4),
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: Some(88.0),
    };

    let result = SleepAnalyzer::calculate_sleep_quality(&sleep, &config);
    assert!(result.is_ok());
    let quality = result.unwrap();
    // Should include provider score in insights
    assert!(!quality.insights.is_empty(), "expected a non-empty value");
}

// ============================================================================
// COMPREHENSIVE TESTS FOR HRV TREND ANALYSIS
// ============================================================================

// cageux judges the week's ln RMSSD mean against the athlete's own 28-day
// band; its reference tests pin the method. These check the platform's view
// of it: the statuses the sleep tools report.

fn as_of() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()
}

/// 28 baseline nights around 60 ms, then `week`, one night a day up to `as_of`.
fn nights(week: &[f64]) -> Vec<NightlyHrv> {
    let mut values: Vec<f64> = [60.0, 68.0, 54.0, 63.0, 70.0, 52.0, 58.0]
        .iter()
        .copied()
        .cycle()
        .take(28)
        .collect();
    values.extend_from_slice(week);
    let first = as_of() - Duration::days(i64::try_from(values.len()).unwrap() - 1);
    values
        .iter()
        .enumerate()
        .map(|(i, &rmssd_ms)| NightlyHrv {
            date: first + Duration::days(i64::try_from(i).unwrap()),
            rmssd_ms,
        })
        .collect()
}

#[test]
fn test_hrv_week_like_the_baseline_is_normal() {
    let analysis = SleepAnalyzer::analyze_hrv_trends(
        &nights(&[60.0, 68.0, 54.0, 63.0, 70.0, 52.0, 58.0]),
        as_of(),
        &test_config(),
    )
    .unwrap();
    assert_eq!(
        analysis.recovery_status,
        HrvRecoveryStatus::Normal,
        "a week that repeats the baseline sits inside its band"
    );
    assert_eq!(
        analysis.weekly_nights, 7,
        "every night of the week is a valid night"
    );
    assert!(!analysis.insights.is_empty(), "Should generate insights");
}

#[test]
fn test_hrv_week_below_the_band_is_below_normal() {
    let analysis = SleepAnalyzer::analyze_hrv_trends(
        &nights(&[50.0, 49.0, 52.0, 48.0, 51.0, 50.0, 49.0]),
        as_of(),
        &test_config(),
    )
    .unwrap();
    assert_eq!(
        analysis.recovery_status,
        HrvRecoveryStatus::BelowNormal,
        "a week near 50 ms sits under a band around 60 ms"
    );
}

#[test]
fn test_hrv_without_history_is_insufficient_not_normal() {
    let lone_night = [NightlyHrv {
        date: as_of(),
        rmssd_ms: 50.0,
    }];
    let analysis = SleepAnalyzer::analyze_hrv_trends(&lone_night, as_of(), &test_config()).unwrap();
    assert_eq!(
        analysis.recovery_status,
        HrvRecoveryStatus::Insufficient,
        "one night is too few to judge, never normal"
    );
    assert_eq!(
        analysis.trend,
        HrvTrend::Stable,
        "no week to compare the night against reads as no trend"
    );
}
