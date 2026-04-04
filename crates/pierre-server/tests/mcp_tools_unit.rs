// ABOUTME: Unit tests for individual MCP analytics tools
// ABOUTME: Tests MCP schema tools and tool execution functionality
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for individual MCP analytics tools

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_mcp_server::mcp::schema::*;

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_mcp_tool_schemas() {
    // Test that all analytics tools are properly defined
    let tools = get_tools();

    // ToolRegistry is the single source of truth. Tool count may grow as
    // McpTool implementations are added. Check minimum threshold instead.
    assert!(
        tools.len() >= 65,
        "Expected at least 65 tools, got {}",
        tools.len()
    );

    // Check key analytics tools are present
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    // Core functionality
    assert!(tool_names.contains(&"get_activities"));
    assert!(tool_names.contains(&"get_athlete"));
    assert!(tool_names.contains(&"get_stats"));
    assert!(tool_names.contains(&"get_activity_intelligence"));

    // Connection management (legacy connect_strava/connect_fitbit removed)
    assert!(tool_names.contains(&"get_connection_status"));
    assert!(tool_names.contains(&"disconnect_provider"));

    // Analytics tools
    assert!(tool_names.contains(&"analyze_activity"));
    assert!(tool_names.contains(&"calculate_metrics"));
    assert!(tool_names.contains(&"analyze_performance_trends"));
    assert!(tool_names.contains(&"compare_activities"));
    assert!(tool_names.contains(&"detect_patterns"));

    // Goal management
    assert!(tool_names.contains(&"set_goal"));
    assert!(tool_names.contains(&"track_progress"));
    assert!(tool_names.contains(&"suggest_goals"));
    assert!(tool_names.contains(&"analyze_goal_feasibility"));

    // Advanced analytics
    assert!(tool_names.contains(&"calculate_fitness_score"));
    assert!(tool_names.contains(&"analyze_training_load"));

    // Fitness configuration tools
    assert!(tool_names.contains(&"get_fitness_config"));
    assert!(tool_names.contains(&"set_fitness_config"));
    assert!(tool_names.contains(&"list_fitness_configs"));
    assert!(tool_names.contains(&"delete_fitness_config"));

    // Recipe management tools (Combat des Chefs)
    assert!(tool_names.contains(&"get_recipe_constraints"));
    assert!(tool_names.contains(&"validate_recipe"));
    assert!(tool_names.contains(&"save_recipe"));
    assert!(tool_names.contains(&"list_recipes"));
    assert!(tool_names.contains(&"get_recipe"));
    assert!(tool_names.contains(&"delete_recipe"));
    assert!(tool_names.contains(&"search_recipes"));

    // Coach management tools (custom AI personas)
    assert!(tool_names.contains(&"list_coaches"));
    assert!(tool_names.contains(&"create_coach"));
    assert!(tool_names.contains(&"get_coach"));
    assert!(tool_names.contains(&"update_coach"));
    assert!(tool_names.contains(&"delete_coach"));
    assert!(tool_names.contains(&"toggle_coach_favorite"));
    assert!(tool_names.contains(&"search_coaches"));
    assert!(tool_names.contains(&"activate_coach"));
    assert!(tool_names.contains(&"deactivate_coach"));
    assert!(tool_names.contains(&"get_active_coach"));
    assert!(tool_names.contains(&"hide_coach"));
    assert!(tool_names.contains(&"show_coach"));
    assert!(tool_names.contains(&"list_hidden_coaches"));

    // Admin coach management tools (system coaches)
    assert!(tool_names.contains(&"admin_list_system_coaches"));
    assert!(tool_names.contains(&"admin_create_system_coach"));
    assert!(tool_names.contains(&"admin_get_system_coach"));
    assert!(tool_names.contains(&"admin_update_system_coach"));
    assert!(tool_names.contains(&"admin_delete_system_coach"));
    assert!(tool_names.contains(&"admin_assign_coach"));
    assert!(tool_names.contains(&"admin_unassign_coach"));
    assert!(tool_names.contains(&"admin_list_coach_assignments"));
}

