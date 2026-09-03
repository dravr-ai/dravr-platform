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
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, FixedOffset, Utc};
use num_traits::ToPrimitive;
use pierre_database::database::repositories::ProfileRepository;
use serde_json::{from_value, json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::capabilities::{ToolCapabilities, PROVIDER_READ};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, task_capable, tool_definition, tool_result_to_response,
};
use crate::protocol::auth::AuthService;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::constants::defaults::DEFAULT_GOAL_TIMEFRAME_DAYS;
use pierre_config::constants::goal_management::MIN_ACTIVITIES_FOR_TRAINING_HISTORY;
use pierre_config::constants::limits::{
    ACTIVITY_CAPACITY_HINT, MAX_TIMEFRAME_DAYS, METERS_PER_KILOMETER, PERCENTAGE_MULTIPLIER,
};
use pierre_config::constants::time_constants::{
    DAYS_PER_MONTH, DAYS_PER_QUARTER, DAYS_PER_WEEK, DAYS_PER_YEAR, SECONDS_PER_HOUR_F64,
};
use pierre_core::errors::{AppError, AppResult, JsonResultExt};
use pierre_core::models::Activity;
use pierre_intelligence::goal_engine::{AdvancedGoalEngine, GoalEngineTrait, GoalSuggestion};
use pierre_intelligence::physiological_constants::goal_feasibility::{
    ADEQUATE_FREQUENCY_DATA_THRESHOLD, ASSUMED_TRAINING_HISTORY_WEEKS, DAYS_PER_MONTH_APPROX,
    DEFAULT_TIMEFRAME_DAYS as GOAL_DEFAULT_TIMEFRAME_DAYS, EXCELLENT_CONFIDENCE_THRESHOLD,
    EXCELLENT_DATA_QUALITY_THRESHOLD, EXCESSIVE_IMPROVEMENT_PENALTY_FACTOR,
    GOAL_SUGGESTION_ACTIVITY_LIMIT, GOOD_CONFIDENCE_LEVEL, GOOD_CONFIDENCE_THRESHOLD,
    GOOD_DATA_QUALITY_THRESHOLD, HIGH_CONFIDENCE_LEVEL, LIMITED_CONFIDENCE_LEVEL, MAX_PERCENTAGE,
    MEDIUM_CONFIDENCE_LEVEL, MINIMUM_CONFIDENCE_LEVEL, MIN_ACTIVITIES_FOR_EXCELLENT_CONFIDENCE,
    MIN_ACTIVITIES_FOR_GOOD_CONFIDENCE, MODERATE_FEASIBILITY_THRESHOLD,
    PROGRESS_TRACKING_ACTIVITY_LIMIT, SAFE_MONTHLY_IMPROVEMENT_RATE_PERCENT,
    SAFE_RANGE_PENALTY_FACTOR, SIMPLE_PROGRESS_THRESHOLD, UNSAFE_IMPROVEMENT_PENALTY_BASE,
    VERY_LOW_CONFIDENCE_LEVEL, VOLUME_DOUBLING_THRESHOLD,
};
use pierre_intelligence::seasonality::build_seasonal_context;
use pierre_intelligence::{FitnessLevel, TimeAvailability, UserFitnessProfile, UserPreferences};
use pierre_mcp_schema::json_schemas::{AnalyzeGoalFeasibilityParams, SetGoalParams};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

// ============================================================================
// Private helpers (inlined from former handlers/goals.rs)
// ============================================================================

/// Safe conversion from usize to f64 for activity counts.
#[inline]
fn safe_usize_to_f64(len: usize) -> f64 {
    len.to_f64().unwrap_or_else(|| f64::from(u32::MAX))
}

/// Safe conversion from i64 to f64 for time durations.
#[inline]
fn safe_i64_to_f64(val: i64) -> f64 {
    val.to_f64().unwrap_or_else(|| f64::from(i32::MAX))
}

/// Safe conversion from f64 to u32 with clamping.
#[inline]
fn safe_f64_to_u32(val: f64) -> u32 {
    if val >= f64::from(u32::MAX) {
        u32::MAX
    } else if val <= 0.0 {
        0
    } else {
        val.to_u32().unwrap_or(u32::MAX)
    }
}

