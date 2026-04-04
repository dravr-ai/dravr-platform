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
//!
//! These tools use the intelligence module directly for efficient analysis.

use std::cmp::Ordering;
use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use tracing::info;

use crate::config::environment::default_provider;
use crate::constants::limits::METERS_PER_KILOMETER;
use crate::errors::AppResult;
use crate::intelligence::physiological_constants::heart_rate::{
    AGE_BASED_MAX_HR_CONSTANT, HIGH_INTENSITY_HR_THRESHOLD,
};
use crate::intelligence::physiological_constants::hr_estimation::ASSUMED_MAX_HR;
use crate::intelligence::weather::WeatherService;
use crate::intelligence::{
    MetricType, PatternDetector, RiskLevel, SafeMetricExtractor, StatisticalAnalyzer,
    TrainingLoadCalculator, TrainingStatus, TrendDataPoint, TrendDirection,
};
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::models::Activity;
use crate::protocols::universal::auth_service::AuthService;
use crate::providers::core::{ActivityQueryParams, FitnessProvider};
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

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

/// Fetch activities for a given time period
async fn fetch_activities(
    provider: &dyn FitnessProvider,
    after_timestamp: i64,
    limit: usize,
) -> Result<Vec<Activity>, String> {
    let query_params = ActivityQueryParams {
        limit: Some(limit),
        offset: None,
        before: None,
        after: Some(after_timestamp),
    };

    provider
        .get_activities_with_params(&query_params)
        .await
        .map_err(|e| format!("Failed to fetch activities: {e}"))
}

/// Build pattern detection JSON response
fn build_pattern_response(
    activities: &[Activity],
    weeks: i64,
    provider_name: &str,
) -> serde_json::Value {
    let hard_easy = PatternDetector::detect_hard_easy_pattern(activities);
    let weekly = PatternDetector::detect_weekly_schedule(activities);
    let volume = PatternDetector::detect_volume_progression(activities);
    let overtraining = PatternDetector::detect_overtraining_signals(activities);

    json!({
        "hard_easy_pattern": {
            "pattern_detected": hard_easy.pattern_detected,
            "description": hard_easy.pattern_description,
            "hard_percentage": hard_easy.hard_percentage,
            "easy_percentage": hard_easy.easy_percentage,
            "adequate_recovery": hard_easy.adequate_recovery,
            "recommendation": if hard_easy.adequate_recovery {
                "Good recovery balance between hard and easy days"
            } else if hard_easy.hard_percentage > 50.0 {
                "Too many hard days - add more easy/recovery days"
            } else {
                "Consider adding more intensity to some sessions"
            }
        },
        "weekly_schedule": {
            "most_common_days": weekly.most_common_days.iter()
                .map(|d| format!("{d:?}"))
                .collect::<Vec<_>>(),
            "day_frequencies": weekly.day_frequencies,
            "consistency_score": weekly.consistency_score,
            "avg_activities_per_week": weekly.avg_activities_per_week,
            "recommendation": if weekly.consistency_score > 70.0 {
                "Consistent training schedule - good for adaptation"
            } else {
                "Irregular schedule - try to establish a routine"
            }
        },
        "volume_progression": {
            "trend": format!("{:?}", volume.trend),
            "weekly_volumes_km": volume.weekly_volumes,
            "volume_spikes_detected": volume.volume_spikes_detected,
            "spike_weeks": volume.spike_weeks,
            "recommendation": volume.recommendation
        },
        "overtraining_signals": {
            "risk_level": format!("{:?}", overtraining.risk_level),
            "hr_drift_detected": overtraining.hr_drift_detected,
            "hr_drift_percent": overtraining.hr_drift_percent,
            "performance_decline": overtraining.performance_decline,
            "insufficient_recovery": overtraining.insufficient_recovery,
            "warnings": overtraining.warnings,
            "recommendation": match overtraining.risk_level {
                RiskLevel::Low => "Training load is manageable",
                RiskLevel::Moderate => "Monitor fatigue levels closely",
                RiskLevel::High => "Take rest - high risk of overtraining",
            }
        },
        "analysis_summary": {
            "weeks_analyzed": weeks,
            "activities_analyzed": activities.len(),
            "provider": provider_name
        }
    })
}

