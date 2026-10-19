// ABOUTME: The typed value a configuration parameter takes, shared by the validator and the tool schemas
// ABOUTME: Float, integer, boolean, string, or a bounded float or integer range
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Configuration parameter values.
//!
//! An athlete's configuration is stored as a document by the
//! `update_user_configuration` tool; [`ConfigValue`] is the shape one of its
//! parameters takes on the way in, as the validator checks it.

use serde::{Deserialize, Serialize};

/// Configuration value types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(tag = "type", content = "value")]
pub enum ConfigValue {
    /// Floating point number value
    Float(f64),
    /// Integer number value
    Integer(i64),
    /// Boolean flag value
    Boolean(bool),
    /// String value
    String(String),
    /// Floating point range with min and max bounds
    FloatRange {
        /// Minimum value
        min: f64,
        /// Maximum value
        max: f64,
    },
    /// Integer range with min and max bounds
    IntegerRange {
        /// Minimum value
        min: i64,
        /// Maximum value
        max: i64,
    },
}
