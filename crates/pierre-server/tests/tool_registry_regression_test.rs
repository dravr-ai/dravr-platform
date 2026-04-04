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
