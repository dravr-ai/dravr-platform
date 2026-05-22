// ABOUTME: Regression tests for unified tool registry architecture.
// ABOUTME: Ensures PUBLIC_DISCOVERY_TOOLS are registered and tool count stays above threshold.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![doc = "Tool registry regression tests for unified architecture"]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use pierre_mcp_server::constants::tools::PUBLIC_DISCOVERY_TOOLS;
use pierre_mcp_server::mcp::schema::get_tools;
use pierre_mcp_server::tools::registry::ToolRegistry;
use std::collections::HashSet;

/// Minimum number of user-visible tools. This threshold prevents silent regressions
/// where tools disappear from the registry without anyone noticing.
const MIN_USER_VISIBLE_TOOLS: usize = 55;

#[test]
fn test_public_discovery_tools_are_all_registered() {
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let registered_names: HashSet<&str> = registry.tool_names().into_iter().collect();

    let mut missing = Vec::new();
    for tool_name in PUBLIC_DISCOVERY_TOOLS {
        if !registered_names.contains(tool_name) {
            missing.push(*tool_name);
        }
    }

    assert!(
        missing.is_empty(),
        "PUBLIC_DISCOVERY_TOOLS contains {} tools not registered in ToolRegistry: {:?}\n\
         This means unauthenticated clients will see tools they cannot execute.\n\
         Fix: Add McpTool implementations for these tools.",
        missing.len(),
        missing
    );

    println!(
        "All {} PUBLIC_DISCOVERY_TOOLS are registered in ToolRegistry",
        PUBLIC_DISCOVERY_TOOLS.len()
    );
}

#[test]
fn test_user_visible_tool_count_above_threshold() {
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let user_schemas = registry.user_visible_schemas();
    let count = user_schemas.len();

    assert!(
        count >= MIN_USER_VISIBLE_TOOLS,
        "User-visible tool count ({count}) dropped below minimum threshold ({MIN_USER_VISIBLE_TOOLS}).\n\
         This indicates a regression: tools may have been accidentally removed.\n\
         If this is intentional, update MIN_USER_VISIBLE_TOOLS."
    );

    println!("User-visible tools: {count} (threshold: {MIN_USER_VISIBLE_TOOLS})");
}

#[test]
fn test_all_transports_return_consistent_tool_list() {
    // The ToolRegistry is the single source of truth for all transports.
    // Verify that get_tools() (used by tests/legacy) returns the same tools
    // as ToolRegistry.all_schemas().
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let registry_tools: HashSet<String> =
        registry.all_schemas().into_iter().map(|t| t.name).collect();

    let get_tools_result: HashSet<String> = get_tools().into_iter().map(|t| t.name).collect();

    assert_eq!(
        registry_tools,
        get_tools_result,
        "get_tools() and ToolRegistry.all_schemas() return different tool sets.\n\
         Only in registry: {:?}\n\
         Only in get_tools: {:?}",
        registry_tools
            .difference(&get_tools_result)
            .collect::<Vec<_>>(),
        get_tools_result
            .difference(&registry_tools)
            .collect::<Vec<_>>()
    );

    println!(
        "All {} tools are consistent across transports",
        registry_tools.len()
    );
}

#[test]
fn test_chat_callable_surface_includes_coach_prompt_dependencies() {
    // Coach prompts in vendor/contremaitre/prompts/coaches/training/ reference
    // these tool names. They must all be in chat_callable_schemas(), otherwise
    // the LLM gets the prose name advertised in "Available Tools" but no
    // matching function declaration — producing truthful "no callable tool"
    // refusals (the bug fixed in this commit).
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let chat_surface: HashSet<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|t| t.name)
        .collect();

    // These names appear in coach prompts and must be callable at chat time.
    // The list is conservative — add new names as coaches reference new tools.
    let required = [
        // Endurance dossier/history (drove the regression that motivated this test)
        "export_dossier",
        "export_latest_snapshot",
        "get_training_history",
        "compute_training_history",
        // Endurance workout templates / prescription
        "list_workout_templates",
        "prescribe_workout",
        // Endurance per-activity exports
        "export_intervals",
        "export_routes",
        "extract_activity_streams",
        // Legacy 15 — must stay callable
        "get_connection_status",
        "connect_provider",
        "disconnect_provider",
        "get_activities",
        "get_athlete",
        "get_stats",
        "analyze_activity",
        "get_activity_intelligence",
        "analyze_performance_trends",
        "compare_activities",
        "calculate_fitness_score",
        "analyze_training_load",
        "calculate_recovery_score",
        "suggest_rest_day",
        "generate_recommendations",
    ];

    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !chat_surface.contains(*name))
        .collect();

    assert!(
        missing.is_empty(),
        "chat_callable_schemas() is missing {} tool(s) referenced by coach prompts: {:?}\n\
         If a tool was intentionally moved off the chat surface, audit \
         vendor/contremaitre/prompts/coaches/ for prose that still says \
         `call \\`{{name}}\\`` and update those prompts in the same commit.",
        missing.len(),
        missing
    );

    println!(
        "chat_callable surface ({} tools) covers all {} coach-prompt tool references",
        chat_surface.len(),
        required.len()
    );
}

#[test]
fn test_chat_callable_surface_excludes_admin_and_management_tools() {
    // Tools that should NOT be callable from chat: coach create/delete/assign
    // (UI actions), store install/uninstall (UI actions), admin_* (operator),
    // config write/delete (admin-ish), verify_claim (debug). The LLM should
    // not fire these on natural-language input.
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let chat_surface: HashSet<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|t| t.name)
        .collect();

    let forbidden = [
        "create_coach",
        "delete_coach",
        "update_coach",
        "admin_assign_coach",
        "admin_create_system_coach",
        "verify_claim",
    ];

    let leaked: Vec<&str> = forbidden
        .iter()
        .copied()
        .filter(|name| chat_surface.contains(*name))
        .collect();

    assert!(
        leaked.is_empty(),
        "Management/admin tool(s) leaked into the chat-callable surface: {leaked:?}\n\
         These categories must stay off function-calling: coaches, admin, store, \
         verification, configuration, fitness_config."
    );
}

#[test]
fn test_five_analytics_tools_are_registered() {
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let required_analytics = [
        "analyze_activity",
        "get_activity_intelligence",
        "calculate_metrics",
        "analyze_performance_trends",
        "compare_activities",
    ];

    for tool_name in &required_analytics {
        assert!(
            registry.contains(tool_name),
            "Analytics tool '{tool_name}' is not registered in ToolRegistry.\n\
             This was a known regression from Feb 2026. These tools must be registered."
        );
    }

    println!("All 5 previously-missing analytics tools are registered");
}

#[test]
fn test_annotations_present_on_analytics_tools() {
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();

    let analytics_tools = registry.tools_in_category("analytics");

    assert!(
        !analytics_tools.is_empty(),
        "No analytics tools registered in 'analytics' category"
    );

    let schemas = registry.all_schemas();
    for schema in &schemas {
        if analytics_tools.contains(&schema.name.as_str()) {
            assert!(
                schema.annotations.is_some(),
                "Analytics tool '{}' is missing annotations. \
                 All analytics tools should have readOnlyHint annotations.",
                schema.name
            );
        }
    }

    println!(
        "All {} analytics tools have annotations",
        analytics_tools.len()
    );
}
