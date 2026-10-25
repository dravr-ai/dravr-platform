// ABOUTME: Configuration validation and type checking utilities
// ABOUTME: Ensures configuration values are valid and within acceptable ranges
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Configuration validation system for ensuring safe and valid parameter changes

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::catalog::{CatalogBuilder, ParameterType};
use super::runtime::ConfigValue;
use pierre_core::models::UserPhysiologicalProfile;

/// Configuration validator
#[derive(Debug)]
pub struct ConfigValidator {
    /// Safety constraints
    safety_rules: Vec<SafetyRule>,
}

/// Validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether all validations passed
    pub is_valid: bool,
    /// Validation errors
    pub errors: Vec<String>,
    /// Validation warnings
    pub warnings: Vec<String>,
    /// Impact analysis
    pub impact_analysis: Option<ImpactAnalysis>,
}

/// Impact analysis of configuration changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    /// Expected change in effort scores
    pub effort_score_change: f64,
    /// Expected change in zone boundaries
    pub zone_boundary_changes: HashMap<String, f64>,
    /// Affected analysis components
    pub affected_components: Vec<String>,
    /// Risk level (low, medium, high)
    pub risk_level: RiskLevel,
}

/// Risk level for configuration changes
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Low risk - safe to apply without concerns
    Low,
    /// Medium risk - review recommended before applying
    Medium,
    /// High risk - careful review required, may impact safety
    High,
}

/// Safety validation rule
#[derive(Debug, Clone)]
pub struct SafetyRule {
    /// Rule name
    pub name: String,
    /// Rule description
    pub description: String,
    /// Validation function
    pub validator: fn(&str, &ConfigValue, Option<&UserPhysiologicalProfile>) -> Result<(), String>,
}

impl ConfigValidator {
    /// Create a new configuration validator
    #[must_use]
    pub fn new() -> Self {
        Self {
            safety_rules: Self::build_safety_rules(),
        }
    }

    /// Validate a set of configuration changes
    #[must_use]
    pub fn validate(
        &self,
        changes: &HashMap<String, ConfigValue>,
        user_profile: Option<&UserPhysiologicalProfile>,
    ) -> ValidationResult {
        let mut result = ValidationResult::new();

        // 1. Validate individual parameters
        for (key, value) in changes {
            if let Err(error) = self.validate_parameter(key, value, user_profile) {
                result.add_error(error);
            }
        }

        // 2. Perform impact analysis
        result.impact_analysis = Some(Self::analyze_impact(changes));

        result
    }

    /// Validate a single parameter
    fn validate_parameter(
        &self,
        key: &str,
        value: &ConfigValue,
        user_profile: Option<&UserPhysiologicalProfile>,
    ) -> Result<(), String> {
        // Check if parameter exists in catalog
        let param_def = CatalogBuilder::get_parameter(key)
            .ok_or_else(|| format!("Unknown parameter: {key}"))?;

        // Validate data type
        match (&param_def.data_type, value) {
            (ParameterType::Float, ConfigValue::Float(_))
            | (ParameterType::Integer, ConfigValue::Integer(_))
            | (ParameterType::Boolean, ConfigValue::Boolean(_))
            | (ParameterType::String, ConfigValue::String(_)) => {}
            _ => return Err(format!("Type mismatch for parameter {key}")),
        }

        // Validate range if specified
        if let Some(valid_range) = &param_def.valid_range {
            match (value, valid_range) {
                (ConfigValue::Float(v), ConfigValue::FloatRange { min, max })
                    if v < min || v > max =>
                {
                    return Err(format!(
                        "Value {v} is outside valid range [{min}, {max}] for {key}"
                    ));
                }
                (ConfigValue::Integer(v), ConfigValue::IntegerRange { min, max })
                    if v < min || v > max =>
                {
                    return Err(format!(
                        "Value {v} is outside valid range [{min}, {max}] for {key}"
                    ));
                }
                _ => {}
            }
        }

        // Check VO2 max requirement
        if param_def.requires_vo2_max
            && (user_profile.is_none() || user_profile.as_ref().is_none_or(|p| p.vo2_max.is_none()))
        {
            return Err(format!("Parameter {key} requires VO2 max data"));
        }

        // Apply safety rules
        for rule in &self.safety_rules {
            if let Err(error) = (rule.validator)(key, value, user_profile) {
                return Err(format!("{}: {}", rule.name, error));
            }
        }

        Ok(())
    }