/// Calculate fitness score components and build response
fn build_fitness_score_response(activities: &[Activity], provider_name: &str) -> serde_json::Value {
    let weeks_active = 6.0_f64;
    #[allow(clippy::cast_precision_loss)]
    let avg_per_week = activities.len() as f64 / weeks_active;
    let consistency_score = (avg_per_week / 5.0 * 100.0).min(100.0);

    // Training load score (using CTL)
    let calculator = TrainingLoadCalculator::new();
    let load_score = calculator
        .calculate_ctl(activities, None, None, None, None, None)
        .map_or(0.0, |ctl| (ctl / 100.0 * 100.0).clamp(0.0, 100.0));

    // Volume score (duration_seconds is u64, acceptable precision loss for hour conversion)
    #[allow(clippy::cast_precision_loss)]
    let total_hours: f64 = activities
        .iter()
        .map(|a| a.duration_seconds() as f64 / 3600.0)
        .sum();
    let avg_hours_per_week = total_hours / weeks_active;
    let volume_score = (avg_hours_per_week / 10.0 * 100.0).min(100.0);

    // Balance score
    let hard_easy = PatternDetector::detect_hard_easy_pattern(activities);
    let balance_score = if hard_easy.adequate_recovery {
        100.0
    } else {
        60.0
    };

    // Calculate overall fitness score using fused multiply-add for accuracy
    let fitness_score = consistency_score
        .mul_add(
            0.25,
            load_score.mul_add(0.35, volume_score.mul_add(0.25, balance_score * 0.15)),
        )
        .round();

    // fitness_score is clamped to 0-100 range, safe for u32 match
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fitness_level = match fitness_score as u32 {
        90..=100 => "Elite",
        75..=89 => "Excellent",
        60..=74 => "Good",
        40..=59 => "Moderate",
        20..=39 => "Developing",
        _ => "Beginner",
    };

    json!({
        "fitness_score": fitness_score,
        "fitness_level": fitness_level,
        "components": {
            "consistency": {
                "score": consistency_score.round(),
                "avg_sessions_per_week": (avg_per_week * 10.0).round() / 10.0,
                "weight": "25%",
                "description": "Based on training frequency"
            },
            "training_load": {
                "score": load_score.round(),
                "weight": "35%",
                "description": "Based on chronic training load (CTL)"
            },
            "volume": {
                "score": volume_score.round(),
                "avg_hours_per_week": (avg_hours_per_week * 10.0).round() / 10.0,
                "weight": "25%",
                "description": "Based on total training volume"
            },
            "balance": {
                "score": balance_score.round(),
                "adequate_recovery": hard_easy.adequate_recovery,
                "weight": "15%",
                "description": "Based on training intensity distribution"
            }
        },
        "analysis_period": {
            "weeks": 6,
            "activities_analyzed": activities.len()
        },
        "provider": provider_name
    })
}

// ============================================================================
// AnalyzeTrainingLoadTool - Calculate CTL/ATL/TSB
// ============================================================================

/// Tool for analyzing training load using CTL/ATL/TSB metrics.
pub struct AnalyzeTrainingLoadTool;

#[async_trait]
impl McpTool for AnalyzeTrainingLoadTool {
    fn name(&self) -> &'static str {
        "analyze_training_load"
    }

    fn description(&self) -> &'static str {
        "Analyze training load using CTL (chronic training load), ATL (acute training load), and TSB (training stress balance) metrics to assess fitness, fatigue, and form"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let days = args
            .get("days")
            .and_then(Value::as_i64)
            .unwrap_or(42)
            .min(180);

        let provider = match create_provider(context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        let after = Utc::now() - Duration::days(days);
        let mut activities = match fetch_activities(provider.as_ref(), after.timestamp(), 500).await
        {
            Ok(acts) => acts,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": e,
                    "provider": provider_name
                })));
            }
        };

        if activities.is_empty() {
            return Ok(ToolResult::ok(json!({
                "message": "No activities found in the analysis period",
                "days_analyzed": days,
                "provider": provider_name
            })));
        }

        // Sort activities oldest-first — EMA calculation requires chronological order
        activities.sort_by_key(Activity::start_date);

        let calculator = TrainingLoadCalculator::new();
        let load =
            match calculator.calculate_training_load(&activities, None, None, None, None, None) {
                Ok(l) => l,
                Err(e) => {
                    return Ok(ToolResult::error(json!({
                        "error": format!("Failed to calculate training load: {e}"),
                        "provider": provider_name
                    })));
                }
            };

        let status = match load.tsb {
            tsb if tsb > 10.0 => TrainingStatus::Detraining,
            tsb if tsb > 0.0 => TrainingStatus::Fresh,
            tsb if tsb > -10.0 => TrainingStatus::Productive,
            _ => TrainingStatus::Overreaching,
        };

        info!(
            "Training load analysis: CTL={:.1}, ATL={:.1}, TSB={:.1}, Status={:?}",
            load.ctl, load.atl, load.tsb, status
        );

        Ok(ToolResult::ok(json!({
            "training_load": {
                "ctl": load.ctl,
                "atl": load.atl,
                "tsb": load.tsb,
                "ctl_description": "Chronic Training Load - your long-term fitness level",
                "atl_description": "Acute Training Load - your recent training stress/fatigue",
                "tsb_description": "Training Stress Balance - your current form (positive = fresh, negative = fatigued)"
            },
            "status": format!("{status:?}"),
            "status_description": match status {
                TrainingStatus::Fresh => "Well rested, ready for hard training or racing",
                TrainingStatus::Productive => "Good balance of fitness and fatigue, optimal training zone",
                TrainingStatus::Overreaching => "High fatigue, prioritize recovery",
                TrainingStatus::Detraining => "Very fresh but risk of fitness loss without training",
            },
            "analysis_period": {
                "days": days,
                "activities_analyzed": activities.len()
            },
            "provider": provider_name
        })))
    }
}

