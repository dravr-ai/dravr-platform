// ABOUTME: Unit tests for recovery calculator module, moved from src/intelligence/recovery_calculator.rs
// ABOUTME: Tests holistic recovery scoring combining form (TSB as a share of CTL), sleep quality, and HRV
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use dravr_cageux::config::intelligence::{IntelligenceConfig, SleepRecoveryConfig};
use dravr_cageux::{
    algorithms::RecoveryAggregationAlgorithm,
    recovery_calculator::{
        DataCompleteness, RecoveryCalculator, RecoveryCategory, RecoveryComponents, RecoveryScore,
        TrainingReadiness,
    },
    sleep_analysis::{
        HrvRecoveryStatus, HrvTrend, HrvTrendAnalysis, SleepData, SleepQualityCategory,
        SleepQualityScore,
    },
    training_load::TrainingLoad,
};

/// Helper to get default test config
fn test_config() -> SleepRecoveryConfig {
    IntelligenceConfig::default().sleep_recovery
}

/// Helper to get default test algorithm (matches config weights)
fn test_algorithm() -> RecoveryAggregationAlgorithm {
    let config = test_config();
    RecoveryAggregationAlgorithm::WeightedAverage {
        tsb_weight_full: config.recovery_scoring.tsb_weight_full,
        sleep_weight_full: config.recovery_scoring.sleep_weight_full,
        hrv_weight_full: config.recovery_scoring.hrv_weight_full,
        tsb_weight_no_hrv: config.recovery_scoring.tsb_weight_no_hrv,
        sleep_weight_no_hrv: config.recovery_scoring.sleep_weight_no_hrv,
    }
}

/// Training load whose form is `pct` percent of a CTL-100 base.
fn load_at_form_pct(pct: f64) -> TrainingLoad {
    TrainingLoad {
        ctl: 100.0,
        atl: 100.0 - pct,
        tsb: pct,
        form_ctl: 100.0,
        tss_history: vec![],
    }
}

fn form_score(pct: f64) -> f64 {
    RecoveryCalculator::score_form(&load_at_form_pct(pct))
}

#[test]
fn test_form_scoring_fresh_band() {
    let score = form_score(10.0);
    assert!((99.0..=100.0).contains(&score));
}

#[test]
fn test_form_scoring_deep_fatigue() {
    let score = form_score(-40.0);
    assert!(score < 30.0);
}

#[test]
fn test_recovery_category_excellent() {
    let config = test_config();
    let category = RecoveryCalculator::categorize_recovery(90.0, &config);
    assert_eq!(category, RecoveryCategory::Excellent);
}

#[test]
fn test_recovery_category_poor() {
    let config = test_config();
    let category = RecoveryCalculator::categorize_recovery(40.0, &config);
    assert_eq!(category, RecoveryCategory::Poor);
}

// ============================================================================
// LOAD SCORING ON FORM (TSB AS A SHARE OF CTL) ACROSS ALL BANDS
// ============================================================================

#[test]
fn test_form_scoring_same_tsb_differs_by_fitness() {
    // TSB -20 is -13% of fitness at CTL 150 and -50% at CTL 40.
    let fit = RecoveryCalculator::score_form(&TrainingLoad {
        ctl: 150.0,
        atl: 170.0,
        tsb: -20.0,
        form_ctl: 150.0,
        tss_history: vec![],
    });
    let base = RecoveryCalculator::score_form(&TrainingLoad {
        ctl: 40.0,
        atl: 60.0,
        tsb: -20.0,
        form_ctl: 40.0,
        tss_history: vec![],
    });
    assert!(
        (50.0..70.0).contains(&fit),
        "productive block should score 50-70"
    );
    assert!(base < 30.0, "deep fatigue should score below 30");
}

#[test]
fn test_form_scoring_heavy_block() {
    let score = form_score(-25.0);
    assert!(
        (30.0..50.0).contains(&score),
        "Heavy block should score 30-50"
    );
}

#[test]
fn test_form_scoring_productive() {
    let score = form_score(-15.0);
    assert!(
        (50.0..70.0).contains(&score),
        "Productive block should score 50-70"
    );
}

#[test]
fn test_form_scoring_balanced() {
    let score = form_score(0.0);
    assert!(
        (70.0..100.0).contains(&score),
        "Balanced form should score 70-100"
    );
}

