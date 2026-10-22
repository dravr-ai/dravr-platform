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
use pierre_core::models::{SportType, ThresholdProvenance, UserPhysiologicalProfile};
use pierre_core::transport::TransportPolicy;
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
        "fitness.vo2_max_threshold_male_recreational".into(),
        ConfigValue::Float(45.0),
    );

    let result = validator.validate(&changes, None);
    assert!(result.is_valid, "{:?}", result.errors);
    assert_eq!(result.errors, [] as [String; 0]);
}

#[test]
fn test_invalid_parameter_range() {
    let validator = ConfigValidator::new();
    let mut changes = HashMap::new();
    changes.insert(
        "lactate.threshold_percentage".into(),
        ConfigValue::Float(150.0), // Above valid range
    );

    let result = validator.validate(&changes, None);
    assert!(!result.is_valid);
    assert_ne!(result.errors, [] as [String; 0]);
}

#[test]
fn the_retired_heart_rate_zone_settings_are_unknown_parameters() {
    // Heart-rate zones are cut by cageux's one zone model from the athlete's
    // own heart rates; the percentage settings no computation read are gone,
    // so a change naming one is refused rather than stored and ignored.
    let validator = ConfigValidator::new();
    for key in [
        "heart_rate.recovery_zone",
        "heart_rate.anaerobic_threshold",
        "hr_zones.elite_zone6_threshold",
    ] {
        let mut changes = HashMap::new();
        changes.insert(key.into(), ConfigValue::Float(80.0));
        let result = validator.validate(&changes, None);
        assert!(!result.is_valid, "{key} was accepted");
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Unknown parameter")),
            "{key}: {:?}",
            result.errors
        );
    }
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
        threshold_provenance: ThresholdProvenance::default(),
        transport_policy: TransportPolicy::AnyTransport,
        sex: None,
    };

    let result = validator.validate(&changes, Some(&profile));
    assert!(result.is_valid);
}
