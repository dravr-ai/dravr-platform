// ABOUTME: Wires built-in tool implementations into the pierre-tool-runtime ToolRegistry.
// ABOUTME: Free functions live in pierre-server because they reach into tools::implementations::*.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Built-in tool registration
//!
//! The [`ToolRegistry`] data structure lives in `pierre-tool-runtime` so other
//! crates (tests, embedded harnesses) can build registries from custom tool
//! sets. The wiring that knows about pierre-server's `tools::implementations::*`
//! modules lives here.
//!
//! The two public entry points mirror the original inherent methods that lived
//! on `ToolRegistry`:
//!
//! - [`register_builtin_tools`] — invoked from `ServerContext::build` to
//!   populate the shared registry at startup.
//! - [`get_tools`] — test-only convenience helper that constructs a one-shot
//!   registry and returns its schemas. Production `tools/list` goes through the
//!   shared registry via the `PierreToolDispatcher` host seam.
//!
//! Each `register_*_tools` helper is a free function taking `&mut ToolRegistry`
//! and is gated by the same Cargo feature flag that gated the original
//! inherent method.

use tracing::info;

use pierre_tool_runtime::read_subject::DataClass;
use pierre_tool_runtime::registry::ToolRegistry;

/// Register all built-in tools based on feature flags.
///
/// This is called at startup to register every tool category enabled via
/// Cargo features.
pub fn register_builtin_tools(registry: &mut ToolRegistry) {
    info!("Registering built-in tools...");

    // Connection tools
    #[cfg(feature = "tools-connection")]
    register_connection_tools(registry);

    // Sync/refresh tools (uses tools-connection gate since they manage provider data)
    #[cfg(feature = "tools-connection")]
    register_sync_tools(registry);

    // Data tools
    #[cfg(feature = "tools-data")]
    register_data_tools(registry);

    // Endurance Phase 1 export tools
    #[cfg(feature = "tools-data")]
    register_endurance_export_tools(registry);

    // Endurance Phase 2 training-history tools
    #[cfg(feature = "tools-data")]
    register_endurance_history_tools(registry);

    // Endurance Phase 3 intervals/routes tools
    #[cfg(feature = "tools-data")]
    register_endurance_intervals_tools(registry);

    // Endurance Phase 5 workout tools
    #[cfg(feature = "tools-data")]
    register_endurance_workout_tools(registry);

    // Analytics tools
    #[cfg(feature = "tools-analytics")]
    register_analytics_tools(registry);

    // Route tools (discover_routes — Overpass API / OSM piste data; compose_route
    // — discovered stretches joined into one itinerary)
    #[cfg(feature = "tools-analytics")]
    register_route_tools(registry);

    // Goals tools
    #[cfg(feature = "tools-goals")]
    register_goals_tools(registry);

    // Group tools (consent-gated peer activity fetch)
    #[cfg(feature = "tools-groups")]
    register_group_tools(registry);

    // Configuration tools
    #[cfg(feature = "tools-config")]
    register_config_tools(registry);

    // Fitness config tools
    #[cfg(feature = "tools-config")]
    register_fitness_config_tools(registry);

    // Athlete physiology write (chat-callable, unlike the two above)
    #[cfg(feature = "tools-config")]
    register_physiology_tools(registry);

    // Nutrition tools
    #[cfg(feature = "tools-nutrition")]
    register_nutrition_tools(registry);

    // Sleep tools
    #[cfg(feature = "tools-sleep")]
    register_sleep_tools(registry);

    // Recipe tools
    #[cfg(feature = "tools-recipes")]
    register_recipe_tools(registry);

    // Agent tools
    #[cfg(feature = "tools-agents")]
    register_agent_tools(registry);

    // Agent Store tools (browse / search / install a published agent)
    #[cfg(feature = "tools-agents")]
    register_store_tools(registry);

    // Admin tools
    #[cfg(feature = "tools-admin")]
    register_admin_tools(registry);

    // Mobility tools
    #[cfg(feature = "tools-mobility")]
    register_mobility_tools(registry);

    // Memory tools (Tier 3 agent-authored memory)
    #[cfg(feature = "tools-memory")]
    register_memory_tools(registry);
    #[cfg(feature = "tools-memory")]
    register_training_plan_tools(registry);
    #[cfg(feature = "tools-memory")]
    register_training_plan_push_tools(registry);
    #[cfg(feature = "tools-memory")]
    register_calendar_preference_tools(registry);
    #[cfg(feature = "tools-memory")]
    register_training_plan_publish_tools(registry);

    // Coaching playbook GDPR/transparency tools (list / forget)
    #[cfg(feature = "tools-memory")]
    register_playbook_tools(registry);

    // Athlete commitment tools (create / cancel)
    #[cfg(feature = "tools-memory")]
    register_commitment_tools(registry);

    // Verification tools (bullshit detector)
    #[cfg(feature = "tools-verification")]
    register_verification_tools(registry);

    declare_data_classes(registry);

    info!("Registered {} built-in tools", registry.len());
}