#[test]
fn test_form_scoring_fresh_boundaries() {
    assert!((form_score(5.0) - 100.0).abs() < 1e-9);
    assert!((form_score(20.0) - 100.0).abs() < 1e-9);
}

#[test]
fn test_form_scoring_detraining() {
    let score = form_score(30.0);
    assert!(
        score <= 90.0,
        "Very fresh (+30%) should score lower due to detraining"
    );
    assert!(form_score(200.0) >= 70.0, "Detraining floors at 70");
}

#[test]
fn test_form_scoring_without_chronic_base_is_neutral() {
    let score = RecoveryCalculator::score_form(&TrainingLoad {
        ctl: 10.0,
        atl: 14.0,
        tsb: -4.0,
        form_ctl: 10.0,
        tss_history: vec![],
    });
    assert!((score - 70.0).abs() < 1e-9);
}

// ============================================================================
// COMPREHENSIVE TESTS FOR HRV SCORING
// ============================================================================

#[test]
fn test_hrv_scoring_highly_fatigued() {
    let hrv = HrvTrendAnalysis {
        current_rmssd: 30.0,
        weekly_average_rmssd: 50.0,
        baseline_rmssd: Some(45.0),
        baseline_deviation_percent: Some(-40.0),
        trend: HrvTrend::Declining,
        recovery_status: HrvRecoveryStatus::HighlyFatigued,
        insights: vec![],
    };
    let score = RecoveryCalculator::score_hrv(&hrv);
    assert!(score < 30.0, "Highly fatigued HRV should score <30");
}

#[test]
fn test_hrv_scoring_fatigued() {
    let hrv = HrvTrendAnalysis {
        current_rmssd: 42.0,
        weekly_average_rmssd: 50.0,
        baseline_rmssd: Some(45.0),
        baseline_deviation_percent: Some(-16.0),
        trend: HrvTrend::Declining,
        recovery_status: HrvRecoveryStatus::Fatigued,
        insights: vec![],
    };
    let score = RecoveryCalculator::score_hrv(&hrv);
    assert!(
        (30.0..60.0).contains(&score),
        "Fatigued HRV should score 30-60"
    );
}

#[test]
fn test_hrv_scoring_normal() {
    let hrv = HrvTrendAnalysis {
        current_rmssd: 50.0,
        weekly_average_rmssd: 49.0,
        baseline_rmssd: Some(45.0),
        baseline_deviation_percent: Some(2.0),
        trend: HrvTrend::Stable,
        recovery_status: HrvRecoveryStatus::Normal,
        insights: vec![],
    };
    let score = RecoveryCalculator::score_hrv(&hrv);
    assert!(
        (60.0..85.0).contains(&score),
        "Normal HRV should score 60-85"
    );
}

#[test]
fn test_hrv_scoring_recovered() {
    let hrv = HrvTrendAnalysis {
        current_rmssd: 58.0,
        weekly_average_rmssd: 50.0,
        baseline_rmssd: Some(45.0),
        baseline_deviation_percent: Some(16.0),
        trend: HrvTrend::Improving,
        recovery_status: HrvRecoveryStatus::Recovered,
        insights: vec![],
    };
    let score = RecoveryCalculator::score_hrv(&hrv);
    assert!(score >= 85.0, "Recovered HRV should score >=85");
}

// ============================================================================
// COMPREHENSIVE TESTS FOR RECOVERY SCORING WITH DIFFERENT COMPONENT COMBINATIONS
// ============================================================================

#[test]
fn test_recovery_score_tsb_only() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 45.0,
        tsb: 5.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 75.0,
        duration_score: 80.0,
        stage_quality_score: 70.0,
        efficiency_score: 75.0,
        quality_category: SleepQualityCategory::Good,
        insights: vec![],
        recommendations: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        None, // No HRV
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert_eq!(recovery.components.components_available, 2);
    // Should weight TSB 50%, Sleep 50%
    assert!(recovery.overall_score > 0.0);
}