// ============================================================================
// DetectPatternsTool - Detect training patterns
// ============================================================================

/// Tool for detecting training patterns and potential issues.
pub struct DetectPatternsTool;

#[async_trait]
impl McpTool for DetectPatternsTool {
    fn name(&self) -> &'static str {
        "detect_patterns"
    }

    fn description(&self) -> &'static str {
        "Detect training patterns including hard/easy day balance, weekly schedule consistency, volume progression, and overtraining warning signs"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let weeks = args
            .get("weeks")
            .and_then(Value::as_i64)
            .unwrap_or(4)
            .min(12);

        let provider = match create_provider(context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        let after = Utc::now() - Duration::weeks(weeks);
        let activities = match fetch_activities(provider.as_ref(), after.timestamp(), 200).await {
            Ok(acts) => acts,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": e,
                    "provider": provider_name
                })));
            }
        };

        if activities.len() < 3 {
            return Ok(ToolResult::ok(json!({
                "message": "Insufficient activities for pattern detection (need at least 3)",
                "weeks_analyzed": weeks,
                "activities_found": activities.len(),
                "provider": provider_name
            })));
        }

        info!(
            "Pattern detection: {} activities over {} weeks",
            activities.len(),
            weeks
        );

        Ok(ToolResult::ok(build_pattern_response(
            &activities,
            weeks,
            &provider_name,
        )))
    }
}

// ============================================================================
// CalculateFitnessScoreTool - Calculate overall fitness score
// ============================================================================

/// Tool for calculating an overall fitness score.
pub struct CalculateFitnessScoreTool;

