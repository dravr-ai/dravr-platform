// ABOUTME: The declared shape of generate_recommendations, across its six deterministic modes and the sampled one
// ABOUTME: One discriminated struct rather than untagged arms, because the modes' required keys nest
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One discriminated shape for all seven ways this tool answers.
//!
//! `generate_recommendations` answers in one of six deterministic modes or
//! from the client's LLM, and the modes overlap heavily: every one of them
//! carries `priority`, `reasoning` and `recommendations`, and the sparse
//! answer — no activities, or none in the last four weeks — carries only
//! those. As untagged variants no client could tell them apart, because the
//! sparse arm validates against every other arm's requirements.
//!
//! So this is one shape with `recommendation_type` saying which mode ran, the
//! same decision `compare_activities` made for the same reason. The fields a
//! mode does not fill are absent rather than null, so what is present is what
//! was measured.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// What `generate_recommendations` answers with.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RecommendationsResult {
    /// Which mode produced this: `training_plan`, `recovery`, `intensity`,
    /// `goal_specific`, `nutrition` or `comprehensive`. Read this first — it
    /// says which of the optional blocks below to expect. A sampled answer
    /// echoes back whatever mode was asked for.
    pub recommendation_type: String,
    /// `high`, `medium` or `low`. How much the athlete should act on this.
    pub priority: String,
    /// Why these recommendations, in the athlete's terms.
    pub reasoning: String,
    /// The recommendations themselves, most important first.
    pub recommendations: Vec<String>,
    /// `mcp_sampling` when the client's LLM wrote the prose. Absent means the
    /// deterministic path produced it, which is the distinction an athlete is
    /// entitled to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    // training_plan
    /// A week's shape, when the mode is `training_plan`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_structure: Vec<WeekStructure>,

    // recovery
    /// The recovery read, when the mode is `recovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_status: Option<String>,
    /// What to actually do about it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovery_actions: Vec<String>,

    // intensity
    /// How to distribute hard and easy work, when the mode is `intensity`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intensity_guidance: Vec<String>,

    // goal_specific
    /// The sport the athlete does most of, when the mode is `goal_specific`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_sport: Option<String>,
    /// Predicted race times, when there is enough history to predict from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_predictions: Option<RacePredictionSummary>,
    /// The phases of a season, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub periodization_phases: Vec<String>,

    // nutrition
    /// The window that matters most after the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_window: Option<String>,
    /// What the numbers say, in prose. Shared by `nutrition` and
    /// `comprehensive`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_insights: Vec<String>,
    /// Concrete meals rather than macro targets alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meal_suggestions: Vec<MealSuggestion>,
    /// The macro targets those meals are meant to hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macronutrient_targets: Option<MacronutrientTargets>,
    /// The session the nutrition advice is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_summary: Option<ActivitySummary>,

    // comprehensive
    /// The training picture the holistic read is built on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_summary: Option<TrainingSummary>,
    /// The four principles the advice is derived from, quoted so the athlete
    /// can check the reasoning rather than take it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_principles: Option<CorePrinciples>,

    /// The numbers behind the recommendation. Which of these are filled
    /// depends on the mode — see each field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<RecommendationMetrics>,
}

/// A week's training shape.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WeekStructure {
    /// What the week is for.
    pub focus: String,
    /// The low end of the session count.
    ///
    /// Two numbers rather than one key, because the key used to hold a number
    /// (`3`, `4`) in two branches and a string (`"5-6"`) in a third. One key
    /// with two JSON types cannot be described by any schema, and a client
    /// parsing it as a number silently dropped the busiest athletes' plan.
    pub sessions_per_week_min: u32,
    /// The high end. Equal to the low end when the plan names one number.
    pub sessions_per_week_max: u32,
    /// The sessions that define the week.
    pub key_workouts: Vec<String>,
}

/// Predicted times, when history supports a prediction.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RacePredictionSummary {
    /// The performance the prediction was derived from, in words.
    pub based_on: String,
    /// The athlete's VDOT, the number the paces come from.
    pub vdot: f64,
    /// Predicted times in minutes, keyed by the distance's name.
    pub race_times: HashMap<String, f64>,
}

/// One meal, with what it is for.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MealSuggestion {
    /// A short name for the option.
    pub option: String,
    /// What it actually is.
    pub description: String,
    /// Protein in grams.
    pub protein_g: u32,
    /// Carbohydrate in grams.
    pub carbs_g: u32,
    /// When to eat it relative to the session.
    pub timing: String,
}

/// Macro targets for the recovery window.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MacronutrientTargets {
    /// Protein in grams.
    pub protein_g: f64,
    /// Carbohydrate in grams.
    pub carbohydrates_g: f64,
    /// Fluid in millilitres.
    pub hydration_ml: f64,
}

/// The session the nutrition advice is about.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ActivitySummary {
    /// The activity's name as the provider recorded it.
    pub name: String,
    /// Its sport.
    #[serde(rename = "type")]
    pub sport: String,
    /// Duration in whole minutes.
    pub duration_minutes: u64,
    /// Distance in kilometres, rounded, when the activity carried one.
    pub distance_km: Option<f64>,
    /// Calories burned, rounded.
    pub calories: f64,
}

/// The training picture behind a comprehensive read.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrainingSummary {
    /// How many activities went into it.
    pub activities_analyzed: usize,
    /// Chronic training load, when it could be computed.
    pub ctl: Option<f64>,
    /// Acute training load.
    pub atl: Option<f64>,
    /// Training stress balance.
    pub tsb: Option<f64>,
    /// Whether volume jumped recently.
    pub volume_spike_detected: bool,
    /// Whether a hard/easy pattern was found at all.
    pub intensity_pattern_detected: bool,
    /// Whether either overtraining signal fired.
    pub overtraining_signals: bool,
}

/// The principles the advice is derived from.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CorePrinciples {
    /// Why regularity beats intensity.
    pub consistency: String,
    /// Why rest is where fitness is made.
    pub recovery: String,
    /// How fast to add volume.
    pub progression: String,
    /// How to split hard and easy.
    pub intensity: String,
}

/// The numbers a recommendation was derived from.
///
/// A union across the three modes that report numbers, because the mode is
/// already named on the parent: `training_plan` fills the schedule and load
/// fields, `recovery` the load and overtraining fields, `intensity` the
/// pattern fields. A field that is absent was not measured for that mode.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RecommendationMetrics {
    /// Sessions per week over the window. `training_plan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_activities_per_week: Option<f64>,
    /// How regular those weeks were, 0 to 1. `training_plan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consistency_score: Option<f64>,
    /// Whether volume jumped. `training_plan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_spike_detected: Option<bool>,
    /// Chronic training load. `training_plan` and `recovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctl: Option<f64>,
    /// Acute training load. `training_plan` and `recovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atl: Option<f64>,
    /// Training stress balance. `recovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tsb: Option<f64>,
    /// Whether heart rate drifted upward at a fixed effort. `recovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hr_drift_detected: Option<bool>,
    /// `low`, `moderate` or `high` — descriptive, never an injury
    /// prediction. `recovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    /// Whether a hard/easy pattern was found. `intensity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_detected: Option<bool>,
    /// The pattern in words. `intensity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_description: Option<String>,
    /// Share of sessions that were hard. `intensity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_percentage: Option<f64>,
    /// Share that were easy. `intensity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easy_percentage: Option<f64>,
    /// Whether the easy days were actually easy enough. `intensity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adequate_recovery: Option<bool>,
}
