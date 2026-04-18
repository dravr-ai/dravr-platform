// ABOUTME: Mobility tools for stretching exercises and yoga poses.
// ABOUTME: Implements list_stretching_exercises, get_stretching_exercise, suggest_stretches_for_activity, list_yoga_poses, get_yoga_pose, suggest_yoga_sequence.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Mobility Tools
//!
//! This module provides tools for mobility, stretching, and yoga:
//! - `ListStretchingExercisesTool` - List stretching exercises with filtering
//! - `GetStretchingExerciseTool` - Get a specific stretching exercise
//! - `SuggestStretchesForActivityTool` - Suggest stretches based on activity type
//! - `ListYogaPosesTool` - List yoga poses with filtering
//! - `GetYogaPoseTool` - Get a specific yoga pose
//! - `SuggestYogaSequenceTool` - Suggest a yoga sequence for recovery
//!
//! All tools use direct database access for seeded mobility data.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};
use crate::tools::universal_delegate::delegate_to_handler;

// ============================================================================
// ListStretchingExercisesTool
// ============================================================================

/// Tool for listing stretching exercises with filtering.
pub struct ListStretchingExercisesTool;

#[async_trait]
impl McpTool for ListStretchingExercisesTool {
    fn name(&self) -> &'static str {
        "list_stretching_exercises"
    }

    fn description(&self) -> &'static str {
        "List stretching exercises with optional filtering by category, difficulty, muscle group, or activity type"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by stretch category: static, dynamic, pnf, ballistic".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "difficulty".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by difficulty: beginner, intermediate, advanced".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "muscle_group".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by muscle group (e.g., hamstrings, quadriceps, calves)".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by recommended activity (e.g., running, cycling, swimming)".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum number of results to return (default: 20)".to_owned()),
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
        delegate_to_handler(
            context,
            args,
            "list_stretching_exercises",
            handlers::handle_list_stretching_exercises,
        )
        .await
    }
}

// ============================================================================
// GetStretchingExerciseTool
// ============================================================================

/// Tool for getting a specific stretching exercise by ID.
pub struct GetStretchingExerciseTool;

#[async_trait]
impl McpTool for GetStretchingExerciseTool {
    fn name(&self) -> &'static str {
        "get_stretching_exercise"
    }

    fn description(&self) -> &'static str {
        "Get detailed information about a specific stretching exercise"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "exercise_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("The unique ID of the stretching exercise".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["exercise_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_stretching_exercise",
            handlers::handle_get_stretching_exercise,
        )
        .await
    }
}

// ============================================================================
// SuggestStretchesForActivityTool
// ============================================================================

/// Tool for suggesting stretches based on activity type.
pub struct SuggestStretchesForActivityTool;

#[async_trait]
impl McpTool for SuggestStretchesForActivityTool {
    fn name(&self) -> &'static str {
        "suggest_stretches_for_activity"
    }

    fn description(&self) -> &'static str {
        "Get personalized stretching recommendations based on your recent activity type"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "activity_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The activity type to get stretches for (e.g., running, cycling, swimming, hiking)"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "focus".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional focus: warmup (dynamic stretches) or cooldown (static stretches)"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum number of stretches to suggest (default: 6)".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["activity_type".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "suggest_stretches_for_activity",
            handlers::handle_suggest_stretches_for_activity,
        )
        .await
    }
}

// ============================================================================
// ListYogaPosesTool
// ============================================================================

/// Tool for listing yoga poses with filtering.
pub struct ListYogaPosesTool;

#[async_trait]
impl McpTool for ListYogaPosesTool {
    fn name(&self) -> &'static str {
        "list_yoga_poses"
    }

    fn description(&self) -> &'static str {
        "List yoga poses with optional filtering by category, difficulty, pose type, or recovery context"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by pose category: standing, seated, supine, prone, inversion, balance, twist"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "difficulty".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by difficulty: beginner, intermediate, advanced".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "pose_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by pose type: stretch, strength, balance, relaxation, breathing"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "muscle_group".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by muscle group (e.g., hamstrings, hips, shoulders)".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "recovery_context".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by recovery context: post_cardio, rest_day, morning, evening"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum number of results to return (default: 20)".to_owned()),
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
        delegate_to_handler(
            context,
            args,
            "list_yoga_poses",
            handlers::handle_list_yoga_poses,
        )
        .await
    }
}

// ============================================================================
// GetYogaPoseTool
// ============================================================================

/// Tool for getting a specific yoga pose by ID.
pub struct GetYogaPoseTool;

#[async_trait]
impl McpTool for GetYogaPoseTool {
    fn name(&self) -> &'static str {
        "get_yoga_pose"
    }

    fn description(&self) -> &'static str {
        "Get detailed information about a specific yoga pose"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "pose_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("The unique ID of the yoga pose".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["pose_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_yoga_pose",
            handlers::handle_get_yoga_pose,
        )
        .await
    }
}

// ============================================================================
// SuggestYogaSequenceTool
// ============================================================================

/// Tool for suggesting a yoga sequence for recovery.
pub struct SuggestYogaSequenceTool;

#[async_trait]
impl McpTool for SuggestYogaSequenceTool {
    fn name(&self) -> &'static str {
        "suggest_yoga_sequence"
    }

    fn description(&self) -> &'static str {
        "Create a personalized yoga sequence for recovery based on your recent activities or goals"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "purpose".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Purpose of the sequence: post_cardio, rest_day, morning, evening, stress_relief"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "duration_minutes".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Target duration in minutes (10, 15, 20, 30). Default: 15".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "difficulty".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Maximum difficulty level: beginner, intermediate, advanced".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "focus_area".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional muscle/area focus: hips, hamstrings, back, shoulders".to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["purpose".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "suggest_yoga_sequence",
            handlers::handle_suggest_yoga_sequence,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all mobility tools for registration
#[must_use]
pub fn create_mobility_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ListStretchingExercisesTool),
        Box::new(GetStretchingExerciseTool),
        Box::new(SuggestStretchesForActivityTool),
        Box::new(ListYogaPosesTool),
        Box::new(GetYogaPoseTool),
        Box::new(SuggestYogaSequenceTool),
    ]
}
