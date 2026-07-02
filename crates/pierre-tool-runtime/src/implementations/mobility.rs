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
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use pierre_database::database::mobility::{
    DifficultyLevel, ListStretchingFilter, ListYogaFilter, StretchingCategory, YogaCategory,
    YogaPoseType,
};
use serde_json::{json, Value};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// ListStretchingExercisesTool
// ============================================================================

/// Tool for listing stretching exercises with filtering.
pub struct ListStretchingExercisesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListStretchingExercisesTool {
    fn definition(&self) -> Tool {
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

        tool_definition(
            "list_stretching_exercises",
            "List stretching exercises with optional filtering by category, difficulty, muscle group, or activity type",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let category = args
                .get("category")
                .and_then(Value::as_str)
                .map(StretchingCategory::parse);

            let difficulty = args
                .get("difficulty")
                .and_then(Value::as_str)
                .map(DifficultyLevel::parse);

            let muscle_group = args
                .get("muscle_group")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            #[allow(clippy::cast_possible_truncation)]
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|l| l.min(100) as u32);

            let filter = ListStretchingFilter {
                category,
                difficulty,
                muscle_group,
                activity_type: None,
                limit,
                offset: None,
            };

            let repo = context.resources.repos().mobility.as_ref();
            let exercises = repo
                .list_stretching_exercises(&filter)
                .await
                .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

            let exercises_json: Vec<Value> = exercises
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id,
                        "name": e.name,
                        "description": e.description,
                        "category": e.category.as_str(),
                        "difficulty": e.difficulty.as_str(),
                        "primary_muscles": e.primary_muscles,
                        "secondary_muscles": e.secondary_muscles,
                        "duration_seconds": e.duration_seconds,
                        "sets": e.sets,
                    })
                })
                .collect();

            Ok(ToolResult::ok(json!({
                "exercises": exercises_json,
                "count": exercises_json.len(),
                "timestamp": Utc::now().to_rfc3339(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetStretchingExerciseTool
// ============================================================================

/// Tool for getting a specific stretching exercise by ID.
pub struct GetStretchingExerciseTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetStretchingExerciseTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "exercise_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("The unique ID of the stretching exercise".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["exercise_id".to_owned()]),
        };

        tool_definition(
            "get_stretching_exercise",
            "Get detailed information about a specific stretching exercise",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let exercise_id = args
                .get("exercise_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("exercise_id is required"))?;

            let repo = context.resources.repos().mobility.as_ref();
            let exercise_opt = repo
                .get_stretching_exercise(exercise_id)
                .await
                .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

            let Some(exercise) = exercise_opt else {
                return Ok(ToolResult::error(json!({
                    "error": format!("Stretching exercise not found: {exercise_id}")
                })));
            };

            Ok(ToolResult::ok(json!({
                "id": exercise.id,
                "name": exercise.name,
                "description": exercise.description,
                "category": exercise.category.as_str(),
                "difficulty": exercise.difficulty.as_str(),
                "primary_muscles": exercise.primary_muscles,
                "secondary_muscles": exercise.secondary_muscles,
                "duration_seconds": exercise.duration_seconds,
                "repetitions": exercise.repetitions,
                "sets": exercise.sets,
                "recommended_for_activities": exercise.recommended_for_activities,
                "contraindications": exercise.contraindications,
                "instructions": exercise.instructions,
                "cues": exercise.cues,
                "image_url": exercise.image_url,
                "video_url": exercise.video_url,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SuggestStretchesForActivityTool
// ============================================================================

/// Tool for suggesting stretches based on activity type.
pub struct SuggestStretchesForActivityTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SuggestStretchesForActivityTool {
    fn definition(&self) -> Tool {
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["activity_type".to_owned()]),
        };

        tool_definition(
            "suggest_stretches_for_activity",
            "Get personalized stretching recommendations based on your recent activity type",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let activity_type = args
                .get("activity_type")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("activity_type is required"))?;

            let difficulty = args
                .get("difficulty")
                .and_then(Value::as_str)
                .map(DifficultyLevel::parse);

            #[allow(clippy::cast_possible_truncation)]
            let duration_minutes = args
                .get("duration_minutes")
                .and_then(Value::as_u64)
                .map(|d| d.min(240) as u32);

            let repo = context.resources.repos().mobility.as_ref();
            let all_exercises = repo
                .get_stretches_for_activity(activity_type, Some(20))
                .await
                .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

            let exercises: Vec<_> = if let Some(ref target_difficulty) = difficulty {
                all_exercises
                    .into_iter()
                    .filter(|e| &e.difficulty == target_difficulty)
                    .collect()
            } else {
                all_exercises
            };

            let max_exercises = duration_minutes.map_or(6, |d| (d / 5).clamp(3, 12) as usize);
            let suggestions: Vec<Value> = exercises
                .iter()
                .take(max_exercises)
                .map(|e| {
                    json!({
                        "id": e.id,
                        "name": e.name,
                        "category": e.category.as_str(),
                        "difficulty": e.difficulty.as_str(),
                        "duration_seconds": e.duration_seconds,
                        "sets": e.sets,
                        "primary_muscles": e.primary_muscles,
                        "instructions": e.instructions,
                    })
                })
                .collect();

            let total_duration_seconds: u32 = exercises
                .iter()
                .take(max_exercises)
                .map(|e| e.duration_seconds * e.sets)
                .sum();

            Ok(ToolResult::ok(json!({
                "activity_type": activity_type,
                "exercises": suggestions,
                "count": suggestions.len(),
                "total_duration_seconds": total_duration_seconds,
                "suggested_at": Utc::now().to_rfc3339(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListYogaPosesTool
// ============================================================================

/// Tool for listing yoga poses with filtering.
pub struct ListYogaPosesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListYogaPosesTool {
    fn definition(&self) -> Tool {
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

        tool_definition(
            "list_yoga_poses",
            "List yoga poses with optional filtering by category, difficulty, pose type, or recovery context",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let category = args
                .get("category")
                .and_then(Value::as_str)
                .map(YogaCategory::parse);

            let difficulty = args
                .get("difficulty")
                .and_then(Value::as_str)
                .map(DifficultyLevel::parse);

            let pose_type = args
                .get("pose_type")
                .and_then(Value::as_str)
                .map(YogaPoseType::parse);

            let recovery_context = args
                .get("recovery_context")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            #[allow(clippy::cast_possible_truncation)]
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|l| l.min(100) as u32);

            let filter = ListYogaFilter {
                category,
                difficulty,
                pose_type,
                muscle_group: None,
                activity_type: None,
                recovery_context,
                limit,
                offset: None,
            };

            let repo = context.resources.repos().mobility.as_ref();
            let poses = repo
                .list_yoga_poses(&filter)
                .await
                .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

            let poses_json: Vec<Value> = poses
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "english_name": p.english_name,
                        "sanskrit_name": p.sanskrit_name,
                        "description": p.description,
                        "category": p.category.as_str(),
                        "difficulty": p.difficulty.as_str(),
                        "pose_type": p.pose_type.as_str(),
                        "primary_muscles": p.primary_muscles,
                        "hold_duration_seconds": p.hold_duration_seconds,
                    })
                })
                .collect();

            Ok(ToolResult::ok(json!({
                "poses": poses_json,
                "count": poses_json.len(),
                "timestamp": Utc::now().to_rfc3339(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetYogaPoseTool
// ============================================================================

/// Tool for getting a specific yoga pose by ID.
pub struct GetYogaPoseTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetYogaPoseTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "pose_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("The unique ID of the yoga pose".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["pose_id".to_owned()]),
        };

        tool_definition(
            "get_yoga_pose",
            "Get detailed information about a specific yoga pose",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let pose_id = args
                .get("pose_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("pose_id is required"))?;

            let repo = context.resources.repos().mobility.as_ref();
            let pose_opt = repo
                .get_yoga_pose(pose_id)
                .await
                .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

            let Some(pose) = pose_opt else {
                return Ok(ToolResult::error(json!({
                    "error": format!("Yoga pose not found: {pose_id}")
                })));
            };

            Ok(ToolResult::ok(json!({
                "id": pose.id,
                "english_name": pose.english_name,
                "sanskrit_name": pose.sanskrit_name,
                "description": pose.description,
                "benefits": pose.benefits,
                "category": pose.category.as_str(),
                "difficulty": pose.difficulty.as_str(),
                "pose_type": pose.pose_type.as_str(),
                "primary_muscles": pose.primary_muscles,
                "secondary_muscles": pose.secondary_muscles,
                "chakras": pose.chakras,
                "hold_duration_seconds": pose.hold_duration_seconds,
                "breath_guidance": pose.breath_guidance,
                "recommended_for_activities": pose.recommended_for_activities,
                "recommended_for_recovery": pose.recommended_for_recovery,
                "contraindications": pose.contraindications,
                "instructions": pose.instructions,
                "modifications": pose.modifications,
                "progressions": pose.progressions,
                "cues": pose.cues,
                "warmup_poses": pose.warmup_poses,
                "followup_poses": pose.followup_poses,
                "image_url": pose.image_url,
                "video_url": pose.video_url,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SuggestYogaSequenceTool
// ============================================================================

/// Tool for suggesting a yoga sequence for recovery.
pub struct SuggestYogaSequenceTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SuggestYogaSequenceTool {
    fn definition(&self) -> Tool {
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["purpose".to_owned()]),
        };

        tool_definition(
            "suggest_yoga_sequence",
            "Create a personalized yoga sequence for recovery based on your recent activities or goals",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let purpose = args
            .get("purpose")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("purpose is required"))?;

        #[allow(clippy::cast_possible_truncation)]
        let duration_minutes = args
            .get("duration_minutes")
            .and_then(Value::as_u64)
            .map_or(15_u32, |v| v.min(240) as u32);

        let difficulty = args
            .get("difficulty")
            .and_then(Value::as_str)
            .map(DifficultyLevel::parse);

        let repo = context.resources.repos().mobility.as_ref();
        let all_poses = repo
            .get_poses_for_recovery(purpose, Some(20))
            .await
            .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

        let poses: Vec<_> = if let Some(ref target_difficulty) = difficulty {
            all_poses
                .into_iter()
                .filter(|p| &p.difficulty == target_difficulty)
                .collect()
        } else {
            all_poses
        };

        let target_seconds = duration_minutes * 60;
        let mut sequence: Vec<Value> = Vec::new();
        let mut total_seconds: u32 = 0;

        for pose in &poses {
            if total_seconds + pose.hold_duration_seconds > target_seconds {
                break;
            }
            sequence.push(json!({
                "order": sequence.len() + 1,
                "id": pose.id,
                "english_name": pose.english_name,
                "sanskrit_name": pose.sanskrit_name,
                "category": pose.category.as_str(),
                "difficulty": pose.difficulty.as_str(),
                "hold_duration_seconds": pose.hold_duration_seconds,
                "breath_guidance": pose.breath_guidance,
                "primary_muscles": pose.primary_muscles,
                "instructions": pose.instructions,
            }));
            total_seconds += pose.hold_duration_seconds;
        }

        Ok(ToolResult::ok(json!({
            "purpose": purpose,
            "sequence": sequence,
            "pose_count": sequence.len(),
            "total_duration_seconds": total_seconds,
            "target_duration_minutes": duration_minutes,
            "guidance": format!(
                "This {} yoga sequence is designed for {}. Take your time with each pose and listen to your body.",
                duration_minutes,
                purpose.replace('_', " ")
            ),
            "suggested_at": Utc::now().to_rfc3339(),
        })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all mobility tools for registration
#[must_use]
pub fn create_mobility_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ListStretchingExercisesTool),
        Box::new(GetStretchingExerciseTool),
        Box::new(SuggestStretchesForActivityTool),
        Box::new(ListYogaPosesTool),
        Box::new(GetYogaPoseTool),
        Box::new(SuggestYogaSequenceTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(GetStretchingExerciseTool => empty);
crate::declare_security!(GetYogaPoseTool => empty);
crate::declare_security!(ListStretchingExercisesTool => empty);
crate::declare_security!(ListYogaPosesTool => empty);
crate::declare_security!(SuggestStretchesForActivityTool => empty);
crate::declare_security!(SuggestYogaSequenceTool => empty);