#[async_trait]
impl McpTool for CalculateFitnessScoreTool {
    fn name(&self) -> &'static str {
        "calculate_fitness_score"
    }

    fn description(&self) -> &'static str {
        "Calculate an overall fitness score (0-100) based on training consistency, CTL, training volume, and recovery balance"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let provider = match create_provider(context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        let after = Utc::now() - Duration::weeks(6);
        let activities = match fetch_activities(provider.as_ref(), after.timestamp(), 200).await {
            Ok(acts) => acts,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": e,
                    "provider": provider_name
                })));
            }
        };

        if activities.is_empty() {
            return Ok(ToolResult::ok(json!({
                "fitness_score": 0,
                "fitness_level": "No Data",
                "message": "No recent activities to calculate fitness score",
                "provider": provider_name
            })));
        }

        info!(
            "Fitness score calculated for user {} ({} activities)",
            context.user_id,
            activities.len()
        );

        Ok(ToolResult::ok(build_fitness_score_response(
            &activities,
            &provider_name,
        )))
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
impl McpTool for AnalyzeWeatherImpactTool {
    fn name(&self) -> &'static str {
        "analyze_weather_impact"
    }

    fn description(&self) -> &'static str {
        "Analyze how weather conditions affected activity performance, including temperature, humidity, wind, and precipitation impact"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["activity_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let units = args
            .get("units")
            .and_then(Value::as_str)
            .unwrap_or("metric");

        let provider = match create_provider(context, &provider_name).await {
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
        if activity.start_latitude().is_none() || activity.start_longitude().is_none() {
            return Ok(ToolResult::ok(json!({
                "activity_id": activity_id,
                "activity_name": activity.name(),
                "weather": null,
                "impact": null,
                "note": "Activity has no GPS location data — weather analysis requires start coordinates",
                "units": units
            })));
        }

        let mut weather_service = WeatherService::with_default_config();

        let weather = match weather_service
            .get_weather_for_activity(
                activity.start_latitude(),
                activity.start_longitude(),
                activity.start_date(),
            )
            .await
        {
            Ok(Some(w)) => w,
            Ok(None) => {
                return Ok(ToolResult::ok(json!({
                    "activity_id": activity_id,
                    "activity_name": activity.name(),
                    "weather": null,
                    "impact": null,
                    "note": "Weather data unavailable — the weather API may be disabled or unconfigured",
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

        let impact = weather_service.analyze_weather_impact(&weather);

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
}

// ============================================================================
// AnalyzeActivityTool - Deep analysis of individual activities
// ============================================================================

/// Tool for performing deep analysis of an individual activity.
pub struct AnalyzeActivityTool;

#[async_trait]
impl McpTool for AnalyzeActivityTool {
    fn name(&self) -> &'static str {
        "analyze_activity"
    }

    fn description(&self) -> &'static str {
        "Perform deep analysis of an individual activity including insights, metrics, and anomaly detection"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let provider = match create_provider(context, &provider_name).await {
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

        let (insights, recommendations) = generate_activity_insights(&activity);

        // Duration conversion: u64 -> f64 safe for realistic durations
        #[allow(clippy::cast_precision_loss)]
        let duration_minutes = activity.duration_seconds() as f64 / 60.0;

        info!(
            "Activity analysis for {}: {} insights generated",
            activity_id,
            insights.len()
        );

        Ok(ToolResult::ok(json!({
            "activity_id": activity_id,
            "activity_name": activity.name(),
            "activity_type": format!("{:?}", activity.sport_type()),
            "analysis": {
                "insights": insights,
                "recommendations": recommendations,
                "performance_metrics": {
                    "distance_km": activity.distance_meters().map(|d| d / METERS_PER_KILOMETER),
                    "duration_minutes": duration_minutes,
                    "elevation_meters": activity.elevation_gain(),
                    "average_heart_rate": activity.average_heart_rate(),
                    "max_heart_rate": activity.max_heart_rate(),
                    "calories": activity.calories()
                }
            },
            "provider": provider_name
        })))
    }
}

// ============================================================================
// GetActivityIntelligenceTool - AI-powered activity insights
// ============================================================================

/// Tool for getting AI-powered intelligence insights for an activity.
pub struct GetActivityIntelligenceTool;

#[async_trait]
impl McpTool for GetActivityIntelligenceTool {
    fn name(&self) -> &'static str {
        "get_activity_intelligence"
    }

    fn description(&self) -> &'static str {
        "Get AI-powered intelligence insights and recommendations for a specific activity"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let provider = match create_provider(context, &provider_name).await {
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

        let (insights, recommendations) = generate_activity_insights(&activity);

        let summary = format!(
            "{:?} activity completed. {} insights generated.",
            activity.sport_type(),
            insights.len()
        );

        #[allow(clippy::cast_precision_loss)]
        let duration_minutes = activity.duration_seconds() as f64 / 60.0;

        info!("Activity intelligence for {}: {}", activity_id, summary);

        Ok(ToolResult::ok(json!({
            "activity_id": activity_id,
            "activity_type": format!("{:?}", activity.sport_type()),
            "intelligence": {
                "summary": summary,
                "insights": insights,
                "recommendations": recommendations,
                "performance_metrics": {
                    "distance_km": activity.distance_meters().map(|d| d / METERS_PER_KILOMETER),
                    "duration_minutes": duration_minutes,
                    "elevation_meters": activity.elevation_gain(),
                    "average_heart_rate": activity.average_heart_rate(),
                    "max_heart_rate": activity.max_heart_rate(),
                    "calories": activity.calories()
                }
            },
            "provider": provider_name
        })))
    }
}

// ============================================================================
// CalculateMetricsTool - Calculate pace, speed, intensity, efficiency
// ============================================================================

/// Conversion factor from m/s to km/h
const MS_TO_KMH: f64 = 3.6;

/// Tool for calculating custom fitness metrics from activity data.
pub struct CalculateMetricsTool;

#[async_trait]
impl McpTool for CalculateMetricsTool {
    fn name(&self) -> &'static str {
        "calculate_metrics"
    }

    fn description(&self) -> &'static str {
        "Calculate advanced fitness metrics for an activity (pace, speed, intensity score, efficiency)"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let max_hr_provided = args.get("max_hr").and_then(Value::as_f64);
        let user_age = args
            .get("age")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok());

        let provider = match create_provider(context, &provider_name).await {
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

        let distance = activity.distance_meters().unwrap_or(0.0);
        let duration = activity.duration_seconds();
        let elevation_gain = activity.elevation_gain().unwrap_or(0.0);
        let heart_rate = activity.average_heart_rate();

        // Determine max HR: explicit > age-based > default
        let (max_hr, max_hr_source) = match (max_hr_provided, user_age) {
            (Some(hr), _) => (hr, "provided"),
            (None, Some(age)) => (
                f64::from(AGE_BASED_MAX_HR_CONSTANT.saturating_sub(age)),
                "age_based",
            ),
            (None, None) => (ASSUMED_MAX_HR, "default_assumed"),
        };

        // Safe duration conversion for division
        #[allow(clippy::cast_precision_loss)]
        let duration_f64 = duration as f64;

        let pace = if distance > 0.0 && duration > 0 {
            duration_f64 / (distance / METERS_PER_KILOMETER)
        } else {
            0.0
        };

        let speed = if duration > 0 {
            (distance / duration_f64) * MS_TO_KMH
        } else {
            0.0
        };

        let intensity_score = if max_hr > 0.0 {
            heart_rate.map_or(0.0, |hr| (f64::from(hr) / max_hr) * 100.0)
        } else {
            0.0
        };

        let efficiency_score = if distance > 0.0 && elevation_gain > 0.0 {
            (distance / elevation_gain).min(100.0)
        } else {
            50.0
        };

        info!(
            "Metrics calculated for activity {}: pace={:.2}, speed={:.2}",
            activity_id, pace, speed
        );

        Ok(ToolResult::ok(json!({
            "activity_id": activity_id,
            "metrics": {
                "pace_min_per_km": pace,
                "speed_kmh": speed,
                "intensity_score": intensity_score,
                "efficiency_score": efficiency_score,
                "max_hr_used": max_hr,
                "max_hr_source": max_hr_source
            },
            "activity_summary": {
                "distance_km": distance / METERS_PER_KILOMETER,
                "duration_seconds": duration,
                "elevation_meters": elevation_gain,
                "average_heart_rate": heart_rate
            },
            "provider": provider_name
        })))
    }
}

// ============================================================================
// AnalyzePerformanceTrendsTool - Track metric trends over time
// ============================================================================

/// Tool for analyzing performance trends over time with statistical analysis.
pub struct AnalyzePerformanceTrendsTool;

#[async_trait]
impl McpTool for AnalyzePerformanceTrendsTool {
    fn name(&self) -> &'static str {
        "analyze_performance_trends"
    }

    fn description(&self) -> &'static str {
        "Analyze performance trends over time with statistical analysis and insights for a specific metric"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned(), "metric".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let metric = args.get("metric").and_then(Value::as_str).unwrap_or("pace");

        let timeframe = args
            .get("timeframe")
            .and_then(Value::as_str)
            .unwrap_or("month");

        let provider = match create_provider(context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        // Fetch activities within the timeframe
        let cutoff = match timeframe {
            "week" => Utc::now() - Duration::days(7),
            "quarter" => Utc::now() - Duration::days(90),
            "year" => Utc::now() - Duration::days(365),
            _ => Utc::now() - Duration::days(30),
        };

        let activities = match fetch_activities(provider.as_ref(), cutoff.timestamp(), 200).await {
            Ok(acts) => acts,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": e,
                    "provider": provider_name
                })));
            }
        };

        if activities.len() < 2 {
            return Ok(ToolResult::ok(json!({
                "metric": metric,
                "timeframe": timeframe,
                "trend": "needs_more_data",
                "activities_analyzed": activities.len(),
                "insights": [format!("Need at least 2 activities for trend analysis. Found {}.", activities.len())],
                "provider": provider_name
            })));
        }

        // Parse metric type
        let metric_type = match metric.to_lowercase().as_str() {
            "pace" => MetricType::Pace,
            "speed" => MetricType::Speed,
            "heart_rate" | "hr" => MetricType::HeartRate,
            "distance" => MetricType::Distance,
            "duration" => MetricType::Duration,
            "elevation" => MetricType::Elevation,
            "power" => MetricType::Power,
            other => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Unknown metric type: {other}"),
                    "supported_metrics": ["pace", "speed", "heart_rate", "distance", "duration", "elevation", "power"]
                })));
            }
        };

        // Extract metric values using SafeMetricExtractor
        let data_points = match SafeMetricExtractor::extract_metric_values(&activities, metric_type)
        {
            Ok(pts) if pts.len() >= 2 => pts,
            _ => {
                return Ok(ToolResult::ok(json!({
                    "metric": metric,
                    "timeframe": timeframe,
                    "trend": "insufficient_data",
                    "activities_analyzed": activities.len(),
                    "insights": [format!("Metric '{metric}' not available in enough activities")],
                    "provider": provider_name
                })));
            }
        };

        // Convert to TrendDataPoint and perform regression
        let trend_data: Vec<TrendDataPoint> = data_points
            .iter()
            .map(|(date, value)| TrendDataPoint {
                date: *date,
                value: *value,
                smoothed_value: None,
            })
            .collect();

        let Ok(regression) = StatisticalAnalyzer::linear_regression(&trend_data) else {
            return Ok(ToolResult::ok(json!({
                "metric": metric,
                "timeframe": timeframe,
                "trend": "calculation_error",
                "activities_analyzed": data_points.len(),
                "insights": ["Unable to calculate trend statistics"],
                "provider": provider_name
            })));
        };

        let slope_threshold = 0.01;
        let trend_direction = StatisticalAnalyzer::determine_trend_direction(
            &regression,
            metric_type.is_lower_better(),
            slope_threshold,
        );

        let trend_label = match trend_direction {
            TrendDirection::Improving => "improving",
            TrendDirection::Stable => "stable",
            TrendDirection::Declining => "declining",
        };

        // Percentage change from first to last data point
        let percent_change = if data_points.len() >= 2 {
            let first = data_points.first().map_or(0.0, |(_, v)| *v);
            let last = data_points.last().map_or(0.0, |(_, v)| *v);
            if first.abs() > f64::EPSILON {
                Some(((last - first) / first) * 100.0)
            } else {
                None
            }
        } else {
            None
        };

        info!(
            "Performance trend for {}: {} ({} data points)",
            metric,
            trend_label,
            data_points.len()
        );

        Ok(ToolResult::ok(json!({
            "metric": metric,
            "timeframe": timeframe,
            "trend": trend_label,
            "activities_analyzed": data_points.len(),
            "statistics": {
                "slope": regression.slope,
                "r_squared": regression.r_squared,
                "correlation": regression.correlation,
                "standard_error": regression.standard_error,
                "p_value": regression.p_value,
                "percent_change": percent_change,
                "start_value": data_points.first().map(|(_, v)| v),
                "end_value": data_points.last().map(|(_, v)| v)
            },
            "provider": provider_name
        })))
    }
}