/// Extract and validate goal feasibility parameters from args.
fn extract_feasibility_params(args: &Value) -> AppResult<(String, f64, u32)> {
    let params: AnalyzeGoalFeasibilityParams = from_value(args.clone())
        .json_context("analyze_goal_feasibility parameters")
        .map_err(|e| AppError::invalid_input(e.to_string()))?;

    let timeframe_days = params.timeframe_days.unwrap_or(GOAL_DEFAULT_TIMEFRAME_DAYS);

    let effective_timeframe = if timeframe_days > MAX_TIMEFRAME_DAYS {
        warn!(
            "Timeframe {timeframe_days} days is unusually long, capping at {}",
            MAX_TIMEFRAME_DAYS
        );
        MAX_TIMEFRAME_DAYS
    } else {
        timeframe_days
    };

    Ok((params.goal_type, params.target_value, effective_timeframe))
}

/// Calculate feasibility score based on current level vs target.
fn calculate_feasibility_score(
    current_level: f64,
    target_value: f64,
    effective_timeframe: u32,
) -> (f64, f64, f64) {
    let improvement_required = if current_level > 0.0 {
        ((target_value - current_level) / current_level) * MAX_PERCENTAGE
    } else {
        MAX_PERCENTAGE
    };

    let months = f64::from(effective_timeframe) / DAYS_PER_MONTH_APPROX;
    let safe_improvement_capacity = SAFE_MONTHLY_IMPROVEMENT_RATE_PERCENT * months;

    let feasibility_score = if improvement_required <= 0.0 {
        MAX_PERCENTAGE
    } else if safe_improvement_capacity <= 0.0 {
        0.0
    } else if improvement_required <= safe_improvement_capacity {
        (improvement_required / safe_improvement_capacity)
            .mul_add(-SAFE_RANGE_PENALTY_FACTOR, MAX_PERCENTAGE)
    } else {
        let excess_improvement = improvement_required - safe_improvement_capacity;
        let penalty =
            (excess_improvement / safe_improvement_capacity) * EXCESSIVE_IMPROVEMENT_PENALTY_FACTOR;
        (UNSAFE_IMPROVEMENT_PENALTY_BASE - penalty).max(0.0)
    };

    (
        feasibility_score,
        improvement_required,
        safe_improvement_capacity,
    )
}

/// Generate recommendations based on feasibility analysis.
fn generate_feasibility_recommendations(
    mut recommendations: Vec<String>,
    feasible: bool,
    improvement_required: f64,
    safe_improvement_capacity: f64,
    current_level: f64,
    goal_type: &str,
    activities_count: usize,
) -> Vec<String> {
    if !feasible && improvement_required > safe_improvement_capacity {
        let suggested_days_f64 = (improvement_required / SAFE_MONTHLY_IMPROVEMENT_RATE_PERCENT)
            .mul_add(f64::from(DAYS_PER_MONTH), 0.0)
            .ceil();
        let suggested_timeframe = safe_f64_to_u32(suggested_days_f64);
        recommendations.push(format!(
            "Consider extending timeframe to {suggested_timeframe} days for safer progression"
        ));

        let safer_target = current_level * (1.0 + (safe_improvement_capacity / 100.0));
        recommendations.push(format!(
            "Or reduce target to {safer_target:.1} {} for current timeframe",
            match goal_type {
                "distance" => "km",
                "duration" => "hours",
                "frequency" => "activities",
                _ => "units",
            }
        ));
    }

    if activities_count < GOOD_DATA_QUALITY_THRESHOLD {
        recommendations
            .push("Build consistent training history for better goal planning".to_owned());
    }

    recommendations
}

/// Parameters for building feasibility response.
struct FeasibilityResponseParams<'a> {
    feasibility_score: f64,
    feasible: bool,
    confidence_level: f64,
    risk_factors: Vec<String>,
    recommendations: Vec<String>,
    target_value: f64,
    current_level: f64,
    safe_improvement_capacity: f64,
    effective_timeframe: u32,
    improvement_required: f64,
    activities_len: usize,
    goal_type: &'a str,
}

/// Build feasibility analysis response payload.
fn build_feasibility_payload(params: &FeasibilityResponseParams) -> Value {
    let months = f64::from(params.effective_timeframe) / DAYS_PER_MONTH_APPROX;
    json!({
        "feasible": params.feasible,
        "feasibility_score": params.feasibility_score.min(100.0),
        "confidence_level": params.confidence_level,
        "risk_factors": params.risk_factors,
        "success_probability": (params.feasibility_score / 100.0).min(1.0),
        "recommendations": params.recommendations,
        "adjusted_target": if params.feasible { params.target_value } else { params.current_level * (1.0 + (params.safe_improvement_capacity / 100.0)) },
        "adjusted_timeframe": if params.feasible {
            params.effective_timeframe
        } else {
            let safe_days_f64 = (params.improvement_required / SAFE_MONTHLY_IMPROVEMENT_RATE_PERCENT).mul_add(
                f64::from(DAYS_PER_MONTH),
                0.0
            ).ceil();
            safe_f64_to_u32(safe_days_f64)
        },
        "analysis": {
            "current_level": params.current_level,
            "target_value": params.target_value,
            "improvement_required_percent": params.improvement_required,
            "safe_improvement_capacity_percent": params.safe_improvement_capacity,
            "timeframe_months": months
        },
        "historical_context": {
            "activities_analyzed": params.activities_len,
            "goal_type": params.goal_type,
            "data_quality": if params.activities_len >= EXCELLENT_DATA_QUALITY_THRESHOLD { "excellent" } else if params.activities_len >= GOOD_DATA_QUALITY_THRESHOLD { "good" } else { "limited" }
        }
    })
}

