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
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

/// One stretch as the list tool reports it.
///
/// A summary, not the row: the list is for choosing, so it carries what you
/// choose on and leaves instructions and cues to `get_stretching_exercise`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StretchingExerciseSummary {
    /// Identifier `get_stretching_exercise` takes.
    pub id: String,
    /// Exercise name.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Which family of stretch it belongs to.
    pub category: String,
    /// How demanding it is.
    pub difficulty: String,
    /// Muscles it targets.
    pub primary_muscles: Vec<String>,
    /// Muscles it also works.
    pub secondary_muscles: Vec<String>,
    /// How long to hold, seconds.
    pub duration_seconds: u32,
    /// How many sets.
    pub sets: u32,
}

/// What `list_stretching_exercises` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListStretchingExercisesResult {
    /// The matching stretches.
    pub exercises: Vec<StretchingExerciseSummary>,
    /// How many were returned.
    pub count: usize,
    /// RFC 3339 timestamp of the read.
    pub timestamp: String,
}

/// One stretch in full, as `get_stretching_exercise` reports it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StretchingExerciseDetail {
    /// Identifier.
    pub id: String,
    /// Exercise name.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Which family of stretch it belongs to.
    pub category: String,
    /// How demanding it is.
    pub difficulty: String,
    /// Muscles it targets.
    pub primary_muscles: Vec<String>,
    /// Muscles it also works.
    pub secondary_muscles: Vec<String>,
    /// How long to hold, seconds.
    pub duration_seconds: u32,
    /// Repetitions, where the stretch is repeated rather than held.
    pub repetitions: Option<u32>,
    /// How many sets.
    pub sets: u32,
    /// Activities it suits.
    pub recommended_for_activities: Vec<String>,
    /// When not to do it.
    pub contraindications: Vec<String>,
    /// How to do it, in order.
    pub instructions: Vec<String>,
    /// What to feel for while doing it.
    pub cues: Vec<String>,
    /// Illustration, where one exists.
    pub image_url: Option<String>,
    /// Demonstration, where one exists.
    pub video_url: Option<String>,
}

/// One stretch in a suggestion, carrying just enough to follow it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SuggestedStretch {
    /// Identifier.
    pub id: String,
    /// Exercise name.
    pub name: String,
    /// Which family of stretch it belongs to.
    pub category: String,
    /// How demanding it is.
    pub difficulty: String,
    /// How long to hold, seconds.
    pub duration_seconds: u32,
    /// How many sets.
    pub sets: u32,
    /// Muscles it targets.
    pub primary_muscles: Vec<String>,
    /// How to do it, in order.
    pub instructions: Vec<String>,
}

/// What `suggest_stretches_for_activity` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SuggestStretchesResult {
    /// The activity the stretches were chosen for.
    pub activity_type: String,
    /// The stretches, in the order to do them.
    pub exercises: Vec<SuggestedStretch>,
    /// How many were suggested.
    pub count: usize,
    /// How long the whole set takes, seconds.
    pub total_duration_seconds: u32,
    /// RFC 3339 timestamp of the suggestion.
    pub suggested_at: String,
}

/// One pose as the list tool reports it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct YogaPoseSummary {
    /// Identifier `get_yoga_pose` takes.
    pub id: String,
    /// Its English name.
    pub english_name: String,
    /// Its Sanskrit name, where the pose has one.
    pub sanskrit_name: Option<String>,
    /// What it does.
    pub description: String,
    /// Which family of pose it belongs to.
    pub category: String,
    /// How demanding it is.
    pub difficulty: String,
    /// What kind of pose it is.
    pub pose_type: String,
    /// Muscles it targets.
    pub primary_muscles: Vec<String>,
    /// How long to hold, seconds.
    pub hold_duration_seconds: u32,
}

/// What `list_yoga_poses` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListYogaPosesResult {
    /// The matching poses.
    pub poses: Vec<YogaPoseSummary>,
    /// How many were returned.
    pub count: usize,
    /// RFC 3339 timestamp of the read.
    pub timestamp: String,
}

