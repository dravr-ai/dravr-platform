// ABOUTME: Unit tests for configuration catalog functionality
// ABOUTME: Validates configuration catalog behavior, edge cases, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_config::catalog::{CatalogBuilder, ParameterType};
use pierre_config::runtime::ConfigValue;

#[test]
fn test_catalog_build() {
    let catalog = CatalogBuilder::build();

    assert!(!catalog.categories.is_empty());
    assert!(catalog.total_parameters > 0);
    assert_eq!(catalog.version, "1.0.0");
}

#[test]
fn test_parameter_lookup() {
    let param = CatalogBuilder::get_parameter("lactate.threshold_percentage");
    assert!(param.is_some());

    let param = param.unwrap();
    assert_eq!(param.key, "lactate.threshold_percentage");
    assert!(matches!(param.default_value, ConfigValue::Float(85.0)));

    // The heart-rate zone settings no computation read are gone: zones come
    // from cageux's one zone model, never from a catalogue percentage.
    assert!(CatalogBuilder::get_parameter("heart_rate.anaerobic_threshold").is_none());
    assert!(CatalogBuilder::get_parameter("hr_zones.elite_zone6_threshold").is_none());
}

#[test]
fn test_module_parameters() {
    let params = CatalogBuilder::get_module_parameters("lactate");
    assert!(!params.is_empty());
    assert!(params.iter().all(|p| p.key.starts_with("lactate.")));
    assert!(CatalogBuilder::get_module_parameters("heart_rate").is_empty());
    assert!(CatalogBuilder::get_module_parameters("heart_rate_zones").is_empty());
}

#[test]
fn test_valid_ranges() {
    let catalog = CatalogBuilder::build();

    for category in &catalog.categories {
        for module in &category.modules {
            for param in &module.parameters {
                if let Some(range) = &param.valid_range {
                    match (&param.data_type, range) {
                        (ParameterType::Float, ConfigValue::FloatRange { min, max }) => {
                            assert!(min < max, "Invalid range for {}", param.key);
                        }
                        (ParameterType::Integer, ConfigValue::IntegerRange { min, max }) => {
                            assert!(min < max, "Invalid range for {}", param.key);
                        }
                        _ => panic!("Mismatched range type for {}", param.key),
                    }
                }
            }
        }
    }
}