/// Extract goal parameters from args.
fn extract_goal_params(args: &Value) -> AppResult<SetGoalParams> {
    from_value(args.clone())
        .json_context("set_goal parameters")
        .map_err(|e| AppError::invalid_input(e.to_string()))
}

/// Build goal creation response payload.
fn build_goal_creation_payload(
    goal_id: &str,
    goal_type: &str,
    target_value: f64,
    timeframe: &str,
    title: &str,
    created_at: chrono::DateTime<Utc>,
) -> Value {
    json!({
        "goal_id": goal_id,
        "goal_type": goal_type,
        "target_value": target_value,
        "timeframe": timeframe,
        "title": title,
        "created_at": created_at.to_rfc3339(),
        "status": "created"
    })
}

/// Load user fitness profile from database.
async fn load_user_profile(
    profiles: &dyn ProfileRepository,
    user_uuid: Uuid,
    user_id: &str,
    activities: &[Activity],
) -> UserFitnessProfile {
    match profiles.get_profile(user_uuid).await {
        Ok(Some(profile_json)) => from_value(profile_json).unwrap_or_else(|e| {
            warn!(
                user_id = %user_id,
                error = %e,
                "Failed to deserialize user fitness profile, using fallback profile"
            );
            create_fallback_profile(user_id.to_owned(), activities)
        }),
        Ok(None) | Err(_) => create_fallback_profile(user_id.to_owned(), activities),
    }
}

/// Format goal suggestions for response.
fn format_goal_suggestions(suggestions: Vec<GoalSuggestion>) -> Vec<Value> {
    suggestions
        .into_iter()
        .map(|g| {
            json!({
                "goal_type": format!("{:?}", g.goal_type),
                "target_value": g.suggested_target,
                "difficulty": format!("{:?}", g.difficulty),
                "rationale": g.rationale,
                "estimated_timeline_days": g.estimated_timeline_days,
                "success_probability": g.success_probability
            })
        })
        .collect()
}

/// Fetch activities for goal suggestions; returns empty vec if auth/fetch fails.
async fn fetch_suggestion_activities(
    context: &ToolExecutionContext,
    provider_name: &str,
    user_uuid: Uuid,
) -> Vec<Activity> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());
    let mut activities = Vec::new();
    if let Ok(provider) = auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id.as_deref())
        .await
    {
        if let Ok(provider_activities) = provider
            .get_activities(Some(GOAL_SUGGESTION_ACTIVITY_LIMIT), None)
            .await
        {
            activities = provider_activities;
        }
    }
    activities
}

/// Fetch activities for goal feasibility analysis; returns empty vec if auth/fetch fails.
async fn fetch_feasibility_activities(
    context: &ToolExecutionContext,
    provider_name: &str,
    user_uuid: Uuid,
) -> Vec<Activity> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());
    let mut activities: Vec<Activity> = Vec::with_capacity(ACTIVITY_CAPACITY_HINT);
    if let Ok(provider) = auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id.as_deref())
        .await
    {
        if let Ok(provider_activities) = provider
            .get_activities(Some(PROGRESS_TRACKING_ACTIVITY_LIMIT), None)
            .await
        {
            activities = provider_activities;
        }
    }
    activities
}

/// Fetch activities for progress tracking; returns error if auth fails.
async fn fetch_progress_activities(
    context: &ToolExecutionContext,
    provider_name: &str,
    user_uuid: Uuid,
) -> AppResult<Vec<Activity>> {
    let auth_service = AuthService::new(context.resources.clone());
    let tenant_id = context.tenant_id.map(|id| id.to_string());
    match auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id.as_deref())
        .await
    {
        Ok(provider) => {
            debug!("Provider authenticated for progress tracking");
            Ok(provider
                .get_activities(Some(PROGRESS_TRACKING_ACTIVITY_LIMIT), None)
                .await
                .unwrap_or_default())
        }
        Err(response) => {
            debug!("Authentication failed for progress tracking");
            Err(AppError::external_service(
                "fitness_provider",
                response
                    .error
                    .unwrap_or_else(|| "Authentication failed".to_owned()),
            ))
        }
    }
}

