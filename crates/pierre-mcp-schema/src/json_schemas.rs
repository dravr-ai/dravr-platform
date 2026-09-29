// ABOUTME: Type-safe JSON schema definitions for API request/response parameters
// ABOUTME: Replaces dynamic serde_json::Value usage with compile-time validated structs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # JSON Schema Types
//!
//! This module provides strongly-typed definitions for JSON parameters that were
//! previously handled using dynamic `serde_json::Value` types.
//!
//! ## Design Principles
//!
//! 1. **Type Safety**: Use structs instead of dynamic `Value` for known schemas
//! 2. **Fail Fast**: Leverage serde's validation instead of manual `.as_*()` chains
//! 3. **Clear Errors**: Provide context about what failed to parse
//! 4. **Backwards Compatibility**: Support field aliases for API evolution
//!
//! ## When to Use These Types
//!
//! - Request parameters with known structure
//! - Configuration values that need validation
//! - API responses that clients depend on
//!
//! ## When NOT to Use
//!
//! - Plugin metadata (unknown schema)
//! - User-defined custom fields
//! - Pass-through JSON-RPC parameters

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use pierre_config::runtime::ConfigValue;

// ============================================================================
// Configuration Types
// ============================================================================

/// Configuration parameter value with type discrimination
///
/// This enum automatically tries each variant during deserialization,
/// allowing natural JSON values to be parsed correctly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValueInput {
    /// Floating point number
    Float(f64),
    /// Integer number
    Integer(i64),
    /// Boolean value
    Boolean(bool),
    /// String value
    String(String),
}

impl ConfigValueInput {
    /// Convert to internal `ConfigValue` type
    ///
    /// This is a helper for migrating from the old `HashMap`<String, Value> pattern
    #[must_use]
    pub fn to_config_value(self) -> ConfigValue {
        match self {
            Self::Float(v) => ConfigValue::Float(v),
            Self::Integer(v) => ConfigValue::Integer(v),
            Self::Boolean(v) => ConfigValue::Boolean(v),
            Self::String(v) => ConfigValue::String(v),
        }
    }
}

/// Request to update user configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfigurationRequest {
    /// Optional profile name to apply
    pub profile: Option<String>,

    /// Parameter overrides as key-value pairs
    #[serde(default)]
    pub parameter_overrides: HashMap<String, ConfigValueInput>,
}

// ============================================================================
// Goals Handler Parameters
// ============================================================================

/// Parameters for `analyze_goal_feasibility` tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeGoalFeasibilityParams {
    /// Type of goal (distance, duration, frequency)
    pub goal_type: String,

    /// Target value for the goal
    pub target_value: f64,

    /// Timeframe in days for goal completion
    #[serde(default)]
    pub timeframe_days: Option<u32>,
}

/// Parameters for `set_goal` tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetGoalParams {
    /// Type of goal (distance, duration, frequency)
    pub goal_type: String,

    /// Target value to achieve
    pub target_value: f64,

    /// Timeframe the target is counted over (week, month, quarter, year).
    /// A call that names none sets a monthly goal, the window `track_progress`
    /// already reads a stored goal without one as.
    #[serde(default = "default_goal_timeframe")]
    pub timeframe: String,

    /// Human-readable goal title
    #[serde(default = "default_goal_title")]
    pub title: String,

    /// The sport the goal counts, as the athlete names it (`run`, `ride`,
    /// `swim`, ...). Absent, every activity counts toward the goal.
    #[serde(default)]
    pub sport: Option<String>,
}

fn default_goal_timeframe() -> String {
    "month".to_owned()
}

fn default_goal_title() -> String {
    "Fitness Goal".to_owned()
}