    /// Analyze impact of configuration changes
    fn analyze_impact(changes: &HashMap<String, ConfigValue>) -> ImpactAnalysis {
        let mut impact = ImpactAnalysis {
            effort_score_change: 0.0,
            zone_boundary_changes: HashMap::new(),
            affected_components: Vec::new(),
            risk_level: RiskLevel::Low,
        };

        // Analyze effort score impact
        for (key, value) in changes {
            if key.contains("distance_divisor") || key.contains("elevation_divisor") {
                if let ConfigValue::Float(new_value) = value {
                    let default_value = match key.as_str() {
                        "performance.run_distance_divisor" => 10.0,
                        "performance.bike_distance_divisor" => 40.0,
                        "performance.swim_distance_divisor" => 2.0,
                        "performance.elevation_divisor" => 100.0,
                        _ => continue,
                    };

                    // Calculate relative change (guard against zero default)
                    if default_value != 0.0 {
                        let relative_change = (new_value - default_value) / default_value;
                        // Scale for visibility
                        impact.effort_score_change =
                            relative_change.mul_add(10.0, impact.effort_score_change);
                    }
                }
            }
        }

        // Analyze zone boundary changes
        for (key, value) in changes {
            if key.contains("threshold") || key.contains("zone") {
                if let ConfigValue::Float(new_value) = value {
                    let zone_name = key.split('.').next_back().unwrap_or(key);
                    impact
                        .zone_boundary_changes
                        .insert(zone_name.to_owned(), *new_value);
                }
            }
        }

        // Identify affected components
        for key in changes.keys() {
            if key.starts_with("performance") {
                impact.affected_components.push("Effort Scoring".into());
            }
            if key.starts_with("efficiency") {
                impact
                    .affected_components
                    .push("Efficiency Calculation".into());
            }
            if key.contains("lactate") {
                impact
                    .affected_components
                    .push("Lactate Threshold Analysis".into());
            }
        }

        // Determine risk level
        impact.risk_level = if impact.effort_score_change.abs() > 20.0
            || impact.zone_boundary_changes.len() > 3
        {
            RiskLevel::High
        } else if impact.effort_score_change.abs() > 10.0 || impact.zone_boundary_changes.len() > 1
        {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        impact
    }

    /// Build safety validation rules
    fn build_safety_rules() -> Vec<SafetyRule> {
        vec![
            SafetyRule {
                name: "Intensity Limits".into(),
                description: "Prevent dangerously high intensity settings".into(),
                validator: |key, value, _profile| {
                    if key.contains("max_intensity") || key.contains("safety") {
                        if let ConfigValue::Float(intensity) = value {
                            if *intensity > 1.0 {
                                return Err("Maximum intensity cannot exceed 100%".into());
                            }
                            if *intensity < 0.3 {
                                return Err(
                                    "Maximum intensity too low for effective training".into()
                                );
                            }
                        }
                    }
                    Ok(())
                },
            },
            SafetyRule {
                name: "Divisor Sanity Check".into(),
                description: "Ensure divisors are within reasonable ranges".into(),
                validator: |key, value, _profile| {
                    if key.contains("divisor") {
                        if let ConfigValue::Float(divisor) = value {
                            if *divisor <= 0.0 {
                                return Err("Divisor must be positive".into());
                            }
                            if *divisor > 1000.0 {
                                return Err("Divisor value unreasonably high".into());
                            }
                        }
                    }
                    Ok(())
                },
            },
        ]
    }
}

impl Default for ConfigValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationResult {
    /// Create a new validation result
    #[must_use]
    pub const fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            impact_analysis: None,
        }
    }

    /// Add an error
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.is_valid = false;
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}
