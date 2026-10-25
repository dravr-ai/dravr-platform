// ABOUTME: Intelligence handler module split into per-tool files for maintainability
// ABOUTME: Re-exports all 9 intelligence handler functions from their dedicated modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Activity intelligence analysis handler
mod activity;
/// Activity comparison handler
mod compare;
/// Fitness score calculation handler
mod fitness_score;
/// Activity metrics calculation handler
mod metrics;
/// Pattern detection handler
mod patterns;
/// Performance prediction handler
mod performance;
/// Training recommendations handler
mod recommendations;
/// The nutrition arm of it, split out to keep that file within budget
mod recommendations_nutrition;
/// Training load analysis handler
mod training_load;
/// Performance trend analysis handler
mod trends;

pub(crate) use activity::handle_get_activity_intelligence;
pub(crate) use activity::process_activity_analysis;
pub(crate) use compare::handle_compare_activities;
pub use fitness_score::calculate_fitness_metrics;
pub(crate) use fitness_score::handle_calculate_fitness_score;
pub(crate) use metrics::handle_calculate_metrics;
pub(crate) use patterns::handle_detect_patterns;
pub(crate) use performance::handle_predict_performance;
// One name per line: the crate re-exports some of these and keeps others
// crate-private, and a grouped `use` can carry only one visibility.
pub use recommendations::generate_training_recommendations;
pub(crate) use recommendations::handle_generate_recommendations;
pub use recommendations::recovery_actions_for;
pub use recommendations_nutrition::{
    generate_nutrition_recommendations, load_nutrition_athlete, NutritionAthlete,
};
pub use training_load::analyze_detailed_training_load;
pub(crate) use training_load::handle_analyze_training_load;
pub(crate) use trends::handle_analyze_performance_trends;