/// What the tools registered above read, beside their categories
/// (carnet#1387).
///
/// A training-class tool reads a coached athlete's training when a call names
/// one: the athlete of a coach's thread about them, or `athlete=` over MCP
/// and A2A. A tool that reads no person's data stays callable in that thread.
/// A tool that reads data and is declared here under neither reads only its
/// caller's own, and the thread withholds it.
///
/// LIMITATION(registre#1387): `DATA_CLASSES` declares no wellness class, so a coach reads none of
/// an athlete's wellness under their `coach_wellness_read`: `get_sleep_sessions`,
/// `get_recovery_metrics`, `get_health_snapshots`, `get_heart_rate_series`, the five sleep tools
/// (`analyze_sleep_quality`, `calculate_recovery_score`, `suggest_rest_day`,
/// `track_sleep_trends`, `optimize_sleep_schedule`), `calculate_daily_nutrition` and
/// `get_nutrient_timing` declare no class, and a training read leaves out the wellness fields of
/// the athlete's physiology (resting heart rate, weight) and refuses its wellness inputs
/// (`read_subject::wellness_readable`).
const DATA_CLASSES: &[(&str, DataClass)] = &[
    // The daily load history and the activities it is computed from, each
    // session's detail and streams.
    ("get_training_history", DataClass::Training),
    ("analyze_training_load", DataClass::Training),
    ("calculate_fitness_score", DataClass::Training),
    ("get_activities", DataClass::Training),
    ("analyze_activity", DataClass::Training),
    ("get_activity_intelligence", DataClass::Training),
    ("extract_activity_streams", DataClass::Training),
    ("export_intervals", DataClass::Training),
    ("export_routes", DataClass::Training),
    ("calculate_metrics", DataClass::Training),
    ("compare_activities", DataClass::Training),
    ("analyze_weather_impact", DataClass::Training),
    ("export_latest_snapshot", DataClass::Training),
    // Trends, best efforts, predictions and the advice derived from the
    // sessions; a mode that would read the athlete's wellness is refused on a
    // coach's read.
    ("analyze_performance_trends", DataClass::Training),
    ("detect_patterns", DataClass::Training),
    ("predict_performance", DataClass::Training),
    ("generate_recommendations", DataClass::Training),
    // Goals and the progress toward them.
    ("suggest_goals", DataClass::Training),
    ("track_progress", DataClass::Training),
    ("analyze_goal_feasibility", DataClass::Training),
    // Training physiology and the estimates read against it.
    ("estimate_vo2max", DataClass::Training),
    ("estimate_lactate_thresholds", DataClass::Training),
    // The athlete's provider profile and totals, their connected sources and
    // how fresh each is.
    ("get_athlete", DataClass::Training),
    ("get_stats", DataClass::Training),
    ("list_data_sources", DataClass::Training),
    ("get_data_freshness", DataClass::Training),
    // Route and weather lookups: OpenStreetMap and forecast data, cached for
    // the caller's own next call.
    ("discover_routes", DataClass::NoSubject),
    ("compose_route", DataClass::NoSubject),
    ("get_weather_forecast", DataClass::NoSubject),
    // The caller's own catalogue of workouts and agents, not anyone's
    // training: what a coach prescribes from.
    ("list_workout_templates", DataClass::NoSubject),
    ("browse_agent_store", DataClass::NoSubject),
    ("search_agent_store", DataClass::NoSubject),
    // Food references and pure calculators.
    ("search_food", DataClass::NoSubject),
    ("get_food_details", DataClass::NoSubject),
    ("analyze_meal_nutrition", DataClass::NoSubject),
    ("get_recipe_constraints", DataClass::NoSubject),
    ("validate_recipe", DataClass::NoSubject),
    // The shared stretching and yoga references.
    ("get_stretching_exercise", DataClass::NoSubject),
    ("get_yoga_pose", DataClass::NoSubject),
    ("list_stretching_exercises", DataClass::NoSubject),
    ("list_yoga_poses", DataClass::NoSubject),
    ("suggest_stretches_for_activity", DataClass::NoSubject),
    ("suggest_yoga_sequence", DataClass::NoSubject),
];