/// Analyze goal performance based on goal type.
fn analyze_goal_by_type(
    goal_type: &str,
    activities: &[Activity],
    target_value: f64,
    timeframe_days: u32,
) -> (f64, f64, Vec<String>, Vec<String>) {
    match goal_type {
        "distance" => analyze_distance_goal_feasibility(activities, target_value, timeframe_days),
        "duration" => analyze_duration_goal_feasibility(activities, target_value, timeframe_days),
        "frequency" => analyze_frequency_goal_feasibility(activities, target_value, timeframe_days),
        _ => (
            0.0,
            VERY_LOW_CONFIDENCE_LEVEL,
            vec!["Unknown goal type".to_owned()],
            vec!["Specify a valid goal type: distance, duration, or frequency".to_owned()],
        ),
    }
}

/// Calculate actual training history weeks from activity date range.
fn calculate_training_history_weeks(activities: &[Activity], min_activities: usize) -> f64 {
    if activities.len() < min_activities {
        return ASSUMED_TRAINING_HISTORY_WEEKS;
    }

    let mut dates: Vec<DateTime<Utc>> = activities.iter().map(Activity::start_date).collect();
    dates.sort();

    if let (Some(first), Some(last)) = (dates.first(), dates.last()) {
        let days = (*last - *first).num_days();
        let weeks = safe_i64_to_f64(days.max(1)) / 7.0;
        weeks.max(1.0)
    } else {
        ASSUMED_TRAINING_HISTORY_WEEKS
    }
}

/// Analyze feasibility of distance goal.
fn analyze_distance_goal_feasibility(
    activities: &[Activity],
    target_km: f64,
    timeframe_days: u32,
) -> (f64, f64, Vec<String>, Vec<String>) {
    if activities.is_empty() {
        return (
            0.0,
            MINIMUM_CONFIDENCE_LEVEL,
            vec!["No historical data available".to_owned()],
            vec!["Start with smaller distance goals to build baseline".to_owned()],
        );
    }

    let recent_total_distance: f64 = activities
        .iter()
        .filter_map(Activity::distance_meters)
        .sum::<f64>()
        / METERS_PER_KILOMETER;

    let activity_count = safe_usize_to_f64(activities.len());
    let avg_distance_per_activity = if activity_count > 0.0 {
        recent_total_distance / activity_count
    } else {
        0.0
    };

    let training_weeks =
        calculate_training_history_weeks(activities, MIN_ACTIVITIES_FOR_TRAINING_HISTORY).max(1.0);
    let weeks_in_timeframe = f64::from(timeframe_days) / 7.0;
    let estimated_activities = (activity_count / training_weeks) * weeks_in_timeframe;

    let projected_distance = avg_distance_per_activity * estimated_activities;

    let mut risk_factors = Vec::new();
    let mut recommendations = Vec::new();

    if projected_distance < target_km * VOLUME_DOUBLING_THRESHOLD {
        risk_factors.push("Target requires more than doubling current volume".to_owned());
        recommendations.push("Increase training frequency gradually".to_owned());
    }

    if activity_count < MIN_ACTIVITIES_FOR_GOOD_CONFIDENCE {
        risk_factors.push("Limited training history".to_owned());
    }

    let confidence = if activity_count >= MIN_ACTIVITIES_FOR_EXCELLENT_CONFIDENCE {
        EXCELLENT_CONFIDENCE_THRESHOLD
    } else if activity_count >= MIN_ACTIVITIES_FOR_GOOD_CONFIDENCE {
        GOOD_CONFIDENCE_THRESHOLD
    } else {
        LIMITED_CONFIDENCE_LEVEL
    };

    (
        projected_distance,
        confidence,
        risk_factors,
        recommendations,
    )
}

