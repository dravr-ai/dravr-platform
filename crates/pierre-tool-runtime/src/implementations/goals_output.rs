// ABOUTME: The shapes the goal tools answer with, and the schemas derived from them
// ABOUTME: Separate from goals.rs because that file is at its size ceiling and frozen
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the four goal tools.
//!
//! These live beside `goals.rs` rather than inside it because that file is
//! already past the 1200-line ceiling and frozen at its current size. Splitting
//! the answer shapes out is the split that was available: they are a coherent
//! unit, they are what the tests and the derived schemas both name, and nothing
//! in them needs the tool plumbing next door.

use chrono::Utc;
use pierre_config::constants::limits::METERS_PER_KILOMETER;
use pierre_config::constants::time_constants::{DAYS_PER_MONTH, SECONDS_PER_HOUR_F64};
use pierre_core::models::Activity;
use pierre_intelligence::physiological_constants::goal_feasibility::{
    DAYS_PER_MONTH_APPROX, EXCELLENT_DATA_QUALITY_THRESHOLD, GOOD_DATA_QUALITY_THRESHOLD,
    SAFE_MONTHLY_IMPROVEMENT_RATE_PERCENT,
};
use tracing::warn;

use super::goals::{safe_f64_to_u32, GoalDetails};
use pierre_intelligence::goal_engine::GoalSuggestion;
use schemars::JsonSchema;
use serde::Serialize;

/// What `set_goal` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetGoalResult {
    /// Identifier the other goal tools take.
    pub goal_id: String,
    /// Which kind of goal was created.
    pub goal_type: String,
    /// The figure the athlete is aiming at.
    pub target_value: f64,
    /// The window they gave themselves.
    pub timeframe: String,
    /// Athlete-facing name.
    pub title: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// Always `created` here; the field exists so a client reads state from a
    /// value rather than from the absence of an error.
    pub status: String,
}

/// One suggestion from `suggest_goals`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GoalSuggestionEntry {
    /// Kind of goal suggested.
    pub goal_type: String,
    /// The figure suggested for it.
    pub target_value: f64,
    /// How hard the engine judges it for this athlete.
    pub difficulty: String,
    /// Why the engine suggested it, in the athlete's terms.
    pub rationale: String,
    /// Days the engine expects it to take.
    pub estimated_timeline_days: i32,
    /// Modelled probability of reaching it, 0.0 to 1.0.
    pub success_probability: f64,
}

/// What `suggest_goals` answers with on success.
///
/// The failure branch answers a different shape, and deliberately so: it rides
/// `ToolResult::error`, which sets `is_error`, and `outputSchema` governs the
/// success payload only. A schema describing both would describe neither.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SuggestGoalsResult {
    /// The suggestions, best first.
    pub suggested_goals: Vec<GoalSuggestionEntry>,
    /// How much history they were drawn from.
    pub activities_analyzed: usize,
}

/// The training behind a progress reading.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProgressSummary {
    /// Activities that counted toward the goal.
    pub total_activities: usize,
    /// Their combined distance in kilometres.
    pub total_distance_km: f64,
    /// Their combined moving time in hours.
    pub total_duration_hours: f64,
}

/// What `track_progress` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TrackProgressResult {
    /// The goal being tracked.
    pub goal_id: String,
    /// Its kind.
    pub goal_type: String,
    /// Where the athlete is now.
    pub current_value: f64,
    /// Where they are aiming.
    pub target_value: f64,
    /// Unit both values are in.
    pub unit: String,
    /// Percentage of the way there, capped at 100.
    pub progress_percentage: f64,
    /// Whether the current rate reaches the target inside the timeframe.
    pub on_track: bool,
    /// Days left in the timeframe.
    pub days_remaining: u32,
    /// Projected days to completion at the current rate; absent when the rate
    /// does not support a projection.
    pub projected_completion_days: Option<f64>,
    /// The window the goal was set over.
    pub timeframe: String,
    /// The training the reading is drawn from.
    pub summary: ProgressSummary,
}

/// The arithmetic behind a feasibility verdict.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FeasibilityAnalysis {
    /// Where the athlete is today.
    pub current_level: f64,
    /// Where the goal asks them to be.
    pub target_value: f64,
    /// The gap, as a percentage of the current level.
    pub improvement_required_percent: f64,
    /// What the timeframe supports at a safe rate, same units.
    pub safe_improvement_capacity_percent: f64,
    /// The timeframe in months.
    pub timeframe_months: f64,
}

/// How much history the verdict rests on.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FeasibilityHistoricalContext {
    /// Activities the analysis read.
    pub activities_analyzed: usize,
    /// The goal kind analysed.
    pub goal_type: String,
    /// `excellent`, `good` or `limited` — stated so the athlete can weigh the
    /// verdict rather than take it flat.
    pub data_quality: String,
}

/// What `analyze_goal_feasibility` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GoalFeasibilityResult {
    /// Whether the goal is reachable as stated.
    pub feasible: bool,
    /// Score behind the verdict, 0 to 100.
    pub feasibility_score: f64,
    /// How much the engine trusts its own score.
    pub confidence_level: f64,
    /// What would make it fail.
    pub risk_factors: Vec<String>,
    /// The score as a probability, 0.0 to 1.0.
    pub success_probability: f64,
    /// What to change.
    pub recommendations: Vec<String>,
    /// The target as stated when feasible; a safe one when not.
    pub adjusted_target: f64,
    /// The timeframe as stated when feasible; one that fits a safe rate when not.
    pub adjusted_timeframe: u32,
    /// The arithmetic behind the verdict.
    pub analysis: FeasibilityAnalysis,
    /// How much history it rests on.
    pub historical_context: FeasibilityHistoricalContext,
}