#[test]
fn test_recovery_score_all_components() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 40.0,
        tsb: 10.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 85.0,
        duration_score: 90.0,
        stage_quality_score: 85.0,
        efficiency_score: 90.0,
        quality_category: SleepQualityCategory::Excellent,
        insights: vec![],
        recommendations: vec![],
    };
    let hrv = HrvTrendAnalysis {
        current_rmssd: 55.0,
        weekly_average_rmssd: 50.0,
        baseline_rmssd: Some(45.0),
        baseline_deviation_percent: Some(10.0),
        trend: HrvTrend::Improving,
        recovery_status: HrvRecoveryStatus::Recovered,
        insights: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        Some(&hrv),
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert_eq!(recovery.components.components_available, 3);
    // Should weight TSB 40%, Sleep 40%, HRV 20%
    assert!(
        recovery.overall_score >= 85.0,
        "All excellent components should score high"
    );
}

#[test]
fn test_recovery_score_conflicting_signals() {
    let config = test_config();
    // Good TSB but poor sleep
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 40.0,
        tsb: 10.0, // Fresh
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 35.0, // Poor
        duration_score: 30.0,
        stage_quality_score: 40.0,
        efficiency_score: 35.0,
        quality_category: SleepQualityCategory::Poor,
        insights: vec![],
        recommendations: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        None,
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    // Should average out to moderate range
    assert!(
        recovery.overall_score >= 50.0 && recovery.overall_score < 75.0,
        "Conflicting signals should result in moderate score"
    );
}

#[test]
fn test_recovery_score_all_poor() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 60.0,
        atl: 80.0,
        tsb: -20.0, // Highly fatigued
        form_ctl: 60.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 30.0,
        duration_score: 25.0,
        stage_quality_score: 35.0,
        efficiency_score: 30.0,
        quality_category: SleepQualityCategory::Poor,
        insights: vec![],
        recommendations: vec![],
    };
    let hrv = HrvTrendAnalysis {
        current_rmssd: 25.0,
        weekly_average_rmssd: 50.0,
        baseline_rmssd: Some(50.0),
        baseline_deviation_percent: Some(-50.0),
        trend: HrvTrend::Declining,
        recovery_status: HrvRecoveryStatus::HighlyFatigued,
        insights: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        Some(&hrv),
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert!(
        recovery.overall_score < 35.0,
        "All poor components should score very low"
    );
    assert_eq!(recovery.recovery_category, RecoveryCategory::Poor);
}

// ============================================================================
// COMPREHENSIVE TESTS FOR RECOVERY CATEGORY BOUNDARIES
// ============================================================================

#[test]
fn test_recovery_category_all_thresholds() {
    let config = test_config();
    // Test Excellent boundary (≥85)
    assert_eq!(
        RecoveryCalculator::categorize_recovery(85.0, &config),
        RecoveryCategory::Excellent
    );
    assert_eq!(
        RecoveryCalculator::categorize_recovery(95.0, &config),
        RecoveryCategory::Excellent
    );

    // Test Good boundary (70-84)
    assert_eq!(
        RecoveryCalculator::categorize_recovery(70.0, &config),
        RecoveryCategory::Good
    );
    assert_eq!(
        RecoveryCalculator::categorize_recovery(84.9, &config),
        RecoveryCategory::Good
    );

    // Test Fair boundary (50-69)
    assert_eq!(
        RecoveryCalculator::categorize_recovery(50.0, &config),
        RecoveryCategory::Fair
    );
    assert_eq!(
        RecoveryCalculator::categorize_recovery(69.9, &config),
        RecoveryCategory::Fair
    );

    // Test Poor boundary (<50)
    assert_eq!(
        RecoveryCalculator::categorize_recovery(49.9, &config),
        RecoveryCategory::Poor
    );
    assert_eq!(
        RecoveryCalculator::categorize_recovery(20.0, &config),
        RecoveryCategory::Poor
    );
}

#[test]
fn test_recovery_category_edge_cases() {
    let config = test_config();
    assert_eq!(
        RecoveryCalculator::categorize_recovery(0.0, &config),
        RecoveryCategory::Poor
    );
    assert_eq!(
        RecoveryCalculator::categorize_recovery(100.0, &config),
        RecoveryCategory::Excellent
    );
}

// ============================================================================
// COMPREHENSIVE TESTS FOR TRAINING READINESS DETERMINATION
// ============================================================================

/// A physically consistent load at the given CTL whose TSB is `tsb`.
///
/// `determine_training_readiness` now bands form against the athlete's own CTL,
/// so these fixtures carry the chronic load the raw TSB used to imply. CTL 100
/// keeps each case in the band its name describes: +10 is fresh, -2 balanced,
/// -15 productive.
fn load_at(ctl: f64, tsb: f64) -> TrainingLoad {
    TrainingLoad {
        ctl,
        atl: ctl - tsb,
        tsb,
        form_ctl: ctl,
        tss_history: Vec::new(),
    }
}

