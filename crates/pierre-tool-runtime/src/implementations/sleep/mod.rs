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

pub(crate) mod inner;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::capabilities::{ToolCapabilities, PROVIDER_READ};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::implementations::handler_bridge;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

// ============================================================================
// AnalyzeSleepQualityTool
// ============================================================================

/// Tool for analyzing sleep quality from sleep data.
pub struct AnalyzeSleepQualityTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeSleepQualityTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to fetch last night's sleep from (whoop, fitbit, garmin, terra). \
                     Omit to auto-select the best connected provider"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sleep_data".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Manual sleep data (used instead of a provider fetch) with fields: \
                     duration_hours, deep_sleep_hours, rem_sleep_hours, \
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
        let schema = object_schema(properties, None);
        tool_definition(
            "analyze_sleep_quality",
            "Analyze last night's sleep to generate quality scores and insights. \
             Fetches from a connected provider (WHOOP, Fitbit, Garmin, Terra) automatically",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(ctx.resources.clone());
            let request =
                handler_bridge::build_universal_request(&ctx, args, "analyze_sleep_quality");
            handler_bridge::map_universal_response(
                "analyze_sleep_quality",
                inner::handle_analyze_sleep_quality(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CalculateRecoveryScoreTool
// ============================================================================

/// Tool for calculating holistic recovery score.
pub struct CalculateRecoveryScoreTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CalculateRecoveryScoreTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to fetch sleep from (whoop, fitbit, garmin, terra). \
                     Omit to auto-select the best connected provider"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to fetch activities for training load (strava, garmin, fitbit, \
                     whoop, terra). Omit to auto-select"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sleep_data".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Manual sleep data for recovery calculation (used instead of a provider fetch)"
                        .to_owned(),
                ),
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
        let schema = object_schema(properties, None);
        tool_definition(
            "calculate_recovery_score",
            "Calculate holistic recovery score combining training stress, sleep, and HRV. \
             Fetches sleep and activity data from connected providers automatically",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(ctx.resources.clone());
            let request =
                handler_bridge::build_universal_request(&ctx, args, "calculate_recovery_score");
            handler_bridge::map_universal_response(
                "calculate_recovery_score",
                inner::handle_calculate_recovery_score(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SuggestRestDayTool
// ============================================================================

/// Tool for AI-powered rest day recommendation.
pub struct SuggestRestDayTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SuggestRestDayTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to fetch last night's sleep from (whoop, fitbit, garmin, terra). \
                     Omit to auto-select the best connected provider"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to fetch activities for training load (strava, garmin, fitbit, \
                     whoop, terra). Omit to auto-select"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sleep_data".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Manual sleep data (used instead of a provider fetch)".to_owned(),
                ),
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
        let schema = object_schema(properties, None);
        tool_definition(
            "suggest_rest_day",
            "Get AI-powered recommendation on whether to rest or train. \
             Fetches sleep and activity data from connected providers automatically",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(ctx.resources.clone());
            let request = handler_bridge::build_universal_request(&ctx, args, "suggest_rest_day");
            handler_bridge::map_universal_response(
                "suggest_rest_day",
                inner::handle_suggest_rest_day(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// TrackSleepTrendsTool
// ============================================================================

/// Tool for tracking sleep trends over time.
pub struct TrackSleepTrendsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for TrackSleepTrendsTool {
    fn definition(&self) -> Tool {
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
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to fetch sleep history from (whoop, fitbit, garmin, terra). \
                     Omit to auto-select the best connected provider"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "days".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Days of sleep history to fetch from the provider (default 7)".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        tool_definition(
            "track_sleep_trends",
            "Analyze sleep patterns over time to identify trends and insights. \
             Fetches history from a connected provider (WHOOP, Fitbit, Garmin, Terra) automatically",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(ctx.resources.clone());
            let request = handler_bridge::build_universal_request(&ctx, args, "track_sleep_trends");
            handler_bridge::map_universal_response(
                "track_sleep_trends",
                inner::handle_track_sleep_trends(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// OptimizeSleepScheduleTool
// ============================================================================

/// Tool for recommending optimal sleep schedule.
pub struct OptimizeSleepScheduleTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for OptimizeSleepScheduleTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(properties, None);
        tool_definition(
            "optimize_sleep_schedule",
            "Get personalized sleep schedule recommendations based on training and recovery needs",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(ctx.resources.clone());
            let request =
                handler_bridge::build_universal_request(&ctx, args, "optimize_sleep_schedule");
            handler_bridge::map_universal_response(
                "optimize_sleep_schedule",
                inner::handle_optimize_sleep_schedule(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all sleep tools for registration
#[must_use]
pub fn create_sleep_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(AnalyzeSleepQualityTool),
        Box::new(CalculateRecoveryScoreTool),
        Box::new(SuggestRestDayTool),
        Box::new(TrackSleepTrendsTool),
        Box::new(OptimizeSleepScheduleTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(AnalyzeSleepQualityTool => empty);
crate::declare_security!(CalculateRecoveryScoreTool => empty);
crate::declare_security!(OptimizeSleepScheduleTool => empty);
crate::declare_security!(SuggestRestDayTool => empty);
crate::declare_security!(TrackSleepTrendsTool => empty);