/// Analyze feasibility of duration goal.
fn analyze_duration_goal_feasibility(
    activities: &[Activity],
    _target_hours: f64,
    timeframe_days: u32,
) -> (f64, f64, Vec<String>, Vec<String>) {
    if activities.is_empty() {
        return (
            0.0,
            MINIMUM_CONFIDENCE_LEVEL,
            vec!["No historical data available".to_owned()],
            vec!["Start tracking activity duration".to_owned()],
        );
    }

    let total_duration: u64 = activities.iter().map(Activity::duration_seconds).sum();
    let current_hours = match u32::try_from(total_duration.min(u64::from(u32::MAX))) {
        Ok(duration_u32) => f64::from(duration_u32) / SECONDS_PER_HOUR_F64,
        Err(e) => {
            warn!(
                total_duration = total_duration,
                error = %e,
                "Duration conversion failed (should not happen after min() with u32::MAX), using u32::MAX"
            );
            f64::from(u32::MAX) / SECONDS_PER_HOUR_F64
        }
    };

    let training_weeks =
        calculate_training_history_weeks(activities, MIN_ACTIVITIES_FOR_TRAINING_HISTORY).max(1.0);
    let weeks_in_timeframe = f64::from(timeframe_days) / 7.0;
    let projected_hours = (current_hours / training_weeks) * weeks_in_timeframe;

    let confidence = if activities.len() >= EXCELLENT_DATA_QUALITY_THRESHOLD {
        HIGH_CONFIDENCE_LEVEL
    } else {
        MEDIUM_CONFIDENCE_LEVEL
    };

    (
        projected_hours,
        confidence,
        Vec::new(),
        vec!["Maintain consistent training schedule".to_owned()],
    )
}

/// Analyze feasibility of frequency goal.
fn analyze_frequency_goal_feasibility(
    activities: &[Activity],
    _target_count: f64,
    timeframe_days: u32,
) -> (f64, f64, Vec<String>, Vec<String>) {
    let current_count = safe_usize_to_f64(activities.len());
    let training_weeks =
        calculate_training_history_weeks(activities, MIN_ACTIVITIES_FOR_TRAINING_HISTORY).max(1.0);
    let weeks_in_timeframe = f64::from(timeframe_days) / 7.0;
    let current_weekly_frequency = current_count / training_weeks;
    let projected_count = current_weekly_frequency * weeks_in_timeframe;

    let confidence = if current_count >= f64::from(ADEQUATE_FREQUENCY_DATA_THRESHOLD) {
        HIGH_CONFIDENCE_LEVEL
    } else {
        GOOD_CONFIDENCE_LEVEL
    };

    (
        projected_count,
        confidence,
        Vec::new(),
        vec!["Schedule training days in advance".to_owned()],
    )
}

/// Calculate training history in months from activity dates.
fn calculate_training_history_months(activities: &[Activity]) -> i32 {
    if activities.is_empty() {
        return 0;
    }

    let Some(earliest_date) = activities.iter().map(Activity::start_date).min() else {
        warn!("No activities found for training history calculation, returning 0 months");
        return 0;
    };

    let now = Utc::now();
    let duration = now.signed_duration_since(earliest_date);
    let days = duration.num_days();

    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    ((days as f64 / 30.44).round() as i32).max(0)
}

/// Detect primary sport from activity frequency.
fn detect_primary_sport(activities: &[Activity]) -> Vec<String> {
    if activities.is_empty() {
        return vec![];
    }

    let mut sport_counts: HashMap<String, usize> = HashMap::new();
    for activity in activities {
        let sport_name = format!("{:?}", activity.sport_type());
        *sport_counts.entry(sport_name).or_insert(0) += 1;
    }

    sport_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(sport, _)| vec![sport])
        .unwrap_or_default()
}

/// Infer fitness level from training consistency.
fn infer_fitness_level(activities: &[Activity]) -> FitnessLevel {
    if activities.is_empty() {
        return FitnessLevel::Beginner;
    }

    let training_weeks =
        calculate_training_history_weeks(activities, MIN_ACTIVITIES_FOR_TRAINING_HISTORY).max(1.0);
    #[allow(clippy::cast_precision_loss)] // Safe: realistic activity counts
    let activities_per_week = activities.len() as f64 / training_weeks;

    if activities_per_week >= 5.0 && training_weeks >= 26.0 {
        FitnessLevel::Advanced
    } else if activities_per_week >= 3.0 && training_weeks >= 12.0 {
        FitnessLevel::Intermediate
    } else {
        FitnessLevel::Beginner
    }
}

/// Create a fallback user profile when database profile is unavailable.
fn create_fallback_profile(user_id: String, activities: &[Activity]) -> UserFitnessProfile {
    let training_history_months = calculate_training_history_months(activities);
    let primary_sports = detect_primary_sport(activities);
    let fitness_level = infer_fitness_level(activities);
    let seasonal_context = activities
        .iter()
        .find_map(Activity::start_latitude)
        .map(|lat| build_seasonal_context(lat, Utc::now().month()));

    UserFitnessProfile {
        user_id,
        age: None,
        gender: None,
        weight: None,
        height: None,
        fitness_level,
        primary_sports,
        training_history_months,
        preferences: UserPreferences {
            preferred_units: "metric".into(),
            training_focus: vec![],
            injury_history: vec![],
            time_availability: TimeAvailability {
                hours_per_week: 3.0,
                preferred_days: vec![],
                preferred_duration_minutes: Some(30),
            },
        },
        seasonal_context,
    }
}

