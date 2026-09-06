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
//! - `endurance_workouts` — list the workout bank by purpose / phase / sport, prescribe one
//!   (`tools-data`)
//! - `fitness_config` — get / set / list / delete fitness config (`tools-config`)
//! - `memory` — coach-authored memory: notes, followups, fact recall
//!   (`tools-memory`)
//! - `physiology` — `set_physiology`, the athlete's typed measurements, and
//!   `estimate_vo2max`, a field-test estimate for it (`tools-config`)
//! - `lactate_thresholds` — `estimate_lactate_thresholds`, LT1 and LT2 from a
//!   lactate step test, the read-only sibling of `set_physiology`
//!   (`tools-config`)
//! - `mobility` — stretching exercises, yoga poses, mobility recommendations
//!   (`tools-mobility`)
//! - `recipes` — recipe constraints / validate / save / list / search
//!   (`tools-recipes`)
//! - `routes` — `discover_routes` OSM + Overpass route discovery
//!   (`tools-analytics`)
//! - `store` — Coach Store browse / search / install (`tools-coaches`)
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

/// Answer shapes for the coach tools, split out because `coaches` is at its
/// size ceiling.
pub mod coaches_output;

/// Result envelope and annotation sets shared by the coach tools (`tools-coaches` feature).
#[cfg(feature = "tools-coaches")]
mod coaches_tool_shape;

/// User configuration tools: `get_configuration_catalog`, `get_user_configuration`,
/// etc. (`tools-config` feature).
#[cfg(feature = "tools-config")]
pub mod configuration;

/// Provider connection tools: `connect_provider`, `get_connection_status`,
/// `disconnect_provider` (`tools-connection` feature).
#[cfg(feature = "tools-connection")]
pub mod connection;

/// Athlete profile and aggregate statistics tools: `get_athlete`, `get_stats`
/// (`tools-data` feature).
#[cfg(feature = "tools-data")]
pub mod athlete_stats;

/// Data access tools: `get_activities`, `get_activity_intelligence`
/// (`tools-data` feature).
#[cfg(feature = "tools-data")]
pub mod data;
/// Shared vocabulary for the data tools, split out of `data` for the size gate.
pub mod data_helpers;

/// Stored health-data tools: `get_sleep_sessions`, `get_recovery_metrics`,
/// `get_health_snapshots`, `list_data_sources` (`tools-data` feature).
#[cfg(feature = "tools-data")]
pub mod stored_data;

/// Endurance Phase 5 workout tools: `list_workout_templates`, `prescribe_workout`
/// (`tools-data` feature).
#[cfg(feature = "tools-data")]
pub mod endurance_workouts;

/// Fitness configuration tools (`tools-config` feature).
#[cfg(feature = "tools-config")]
pub mod fitness_config;

/// Athlete physiology tools (`tools-config` feature).
///
/// `set_physiology` is the only production writer of
/// `user_physiological_profiles`; `estimate_vo2max` turns a field test the
/// athlete describes into a number for it. The lactate step test has its own
/// module, [`lactate_thresholds`].
#[cfg(feature = "tools-config")]
pub mod physiology;

/// `estimate_lactate_thresholds` — LT1 and LT2 from a lactate step test the
/// athlete reports, the read-only sibling of `set_physiology`
/// (`tools-config` feature).
#[cfg(feature = "tools-config")]
pub mod lactate_thresholds;

/// The coach-facing per-activity DTO rendered by `mode=summary`.
#[cfg(any(feature = "tools-data", feature = "tools-analytics"))]
pub mod activity_summary;

/// Shared support for the fitness-provider API tools.
/// Renders the activity window as the prose list the coach reads and cites
#[cfg(any(feature = "tools-data", feature = "tools-analytics"))]
pub mod activity_list_render;

#[cfg(any(feature = "tools-data", feature = "tools-analytics"))]
pub mod fitness_support;

/// Goal management tools: `set_goal`, `track_progress`, `suggest_goals`,
/// `analyze_goal_feasibility` (`tools-goals` feature).
#[cfg(feature = "tools-goals")]
pub mod goals;

/// Answer shapes for the goal tools, split out because `goals` is at its size
/// ceiling (`tools-goals` feature).
#[cfg(feature = "tools-goals")]
pub mod goals_output;

/// Group tools: consent-gated peer activity fetch (`tools-groups` feature).
#[cfg(feature = "tools-groups")]
pub mod groups;

/// Shared helpers for every tool that writes to an athlete's provider calendar.
pub mod calendar;
/// Athlete commitment tools: `commitment_create`, `commitment_cancel` (`tools-memory` feature).
#[cfg(feature = "tools-memory")]
pub mod commitments;
/// Training-plan persistence tools (get/save).
pub mod guided_flow;
/// Memory tools: coach-authored notes, followups, fact recall (`tools-memory` feature).
#[cfg(feature = "tools-memory")]
pub mod memory;
/// Whose plan the training-plan tools act on: the caller's own, or a coached athlete's.
pub mod plan_scope;
/// Coaching playbook GDPR/transparency tools: list_coaching_playbooks, forget_playbook (`tools-memory`).
#[cfg(feature = "tools-memory")]
pub mod playbooks;
/// `push_training_plan` — the athlete's active plan onto their provider calendar, reconciled.
pub mod training_plan_push;
/// The schema `save_training_plan` advertises, and the rejection skeleton generated from it.
pub mod training_plan_schema;
/// Notify telemetry for training-plan writes: what was saved, and what it leaves uncovered.
pub mod training_plan_telemetry;
/// The vision half of a save payload — flavour provenance, phase targets, template references.
pub mod training_plan_vision;
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

/// Process-wide shared USDA client for the nutrition + recipe fan-out tools.
#[cfg(any(feature = "tools-nutrition", feature = "tools-recipes"))]
pub mod usda_shared;

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

/// Coach Store tools: `browse_coach_store`, `search_coach_store`,
/// `install_coach_from_store` (`tools-coaches` feature).
#[cfg(feature = "tools-coaches")]
pub mod store;

/// Sync / refresh tools: `refresh_provider_data`, `get_data_freshness`
/// (`tools-connection` feature).
#[cfg(feature = "tools-connection")]
pub mod sync;

/// Verification tools: `verify_claim` (`tools-verification` feature).
#[cfg(feature = "tools-verification")]
pub mod verification;
