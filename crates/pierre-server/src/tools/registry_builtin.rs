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
//!   shared registry via `McpRequestProcessor::handle_tools_list`.
//!
//! Each `register_*_tools` helper is a free function taking `&mut ToolRegistry`
//! and is gated by the same Cargo feature flag that gated the original
//! inherent method.

use std::sync::Arc;

use tracing::{debug, info};

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

    // Route discovery tools (discover_routes — Overpass API / OSM piste data)
    #[cfg(feature = "tools-analytics")]
    register_route_tools(registry);

    // Goals tools
    #[cfg(feature = "tools-goals")]
    register_goals_tools(registry);

    // Configuration tools
    #[cfg(feature = "tools-config")]
    register_config_tools(registry);

    // Fitness config tools
    #[cfg(feature = "tools-config")]
    register_fitness_config_tools(registry);

    // Nutrition tools
    #[cfg(feature = "tools-nutrition")]
    register_nutrition_tools(registry);

    // Sleep tools
    #[cfg(feature = "tools-sleep")]
    register_sleep_tools(registry);

    // Recipe tools
    #[cfg(feature = "tools-recipes")]
    register_recipe_tools(registry);

    // Coach tools
    #[cfg(feature = "tools-coaches")]
    register_coach_tools(registry);

    // Admin tools
    #[cfg(feature = "tools-admin")]
    register_admin_tools(registry);

    // Mobility tools
    #[cfg(feature = "tools-mobility")]
    register_mobility_tools(registry);

    // Store tools
    #[cfg(feature = "tools-store")]
    register_store_tools(registry);

    // Memory tools (Tier 3 coach-authored memory)
    #[cfg(feature = "tools-memory")]
    register_memory_tools(registry);

    // Verification tools (Tier 5.5 bullshit detector)
    #[cfg(feature = "tools-verification")]
    register_verification_tools(registry);

    info!("Registered {} built-in tools", registry.len());
}

/// Register Tier 3 coach-authored memory tools.
#[cfg(feature = "tools-memory")]
fn register_memory_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::memory::create_memory_tools;

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

/// Register Tier 5.5 verification tools.
#[cfg(feature = "tools-verification")]
fn register_verification_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::verification::create_verification_tools;

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

/// Register configuration tools
#[cfg(feature = "tools-config")]
fn register_config_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::configuration::create_configuration_tools;

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

/// Register fitness config tools
#[cfg(feature = "tools-config")]
fn register_fitness_config_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::fitness_config::create_fitness_config_tools;

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

/// Register coach tools
#[cfg(feature = "tools-coaches")]
fn register_coach_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::coaches::create_coach_tools;

    debug!(
        "Registering coach tools (registry has {} tools)",
        registry.len()
    );

    // Register all coach tools with the "coaches" category
    for tool in create_coach_tools() {
        registry.register_with_category(Arc::from(tool), "coaches");
    }

    info!(
        "Registered coach tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register admin tools
#[cfg(feature = "tools-admin")]
fn register_admin_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::admin::create_admin_tools;

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

/// Register store tools (browse, search, install coaches)
#[cfg(feature = "tools-store")]
fn register_store_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::store::create_store_tools;

    debug!(
        "Registering store tools (registry has {} tools)",
        registry.len()
    );

    // Register all store tools with the "store" category
    for tool in create_store_tools() {
        registry.register_with_category(Arc::from(tool), "store");
    }

    info!(
        "Registered store tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register sync/refresh tools (`refresh_provider_data`, `get_data_freshness`)
#[cfg(feature = "tools-connection")]
fn register_sync_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::sync::create_sync_tools;

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

/// Register route-discovery tools (`discover_routes` — Overpass + OSM piste data)
#[cfg(feature = "tools-analytics")]
fn register_route_tools(registry: &mut ToolRegistry) {
    use pierre_tool_runtime::implementations::routes::create_route_tools;

    debug!(
        "Registering route tools (registry has {} tools)",
        registry.len()
    );

    for tool in create_route_tools() {
        registry.register_with_category(Arc::from(tool), "analytics");
    }

    info!(
        "Registered route tools (registry now has {} tools)",
        registry.len()
    );
}

/// Register Endurance export tools (`export_latest_snapshot`, `export_dossier`).
///
/// Both are read-only data tools that surface the same payloads as the
/// `/api/v1/endurance/{latest,dossier}` HTTP endpoints, so coaches can
/// pull the structured Endurance contracts via MCP.
#[cfg(feature = "tools-data")]
fn register_endurance_export_tools(registry: &mut ToolRegistry) {
    use super::implementations::endurance_export::create_endurance_export_tools;

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
/// payloads coaches need for tempo/threshold analysis and terrain
/// classification.
#[cfg(feature = "tools-data")]
fn register_endurance_intervals_tools(registry: &mut ToolRegistry) {
    use super::implementations::endurance_intervals::create_endurance_intervals_tools;

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
/// on-demand backfill / recompute. Both share the
/// `/api/v1/endurance/history` semantics.
#[cfg(feature = "tools-data")]
fn register_endurance_history_tools(registry: &mut ToolRegistry) {
    use super::implementations::endurance_history::create_endurance_history_tools;

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
