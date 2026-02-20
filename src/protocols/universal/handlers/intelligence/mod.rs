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
/// Training load analysis handler
mod training_load;
/// Performance trend analysis handler
mod trends;

pub use activity::handle_get_activity_intelligence;
pub use compare::handle_compare_activities;
pub use fitness_score::handle_calculate_fitness_score;
pub use metrics::handle_calculate_metrics;
pub use patterns::handle_detect_patterns;
pub use performance::handle_predict_performance;
pub use recommendations::handle_generate_recommendations;
pub use training_load::handle_analyze_training_load;
pub use trends::handle_analyze_performance_trends;