// ============================================================================
// CompareActivitiesTool - Compare activities against similar/PRs
// ============================================================================

/// Tool for comparing an activity against similar activities or personal records.
pub struct CompareActivitiesTool;

#[async_trait]
impl McpTool for CompareActivitiesTool {
    fn name(&self) -> &'static str {
        "compare_activities"
    }

    fn description(&self) -> &'static str {
        "Compare an activity against similar activities, personal bests, or a specific other activity"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned(), "activity_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(analytics_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .map_or_else(default_provider, String::from);

        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let comparison_type = args
            .get("comparison_type")
            .and_then(Value::as_str)
            .unwrap_or("similar_activities");

        let compare_activity_id = args.get("compare_activity_id").and_then(Value::as_str);

        let provider = match create_provider(context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        // Fetch target activity
        let target = match provider.get_activity(activity_id).await {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch activity: {e}"),
                    "activity_id": activity_id,
                    "provider": provider_name
                })));
            }
        };

        // Fetch recent activities for comparison
        let all_activities = provider
            .get_activities(Some(50), None)
            .await
            .unwrap_or_default();

        let comparison = match comparison_type {
            "pr_comparison" => build_pr_comparison(&target, &all_activities),
            "specific_activity" => compare_activity_id.map_or_else(
                || build_similar_comparison(&target, &all_activities),
                |cmp_id| build_specific_comparison(&target, &all_activities, cmp_id),
            ),
            _ => build_similar_comparison(&target, &all_activities),
        };

        info!(
            "Activity comparison for {}: type={}",
            activity_id, comparison_type
        );

        Ok(ToolResult::ok(comparison))
    }
}

