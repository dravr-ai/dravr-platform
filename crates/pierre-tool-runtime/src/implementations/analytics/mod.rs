// ABOUTME: Analytics tools for fitness data analysis and insights.
// ABOUTME: Uses intelligence module directly for clean, efficient analysis.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Analytics Tools
//!
//! This module provides tools for fitness analytics:
//! - `AnalyzeActivityTool` - Deep analysis of individual activities
//! - `GetActivityIntelligenceTool` - AI-powered activity insights
//! - `CalculateMetricsTool` - Calculate pace, speed, distance per climb, HR reading
//! - `AnalyzePerformanceTrendsTool` - Track metric trends over time
//! - `CompareActivitiesTool` - Compare activities against similar/PRs
//! - `AnalyzeTrainingLoadTool` - Calculate CTL/ATL/TSB training metrics
//! - `DetectPatternsTool` - Detect training patterns and overtraining signs
//! - `CalculateFitnessScoreTool` - Calculate overall fitness score
//! - `AnalyzeWeatherImpactTool` - Analyze weather impact on activity performance
//! - `GenerateRecommendationsTool` - Generate personalized training recommendations
//! - `PredictPerformanceTool` - Predict future performance based on training
//!
//! These tools use the intelligence module directly for efficient analysis.

pub(crate) mod inner;

/// The shapes the analytics tools answer with, and their derived schemas.
pub mod output;
pub mod recommendations_output;
mod weather_impact;

pub use weather_impact::AnalyzeWeatherImpactTool;

// The training-load payload builder is reachable from integration tests: this
// crate keeps tests external, so content coverage of the JSON the model reads
// needs the builder public. Its production caller is the tool handler below.
pub use inner::analyze_detailed_training_load;
// Its own line: the pre-push moved-symbol check and rustfmt's 100-column wrap
// both read `pub use` line by line.
pub use inner::calculate_fitness_metrics;
// The nutrition mode of `generate_recommendations` and the recovery mode's
// actions, public for the same reason: tests assert what the athlete is told.
// Their production caller is the recommendations handler.
pub use inner::recovery_actions_for;
pub use inner::{generate_nutrition_recommendations, load_nutrition_athlete, NutritionAthlete};
// Every mode's answer, for the same reason: the training-load modes' metrics
// are asserted against the thresholds the athlete saved.
pub use inner::generate_training_recommendations;

use crate::annotations::{annotations, Effect, Reach};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema_with_format, task_capable, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::analytics::inner::process_activity_analysis;
use crate::implementations::analytics::output::{
    ActivityIntelligenceResult, ActivityMetricsResult, CompareActivitiesResult, FitnessScoreResult,
    PatternsResult, PerformanceTrendsResult, RacePredictionResult, TrainingLoadResult,
};
use crate::implementations::analytics::recommendations_output::RecommendationsResult;
use crate::implementations::handler_bridge;
use crate::protocol::auth::AuthService;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::PropertySchema;
use pierre_providers::backend_resolver;
use pierre_providers::core::FitnessProvider;
use pierre_tools_core::ToolResult;

// ============================================================================
// Helper functions for provider creation and activity fetching
// ============================================================================

/// Create an authenticated provider from context
async fn create_provider(
    context: &ToolExecutionContext,
    provider_name: &str,
) -> Result<Box<dyn FitnessProvider>, ToolResult> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());

    auth_service
        .create_authenticated_provider(provider_name, context.user_id, tenant_id.as_deref())
        .await
        .map_err(|response| {
            ToolResult::error(json!({
                "error": response.error.unwrap_or_else(|| "Authentication failed".to_owned()),
                "provider": backend_resolver::user_facing_name(provider_name)
            }))
        })
}
// ============================================================================
// AnalyzeTrainingLoadTool - Calculate CTL/ATL/TSB
// ============================================================================