#[test]
fn test_training_readiness_excellent_recovery() {
    let config = test_config();
    let readiness = RecoveryCalculator::determine_training_readiness(
        90.0,                  // overall_score
        &load_at(100.0, 10.0), // fresh: +10% of CTL
        SleepQualityCategory::Excellent,
        Some(HrvRecoveryStatus::Recovered),
        &config,
    );
    assert_eq!(
        readiness,
        TrainingReadiness::ReadyForHard,
        "Excellent recovery should indicate ready for hard training"
    );
}

#[test]
fn test_training_readiness_good_recovery() {
    let config = test_config();
    let readiness = RecoveryCalculator::determine_training_readiness(
        75.0,                 // overall_score
        &load_at(100.0, 5.0), // +5% of CTL
        SleepQualityCategory::Good,
        Some(HrvRecoveryStatus::Normal),
        &config,
    );
    assert_eq!(
        readiness,
        TrainingReadiness::ReadyForModerate,
        "Good recovery should indicate ready for moderate training"
    );
}

#[test]
fn test_training_readiness_fair_recovery() {
    let config = test_config();
    let readiness = RecoveryCalculator::determine_training_readiness(
        60.0,                  // overall_score
        &load_at(100.0, -2.0), // balanced: -2% of CTL
        SleepQualityCategory::Fair,
        Some(HrvRecoveryStatus::Normal),
        &config,
    );
    assert_eq!(
        readiness,
        TrainingReadiness::EasyOnly,
        "Fair recovery should indicate easy training only"
    );
}

#[test]
fn test_training_readiness_poor_recovery() {
    let config = test_config();
    let readiness = RecoveryCalculator::determine_training_readiness(
        35.0,                   // overall_score
        &load_at(100.0, -15.0), // productive: -15% of CTL
        SleepQualityCategory::Poor,
        Some(HrvRecoveryStatus::Fatigued),
        &config,
    );
    assert_eq!(
        readiness,
        TrainingReadiness::RestNeeded,
        "Poor recovery should indicate rest needed"
    );
}

#[test]
fn test_training_readiness_high_tsb_but_poor_sleep() {
    let config = test_config();
    // Edge case: Good TSB but poor sleep should limit readiness
    let readiness = RecoveryCalculator::determine_training_readiness(
        55.0,                  // overall_score - fair due to poor sleep
        &load_at(100.0, 12.0), // fresh: +12% of CTL
        SleepQualityCategory::Poor,
        None,
        &config,
    );
    assert_eq!(
        readiness,
        TrainingReadiness::RestNeeded,
        "Poor sleep should limit training readiness despite good TSB"
    );
}

// ============================================================================
// COMPREHENSIVE TESTS FOR REST DAY RECOMMENDATION
// ============================================================================