/// Generate insights and recommendations from activity data
fn generate_activity_insights(activity: &Activity) -> (Vec<String>, Vec<&'static str>) {
    let mut insights = Vec::new();
    let mut recommendations = Vec::new();

    if let Some(distance) = activity.distance_meters() {
        let km = distance / METERS_PER_KILOMETER;
        insights.push(format!("Activity covered {km:.2} km"));
        if km > 20.0 {
            recommendations.push("Great long-distance effort! Ensure proper recovery time");
        }
    }

    if let Some(elevation) = activity.elevation_gain() {
        insights.push(format!("Total elevation gain: {elevation:.0} meters"));
        if elevation > 500.0 {
            recommendations.push("Significant elevation - consider targeted hill training");
        }
    }

    if let Some(avg_hr) = activity.average_heart_rate() {
        insights.push(format!("Average heart rate: {avg_hr} bpm"));
        if avg_hr > HIGH_INTENSITY_HR_THRESHOLD {
            recommendations.push("High-intensity effort detected - monitor recovery");
        }
    }

    if let Some(calories) = activity.calories() {
        insights.push(format!("Calories burned: {calories}"));
    }

    (insights, recommendations)
}

/// Build comparison with similar activities (same sport, similar distance)
fn build_similar_comparison(target: &Activity, all_activities: &[Activity]) -> Value {
    let similar: Vec<&Activity> = all_activities
        .iter()
        .filter(|a| {
            a.id() != target.id()
                && a.sport_type() == target.sport_type()
                && is_similar_distance(a.distance_meters(), target.distance_meters())
        })
        .take(5)
        .collect();

    if similar.is_empty() {
        return json!({
            "activity_id": target.id(),
            "comparison_type": "similar_activities",
            "comparison_count": 0,
            "insights": ["No similar activities found for comparison"]
        });
    }

    let mut comparisons = Vec::new();
    let mut insights = Vec::new();

    // Pace comparison
    let target_pace = activity_pace(target);
    let avg_pace = similar
        .iter()
        .filter_map(|a| activity_pace(a))
        .collect::<Vec<_>>();

    if let Some(tp) = target_pace {
        if !avg_pace.is_empty() {
            #[allow(clippy::cast_precision_loss)]
            let avg_p = avg_pace.iter().sum::<f64>() / avg_pace.len() as f64;
            if avg_p > 0.0 {
                let diff_pct = ((tp - avg_p) / avg_p) * 100.0;
                comparisons.push(json!({
                    "metric": "pace",
                    "current": tp,
                    "average": avg_p,
                    "difference_percent": diff_pct,
                    "improved": diff_pct < 0.0
                }));
                if diff_pct < -5.0 {
                    insights.push(format!(
                        "Pace improved by {:.1}% vs similar activities",
                        diff_pct.abs()
                    ));
                } else if diff_pct > 5.0 {
                    insights.push(format!(
                        "Pace was {diff_pct:.1}% slower than similar activities"
                    ));
                }
            }
        }
    }

    if insights.is_empty() {
        insights.push(format!(
            "Compared with {} similar activities",
            similar.len()
        ));
    }

    json!({
        "activity_id": target.id(),
        "comparison_type": "similar_activities",
        "sport_type": format!("{:?}", target.sport_type()),
        "comparison_count": similar.len(),
        "comparisons": comparisons,
        "insights": insights
    })
}