/// Goal details extracted from database.
struct GoalDetails {
    goal_type: String,
    goal_target: f64,
    timeframe: String,
    created_at: Option<DateTime<FixedOffset>>,
}

/// Extract goal details from JSON map.
fn extract_goal_details(goal: &serde_json::Map<String, Value>) -> Option<GoalDetails> {
    let goal_type = goal
        .get("goal_type")
        .and_then(|v| v.as_str())
        .unwrap_or("distance")
        .to_owned();

    let goal_target = goal.get("target_value").and_then(Value::as_f64)?;

    let timeframe = goal
        .get("timeframe")
        .and_then(|v| v.as_str())
        .unwrap_or("month")
        .to_owned();

    let created_at = goal
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok());

    Some(GoalDetails {
        goal_type,
        goal_target,
        timeframe,
        created_at,
    })
}

/// Calculate days remaining in goal timeframe.
fn calculate_days_remaining(created_at: Option<DateTime<FixedOffset>>, timeframe: &str) -> u32 {
    created_at.map_or(DEFAULT_GOAL_TIMEFRAME_DAYS, |created| {
        let timeframe_days = match timeframe {
            "week" => DAYS_PER_WEEK,
            "month" => DAYS_PER_MONTH,
            "quarter" => DAYS_PER_QUARTER,
            "year" => DAYS_PER_YEAR,
            _ => DEFAULT_GOAL_TIMEFRAME_DAYS,
        };
        let elapsed = (Utc::now() - created.with_timezone(&chrono::Utc)).num_days();
        let elapsed_u32 = match u32::try_from(elapsed.max(0)) {
            Ok(val) => val,
            Err(e) => {
                warn!(
                    elapsed = elapsed,
                    error = %e,
                    "Elapsed days conversion failed (negative or too large), using 0"
                );
                0
            }
        };
        timeframe_days.saturating_sub(elapsed_u32)
    })
}

/// Calculate current progress value based on goal type.
fn calculate_current_progress(goal_type: &str, activities: &[&Activity]) -> (f64, &'static str) {
    match goal_type {
        "distance" => {
            let total_distance: f64 = activities
                .iter()
                .filter_map(|a| a.distance_meters())
                .sum::<f64>()
                / METERS_PER_KILOMETER;
            (total_distance, "km")
        }
        "duration" => {
            let total_duration: u64 = activities.iter().map(|a| a.duration_seconds()).sum();
            let hours = match u32::try_from(total_duration.min(u64::from(u32::MAX))) {
                Ok(duration_u32) => f64::from(duration_u32) / SECONDS_PER_HOUR_F64,
                Err(e) => {
                    warn!(
                        total_duration = total_duration,
                        error = %e,
                        "Duration conversion failed in progress calculation, using u32::MAX"
                    );
                    f64::from(u32::MAX) / SECONDS_PER_HOUR_F64
                }
            };
            (hours, "hours")
        }
        "frequency" => {
            let count = safe_usize_to_f64(activities.len());
            (count, "activities")
        }
        _ => (0.0, "unknown"),
    }
}

/// Calculate projected completion days.
fn calculate_projected_completion(
    current_value: f64,
    goal_target: f64,
    created_at: Option<DateTime<FixedOffset>>,
) -> Option<f64> {
    if current_value > 0.0 {
        let days_elapsed = created_at.map_or(1, |c| {
            (Utc::now() - c.with_timezone(&chrono::Utc))
                .num_days()
                .max(1)
        });
        let days_elapsed_f64 = safe_i64_to_f64(days_elapsed);
        let daily_rate = current_value / days_elapsed_f64;
        let remaining_value = goal_target - current_value;
        let days_needed = (remaining_value / daily_rate).ceil();
        Some(days_needed)
    } else {
        None
    }
}

/// Parameters for building progress tracking response.
struct ProgressResponseParams<'a> {
    goal_id: &'a str,
    details: &'a GoalDetails,
    current_value: f64,
    unit: &'a str,
    progress_percentage: f64,
    on_track: bool,
    days_remaining: u32,
    projected_completion: Option<f64>,
    relevant_activities: &'a [&'a Activity],
    total_duration: u64,
}