#[test]
fn test_rest_day_not_needed() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 40.0,
        tsb: 10.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let recovery_score = RecoveryScore {
        overall_score: 85.0,
        recovery_category: RecoveryCategory::Excellent,
        data_completeness: DataCompleteness::Full,
        components: RecoveryComponents {
            tsb_score: 90.0,
            sleep_score: Some(85.0),
            hrv_score: Some(80.0),
            components_available: 3,
        },
        training_readiness: TrainingReadiness::ReadyForHard,
        insights: vec![],
        recommendations: vec![],
        rest_day_recommended: false,
        reasoning: vec![],
        limitations: vec![],
    };
    let sleep_data = SleepData {
        date: Utc::now(),
        duration_hours: 8.0,
        deep_sleep_hours: Some(1.6),
        rem_sleep_hours: Some(2.0),
        light_sleep_hours: Some(4.0),
        awake_hours: Some(0.4),
        efficiency_percent: Some(90.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let recommendation = RecoveryCalculator::recommend_rest_day(
        &recovery_score,
        &sleep_data,
        &training_load,
        &config,
    )
    .unwrap();
    assert!(
        !recommendation.rest_recommended,
        "Excellent recovery should not need rest"
    );
    assert!(
        recommendation.confidence < 50.0,
        "Should have low confidence when rest not needed"
    );
}

#[test]
fn test_rest_day_strongly_recommended() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 60.0,
        atl: 80.0,
        tsb: -20.0,
        form_ctl: 60.0,
        tss_history: vec![],
    };
    let recovery_score = RecoveryScore {
        overall_score: 25.0,
        recovery_category: RecoveryCategory::Poor,
        data_completeness: DataCompleteness::Full,
        components: RecoveryComponents {
            tsb_score: 20.0,
            sleep_score: Some(30.0),
            hrv_score: Some(25.0),
            components_available: 3,
        },
        training_readiness: TrainingReadiness::RestNeeded,
        insights: vec![],
        recommendations: vec![],
        rest_day_recommended: true,
        reasoning: vec![],
        limitations: vec![],
    };
    let sleep_data = SleepData {
        date: Utc::now(),
        duration_hours: 5.5, // Short sleep
        deep_sleep_hours: Some(0.5),
        rem_sleep_hours: Some(1.0),
        light_sleep_hours: Some(3.5),
        awake_hours: Some(0.5),
        efficiency_percent: Some(70.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let recommendation = RecoveryCalculator::recommend_rest_day(
        &recovery_score,
        &sleep_data,
        &training_load,
        &config,
    )
    .unwrap();
    assert!(
        recommendation.rest_recommended,
        "Severe fatigue should strongly recommend rest"
    );
    assert!(
        recommendation.confidence >= 75.0,
        "Should have high confidence with all factors poor"
    );
    assert!(
        recommendation.estimated_recovery_hours.unwrap() >= 36.0,
        "Should estimate significant recovery time"
    );
}

#[test]
fn test_rest_day_moderate_confidence() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 55.0,
        atl: 60.0,
        tsb: -5.0,
        form_ctl: 55.0,
        tss_history: vec![],
    };
    let recovery_score = RecoveryScore {
        overall_score: 55.0,
        recovery_category: RecoveryCategory::Fair,
        data_completeness: DataCompleteness::Full,
        components: RecoveryComponents {
            tsb_score: 60.0,
            sleep_score: Some(55.0),
            hrv_score: Some(50.0),
            components_available: 3,
        },
        training_readiness: TrainingReadiness::EasyOnly,
        insights: vec![],
        recommendations: vec![],
        rest_day_recommended: false,
        reasoning: vec![],
        limitations: vec![],
    };
    let sleep_data = SleepData {
        date: Utc::now(),
        duration_hours: 7.0,
        deep_sleep_hours: Some(1.2),
        rem_sleep_hours: Some(1.6),
        light_sleep_hours: Some(4.0),
        awake_hours: Some(0.2),
        efficiency_percent: Some(85.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let recommendation = RecoveryCalculator::recommend_rest_day(
        &recovery_score,
        &sleep_data,
        &training_load,
        &config,
    )
    .unwrap();
    // Borderline case - might or might not recommend rest
    if recommendation.rest_recommended {
        assert!(
            recommendation.confidence < 75.0,
            "Borderline case should have moderate confidence"
        );
    }
}

#[test]
fn test_rest_day_reasoning_generated() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 60.0,
        atl: 75.0,
        tsb: -15.0,
        form_ctl: 60.0,
        tss_history: vec![],
    };
    let recovery_score = RecoveryScore {
        overall_score: 40.0,
        recovery_category: RecoveryCategory::Poor,
        data_completeness: DataCompleteness::Partial,
        components: RecoveryComponents {
            tsb_score: 35.0,
            sleep_score: Some(45.0),
            hrv_score: None,
            components_available: 2,
        },
        training_readiness: TrainingReadiness::RestNeeded,
        insights: vec![],
        recommendations: vec![],
        rest_day_recommended: true,
        reasoning: vec![],
        limitations: vec![],
    };
    let sleep_data = SleepData {
        date: Utc::now(),
        duration_hours: 6.5,
        deep_sleep_hours: Some(1.0),
        rem_sleep_hours: Some(1.5),
        light_sleep_hours: Some(3.5),
        awake_hours: Some(0.5),
        efficiency_percent: Some(80.0),
        hrv_rmssd_ms: None,
        resting_hr_bpm: None,
        provider_score: None,
    };

    let recommendation = RecoveryCalculator::recommend_rest_day(
        &recovery_score,
        &sleep_data,
        &training_load,
        &config,
    )
    .unwrap();
    assert!(
        !recommendation.primary_reasons.is_empty(),
        "Should provide reasoning for recommendation"
    );
}