#[test]
fn test_analytics_tool_schemas() {
    let tools = get_tools();

    // Test analyze_activity tool schema
    let analyze_activity = tools
        .iter()
        .find(|t| t.name == "analyze_activity")
        .expect("analyze_activity tool should exist");

    assert_eq!(analyze_activity.description, "Perform deep analysis of an individual activity including insights, metrics, and anomaly detection");

    // Check required parameters
    let schema = &analyze_activity.input_schema;
    if let Some(required) = &schema.required {
        assert!(required.contains(&"provider".to_owned()));
        assert!(required.contains(&"activity_id".to_owned()));
    } else {
        panic!("analyze_activity should have required parameters");
    }

    // Test calculate_fitness_score tool
    let fitness_score = tools
        .iter()
        .find(|t| t.name == "calculate_fitness_score")
        .expect("calculate_fitness_score tool should exist");

    assert!(
        fitness_score.description.contains("fitness score"),
        "calculate_fitness_score description should mention 'fitness score'"
    );
}

#[test]
fn test_recipe_tool_schemas() {
    let tools = get_tools();

    // Test get_recipe_constraints tool schema
    let get_constraints = tools
        .iter()
        .find(|t| t.name == "get_recipe_constraints")
        .expect("get_recipe_constraints tool should exist");

    assert!(
        !get_constraints.description.is_empty(),
        "get_recipe_constraints should have a description"
    );

    // Test list_recipes tool schema (no required params)
    let list_recipes = tools
        .iter()
        .find(|t| t.name == "list_recipes")
        .expect("list_recipes tool should exist");

    assert!(list_recipes.description.contains("recipes"));

    // Test get_recipe tool schema
    let get_recipe = tools
        .iter()
        .find(|t| t.name == "get_recipe")
        .expect("get_recipe tool should exist");

    if let Some(required) = &get_recipe.input_schema.required {
        assert!(required.contains(&"recipe_id".to_owned()));
    } else {
        panic!("get_recipe should have required parameters");
    }

    // Test search_recipes tool schema
    let search_recipes = tools
        .iter()
        .find(|t| t.name == "search_recipes")
        .expect("search_recipes tool should exist");

    if let Some(required) = &search_recipes.input_schema.required {
        assert!(required.contains(&"query".to_owned()));
    } else {
        panic!("search_recipes should have required parameters");
    }

    // Test delete_recipe tool schema
    let delete_recipe = tools
        .iter()
        .find(|t| t.name == "delete_recipe")
        .expect("delete_recipe tool should exist");

    if let Some(required) = &delete_recipe.input_schema.required {
        assert!(required.contains(&"recipe_id".to_owned()));
    } else {
        panic!("delete_recipe should have required parameters");
    }
}

#[test]
fn test_tool_parameter_validation() {
    let tools = get_tools();

    for tool in &tools {
        // Each tool should have proper schema structure
        assert_eq!(tool.input_schema.schema_type, "object");

        // Some tools may not have input properties (like get_connection_status)
        if tool.input_schema.properties.is_none() {
            continue;
        }

        // Required parameters should be valid
        if let Some(required) = &tool.input_schema.required {
            let properties = tool.input_schema.properties.as_ref().unwrap();

            for param_name in required {
                assert!(
                    properties.contains_key(param_name),
                    "Tool {} requires parameter '{}' but it's not in properties",
                    tool.name,
                    param_name
                );
            }
        }
    }
}

#[test]
fn test_initialize_response() {
    common::init_server_config();
    let response = InitializeResponse::new(
        "2025-06-18".to_owned(),
        "pierre-mcp-server-multitenant".to_owned(),
        "0.1.0".to_owned(),
    );

    assert_eq!(response.protocol_version, "2025-06-18");
    assert_eq!(response.server_info.name, "pierre-mcp-server-multitenant");
    assert_eq!(response.server_info.version, "0.1.0");
    assert!(response.capabilities.tools.is_some());
}

