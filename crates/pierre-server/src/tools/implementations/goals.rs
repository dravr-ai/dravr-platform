// ABOUTME: Goal management tools for setting and tracking fitness goals.
// ABOUTME: Implements set_goal, suggest_goals, track_progress, analyze_goal_feasibility.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Goal Management Tools
//!
//! This module provides tools for fitness goal management:
//! - `SetGoalTool` - Create a new fitness goal
//! - `SuggestGoalsTool` - Get AI-suggested fitness goals
//! - `TrackProgressTool` - Track progress toward goals
//! - `AnalyzeGoalFeasibilityTool` - Assess goal achievability
//!
//! Uses the goal engine directly for clean, efficient goal management.

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
// SetGoalTool - Create a new fitness goal
// ============================================================================

/// Tool for creating a new fitness goal.
pub struct SetGoalTool;

#[async_trait]
impl McpTool for SetGoalTool {
    fn name(&self) -> &'static str {
        "set_goal"
    }

    fn description(&self) -> &'static str {
        "Create a new fitness goal with specified type, target value, and timeframe"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "goal_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Type of goal: 'distance', 'time', 'frequency', or 'performance'".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "target_value".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Target value for the goal (km for distance, sessions for frequency, etc.)"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "timeframe".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Goal timeframe: 'week', 'month', 'quarter', or 'year'. Default: 'month'"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Title or description for the goal".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "sport".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Sport type for the goal (e.g., 'Running', 'Cycling'). Default: 'Running'"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["goal_type".to_owned(), "target_value".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        // Single execution path: delegate to the UniversalExecutor handler so
        // MCP protocol + chat pipeline + A2A all resolve to the same body.
        // Retired with handler inlining in Stage 5 of the registry unification.
        dispatch_handler(context, args, "set_goal", handlers::handle_set_goal).await
    }
}

// ============================================================================
// SuggestGoalsTool - Get AI-suggested goals
// ============================================================================

/// Tool for getting AI-suggested fitness goals.
pub struct SuggestGoalsTool;

#[async_trait]
impl McpTool for SuggestGoalsTool {
    fn name(&self) -> &'static str {
        "suggest_goals"
    }

    fn description(&self) -> &'static str {
        "Get AI-suggested fitness goals based on your activity history and fitness level"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to analyze. Defaults to configured provider.".to_owned(),
                ),
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

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        // Single execution path: delegate to the UniversalExecutor handler so
        // MCP protocol + chat pipeline + A2A all resolve to the same body.
        // Retired with handler inlining in Stage 5 of the registry unification.
        dispatch_handler(
            context,
            args,
            "suggest_goals",
            handlers::handle_suggest_goals,
        )
        .await
    }
}

// ============================================================================
// TrackProgressTool - Track goal progress
// ============================================================================

/// Tool for tracking progress toward fitness goals.
pub struct TrackProgressTool;

#[async_trait]
impl McpTool for TrackProgressTool {
    fn name(&self) -> &'static str {
        "track_progress"
    }

    fn description(&self) -> &'static str {
        "Track progress toward a specific fitness goal with milestone achievements and projections"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "goal_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the goal to track progress for".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query. Defaults to configured provider.".to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["goal_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        // Single execution path: delegate to the UniversalExecutor handler so
        // MCP protocol + chat pipeline + A2A all resolve to the same body.
        // Retired with handler inlining in Stage 5 of the registry unification.
        dispatch_handler(
            context,
            args,
            "track_progress",
            handlers::handle_track_progress,
        )
        .await
    }
}

// ============================================================================
// AnalyzeGoalFeasibilityTool - Assess goal achievability
// ============================================================================

/// Tool for analyzing if a fitness goal is achievable.
pub struct AnalyzeGoalFeasibilityTool;

#[async_trait]
impl McpTool for AnalyzeGoalFeasibilityTool {
    fn name(&self) -> &'static str {
        "analyze_goal_feasibility"
    }

    fn description(&self) -> &'static str {
        "Analyze whether a fitness goal is achievable based on your current fitness level and training history"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "goal_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Type of goal: 'distance', 'time', 'frequency', or 'performance'".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "target_value".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Target value for the goal".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "timeframe_days".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Number of days to achieve the goal. Default: 30.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to analyze. Defaults to configured provider.".to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["goal_type".to_owned(), "target_value".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        // Single execution path: delegate to the UniversalExecutor handler so
        // MCP protocol + chat pipeline + A2A all resolve to the same body.
        // Retired with handler inlining in Stage 5 of the registry unification.
        dispatch_handler(
            context,
            args,
            "analyze_goal_feasibility",
            handlers::handle_analyze_goal_feasibility,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all goal management tools for registration
#[must_use]
pub fn create_goal_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(SetGoalTool),
        Box::new(SuggestGoalsTool),
        Box::new(TrackProgressTool),
        Box::new(AnalyzeGoalFeasibilityTool),
    ]
}