// ============================================================================
// COMPREHENSIVE TESTS FOR EDGE CASES AND BOUNDARY CONDITIONS
// ============================================================================

#[test]
fn test_recovery_score_zero_sleep() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 45.0,
        tsb: 5.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 0.0,
        duration_score: 0.0,
        stage_quality_score: 0.0,
        efficiency_score: 0.0,
        quality_category: SleepQualityCategory::Poor,
        insights: vec![],
        recommendations: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        None,
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert!(
        recovery.overall_score <= 50.0,
        "Zero sleep should result in poor overall score"
    );
}

#[test]
fn test_recovery_components_display() {
    let components = RecoveryComponents {
        tsb_score: 85.0,
        sleep_score: Some(75.0),
        hrv_score: Some(90.0),
        components_available: 3,
    };
    // Test that components struct is properly constructed
    assert_eq!(components.components_available, 3);
    assert!(components.hrv_score.is_some());
    assert!(components.sleep_score.is_some());
}

#[test]
fn test_recovery_insights_generation() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 40.0,
        tsb: 10.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 80.0,
        duration_score: 85.0,
        stage_quality_score: 80.0,
        efficiency_score: 75.0,
        quality_category: SleepQualityCategory::Good,
        insights: vec![],
        recommendations: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        None,
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert!(
        !recovery.insights.is_empty(),
        "Should generate recovery insights"
    );
}

// ============================================================================
// TESTS FOR TSB-ONLY FALLBACK MODE
// ============================================================================

#[test]
fn test_tsb_only_recovery_score_fresh() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 40.0,
        tsb: 10.0, // Fresh TSB
        form_ctl: 50.0,
        tss_history: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config);
    assert!(result.is_ok());
    let recovery = result.unwrap();

    // Should have TSB-only data completeness
    assert_eq!(recovery.data_completeness, DataCompleteness::TsbOnly);
    assert_eq!(recovery.components.components_available, 1);
    assert!(recovery.components.sleep_score.is_none());
    assert!(recovery.components.hrv_score.is_none());

    // Fresh TSB should result in good score (75-89 range)
    assert!(
        (75.0..=100.0).contains(&recovery.overall_score),
        "TSB=10 should score 75-100, got {}",
        recovery.overall_score
    );

    // Should have limitations noted
    assert!(!recovery.limitations.is_empty());
    assert!(recovery
        .limitations
        .iter()
        .any(|l| l.contains("Sleep data unavailable")));
}

#[test]
fn test_tsb_only_recovery_score_fatigued() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 60.0,
        atl: 80.0,
        tsb: -20.0, // Highly fatigued TSB
        form_ctl: 60.0,
        tss_history: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config);
    assert!(result.is_ok());
    let recovery = result.unwrap();

    // Highly fatigued should result in poor score
    assert!(
        recovery.overall_score < 35.0,
        "TSB=-20 should score <35, got {}",
        recovery.overall_score
    );
    assert_eq!(recovery.recovery_category, RecoveryCategory::Poor);
    assert_eq!(recovery.training_readiness, TrainingReadiness::RestNeeded);
    assert!(recovery.rest_day_recommended);
}

#[test]
fn test_tsb_only_recovery_score_neutral() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 50.0,
        tsb: 0.0, // Neutral TSB
        form_ctl: 50.0,
        tss_history: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config);
    assert!(result.is_ok());
    let recovery = result.unwrap();

    // Form 0% of CTL sits in the balanced band (70-100 across -10%..+5%)
    assert!(
        (recovery.overall_score - 90.0).abs() < 1e-9,
        "form 0% should score 90, got {}",
        recovery.overall_score
    );
    // Balanced is not fresh: TSB-only mode holds hard work for the fresh band
    assert_eq!(
        recovery.training_readiness,
        TrainingReadiness::ReadyForModerate
    );
}

#[test]
fn test_tsb_only_rest_day_recommendation_rest_needed() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 60.0,
        atl: 80.0,
        tsb: -20.0,
        form_ctl: 60.0,
        tss_history: vec![],
    };

    // First get TSB-only recovery score
    let recovery_score =
        RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config).unwrap();

    // Then get TSB-only recommendation
    let recommendation =
        RecoveryCalculator::recommend_rest_day_tsb_only(&recovery_score, &training_load, &config)
            .unwrap();

    assert!(recommendation.rest_recommended);
    // Confidence should be lower than full mode (max 75%)
    assert!(
        recommendation.confidence <= 75.0,
        "TSB-only confidence should be <= 75%, got {}",
        recommendation.confidence
    );
    assert!(!recommendation.primary_reasons.is_empty());
    // Should note it's TSB-only assessment
    assert!(recommendation
        .supporting_factors
        .iter()
        .any(|f| f.contains("training load only")));
}