#[test]
fn test_tool_descriptions_quality() {
    let tools = get_tools();

    for tool in &tools {
        // Each tool should have a meaningful description
        assert!(
            !tool.description.is_empty(),
            "Tool {} has empty description",
            tool.name
        );
        assert!(
            tool.description.len() > 10,
            "Tool {} description too short: '{}'",
            tool.name,
            tool.description
        );

        // Analytics tools should mention their purpose
        if tool.name.contains("analyze") || tool.name.contains("calculate") {
            assert!(
                tool.description.to_lowercase().contains("analy")
                    || tool.description.to_lowercase().contains("calculat")
                    || tool.description.to_lowercase().contains("assess")
                    || tool.description.to_lowercase().contains("generat"),
                "Analytics tool {} should have analysis-related description: '{}'",
                tool.name,
                tool.description
            );
        }
    }
}

#[test]
fn test_provider_parameter_consistency() {
    let tools = get_tools();

    // Tools that should have a provider parameter in properties
    let provider_tools = [
        "get_activities",
        "get_athlete",
        "get_stats",
        "get_activity_intelligence",
        "analyze_activity",
        "calculate_metrics",
        "analyze_performance_trends",
        "compare_activities",
        "detect_patterns",
        "calculate_fitness_score",
        "analyze_training_load",
    ];

    for tool_name in &provider_tools {
        let tool = tools
            .iter()
            .find(|t| t.name == *tool_name)
            .unwrap_or_else(|| panic!("Tool {tool_name} should exist"));

        if let Some(properties) = &tool.input_schema.properties {
            assert!(
                properties.contains_key("provider"),
                "Tool {tool_name} should have 'provider' in properties"
            );
            let provider_prop = &properties["provider"];
            assert_eq!(provider_prop.property_type, "string");
        } else {
            panic!("Tool {tool_name} should have properties");
        }
    }
}

#[test]
fn test_goal_tools_consistency() {
    let tools = get_tools();

    // Goal-related tools should have consistent parameter naming
    let goal_tools = ["set_goal", "track_progress", "analyze_goal_feasibility"];

    for tool_name in &goal_tools {
        let tool = tools
            .iter()
            .find(|t| t.name == *tool_name)
            .unwrap_or_else(|| panic!("Tool {tool_name} should exist"));

        // Description should mention goals
        assert!(
            tool.description.to_lowercase().contains("goal"),
            "Goal tool {tool_name} should mention 'goal' in description"
        );
    }
}

/// Test that validates the exact tools we used in our fitness report demo
#[test]
fn test_fitness_report_tools_available() {
    let tools = get_tools();
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    // These are the tools used to generate fitness reports
    let required_tools = [
        "get_activities",
        "calculate_fitness_score",
        "analyze_training_load",
        "detect_patterns",
        "analyze_performance_trends",
        "get_connection_status",
    ];

    for tool_name in &required_tools {
        assert!(
            tool_names.contains(tool_name),
            "Required tool '{tool_name}' for fitness reporting is missing"
        );
    }

    // Verify these tools have the parameters we used
    let get_activities = tools.iter().find(|t| t.name == "get_activities").unwrap();
    if let Some(properties) = &get_activities.input_schema.properties {
        assert!(properties.contains_key("limit"));
        assert!(properties.contains_key("provider"));
    } else {
        panic!("get_activities should have properties");
    }

    let fitness_score = tools
        .iter()
        .find(|t| t.name == "calculate_fitness_score")
        .unwrap();
    if let Some(properties) = &fitness_score.input_schema.properties {
        assert!(properties.contains_key("provider"));
    } else {
        panic!("calculate_fitness_score should have properties");
    }
}