/// Tool for analyzing training load using CTL/ATL/TSB metrics.
#[derive(Debug)]
pub struct AnalyzeTrainingLoadTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeTrainingLoadTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Sleep provider (e.g., 'whoop'): adds last night's sleep score, duration and \
                     HRV as recovery context; the load numbers do not change."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);
        answers_with::<Formatted<TrainingLoadResult>>(task_capable(tool_definition(
            "analyze_training_load",
            "Analyze training load from per-activity TSS (the provider's, else estimated from \
             power vs the FTP saved with set_physiology, heart rate vs the saved threshold heart \
             rate, or pace or duration). Returns CTL (fitness), ATL (fatigue) and TSB (form), \
             form as a percentage of CTL with its band, weekly TSS totals and a taper reading.",
            schema,
            annotations(
                "Analyze training load",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "analyze_training_load");
            handler_bridge::map_universal_response(
                "analyze_training_load",
                inner::handle_analyze_training_load(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DetectPatternsTool - Detect training patterns
// ============================================================================

/// Tool for detecting training patterns and potential issues.
#[derive(Debug)]
pub struct DetectPatternsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DetectPatternsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "pattern_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "'weekly_schedule' (default; training days, consistency), 'training_blocks' \
                     (hard/easy share, recovery between hard efforts), 'progression' (weekly \
                     volume, spike weeks) or 'overtraining' (heart-rate drift, performance \
                     decline, short recovery); any other value is refused."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);
        answers_with::<Formatted<PatternsResult>>(task_capable(tool_definition(
            "detect_patterns",
            "Detect one training pattern per call: weekly schedule, hard/easy balance, volume \
             progression or overtraining signs. Needs at least 3 activities.",
            schema,
            annotations(
                "Detect training patterns",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "detect_patterns");
            handler_bridge::map_universal_response(
                "detect_patterns",
                inner::handle_detect_patterns(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CalculateFitnessScoreTool - Calculate overall fitness score
// ============================================================================

/// Tool for calculating an overall fitness score.
#[derive(Debug)]
pub struct CalculateFitnessScoreTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CalculateFitnessScoreTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Sleep provider (e.g., 'whoop'): last night's sleep adjusts the score, from \
                     +5% (excellent recovery) to -10% (poor)."
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
                    "Period scored for consistency and pace: 'month' (30 days, the default), \
                     'quarter' (90), 'year' (365) or 'all_time'. Set it to the period the \
                     athlete named; the default cannot answer a question about three months or \
                     a season. Chronic training load always uses the full history."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);
        answers_with::<Formatted<FitnessScoreResult>>(task_capable(tool_definition(
            "calculate_fitness_score",
            "Calculate a fitness score (0-100): chronic training load 40% (sessions scored \
             against the FTP and threshold heart rate saved with set_physiology, as \
             analyze_training_load does), consistency 30% (share of weeks with 3+ activities) \
             and pace improvement 30%, over the `timeframe` period.",
            schema,
            annotations(
                "Calculate fitness score",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "calculate_fitness_score");
            handler_bridge::map_universal_response(
                "calculate_fitness_score",
                inner::handle_calculate_fitness_score(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AnalyzeActivityTool - Deep analysis of individual activities
// ============================================================================

/// Tool for performing deep analysis of an individual activity.
#[derive(Debug)]
pub struct AnalyzeActivityTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeActivityTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "ID of the activity to analyze; get IDs from get_activities.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, Some(vec!["activity_id".to_owned()]));
        answers_with::<Formatted<ActivityIntelligenceResult>>(task_capable(tool_definition(
            "analyze_activity",
            "Analyze a single activity: a short summary with insights and recommendations drawn from its distance, elevation gain, heart rate and calories, plus its performance metrics (distance, duration, elevation, average and peak heart rate, calories). An ID that is not found analyzes the most recent activity instead and says so under `auto_selected`.",
            schema,
            annotations("Analyze activity", Effect::ReadOnly, Reach::AthleteAccount),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            // Resolve provider + required activity_id from args. The schema lists
            // `activity_id` as required so this branch surfaces invalid_input back
            // to the loop when the LLM forgets it.
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            let activity_id = args
                .get("activity_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| AppError::invalid_input("activity_id parameter required"))?;

            let executor = UniversalExecutor::new(context.resources.clone());

            // No pre-flight fetch: `get_activity_intelligence` authenticates
            // and fetches the activity itself, so a validation fetch here
            // doubled the provider round trips (two headless-browser passes on
            // a scrape-backed mirror) — and worse, its early bail on a missing
            // id suppressed the analytics handler's deliberate not-found
            // fallback, which analyzes the most recent activity and names the
            // missing id in an `auto_selected` block.
            //
            // Dispatch into `get_activity_intelligence` through the shared registry.
            // process_activity_analysis takes ownership of the request and rewrites
            // its tool_name; build a fresh envelope here so the registry sees the
            // analytics tool name rather than `analyze_activity`.
            let dispatch_request = handler_bridge::build_universal_request(
                &context,
                json!({
                    "provider": &provider_name,
                    "activity_id": &activity_id,
                }),
                "get_activity_intelligence",
            );

            handler_bridge::map_universal_response(
                "analyze_activity",
                process_activity_analysis(
                    &executor,
                    dispatch_request,
                    &activity_id,
                    context.user_id,
                )
                .await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetActivityIntelligenceTool - AI-powered activity insights
// ============================================================================

/// Tool for getting AI-powered intelligence insights for an activity.
#[derive(Debug)]
pub struct GetActivityIntelligenceTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetActivityIntelligenceTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the activity to analyze".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, Some(vec!["activity_id".to_owned()]));
        answers_with::<Formatted<ActivityIntelligenceResult>>(task_capable(tool_definition(
            "get_activity_intelligence",
            "Get insights and recommendations for one activity from its distance, elevation \
             gain, heart rate and calories, with its performance metrics. An ID that is not \
             found analyzes the most recent activity instead and says so under `auto_selected`.",
            schema,
            annotations(
                "Get activity insights",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request = handler_bridge::build_universal_request(
                &context,
                args,
                "get_activity_intelligence",
            );
            handler_bridge::map_universal_response(
                "get_activity_intelligence",
                inner::handle_get_activity_intelligence(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CalculateMetricsTool - Calculate pace, speed, distance per climb, HR reading
// ============================================================================

/// Tool for calculating custom fitness metrics from activity data.
#[derive(Debug)]
pub struct CalculateMetricsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CalculateMetricsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider name".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the activity to calculate metrics for".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "max_hr".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Maximum heart rate in bpm; else the saved one, else estimated from age. \
                     With the saved resting heart rate it sets the zones the average heart rate \
                     is read on."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "age".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Age in years, else the saved one; estimates the maximum heart rate \
                     (configured algorithm.maxhr) when none is given or saved."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(
            properties,
            Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        );
        answers_with::<Formatted<ActivityMetricsResult>>(task_capable(tool_definition(
            "calculate_metrics",
            "Calculate metrics for an activity: pace, speed, metres covered per metre climbed, and its average heart rate read on the athlete's zones",
            schema,
            annotations("Calculate activity metrics", Effect::ReadOnly, Reach::AthleteAccount),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "calculate_metrics");
            handler_bridge::map_universal_response(
                "calculate_metrics",
                inner::handle_calculate_metrics(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AnalyzePerformanceTrendsTool - Track metric trends over time
// ============================================================================

/// Tool for analyzing performance trends over time with statistical analysis.
#[derive(Debug)]
pub struct AnalyzePerformanceTrendsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzePerformanceTrendsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "metric".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "'pace' (default), 'speed', 'heart_rate', 'distance', 'duration', \
                     'elevation' or 'power'"
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
                    "'week', 'month' (default), 'quarter' or 'year'; any other value reads as \
                     'month'"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);
        answers_with::<Formatted<PerformanceTrendsResult>>(task_capable(tool_definition(
            "analyze_performance_trends",
            "Analyze how one metric trends over time, with statistics and insights.",
            schema,
            annotations(
                "Analyze performance trends",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request = handler_bridge::build_universal_request(
                &context,
                args,
                "analyze_performance_trends",
            );
            handler_bridge::map_universal_response(
                "analyze_performance_trends",
                inner::handle_analyze_performance_trends(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CompareActivitiesTool - Compare activities against similar/PRs
// ============================================================================

/// Tool for comparing an activity against similar activities or personal records.
#[derive(Debug)]
pub struct CompareActivitiesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CompareActivitiesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Primary activity to compare".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "comparison_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "'similar_activities' (default), 'pr_comparison' or 'specific_activity'; any \
                     other value is refused"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "compare_activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Activity to compare against; required for 'specific_activity'".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, Some(vec!["activity_id".to_owned()]));
        answers_with::<Formatted<CompareActivitiesResult>>(task_capable(tool_definition(
            "compare_activities",
            "Compare an activity against similar activities, personal bests, or a specific other activity",
            schema,
            annotations("Compare activities", Effect::ReadOnly, Reach::AthleteAccount),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "compare_activities");
            handler_bridge::map_universal_response(
                "compare_activities",
                inner::handle_compare_activities(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GenerateRecommendationsTool - Personalized training recommendations
// ============================================================================

/// Tool for generating personalized training recommendations based on recent activity.
#[derive(Debug)]
pub struct GenerateRecommendationsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GenerateRecommendationsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "recommendation_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "'all' (default), 'training_plan', 'recovery', 'intensity', 'goal_specific' \
                     or 'nutrition'; any other value is refused."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);
        answers_with::<Formatted<RecommendationsResult>>(task_capable(tool_definition(
            "generate_recommendations",
            "Generate personalized training recommendations. `training_plan`, `recovery` and \
             `all` advise on CTL, ATL and TSB as analyze_training_load reports them. In \
             `nutrition` mode an athlete with a medical/PAR-Q flag on file gets foods and timing \
             with no amounts, plus `figures_withheld` (their clinician sets those); the session \
             is still described. Calories burned: the device's figure, else Keytel's heart-rate \
             estimate from the saved weight, age and sex, else null with `calories_unavailable` \
             saying why.",
            schema,
            annotations(
                "Generate training recommendations",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "generate_recommendations");
            handler_bridge::map_universal_response(
                "generate_recommendations",
                inner::handle_generate_recommendations(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// PredictPerformanceTool - Predict future performance
// ============================================================================

/// Tool for predicting future performance based on training history.
#[derive(Debug)]
pub struct PredictPerformanceTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for PredictPerformanceTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "target_sport".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Only running is modelled: 'Run' (the default) or another running label; any \
                     other sport is refused."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);
        answers_with::<Formatted<RacePredictionResult>>(task_capable(tool_definition(
            "predict_performance",
            "Predict 5K, 10K, half-marathon and marathon times (VDOT, Daniels) from the \
             fastest-paced recent run of 3 km or more that took under 2 hours, returned under \
             `best_performance`. Running only.",
            schema,
            annotations(
                "Predict race times",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let executor = UniversalExecutor::new(context.resources.clone());
            let request =
                handler_bridge::build_universal_request(&context, args, "predict_performance");
            handler_bridge::map_universal_response(
                "predict_performance",
                inner::handle_predict_performance(&executor, request).await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Create all analytics tools for registration
#[must_use]
pub fn create_analytics_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(AnalyzeActivityTool),
        Box::new(GetActivityIntelligenceTool),
        Box::new(CalculateMetricsTool),
        Box::new(AnalyzePerformanceTrendsTool),
        Box::new(CompareActivitiesTool),
        Box::new(AnalyzeTrainingLoadTool),
        Box::new(DetectPatternsTool),
        Box::new(CalculateFitnessScoreTool),
        Box::new(AnalyzeWeatherImpactTool),
        Box::new(GenerateRecommendationsTool),
        Box::new(PredictPerformanceTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(AnalyzeActivityTool => empty);
crate::declare_security!(AnalyzePerformanceTrendsTool => empty);
crate::declare_security!(AnalyzeTrainingLoadTool => empty);
crate::declare_security!(AnalyzeWeatherImpactTool => empty);
crate::declare_security!(CalculateFitnessScoreTool => empty);
crate::declare_security!(CalculateMetricsTool => empty);
crate::declare_security!(CompareActivitiesTool => empty);
crate::declare_security!(DetectPatternsTool => empty);
crate::declare_security!(GenerateRecommendationsTool => empty);
crate::declare_security!(GetActivityIntelligenceTool => empty);
crate::declare_security!(PredictPerformanceTool => empty);