#[test]
fn test_tsb_only_rest_day_recommendation_no_rest_needed() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 40.0,
        tsb: 15.0, // Very fresh
        form_ctl: 50.0,
        tss_history: vec![],
    };

    let recovery_score =
        RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config).unwrap();

    let recommendation =
        RecoveryCalculator::recommend_rest_day_tsb_only(&recovery_score, &training_load, &config)
            .unwrap();

    assert!(!recommendation.rest_recommended);
}

#[test]
fn test_tsb_only_insights_generated() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 55.0,
        tsb: -5.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };

    let result = RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config);
    assert!(result.is_ok());
    let recovery = result.unwrap();

    // Should have insights about TSB-only mode
    assert!(!recovery.insights.is_empty());
    assert!(recovery
        .insights
        .iter()
        .any(|i| i.contains("TSB-only") || i.contains("partial")));
    // Should mention the TSB value somewhere in the insights
    assert!(recovery
        .insights
        .iter()
        .any(|i| i.contains("TSB") && i.contains("-5")));
}

#[test]
fn test_tsb_only_training_readiness_conservative() {
    let config = test_config();

    // TSB=5 should be ready for moderate in TSB-only mode (more conservative)
    let training_load_moderate = TrainingLoad {
        ctl: 50.0,
        atl: 45.0,
        tsb: 5.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let result =
        RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load_moderate, &config);
    let recovery = result.unwrap();
    // In TSB-only mode, tsb=5 should be at most ReadyForModerate (needs tsb >= 10 for hard)
    assert!(
        recovery.training_readiness == TrainingReadiness::ReadyForModerate
            || recovery.training_readiness == TrainingReadiness::ReadyForHard,
        "TSB=5 should be ready for moderate or hard training"
    );

    // TSB=15 with good score should be ready for hard
    let training_load_hard = TrainingLoad {
        ctl: 50.0,
        atl: 35.0,
        tsb: 15.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let result =
        RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load_hard, &config);
    let recovery = result.unwrap();
    assert_eq!(
        recovery.training_readiness,
        TrainingReadiness::ReadyForHard,
        "TSB=15 with good score should be ready for hard training"
    );
}

#[test]
fn test_data_completeness_values() {
    let config = test_config();
    let training_load = TrainingLoad {
        ctl: 50.0,
        atl: 45.0,
        tsb: 5.0,
        form_ctl: 50.0,
        tss_history: vec![],
    };
    let sleep_quality = SleepQualityScore {
        overall_score: 75.0,
        duration_score: 80.0,
        stage_quality_score: 70.0,
        efficiency_score: 75.0,
        quality_category: SleepQualityCategory::Good,
        insights: vec![],
        recommendations: vec![],
    };

    // TSB + Sleep (no HRV) should be Partial
    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        None,
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert_eq!(recovery.data_completeness, DataCompleteness::Partial);
    assert_eq!(recovery.components.components_available, 2);
    assert!(recovery.components.sleep_score.is_some());

    // TSB + Sleep + HRV should be Full
    let hrv = HrvTrendAnalysis {
        current_rmssd: 50.0,
        weekly_average_rmssd: 49.0,
        baseline_rmssd: Some(45.0),
        baseline_deviation_percent: Some(2.0),
        trend: HrvTrend::Stable,
        recovery_status: HrvRecoveryStatus::Normal,
        insights: vec![],
    };
    let result = RecoveryCalculator::calculate_recovery_score(
        &training_load,
        &sleep_quality,
        Some(&hrv),
        &config,
        &test_algorithm(),
    );
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert_eq!(recovery.data_completeness, DataCompleteness::Full);
    assert_eq!(recovery.components.components_available, 3);

    // TSB-only should be TsbOnly
    let result = RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, &config);
    assert!(result.is_ok());
    let recovery = result.unwrap();
    assert_eq!(recovery.data_completeness, DataCompleteness::TsbOnly);
    assert_eq!(recovery.components.components_available, 1);
}
