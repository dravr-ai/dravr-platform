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
//! - `CalculateMetricsTool` - Calculate pace, speed, intensity, efficiency
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

// The training-load payload builder is reachable from integration tests: this
// crate keeps tests external, so content coverage of the JSON the model reads
// needs the builder public. Its production caller is the tool handler below.
pub use inner::{analyze_detailed_training_load, UserPhysiologicalParams};

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use crate::capabilities::{PROVIDER_ANALYTICS, PROVIDER_READ};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, task_capable, tool_definition, tool_result_to_response,
};
use crate::implementations::fitness_support::process_activity_analysis;
use crate::implementations::handler_bridge;
use crate::protocol::auth::AuthService;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::environment::default_provider;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_fitness_compute::weather::{analyze_weather_impact, build_provider};
use pierre_fitness_compute::weather_cache_adapter::WeatherCacheRepoAdapter;
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_providers::core::FitnessProvider;
use pierre_tools_core::ToolResult;
use pierre_weather::WeatherQuery;

/// Annotations shared by all analytics tools: read-only, idempotent, open-world (external provider)
fn analytics_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

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
                "provider": provider_name
            }))
        })
}
// ============================================================================
// AnalyzeTrainingLoadTool - Calculate CTL/ATL/TSB
// ============================================================================

/// Tool for analyzing training load using CTL/ATL/TSB metrics.
pub struct AnalyzeTrainingLoadTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeTrainingLoadTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava'). Defaults to configured provider."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "days".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Number of days of history to analyze. Default: 42 (6 weeks).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional sleep/recovery provider (e.g., 'whoop', 'garmin'). If specified, factors recovery data into training load analysis.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        task_capable(tool_definition(
            "analyze_training_load",
            "Analyze training load using CTL (chronic training load), ATL (acute training load), and TSB (training stress balance) metrics to assess fitness, fatigue, and form",
            schema,
            Some(analytics_annotations()),
        ))
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
pub struct DetectPatternsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DetectPatternsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
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
        properties.insert(
            "weeks".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Number of weeks to analyze for patterns. Default: 4.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        task_capable(tool_definition(
            "detect_patterns",
            "Detect training patterns including hard/easy day balance, weekly schedule consistency, volume progression, and overtraining warning signs",
            schema,
            Some(analytics_annotations()),
        ))
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
pub struct CalculateFitnessScoreTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CalculateFitnessScoreTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
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
        properties.insert(
            "sleep_provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional sleep/recovery provider (e.g., 'whoop', 'garmin'). If specified, factors recovery quality into fitness score.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        task_capable(tool_definition(
            "calculate_fitness_score",
            "Calculate an overall fitness score (0-100) based on training consistency, CTL, training volume, and recovery balance",
            schema,
            Some(analytics_annotations()),
        ))
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
// AnalyzeWeatherImpactTool - Analyze weather conditions for an activity
// ============================================================================

/// Conversion factor from Celsius to Fahrenheit: F = C * 1.8 + 32
const CELSIUS_TO_FAHRENHEIT_FACTOR: f64 = 1.8;
/// Offset added after scaling for Celsius to Fahrenheit conversion
const FAHRENHEIT_OFFSET: f64 = 32.0;
/// Conversion factor from km/h to mph
const KMH_TO_MPH_FACTOR: f32 = 0.621_371;