pub(crate) fn build_goal_creation_payload(
    goal_id: &str,
    goal_type: &str,
    target_value: f64,
    timeframe: &str,
    title: &str,
    created_at: chrono::DateTime<Utc>,
) -> SetGoalResult {
    SetGoalResult {
        goal_id: goal_id.to_owned(),
        goal_type: goal_type.to_owned(),
        target_value,
        timeframe: timeframe.to_owned(),
        title: title.to_owned(),
        created_at: created_at.to_rfc3339(),
        status: "created".to_owned(),
    }
}

/// Format goal suggestions for response.
pub(crate) fn format_goal_suggestions(
    suggestions: Vec<GoalSuggestion>,
) -> Vec<GoalSuggestionEntry> {
    suggestions
        .into_iter()
        .map(|g| GoalSuggestionEntry {
            goal_type: format!("{:?}", g.goal_type),
            target_value: g.suggested_target,
            difficulty: format!("{:?}", g.difficulty),
            rationale: g.rationale,
            estimated_timeline_days: g.estimated_timeline_days,
            success_probability: g.success_probability,
        })
        .collect()
}

/// Parameters for building feasibility response.
pub(crate) struct FeasibilityResponseParams<'a> {
    pub(crate) feasibility_score: f64,
    pub(crate) feasible: bool,
    pub(crate) confidence_level: f64,
    pub(crate) risk_factors: Vec<String>,
    pub(crate) recommendations: Vec<String>,
    pub(crate) target_value: f64,
    pub(crate) current_level: f64,
    pub(crate) safe_improvement_capacity: f64,
    pub(crate) effective_timeframe: u32,
    pub(crate) improvement_required: f64,
    pub(crate) activities_len: usize,
    pub(crate) goal_type: &'a str,
}

/// Build feasibility analysis response payload.
pub(crate) fn build_feasibility_payload(
    params: &FeasibilityResponseParams,
) -> GoalFeasibilityResult {
    let months = f64::from(params.effective_timeframe) / DAYS_PER_MONTH_APPROX;
    let adjusted_target = if params.feasible {
        params.target_value
    } else {
        params.current_level * (1.0 + (params.safe_improvement_capacity / 100.0))
    };
    let adjusted_timeframe = if params.feasible {
        params.effective_timeframe
    } else {
        let safe_days_f64 = (params.improvement_required / SAFE_MONTHLY_IMPROVEMENT_RATE_PERCENT)
            .mul_add(f64::from(DAYS_PER_MONTH), 0.0)
            .ceil();
        safe_f64_to_u32(safe_days_f64)
    };
    let data_quality = if params.activities_len >= EXCELLENT_DATA_QUALITY_THRESHOLD {
        "excellent"
    } else if params.activities_len >= GOOD_DATA_QUALITY_THRESHOLD {
        "good"
    } else {
        "limited"
    };

    GoalFeasibilityResult {
        feasible: params.feasible,
        feasibility_score: params.feasibility_score.min(100.0),
        confidence_level: params.confidence_level,
        risk_factors: params.risk_factors.clone(),
        success_probability: (params.feasibility_score / 100.0).min(1.0),
        recommendations: params.recommendations.clone(),
        adjusted_target,
        adjusted_timeframe,
        analysis: FeasibilityAnalysis {
            current_level: params.current_level,
            target_value: params.target_value,
            improvement_required_percent: params.improvement_required,
            safe_improvement_capacity_percent: params.safe_improvement_capacity,
            timeframe_months: months,
        },
        historical_context: FeasibilityHistoricalContext {
            activities_analyzed: params.activities_len,
            goal_type: params.goal_type.to_owned(),
            data_quality: data_quality.to_owned(),
        },
    }
}

/// Parameters for building progress tracking response.
pub(crate) struct ProgressResponseParams<'a> {
    pub(crate) goal_id: &'a str,
    pub(crate) details: &'a GoalDetails,
    pub(crate) current_value: f64,
    pub(crate) unit: &'a str,
    pub(crate) progress_percentage: f64,
    pub(crate) on_track: bool,
    pub(crate) days_remaining: u32,
    pub(crate) projected_completion: Option<f64>,
    pub(crate) relevant_activities: &'a [&'a Activity],
    pub(crate) total_duration: u64,
}

/// Build progress tracking response payload.
pub(crate) fn build_progress_payload(params: &ProgressResponseParams) -> TrackProgressResult {
    let total_distance_km = params
        .relevant_activities
        .iter()
        .filter_map(|a| a.distance_meters())
        .sum::<f64>()
        / METERS_PER_KILOMETER;
    let total_duration_hours = match u32::try_from(params.total_duration.min(u64::from(u32::MAX))) {
        Ok(duration_u32) => f64::from(duration_u32) / SECONDS_PER_HOUR_F64,
        Err(e) => {
            warn!(
                total_duration = params.total_duration,
                error = %e,
                "Duration conversion failed in response summary, using u32::MAX"
            );
            f64::from(u32::MAX) / SECONDS_PER_HOUR_F64
        }
    };

    TrackProgressResult {
        goal_id: params.goal_id.to_owned(),
        goal_type: params.details.goal_type.clone(),
        current_value: params.current_value,
        target_value: params.details.goal_target,
        unit: params.unit.to_owned(),
        progress_percentage: params.progress_percentage.min(100.0),
        on_track: params.on_track,
        days_remaining: params.days_remaining,
        projected_completion_days: params.projected_completion,
        timeframe: params.details.timeframe.clone(),
        summary: ProgressSummary {
            total_activities: params.relevant_activities.len(),
            total_distance_km,
            total_duration_hours,
        },
    }
}
