// ABOUTME: Tool implementations relocated into pierre-tool-runtime as their dependencies stabilized.
// ABOUTME: Each submodule is feature-gated identically to the original pierre-server location.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Tool Implementations
//!
//! Concrete [`McpTool`](crate::McpTool) implementations that depend
//! only on the scaffolding in this crate plus shared workspace deps
//! (`pierre-core`, `pierre-database`, `pierre-formatters`, `pierre-mcp-schema`,
//! `pierre-tools-core`).
//!
//! Modules here are feature-gated by the same Cargo feature flags used at
//! the pierre-server boundary so consumers can compile a subset.
//!
//! - `admin` — admin-only coach + assignment management (`tools-admin`)
//! - `coaches` — coach CRUD / activate / favorites (`tools-coaches`)
//! - `endurance_workouts` — list / prescribe Endurance cornerstone workouts
//!   (`tools-data`)
//! - `fitness_config` — get / set / list / delete fitness config (`tools-config`)
//! - `memory` — coach-authored memory: notes, followups, fact recall
//!   (`tools-memory`)
//! - `mobility` — stretching exercises, yoga poses, mobility recommendations
//!   (`tools-mobility`)
//! - `recipes` — recipe constraints / validate / save / list / search
//!   (`tools-recipes`)
//! - `routes` — `discover_routes` OSM + Overpass route discovery
//!   (`tools-analytics`)
//! - `sync` — `refresh_provider_data`, `get_data_freshness` (`tools-connection`)
//! - `verification` — `verify_claim` tool (`tools-verification`)

/// Shared bridge helpers for `McpTool::execute` impls that delegate to legacy
/// boxed-future handler functions (`build_universal_request`,
/// `map_universal_response`). Compiled whenever any feature uses it.
#[cfg(any(
    feature = "tools-data",
    feature = "tools-sleep",
    feature = "tools-analytics"
))]
pub(crate) mod handler_bridge;

/// Admin-only tools (`tools-admin` feature).
#[cfg(feature = "tools-admin")]
pub mod admin;

/// Analytics tools: `analyze_activity`, `calculate_metrics`,
/// `analyze_performance_trends`, etc. (`tools-analytics` feature).
#[cfg(feature = "tools-analytics")]
pub mod analytics;

/// Coach CRUD tools (`tools-coaches` feature).
#[cfg(feature = "tools-coaches")]
pub mod coaches;

/// User configuration tools: `get_configuration_catalog`, `get_user_configuration`,
/// etc. (`tools-config` feature).
#[cfg(feature = "tools-config")]
pub mod configuration;

/// Provider connection tools: `connect_provider`, `get_connection_status`,
/// `disconnect_provider` (`tools-connection` feature).
#[cfg(feature = "tools-connection")]
pub mod connection;

/// Data access tools: `get_activities`, `get_athlete`, `get_stats`,
/// `get_activity_intelligence` (`tools-data` feature).
#[cfg(feature = "tools-data")]
pub mod data;

/// Endurance Phase 5 workout tools: `list_workout_templates`, `prescribe_workout`
/// (`tools-data` feature).
#[cfg(feature = "tools-data")]
pub mod endurance_workouts;

/// Fitness configuration tools (`tools-config` feature).
#[cfg(feature = "tools-config")]
pub mod fitness_config;

/// Shared support for the fitness-provider API tools.
#[cfg(any(feature = "tools-data", feature = "tools-analytics"))]
pub mod fitness_support;

/// Goal management tools: `set_goal`, `track_progress`, `suggest_goals`,
/// `analyze_goal_feasibility` (`tools-goals` feature).
#[cfg(feature = "tools-goals")]
pub mod goals;

/// Group tools: consent-gated peer activity fetch (`tools-groups` feature).
#[cfg(feature = "tools-groups")]
pub mod groups;

/// Athlete commitment tools: `commitment_create`, `commitment_cancel` (`tools-memory` feature).
#[cfg(feature = "tools-memory")]
pub mod commitments;
/// Memory tools: coach-authored notes, followups, fact recall (`tools-memory` feature).
#[cfg(feature = "tools-memory")]
pub mod memory;
/// Coaching playbook GDPR/transparency tools: list_coaching_playbooks, forget_playbook (`tools-memory`).
#[cfg(feature = "tools-memory")]
pub mod playbooks;
/// Training-plan persistence tools (get/save).
pub mod training_plans;

/// Mobility / stretching / yoga tools (`tools-mobility` feature).
#[cfg(feature = "tools-mobility")]
pub mod mobility;

/// Nutrition and meal planning tools: `calculate_daily_nutrition`,
/// `get_nutrient_timing`, `search_food`, etc. (`tools-nutrition` feature).
#[cfg(feature = "tools-nutrition")]
pub mod nutrition;

/// Recipe management tools: get/validate/save/list/get/delete/search
/// (`tools-recipes` feature).
#[cfg(feature = "tools-recipes")]
pub mod recipes;

/// Route discovery tools: `discover_routes` (Overpass + OSM piste data)
/// (`tools-analytics` feature).
#[cfg(feature = "tools-analytics")]
pub mod routes;

/// Weather forecast tool: `get_weather_forecast` (Open-Meteo forecast API)
/// (`tools-analytics` feature).
#[cfg(feature = "tools-analytics")]
pub mod weather_forecast;

/// Sleep and recovery tools: `analyze_sleep_quality`, `calculate_recovery_score`,
/// `suggest_rest_day`, etc. (`tools-sleep` feature).
#[cfg(feature = "tools-sleep")]
pub mod sleep;

/// Sync / refresh tools: `refresh_provider_data`, `get_data_freshness`
/// (`tools-connection` feature).
#[cfg(feature = "tools-connection")]
pub mod sync;

/// Verification tools: `verify_claim` (`tools-verification` feature).
#[cfg(feature = "tools-verification")]
pub mod verification;