/// Build progress tracking response payload.
fn build_progress_payload(params: &ProgressResponseParams) -> Value {
    json!({
        "goal_id": params.goal_id,
        "goal_type": params.details.goal_type,
        "current_value": params.current_value,
        "target_value": params.details.goal_target,
        "unit": params.unit,
        "progress_percentage": params.progress_percentage.min(100.0),
        "on_track": params.on_track,
        "days_remaining": params.days_remaining,
        "projected_completion_days": params.projected_completion,
        "timeframe": params.details.timeframe,
        "summary": {
            "total_activities": params.relevant_activities.len(),
            "total_distance_km": params.relevant_activities.iter().filter_map(|a| a.distance_meters()).sum::<f64>() / METERS_PER_KILOMETER,
            "total_duration_hours": match u32::try_from(params.total_duration.min(u64::from(u32::MAX))) {
                Ok(duration_u32) => f64::from(duration_u32) / SECONDS_PER_HOUR_F64,
                Err(e) => {
                    warn!(
                        total_duration = params.total_duration,
                        error = %e,
                        "Duration conversion failed in response summary, using u32::MAX"
                    );
                    f64::from(u32::MAX) / SECONDS_PER_HOUR_F64
                }
            }
        }
    })
}

/// Result of looking up a goal: `Ok(Some(...))` on hit, `Ok(None)` when the
/// goal is missing or malformed (a tool-level "soft" failure that should be
/// reported to the caller as `success: false`), `Err` for real I/O errors.
async fn fetch_and_validate_goal(
    profiles: &dyn ProfileRepository,
    user_uuid: Uuid,
    goal_id: &str,
) -> AppResult<Option<GoalDetails>> {
    let goals = profiles
        .get_goals(user_uuid)
        .await
        .map_err(|e| AppError::internal(format!("Failed to load goals from database: {e}")))?;

    let Some(goal) = goals
        .iter()
        .find(|g| g.get("goal_id").and_then(|v| v.as_str()) == Some(goal_id))
    else {
        return Ok(None);
    };

    let Some(goal_object) = goal.as_object() else {
        return Ok(None);
    };

    Ok(extract_goal_details(goal_object))
}

/// Filter activities relevant to goal timeframe.
fn filter_relevant_activities(
    activities: &[Activity],
    created_at: Option<DateTime<FixedOffset>>,
) -> Vec<&Activity> {
    created_at.map_or_else(
        || activities.iter().collect(),
        |created| {
            activities
                .iter()
                .filter(|a| a.start_date() > created)
                .collect::<Vec<_>>()
        },
    )
}

/// Calculate progress metrics for goal tracking.
fn calculate_progress_metrics(
    goal_type: &str,
    relevant_activities: &[&Activity],
    goal_target: f64,
) -> (f64, &'static str, f64, bool) {
    let (current_value, unit) = calculate_current_progress(goal_type, relevant_activities);

    let progress_percentage = if goal_target > 0.0 {
        (current_value / goal_target) * PERCENTAGE_MULTIPLIER
    } else {
        0.0
    };
    let on_track = progress_percentage >= SIMPLE_PROGRESS_THRESHOLD;

    (current_value, unit, progress_percentage, on_track)
}

// ============================================================================
// SetGoalTool - Create a new fitness goal
// ============================================================================