/// Declare [`DATA_CLASSES`] on the tools this build registered.
fn declare_data_classes(registry: &mut ToolRegistry) {
    for (tool, class) in DATA_CLASSES {
        registry.declare_data_class(tool, *class);
    }
}

/// Register Tier 3 agent-authored memory tools.
#[cfg(feature = "tools-memory")]
fn register_memory_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::memory::create_memory_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering memory tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_memory_tools() {
        registry.register_with_category(Arc::from(tool), "memory");
    }

    info!(
        "Registered memory tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the training-plan persistence tools (get/save).
///
/// Both are chat-callable (category `memory`): the agent reads the stored
/// plan before answering plan questions and persists agreed plans in the
/// same turn it states them.
#[cfg(feature = "tools-memory")]
fn register_training_plan_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::training_plans::create_training_plan_tools;
    use std::sync::Arc;

    for tool in create_training_plan_tools() {
        registry.register_with_category(Arc::from(tool), "memory");
    }

    info!(
        "Registered training-plan tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the plan-push tool (`push_training_plan`).
///
/// Chat-callable (category `memory`, next to the plan it pushes): the agent
/// puts the stored plan on the athlete's provider calendar when asked, and
/// brings the calendar back in line after the plan changed.
#[cfg(feature = "tools-memory")]
fn register_training_plan_push_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::training_plan_push::create_training_plan_push_tools;
    use std::sync::Arc;

    for tool in create_training_plan_push_tools() {
        registry.register_with_category(Arc::from(tool), "memory");
    }

    info!(
        "Registered plan-push tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the preferred-calendar tool (`set_preferred_calendar`).
///
/// Chat-callable (category `memory`, beside the push it steers): the agent
/// records the athlete's answer when a push with several calendars connected
/// asks which one, or when they ask to switch (carnet#1393).
#[cfg(feature = "tools-memory")]
fn register_calendar_preference_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::calendar_preference::create_calendar_preference_tools;
    use std::sync::Arc;

    for tool in create_calendar_preference_tools() {
        registry.register_with_category(Arc::from(tool), "memory");
    }

    info!(
        "Registered preferred-calendar tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the draft publish tool (`publish_training_plan`).
///
/// Chat-callable (category `memory`, next to the plan it publishes): a
/// group's human coach makes their draft of a coached athlete's plan live, or
/// throws it away, when they ask to.
#[cfg(feature = "tools-memory")]
fn register_training_plan_publish_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::training_plan_drafts::create_training_plan_publish_tools;
    use std::sync::Arc;

    for tool in create_training_plan_publish_tools() {
        registry.register_with_category(Arc::from(tool), "memory");
    }

    info!(
        "Registered plan-publish tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the coaching-playbook GDPR/transparency tools.
///
/// `list_coaching_playbooks` is chat-callable (category `memory`) so the agent
/// can surface what it has learned; `forget_playbook` is registered under a
/// non-chat `playbook` category so the LLM cannot delete a playbook on its own.
#[cfg(feature = "tools-memory")]
fn register_playbook_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::playbooks::{
        ForgetPlaybookTool, ListCoachingPlaybooksTool,
    };
    use std::sync::Arc;
    registry.register_with_category(Arc::new(ListCoachingPlaybooksTool), "memory");
    registry.register_with_category(Arc::new(ForgetPlaybookTool), "playbook");
}

/// Register athlete-commitment tools.
///
/// Category `memory` so the agent can reach them on a chat turn — the sweep is
/// worthless if the only surface that can record a promise is an operator one.
#[cfg(feature = "tools-memory")]
fn register_commitment_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::commitments::create_commitment_tools;
    use std::sync::Arc;

    for tool in create_commitment_tools() {
        registry.register_with_category(Arc::from(tool), "memory");
    }

    info!(
        "Registered commitment tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register verification tools.
#[cfg(feature = "tools-verification")]
fn register_verification_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::verification::create_verification_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering verification tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_verification_tools() {
        registry.register_with_category(Arc::from(tool), "verification");
    }

    info!(
        "Registered verification tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register connection management tools
#[cfg(feature = "tools-connection")]
fn register_connection_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::connection::create_connection_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering connection tools (registry has {} tools)",
        registry.len()
    );

    // Register all connection tools with the "connection" category
    for tool in create_connection_tools() {
        registry.register_with_category(Arc::from(tool), "connection");
    }

    info!(
        "Registered connection tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register data access tools
#[cfg(feature = "tools-data")]
fn register_data_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::data::create_data_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering data tools (registry has {} tools)",
        registry.len()
    );

    // Register all data tools with the "data" category
    for tool in create_data_tools() {
        registry.register_with_category(Arc::from(tool), "data");
    }

    info!(
        "Registered data tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register analytics tools
#[cfg(feature = "tools-analytics")]
fn register_analytics_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::analytics::create_analytics_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering analytics tools (registry has {} tools)",
        registry.len()
    );

    // Register all analytics tools with the "analytics" category
    for tool in create_analytics_tools() {
        registry.register_with_category(Arc::from(tool), "analytics");
    }

    info!(
        "Registered analytics tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register goal management tools
#[cfg(feature = "tools-goals")]
fn register_goals_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::goals::create_goal_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering goals tools (registry has {} tools)",
        registry.len()
    );

    // Register all goal tools with the "goals" category
    for tool in create_goal_tools() {
        registry.register_with_category(Arc::from(tool), "goals");
    }

    info!(
        "Registered goals tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register group tools (consent-gated peer activity fetch)
#[cfg(feature = "tools-groups")]
fn register_group_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::groups::create_group_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering group tools (registry has {} tools)",
        registry.len()
    );

    // Register all group tools with the "groups" category
    for tool in create_group_tools() {
        registry.register_with_category(Arc::from(tool), "groups");
    }

    info!(
        "Registered group tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register configuration tools
#[cfg(feature = "tools-config")]
fn register_config_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::configuration::create_configuration_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering configuration tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_configuration_tools() {
        registry.register_with_category(Arc::from(tool), "configuration");
    }

    info!(
        "Registered configuration tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the athlete physiology write tool.
///
/// Category `physiology` rather than `configuration`: the category is what
/// `ToolRegistry::chat_callable_schemas` gates on, and an athlete stating
/// their FTP mid-conversation is self-report the agent must be able to save
/// on that turn — not an operator configuration write.
#[cfg(feature = "tools-config")]
fn register_physiology_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::physiology::create_physiology_tools;
    use std::sync::Arc;

    for tool in create_physiology_tools() {
        registry.register_with_category(Arc::from(tool), "physiology");
    }

    info!(
        "Registered physiology tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register fitness config tools
#[cfg(feature = "tools-config")]
fn register_fitness_config_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::fitness_config::create_fitness_config_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering fitness config tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_fitness_config_tools() {
        registry.register_with_category(Arc::from(tool), "fitness_config");
    }

    info!(
        "Registered fitness config tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register nutrition tools
#[cfg(feature = "tools-nutrition")]
fn register_nutrition_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::nutrition::create_nutrition_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering nutrition tools (registry has {} tools)",
        registry.len()
    );

    // Register all nutrition tools with the "nutrition" category
    for tool in create_nutrition_tools() {
        registry.register_with_category(Arc::from(tool), "nutrition");
    }

    info!(
        "Registered nutrition tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register sleep/recovery tools
#[cfg(feature = "tools-sleep")]
fn register_sleep_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::sleep::create_sleep_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering sleep tools (registry has {} tools)",
        registry.len()
    );

    // Register all sleep tools with the "sleep" category
    for tool in create_sleep_tools() {
        registry.register_with_category(Arc::from(tool), "sleep");
    }

    info!(
        "Registered sleep tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register recipe tools
#[cfg(feature = "tools-recipes")]
fn register_recipe_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::recipes::create_recipe_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering recipe tools (registry has {} tools)",
        registry.len()
    );

    // Register all recipe tools with the "recipes" category
    for tool in create_recipe_tools() {
        registry.register_with_category(Arc::from(tool), "recipes");
    }

    info!(
        "Registered recipe tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register agent tools
#[cfg(feature = "tools-agents")]
fn register_agent_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::agents::create_agent_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering coach tools (registry has {} tools)",
        registry.len()
    );

    // Register all agent tools with the "coaches" category
    for tool in create_agent_tools() {
        registry.register_with_category(Arc::from(tool), "coaches");
    }

    info!(
        "Registered coach tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the Agent Store tools.
///
/// Category `store`, which is chat-callable: the marketplace used to be
/// reachable only from the web UI because `CHAT_CALLABLE_CATEGORIES` named no
/// store category, so no chat surface — web, mobile or messaging — could
/// browse or install an agent. Uninstall stays out of the category: removing a
/// agent the athlete has history with is a deliberate UI gesture, not an
/// inference from a sentence.
#[cfg(feature = "tools-agents")]
fn register_store_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::store::create_store_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering Coach Store tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_store_tools() {
        registry.register_with_category(Arc::from(tool), "store");
    }

    info!(
        "Registered Coach Store tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register admin tools
#[cfg(feature = "tools-admin")]
fn register_admin_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::admin::create_admin_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering admin tools (registry has {} tools)",
        registry.len()
    );

    // Register all admin tools with the "admin" category
    for tool in create_admin_tools() {
        registry.register_with_category(Arc::from(tool), "admin");
    }

    info!(
        "Registered admin tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register mobility tools (stretching exercises, yoga poses)
#[cfg(feature = "tools-mobility")]
fn register_mobility_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::mobility::create_mobility_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering mobility tools (registry has {} tools)",
        registry.len()
    );

    // Register all mobility tools with the "mobility" category
    for tool in create_mobility_tools() {
        registry.register_with_category(Arc::from(tool), "mobility");
    }

    info!(
        "Registered mobility tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register sync/refresh tools (`refresh_provider_data`, `get_data_freshness`)
#[cfg(feature = "tools-connection")]
fn register_sync_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::sync::create_sync_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering sync tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_sync_tools() {
        registry.register_with_category(Arc::from(tool), "connection");
    }

    info!(
        "Registered sync tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register the route tools: `discover_routes` (Overpass + OSM piste data) and
/// `compose_route` (discovered stretches joined into one drawable itinerary)
#[cfg(feature = "tools-analytics")]
fn register_route_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::routes::create_route_tools;
    use pierre_tool_runtime::implementations::weather_forecast::create_weather_forecast_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering route tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_route_tools() {
        registry.register_with_category(Arc::from(tool), "analytics");
    }

    for tool in create_weather_forecast_tools() {
        registry.register_with_category(Arc::from(tool), "analytics");
    }

    info!(
        "Registered route tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register Endurance export tools (`export_latest_snapshot`, `export_dossier`).
///
/// Both are read-only data tools through which agents pull the structured
/// Endurance latest-snapshot and dossier contracts via MCP.
#[cfg(feature = "tools-data")]
fn register_endurance_export_tools(registry: &mut ToolRegistry) {
    use super::implementations::endurance_export::create_endurance_export_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering Endurance export tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_endurance_export_tools() {
        registry.register_with_category(Arc::from(tool), "data");
    }

    info!(
        "Registered Endurance export tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register Endurance workout tools (`list_workout_templates`,
/// `prescribe_workout`).
#[cfg(feature = "tools-data")]
fn register_endurance_workout_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::endurance_workouts::create_endurance_workout_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering Endurance workout tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_endurance_workout_tools() {
        registry.register_with_category(Arc::from(tool), "data");
    }

    info!(
        "Registered Endurance workout tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register Endurance intervals/routes tools (`export_intervals`,
/// `export_routes`, `extract_activity_streams`).
///
/// All three are read-only data tools that surface the per-activity
/// payloads agents need for tempo/threshold analysis and terrain
/// classification.
#[cfg(feature = "tools-data")]
fn register_endurance_intervals_tools(registry: &mut ToolRegistry) {
    use super::implementations::endurance_intervals::create_endurance_intervals_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering Endurance intervals tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_endurance_intervals_tools() {
        registry.register_with_category(Arc::from(tool), "data");
    }

    info!(
        "Registered Endurance intervals tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register Endurance training-history tools (`compute_training_history`,
/// `get_training_history`).
///
/// Read tool surfaces the persisted daily rollup; write tool triggers an
/// on-demand backfill / recompute. Both read and write the
/// `training_history` rollup.
#[cfg(feature = "tools-data")]
fn register_endurance_history_tools(registry: &mut ToolRegistry) {
    use super::implementations::endurance_history::create_endurance_history_tools;
    use std::sync::Arc;
    use tracing::debug;

    debug!(
        "Registering Endurance history tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_endurance_history_tools() {
        registry.register_with_category(Arc::from(tool), "data");
    }

    info!(
        "Registered Endurance history tools (registry now has {} tools)",
        registry.len()
    );
}

/// Build a fresh `ToolRegistry` populated with all built-in tools and return
/// their schemas.
///
/// Production code should use the shared `ToolRegistry` from `ServerContext`
/// instead — this helper exists for tests, where the registry instance is
/// constructed on demand.
#[must_use]
pub fn get_tools() -> Vec<pierre_mcp_schema::ToolSchema> {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry.all_schemas()
}
