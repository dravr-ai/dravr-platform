// ABOUTME: Sleep and recovery tools for rest optimization.
// ABOUTME: Implements analyze_sleep_quality, calculate_recovery_score, suggest_rest_day, track_sleep_trends, optimize_sleep_schedule.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Sleep and Recovery Tools
//!
//! This module provides tools for sleep and recovery analysis with direct business logic:
//! - `AnalyzeSleepQualityTool` - Analyze sleep patterns and generate quality scores
//! - `CalculateRecoveryScoreTool` - Calculate holistic recovery score
//! - `SuggestRestDayTool` - AI-powered rest day recommendation
//! - `TrackSleepTrendsTool` - Track sleep trends over time
//! - `OptimizeSleepScheduleTool` - Sleep schedule recommendations
//!
//! All tools use direct `SleepAnalyzer` and `RecoveryCalculator` access.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::dispatch::dispatch_handler;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

// ============================================================================
// AnalyzeSleepQualityTool
// ============================================================================

/// Tool for analyzing sleep quality from sleep data.
pub struct AnalyzeSleepQualityTool;

#[async_trait]
impl McpTool for AnalyzeSleepQualityTool {
    fn name(&self) -> &'static str {
        "analyze_sleep_quality"
    }

    fn description(&self) -> &'static str {
        "Analyze sleep data to generate quality scores and insights"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_data".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Sleep data with fields: duration_hours, deep_sleep_hours, rem_sleep_hours, \
                     light_sleep_hours, awake_hours, efficiency_percent, hrv_rmssd_ms"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "recent_hrv_values".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Array of recent HRV values for trend analysis".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "number".to_owned(),
                    description: Some("HRV RMSSD value in milliseconds".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "baseline_hrv".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("User's baseline HRV for comparison".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["sleep_data".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "analyze_sleep_quality",
            handlers::handle_analyze_sleep_quality,
        )
        .await
    }
}

// ============================================================================
// CalculateRecoveryScoreTool
// ============================================================================

/// Tool for calculating holistic recovery score.
pub struct CalculateRecoveryScoreTool;

#[async_trait]
impl McpTool for CalculateRecoveryScoreTool {
    fn name(&self) -> &'static str {
        "calculate_recovery_score"
    }

    fn description(&self) -> &'static str {
        "Calculate holistic recovery score combining training stress, sleep, and HRV"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_data".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Sleep data for recovery calculation".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "training_load".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Training load data with ctl, atl, tsb values (optional)".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "recent_hrv_values".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Array of recent HRV values".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "number".to_owned(),
                    description: Some("HRV RMSSD value in milliseconds".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "baseline_hrv".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("User's baseline HRV".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["sleep_data".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "calculate_recovery_score",
            handlers::handle_calculate_recovery_score,
        )
        .await
    }
}

// ============================================================================
// SuggestRestDayTool
// ============================================================================

/// Tool for AI-powered rest day recommendation.
pub struct SuggestRestDayTool;

#[async_trait]
impl McpTool for SuggestRestDayTool {
    fn name(&self) -> &'static str {
        "suggest_rest_day"
    }

    fn description(&self) -> &'static str {
        "Get AI-powered recommendation on whether to rest or train"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_data".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Last night's sleep data".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "training_load".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Training load data (ctl, atl, tsb)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "recent_hrv_values".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Recent HRV values for trend analysis".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "number".to_owned(),
                    description: Some("HRV RMSSD value in milliseconds".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "baseline_hrv".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("User's baseline HRV".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["sleep_data".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "suggest_rest_day",
            handlers::handle_suggest_rest_day,
        )
        .await
    }
}

// ============================================================================
// TrackSleepTrendsTool
// ============================================================================

/// Tool for tracking sleep trends over time.
pub struct TrackSleepTrendsTool;

#[async_trait]
impl McpTool for TrackSleepTrendsTool {
    fn name(&self) -> &'static str {
        "track_sleep_trends"
    }

    fn description(&self) -> &'static str {
        "Analyze sleep patterns over time to identify trends and insights"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_history".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Array of sleep data objects (minimum 7 days)".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "object".to_owned(),
                    properties: Some(HashMap::from([
                        (
                            "date".to_owned(),
                            PropertySchema {
                                property_type: "string".to_owned(),
                                description: Some("Date of sleep record".to_owned()),
                                ..Default::default()
                            },
                        ),
                        (
                            "duration_hours".to_owned(),
                            PropertySchema {
                                property_type: "number".to_owned(),
                                description: Some("Sleep duration in hours".to_owned()),
                                ..Default::default()
                            },
                        ),
                    ])),
                    required: Some(vec!["date".to_owned(), "duration_hours".to_owned()]),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["sleep_history".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "track_sleep_trends",
            handlers::handle_track_sleep_trends,
        )
        .await
    }
}

// ============================================================================
// OptimizeSleepScheduleTool
// ============================================================================

/// Tool for recommending optimal sleep schedule.
pub struct OptimizeSleepScheduleTool;

#[async_trait]
impl McpTool for OptimizeSleepScheduleTool {
    fn name(&self) -> &'static str {
        "optimize_sleep_schedule"
    }

    fn description(&self) -> &'static str {
        "Get personalized sleep schedule recommendations based on training and recovery needs"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "training_load".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Training load data (ctl, atl, tsb)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "upcoming_workout_intensity".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("low, moderate, or high".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "typical_wake_time".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Wake time in HH:MM format (default: 06:00)".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "optimize_sleep_schedule",
            handlers::handle_optimize_sleep_schedule,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all sleep tools for registration
#[must_use]
pub fn create_sleep_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(AnalyzeSleepQualityTool),
        Box::new(CalculateRecoveryScoreTool),
        Box::new(SuggestRestDayTool),
        Box::new(TrackSleepTrendsTool),
        Box::new(OptimizeSleepScheduleTool),
    ]
}