/// One pose in full, as `get_yoga_pose` reports it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct YogaPoseDetail {
    /// Identifier.
    pub id: String,
    /// Its English name.
    pub english_name: String,
    /// Its Sanskrit name, where the pose has one.
    pub sanskrit_name: Option<String>,
    /// What it does.
    pub description: String,
    /// What it is good for.
    pub benefits: Vec<String>,
    /// Which family of pose it belongs to.
    pub category: String,
    /// How demanding it is.
    pub difficulty: String,
    /// What kind of pose it is.
    pub pose_type: String,
    /// Muscles it targets.
    pub primary_muscles: Vec<String>,
    /// Muscles it also works.
    pub secondary_muscles: Vec<String>,
    /// Chakras the tradition associates with it.
    pub chakras: Vec<String>,
    /// How long to hold, seconds.
    pub hold_duration_seconds: u32,
    /// How to breathe in it.
    pub breath_guidance: Option<String>,
    /// Activities it suits.
    pub recommended_for_activities: Vec<String>,
    /// Recovery states it suits.
    pub recommended_for_recovery: Vec<String>,
    /// When not to do it.
    pub contraindications: Vec<String>,
    /// How to enter and hold it.
    pub instructions: Vec<String>,
    /// Easier versions.
    pub modifications: Vec<String>,
    /// Harder versions.
    pub progressions: Vec<String>,
    /// What to feel for while holding it.
    pub cues: Vec<String>,
    /// Poses to do first.
    pub warmup_poses: Vec<String>,
    /// Poses that follow well.
    pub followup_poses: Vec<String>,
    /// Illustration, where one exists.
    pub image_url: Option<String>,
    /// Demonstration, where one exists.
    pub video_url: Option<String>,
}

/// One pose in a sequence, in the order it is done.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SequencePose {
    /// Position in the sequence, 1-based.
    pub order: usize,
    /// Identifier.
    pub id: String,
    /// Its English name.
    pub english_name: String,
    /// Its Sanskrit name, where the pose has one.
    pub sanskrit_name: Option<String>,
    /// Which family of pose it belongs to.
    pub category: String,
    /// How demanding it is.
    pub difficulty: String,
    /// How long to hold, seconds.
    pub hold_duration_seconds: u32,
    /// How to breathe in it.
    pub breath_guidance: Option<String>,
    /// Muscles it targets.
    pub primary_muscles: Vec<String>,
    /// How to enter and hold it.
    pub instructions: Vec<String>,
}