/// Tool for analyzing how weather conditions affected activity performance.
pub struct AnalyzeWeatherImpactTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeWeatherImpactTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the activity to analyze".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava'). Defaults to configured provider."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "units".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Temperature and distance units: 'metric' (default) or 'imperial'.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["activity_id".to_owned()]));
        task_capable(tool_definition(
            "analyze_weather_impact",
            "Analyze how weather conditions affected activity performance, including temperature, humidity, wind, and precipitation impact",
            schema,
            Some(analytics_annotations()),
        ))
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let provider_name = if let Some(p) = args
            .get("provider")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            p.to_owned()
        } else if let Some(env_p) = default_provider() {
            env_p
        } else {
            let tenant = context.tenant_id.map(TenantId::from_uuid);
            match context
                .resources
                .repos()
                .provider_connections
                .resolve_most_recent(context.user_id, tenant)
                .await
            {
                Ok(Some(conn)) => conn.provider,
                Ok(None) | Err(_) => {
                    return Ok(ToolResult::error(json!({
                        "error": "No fitness provider connected. Connect Strava, Garmin, or another provider before asking for activity data.",
                        "auth_required_provider": "sciotte"
                    })));
                }
            }
        };

        let units = args
            .get("units")
            .and_then(Value::as_str)
            .unwrap_or("metric");

        let provider = match create_provider(&context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        let activity = match provider.get_activity(activity_id).await {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch activity: {e}"),
                    "activity_id": activity_id,
                    "provider": provider_name
                })));
            }
        };

        // GPS location is needed for weather lookup
        let (Some(latitude), Some(longitude)) =
            (activity.start_latitude(), activity.start_longitude())
        else {
            return Ok(ToolResult::ok(json!({
                "activity_id": activity_id,
                "activity_name": activity.name(),
                "weather": null,
                "impact": null,
                "note": "Activity has no GPS location data — weather analysis requires start coordinates",
                "units": units
            })));
        };

        let cache_repo = context.resources.repos().weather_cache.clone();
        let cache_store = Arc::new(WeatherCacheRepoAdapter::new(cache_repo));
        let provider = build_provider(cache_store);

        let weather = match provider
            .weather_at(WeatherQuery {
                latitude,
                longitude,
                timestamp: activity.start_date(),
            })
            .await
        {
            Ok(sample) => sample,
            Err(pierre_weather::WeatherError::Disabled) => {
                return Ok(ToolResult::ok(json!({
                    "activity_id": activity_id,
                    "activity_name": activity.name(),
                    "weather": null,
                    "impact": null,
                    "note": "Weather provider is disabled by configuration",
                    "units": units
                })));
            }
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Weather lookup failed: {e}"),
                    "activity_id": activity_id,
                    "provider": provider_name
                })));
            }
        };

        let impact = analyze_weather_impact(&weather);

        let weather_json = if units == "imperial" {
            json!({
                "temperature_fahrenheit": f64::from(weather.temperature_celsius).mul_add(CELSIUS_TO_FAHRENHEIT_FACTOR, FAHRENHEIT_OFFSET).round(),
                "humidity_percentage": weather.humidity_percentage,
                "wind_speed_mph": weather.wind_speed_kmh.map(|w| (w * KMH_TO_MPH_FACTOR * 10.0).round() / 10.0),
                "conditions": weather.conditions
            })
        } else {
            json!({
                "temperature_celsius": weather.temperature_celsius,
                "humidity_percentage": weather.humidity_percentage,
                "wind_speed_kmh": weather.wind_speed_kmh,
                "conditions": weather.conditions
            })
        };

        info!(
            "Weather impact analysis for activity {}: {:?}",
            activity_id, impact.difficulty_level
        );

        Ok(ToolResult::ok(json!({
            "activity_id": activity_id,
            "activity_name": activity.name(),
            "weather": weather_json,
            "impact": {
                "difficulty_level": impact.difficulty_level,
                "impact_factors": impact.impact_factors,
                "performance_adjustment": impact.performance_adjustment
            },
            "units": units
        })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AnalyzeActivityTool - Deep analysis of individual activities
// ============================================================================

/// Tool for performing deep analysis of an individual activity.
pub struct AnalyzeActivityTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeActivityTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider name (e.g., 'strava', 'fitbit')".to_owned()),
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
        let schema = object_schema(
            properties,
            Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        );
        task_capable(tool_definition(
            "analyze_activity",
            "Perform deep analysis of an individual activity including insights, metrics, and anomaly detection",
            schema,
            Some(analytics_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_ANALYTICS)
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
pub struct GetActivityIntelligenceTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetActivityIntelligenceTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider name (e.g., 'strava'). Defaults to configured provider."
                        .to_owned(),
                ),
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
        let schema = object_schema(
            properties,
            Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        );
        task_capable(tool_definition(
            "get_activity_intelligence",
            "Get AI-powered intelligence insights and recommendations for a specific activity",
            schema,
            Some(analytics_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_ANALYTICS)
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
// CalculateMetricsTool - Calculate pace, speed, intensity, efficiency
// ============================================================================

/// Tool for calculating custom fitness metrics from activity data.
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
                    "Maximum heart rate (optional, used for intensity calculation)".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "age".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "User age (optional, used to estimate max HR via Fox formula if max_hr not provided)"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        );
        task_capable(tool_definition(
            "calculate_metrics",
            "Calculate advanced fitness metrics for an activity (pace, speed, intensity score, efficiency)",
            schema,
            Some(analytics_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_ANALYTICS)
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
pub struct AnalyzePerformanceTrendsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzePerformanceTrendsTool {
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
            "metric".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Metric to analyze: 'pace', 'speed', 'heart_rate', 'distance', 'duration', 'elevation', 'power'"
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
                    "Time period: 'week', 'month' (default), 'quarter', 'year'".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["provider".to_owned(), "metric".to_owned()]),
        );
        task_capable(tool_definition(
            "analyze_performance_trends",
            "Analyze performance trends over time with statistical analysis and insights for a specific metric",
            schema,
            Some(analytics_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_ANALYTICS)
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
pub struct CompareActivitiesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CompareActivitiesTool {
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
                description: Some("Primary activity to compare".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "comparison_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Type of comparison: 'similar_activities' (default), 'pr_comparison', 'specific_activity'"
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
                    "Activity ID to compare against (required for 'specific_activity' type)"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        );
        task_capable(tool_definition(
            "compare_activities",
            "Compare an activity against similar activities, personal bests, or a specific other activity",
            schema,
            Some(analytics_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_ANALYTICS)
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
pub struct GenerateRecommendationsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GenerateRecommendationsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava'). Defaults to configured provider."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "recommendation_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Type of recommendations to generate: 'all' (default), 'training_plan', 'recovery', 'intensity', 'goal_specific', or 'nutrition'.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        task_capable(tool_definition(
            "generate_recommendations",
            "Generate personalized training recommendations",
            schema,
            Some(analytics_annotations()),
        ))
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
pub struct PredictPerformanceTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for PredictPerformanceTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava'). Defaults to configured provider."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "target_sport".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Target sport for performance prediction (e.g., 'Run', 'Ride', 'Swim'). Default: 'Run'.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        task_capable(tool_definition(
            "predict_performance",
            "Predict future performance based on training",
            schema,
            Some(analytics_annotations()),
        ))
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
crate::declare_security!(AnalyzeActivityTool => UNTRUSTED_OUTPUT);
crate::declare_security!(AnalyzePerformanceTrendsTool => empty);
crate::declare_security!(AnalyzeTrainingLoadTool => empty);
crate::declare_security!(AnalyzeWeatherImpactTool => UNTRUSTED_OUTPUT);
crate::declare_security!(CalculateFitnessScoreTool => empty);
crate::declare_security!(CalculateMetricsTool => empty);
crate::declare_security!(CompareActivitiesTool => UNTRUSTED_OUTPUT);
crate::declare_security!(DetectPatternsTool => empty);
crate::declare_security!(GenerateRecommendationsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetActivityIntelligenceTool => UNTRUSTED_OUTPUT);
crate::declare_security!(PredictPerformanceTool => empty);
