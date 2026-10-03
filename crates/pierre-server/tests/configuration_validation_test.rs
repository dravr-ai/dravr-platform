// ABOUTME: Integration tests for configuration validation functionality
// ABOUTME: Tests configuration parameter validation, safety rules, and impact analysis
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_config::runtime::ConfigValue;
use pierre_config::validation::ConfigValidator;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{SportType, UserPhysiologicalProfile};
use std::collections::HashMap;
use uuid::Uuid;

#[test]
fn test_validator_creation() {
    let validator = ConfigValidator::new();
    // Test that the validator can be created successfully
    // Private fields cannot be accessed directly, but we can test functionality
    let mut changes = HashMap::new();
    changes.insert("test_param".into(), ConfigValue::Float(50.0));

    // The validator should handle unknown parameters gracefully
    let result = validator.validate(&changes, None);
    // We expect this to fail because test_param is not in the catalog
    assert!(!result.is_valid);
}

#[test]
fn test_valid_parameter() {
    let validator = ConfigValidator::new();
    let mut changes = HashMap::new();
    changes.insert(
        "heart_rate.anaerobic_threshold".into(),
        ConfigValue::Float(85.0),
    );

    let result = validator.validate(&changes, None);
    assert!(result.is_valid);
    assert!(result.errors.is_empty());
}

#[test]
fn test_invalid_parameter_range() {
    let validator = ConfigValidator::new();
    let mut changes = HashMap::new();
    changes.insert(
        "heart_rate.anaerobic_threshold".into(),
        ConfigValue::Float(150.0), // Above valid range
    );

    let result = validator.validate(&changes, None);
    assert!(!result.is_valid);
    assert!(!result.errors.is_empty());
}

#[test]
fn test_zone_order_validation() {
    let validator = ConfigValidator::new();
    let mut changes = HashMap::new();
    changes.insert("heart_rate.recovery_zone".into(), ConfigValue::Float(80.0));
    changes.insert(
        "heart_rate.endurance_zone".into(),
        ConfigValue::Float(70.0), // Lower than recovery zone
    );

    let result = validator.validate(&changes, None);
    assert!(!result.is_valid);
    assert!(result
        .errors
        .iter()
        .any(|e| e.contains("higher than previous zone")));
}

#[test]
fn test_impact_analysis() {
    let validator = ConfigValidator::new();
    let mut changes = HashMap::new();
    changes.insert(
        "performance.run_distance_divisor".into(),
        ConfigValue::Float(20.0), // Double the default
    );

    let result = validator.validate(&changes, None);
    assert!(result.impact_analysis.is_some());

    let impact = result.impact_analysis.unwrap();
    assert!(impact.effort_score_change != 0.0);
    assert!(impact
        .affected_components
        .contains(&"Effort Scoring".into()));
}

#[test]
fn test_vo2_max_requirement() {
    let validator = ConfigValidator::new();
    let mut changes = HashMap::new();
    changes.insert(
        "lactate.threshold_percentage".into(),
        ConfigValue::Float(85.0),
    );

    // Without VO2 max profile
    let result = validator.validate(&changes, None);
    assert!(!result.is_valid);
    assert!(result.errors.iter().any(|e| e.contains("requires VO2 max")));

    // With VO2 max profile
    let profile = UserPhysiologicalProfile {
        user_id: Uuid::new_v4(),
        vo2_max: Some(50.0),
        resting_hr: Some(60),
        max_hr: Some(180),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(30),
        weight: Some(70.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Run,
        training_experience_years: Some(5),
        ftp_watts: None,
        threshold_pace_sec_per_km: None,
        hr_zones: None,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
    };

    let result = validator.validate(&changes, Some(&profile));
    assert!(result.is_valid);
}

/// The lactate threshold is a percentage of `VO2max` and the anaerobic
/// threshold a percentage of max HR; the consistency rule reads the first on
/// the second's scale (Swain et al. 1994) before comparing them.
#[test]
fn lactate_and_anaerobic_thresholds_are_compared_on_the_max_hr_scale() {
    let validator = ConfigValidator::new();
    let mut profile = UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Run);
    profile.vo2_max = Some(55.0);
    let pair = |lactate: f64, anaerobic: f64| {
        let mut changes = HashMap::new();
        changes.insert(
            "lactate.threshold_percentage".into(),
            ConfigValue::Float(lactate),
        );
        changes.insert(
            "heart_rate.anaerobic_threshold".into(),
            ConfigValue::Float(anaerobic),
        );
        validator.validate(&changes, Some(&profile))
    };

    // 70% of VO2max is about 82% of max HR: consistent with 88% of max HR,
    // though the raw numbers sit 18 points apart.
    let consistent = pair(70.0, 88.0);
    assert!(consistent.is_valid, "{:?}", consistent.errors);

    // 85% of VO2max is about 92% of max HR, 17 points above 75% of max HR,
    // though the raw numbers sit only 10 apart.
    let inconsistent = pair(85.0, 75.0);
    assert!(!inconsistent.is_valid);
    assert!(
        inconsistent.errors.iter().any(|e| e.contains(
            "Lactate threshold (85% of VO2max, about 92% of max HR) and HR anaerobic threshold (75% of max HR)"
        )),
        "{:?}",
        inconsistent.errors
    );
}