/// Tool for creating a new fitness goal.
pub struct SetGoalTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SetGoalTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(
            properties,
            Some(vec!["goal_type".to_owned(), "target_value".to_owned()]),
        );
        tool_definition(
            "set_goal",
            "Create a new fitness goal with specified type, target value, and timeframe",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let params = extract_goal_params(&args)?;
            let user_uuid = context.user_id;

            let created_at = Utc::now();
            let goal_data = json!({
                "goal_type": params.goal_type,
                "target_value": params.target_value,
                "timeframe": params.timeframe,
                "title": params.title,
                "created_at": created_at.to_rfc3339()
            });

            let goal_id = context
                .resources
                .repos()
                .profiles
                .create_goal(user_uuid, goal_data)
                .await
                .map_err(|e| AppError::internal(format!("Database error: {e}")))?;

            Ok(ToolResult::ok(build_goal_creation_payload(
                &goal_id,
                &params.goal_type,
                params.target_value,
                &params.timeframe,
                &params.title,
                created_at,
            )))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SuggestGoalsTool - Get AI-suggested goals
// ============================================================================

/// Tool for getting AI-suggested fitness goals.
pub struct SuggestGoalsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SuggestGoalsTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(properties, None);
        task_capable(tool_definition(
            "suggest_goals",
            "Get AI-suggested fitness goals based on your activity history and fitness level",
            schema,
            None,
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
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };
            let user_uuid = context.user_id;

            let activities = fetch_suggestion_activities(&context, &provider_name, user_uuid).await;

            let cageux_config = context.cageux_config();
            let goal_engine = AdvancedGoalEngine::new(&cageux_config);
            let user_profile = load_user_profile(
                context.resources.repos().profiles.as_ref(),
                user_uuid,
                &user_uuid.to_string(),
                &activities,
            )
            .await;

            match goal_engine.suggest_goals(&user_profile, &activities).await {
                Ok(suggestions) => Ok(ToolResult::ok(json!({
                    "suggested_goals": format_goal_suggestions(suggestions),
                    "activities_analyzed": activities.len()
                }))),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to suggest goals: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// TrackProgressTool - Track goal progress
// ============================================================================

/// Tool for tracking progress toward fitness goals.
pub struct TrackProgressTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for TrackProgressTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(properties, Some(vec!["goal_id".to_owned()]));
        task_capable(tool_definition(
            "track_progress",
            "Track progress toward a specific fitness goal with milestone achievements and projections",
            schema,
            None,
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
            let goal_id = args
                .get("goal_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::invalid_input("goal_id is required"))?
                .to_owned();

            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };
            let user_uuid = context.user_id;

            let Some(details) = fetch_and_validate_goal(
                context.resources.repos().profiles.as_ref(),
                user_uuid,
                &goal_id,
            )
            .await?
            else {
                return Ok(ToolResult::error(json!({
                    "error": format!("Goal {goal_id} not found"),
                })));
            };

            let days_remaining = calculate_days_remaining(details.created_at, &details.timeframe);

            let activities = fetch_progress_activities(&context, &provider_name, user_uuid).await?;

            let relevant_activities = filter_relevant_activities(&activities, details.created_at);
            let (current_value, unit, progress_percentage, on_track) = calculate_progress_metrics(
                &details.goal_type,
                &relevant_activities,
                details.goal_target,
            );

            let total_duration: u64 = relevant_activities
                .iter()
                .map(|a| a.duration_seconds())
                .sum();
            let projected_completion = calculate_projected_completion(
                current_value,
                details.goal_target,
                details.created_at,
            );

            Ok(ToolResult::ok(build_progress_payload(
                &ProgressResponseParams {
                    goal_id: &goal_id,
                    details: &details,
                    current_value,
                    unit,
                    progress_percentage,
                    on_track,
                    days_remaining,
                    projected_completion,
                    relevant_activities: &relevant_activities,
                    total_duration,
                },
            )))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AnalyzeGoalFeasibilityTool - Assess goal achievability
// ============================================================================

/// Tool for analyzing if a fitness goal is achievable.
pub struct AnalyzeGoalFeasibilityTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeGoalFeasibilityTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(
            properties,
            Some(vec!["goal_type".to_owned(), "target_value".to_owned()]),
        );
        task_capable(tool_definition(
            "analyze_goal_feasibility",
            "Analyze whether a fitness goal is achievable based on your current fitness level and training history",
            schema,
            None,
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
            let (goal_type, target_value, effective_timeframe) = extract_feasibility_params(&args)?;
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };
            let user_uuid = context.user_id;

            let activities =
                fetch_feasibility_activities(&context, &provider_name, user_uuid).await;

            let (current_level, confidence_level, risk_factors, recommendations) =
                analyze_goal_by_type(&goal_type, &activities, target_value, effective_timeframe);

            let (feasibility_score, improvement_required, safe_improvement_capacity) =
                calculate_feasibility_score(current_level, target_value, effective_timeframe);
            let feasible = feasibility_score >= MODERATE_FEASIBILITY_THRESHOLD;

            let final_recommendations = generate_feasibility_recommendations(
                recommendations,
                feasible,
                improvement_required,
                safe_improvement_capacity,
                current_level,
                &goal_type,
                activities.len(),
            );

            Ok(ToolResult::ok(build_feasibility_payload(
                &FeasibilityResponseParams {
                    feasibility_score,
                    feasible,
                    confidence_level,
                    risk_factors,
                    recommendations: final_recommendations,
                    target_value,
                    current_level,
                    safe_improvement_capacity,
                    effective_timeframe,
                    improvement_required,
                    activities_len: activities.len(),
                    goal_type: &goal_type,
                },
            )))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all goal management tools for registration.
#[must_use]
pub fn create_goal_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(SetGoalTool),
        Box::new(SuggestGoalsTool),
        Box::new(TrackProgressTool),
        Box::new(AnalyzeGoalFeasibilityTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(SetGoalTool => empty);
crate::declare_security!(SuggestGoalsTool => empty);
crate::declare_security!(TrackProgressTool => empty);
crate::declare_security!(AnalyzeGoalFeasibilityTool => empty);