/// Build comparison with personal records
fn build_pr_comparison(target: &Activity, all_activities: &[Activity]) -> Value {
    let same_sport: Vec<&Activity> = all_activities
        .iter()
        .filter(|a| a.sport_type() == target.sport_type())
        .collect();

    if same_sport.is_empty() {
        return json!({
            "activity_id": target.id(),
            "comparison_type": "pr_comparison",
            "insights": ["No other activities of this sport type found"]
        });
    }

    let mut pr_comparisons = Vec::new();
    let mut insights = Vec::new();

    // Distance PR
    if let Some(distance) = target.distance_meters() {
        let max_distance = same_sport
            .iter()
            .filter_map(|a| a.distance_meters())
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        if let Some(max_d) = max_distance {
            let is_pr = distance >= max_d;
            pr_comparisons.push(json!({
                "metric": "distance",
                "current": distance,
                "personal_record": max_d,
                "is_record": is_pr,
                "percent_of_pr": if max_d > 0.0 { (distance / max_d) * 100.0 } else { 0.0 }
            }));
            if is_pr && (distance - max_d).abs() > 100.0 {
                insights.push("New distance PR!".to_owned());
            }
        }
    }

    // Pace PR
    let target_pace = activity_pace(target);
    let best_pace = same_sport
        .iter()
        .filter_map(|a| activity_pace(a))
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    if let (Some(tp), Some(bp)) = (target_pace, best_pace) {
        let is_pr = tp <= bp;
        pr_comparisons.push(json!({
            "metric": "pace",
            "current": tp,
            "personal_record": bp,
            "is_record": is_pr
        }));
        if is_pr && (bp - tp).abs() > 0.1 {
            insights.push("New pace PR!".to_owned());
        }
    }

    if insights.is_empty() {
        insights.push(format!(
            "Compared with {} activities in this sport",
            same_sport.len()
        ));
    }

    json!({
        "activity_id": target.id(),
        "comparison_type": "pr_comparison",
        "sport_type": format!("{:?}", target.sport_type()),
        "pr_comparisons": pr_comparisons,
        "insights": insights
    })
}