/// What `suggest_yoga_sequence` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SuggestYogaSequenceResult {
    /// What the sequence is for.
    pub purpose: String,
    /// The poses, in order.
    pub sequence: Vec<SequencePose>,
    /// How many poses it holds.
    pub pose_count: usize,
    /// How long it actually runs, seconds — poses are added while they fit,
    /// so this lands at or under the target rather than on it.
    pub total_duration_seconds: u32,
    /// The length that was asked for, minutes.
    pub target_duration_minutes: u32,
    /// How to approach the sequence.
    pub guidance: String,
    /// RFC 3339 timestamp of the suggestion.
    pub suggested_at: String,
}

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
        let schema = object_schema(properties, None);

        answers_with::<ListStretchingExercisesResult>(tool_definition(
            "list_stretching_exercises",
            "List stretching exercises with optional filtering by category, difficulty, muscle group, or activity type",
            schema,
            None,
        ))
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

            let summaries: Vec<StretchingExerciseSummary> = exercises
                .iter()
                .map(|e| StretchingExerciseSummary {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    description: e.description.clone(),
                    category: e.category.as_str().to_owned(),
                    difficulty: e.difficulty.as_str().to_owned(),
                    primary_muscles: e.primary_muscles.clone(),
                    secondary_muscles: e.secondary_muscles.clone(),
                    duration_seconds: e.duration_seconds,
                    sets: e.sets,
                })
                .collect();

            ok_typed(
                "list_stretching_exercises",
                ListStretchingExercisesResult {
                    count: summaries.len(),
                    exercises: summaries,
                    timestamp: Utc::now().to_rfc3339(),
                },
            )
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
        let schema = object_schema(properties, Some(vec!["exercise_id".to_owned()]));

        answers_with::<StretchingExerciseDetail>(tool_definition(
            "get_stretching_exercise",
            "Get detailed information about a specific stretching exercise",
            schema,
            None,
        ))
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

            ok_typed(
                "get_stretching_exercise",
                StretchingExerciseDetail {
                    id: exercise.id,
                    name: exercise.name,
                    description: exercise.description,
                    category: exercise.category.as_str().to_owned(),
                    difficulty: exercise.difficulty.as_str().to_owned(),
                    primary_muscles: exercise.primary_muscles,
                    secondary_muscles: exercise.secondary_muscles,
                    duration_seconds: exercise.duration_seconds,
                    repetitions: exercise.repetitions,
                    sets: exercise.sets,
                    recommended_for_activities: exercise.recommended_for_activities,
                    contraindications: exercise.contraindications,
                    instructions: exercise.instructions,
                    cues: exercise.cues,
                    image_url: exercise.image_url,
                    video_url: exercise.video_url,
                },
            )
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
        let schema = object_schema(properties, Some(vec!["activity_type".to_owned()]));

        answers_with::<SuggestStretchesResult>(tool_definition(
            "suggest_stretches_for_activity",
            "Get personalized stretching recommendations based on your recent activity type",
            schema,
            None,
        ))
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
            let suggestions: Vec<SuggestedStretch> = exercises
                .iter()
                .take(max_exercises)
                .map(|e| SuggestedStretch {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    category: e.category.as_str().to_owned(),
                    difficulty: e.difficulty.as_str().to_owned(),
                    duration_seconds: e.duration_seconds,
                    sets: e.sets,
                    primary_muscles: e.primary_muscles.clone(),
                    instructions: e.instructions.clone(),
                })
                .collect();

            let total_duration_seconds: u32 = exercises
                .iter()
                .take(max_exercises)
                .map(|e| e.duration_seconds * e.sets)
                .sum();

            ok_typed(
                "suggest_stretches_for_activity",
                SuggestStretchesResult {
                    activity_type: activity_type.to_owned(),
                    count: suggestions.len(),
                    exercises: suggestions,
                    total_duration_seconds,
                    suggested_at: Utc::now().to_rfc3339(),
                },
            )
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
        let schema = object_schema(properties, None);

        answers_with::<ListYogaPosesResult>(tool_definition(
            "list_yoga_poses",
            "List yoga poses with optional filtering by category, difficulty, pose type, or recovery context",
            schema,
            None,
        ))
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

            let summaries: Vec<YogaPoseSummary> = poses
                .iter()
                .map(|p| YogaPoseSummary {
                    id: p.id.clone(),
                    english_name: p.english_name.clone(),
                    sanskrit_name: p.sanskrit_name.clone(),
                    description: p.description.clone(),
                    category: p.category.as_str().to_owned(),
                    difficulty: p.difficulty.as_str().to_owned(),
                    pose_type: p.pose_type.as_str().to_owned(),
                    primary_muscles: p.primary_muscles.clone(),
                    hold_duration_seconds: p.hold_duration_seconds,
                })
                .collect();

            ok_typed(
                "list_yoga_poses",
                ListYogaPosesResult {
                    count: summaries.len(),
                    poses: summaries,
                    timestamp: Utc::now().to_rfc3339(),
                },
            )
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
        let schema = object_schema(properties, Some(vec!["pose_id".to_owned()]));

        answers_with::<YogaPoseDetail>(tool_definition(
            "get_yoga_pose",
            "Get detailed information about a specific yoga pose",
            schema,
            None,
        ))
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

            ok_typed(
                "get_yoga_pose",
                YogaPoseDetail {
                    id: pose.id,
                    english_name: pose.english_name,
                    sanskrit_name: pose.sanskrit_name,
                    description: pose.description,
                    benefits: pose.benefits,
                    category: pose.category.as_str().to_owned(),
                    difficulty: pose.difficulty.as_str().to_owned(),
                    pose_type: pose.pose_type.as_str().to_owned(),
                    primary_muscles: pose.primary_muscles,
                    secondary_muscles: pose.secondary_muscles,
                    chakras: pose.chakras,
                    hold_duration_seconds: pose.hold_duration_seconds,
                    breath_guidance: pose.breath_guidance,
                    recommended_for_activities: pose.recommended_for_activities,
                    recommended_for_recovery: pose.recommended_for_recovery,
                    contraindications: pose.contraindications,
                    instructions: pose.instructions,
                    modifications: pose.modifications,
                    progressions: pose.progressions,
                    cues: pose.cues,
                    warmup_poses: pose.warmup_poses,
                    followup_poses: pose.followup_poses,
                    image_url: pose.image_url,
                    video_url: pose.video_url,
                },
            )
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
        let schema = object_schema(properties, Some(vec!["purpose".to_owned()]));

        answers_with::<SuggestYogaSequenceResult>(tool_definition(
            "suggest_yoga_sequence",
            "Create a personalized yoga sequence for recovery based on your recent activities or goals",
            schema,
            None,
        ))
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
        let mut sequence: Vec<SequencePose> = Vec::new();
        let mut total_seconds: u32 = 0;

        for pose in &poses {
            if total_seconds + pose.hold_duration_seconds > target_seconds {
                break;
            }
            sequence.push(SequencePose {
                order: sequence.len() + 1,
                id: pose.id.clone(),
                english_name: pose.english_name.clone(),
                sanskrit_name: pose.sanskrit_name.clone(),
                category: pose.category.as_str().to_owned(),
                difficulty: pose.difficulty.as_str().to_owned(),
                hold_duration_seconds: pose.hold_duration_seconds,
                breath_guidance: pose.breath_guidance.clone(),
                primary_muscles: pose.primary_muscles.clone(),
                instructions: pose.instructions.clone(),
            });
            total_seconds += pose.hold_duration_seconds;
        }

        ok_typed(
            "suggest_yoga_sequence",
            SuggestYogaSequenceResult {
                purpose: purpose.to_owned(),
                pose_count: sequence.len(),
                sequence,
                total_duration_seconds: total_seconds,
                target_duration_minutes: duration_minutes,
                guidance: format!(
                    "This {} yoga sequence is designed for {}. Take your time with each pose and listen to your body.",
                    duration_minutes,
                    purpose.replace('_', " ")
                ),
                suggested_at: Utc::now().to_rfc3339(),
            },
        )
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