/// Build comparison with a specific activity
fn build_specific_comparison(
    target: &Activity,
    all_activities: &[Activity],
    compare_id: &str,
) -> Value {
    let compare = all_activities.iter().find(|a| a.id() == compare_id);

    let Some(cmp) = compare else {
        return json!({
            "activity_id": target.id(),
            "comparison_type": "specific_activity",
            "error": format!("Activity '{compare_id}' not found"),
            "insights": [format!("Could not find activity '{compare_id}' for comparison")]
        });
    };

    let mut comparisons = Vec::new();
    let mut insights = Vec::new();

    // Pace comparison
    if let (Some(tp), Some(cp)) = (activity_pace(target), activity_pace(cmp)) {
        if cp > 0.0 {
            let diff = ((tp - cp) / cp) * 100.0;
            comparisons.push(json!({
                "metric": "pace",
                "current": tp,
                "comparison": cp,
                "difference_percent": diff,
                "improved": diff < 0.0
            }));
            if diff < -5.0 {
                insights.push(format!("Pace improved by {:.1}%", diff.abs()));
            } else if diff > 5.0 {
                insights.push(format!("Pace was {diff:.1}% slower"));
            }
        }
    }

    // Heart rate comparison
    if let (Some(th), Some(ch)) = (
        target.average_heart_rate().map(f64::from),
        cmp.average_heart_rate().map(f64::from),
    ) {
        if ch > 0.0 {
            let diff = ((th - ch) / ch) * 100.0;
            comparisons.push(json!({
                "metric": "heart_rate",
                "current": th,
                "comparison": ch,
                "difference_percent": diff,
                "improved": diff < 0.0
            }));
        }
    }

    if insights.is_empty() {
        insights.push("Metrics are similar to the comparison activity".to_owned());
    }

    json!({
        "activity_id": target.id(),
        "comparison_type": "specific_activity",
        "comparison_activity_id": compare_id,
        "comparison_activity_name": cmp.name(),
        "sport_type": format!("{:?}", target.sport_type()),
        "comparisons": comparisons,
        "insights": insights
    })
}

/// Calculate pace in min/km for an activity
fn activity_pace(activity: &Activity) -> Option<f64> {
    if let Some(distance) = activity.distance_meters() {
        if distance > 0.0 && activity.duration_seconds() > 0 {
            #[allow(clippy::cast_precision_loss)]
            let seconds_per_km =
                (activity.duration_seconds() as f64 / distance) * METERS_PER_KILOMETER;
            return Some(seconds_per_km / 60.0);
        }
    }
    None
}

/// Check if two distances are similar (within 10%)
fn is_similar_distance(dist1: Option<f64>, dist2: Option<f64>) -> bool {
    match (dist1, dist2) {
        (Some(d1), Some(d2)) if d2 > 0.0 => (d1 / d2 - 1.0).abs() < 0.1,
        _ => false,
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all analytics tools for registration
#[must_use]
pub fn create_analytics_tools() -> Vec<Box<dyn McpTool>> {
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
    ]
}
