// ABOUTME: Every registered tool advertises a display title and all four MCP behaviour hints, consistent with what it declares elsewhere
// ABOUTME: Also caps each tool's input schema, since clients trim oversized schemas starting with argument descriptions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Clients act on a tool's annotations: they fast-track a read-only call,
//! confirm a destructive one, and the `OpenAI` plugin directory rejects a
//! server with any unannotated tool. A missing hint is read as its most
//! conservative value, so every hint is pinned here (carnet#1198).

use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;

use crate::common;

/// Ceiling on one tool's serialized input schema. The largest today is
/// `save_training_plan` at about 12 KB; most are under 1 KB.
const MAX_INPUT_SCHEMA_BYTES: usize = 16 * 1024;

fn builtin_registry() -> ToolRegistry {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry
}

#[test]
fn every_tool_sets_all_four_behaviour_hints() {
    let registry = builtin_registry();
    let mut missing = Vec::new();
    for (name, _, _, _) in registry.all_tool_metadata() {
        let definition = registry.get(&name).expect("listed tool").definition();
        let complete = definition.annotations.as_ref().is_some_and(|a| {
            a.read_only_hint.is_some()
                && a.destructive_hint.is_some()
                && a.idempotent_hint.is_some()
                && a.open_world_hint.is_some()
        });
        if !complete {
            missing.push(name);
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "tools missing a behaviour hint (build them with pierre_tool_runtime::annotations::annotations): {missing:?}"
    );
}

#[test]
fn every_tool_has_a_display_title() {
    // Hosts show the title in place of the snake_case name, and the Claude
    // connector directory rejects a server with any untitled tool.
    let registry = builtin_registry();
    let mut untitled = Vec::new();
    for (name, _, _, _) in registry.all_tool_metadata() {
        let definition = registry.get(&name).expect("listed tool").definition();
        let titled = definition
            .annotations
            .as_ref()
            .and_then(|a| a.title.as_deref())
            .is_some_and(|title| !title.trim().is_empty() && title != name);
        if !titled {
            untitled.push(name);
        }
    }
    untitled.sort();
    assert!(
        untitled.is_empty(),
        "tools without a display title (pass one to pierre_tool_runtime::annotations::annotations): {untitled:?}"
    );
}

#[test]
fn read_only_hint_agrees_with_the_write_capability() {
    let registry = builtin_registry();
    let mut disagree = Vec::new();
    for (name, _, capabilities, _) in registry.all_tool_metadata() {
        let definition = registry.get(&name).expect("listed tool").definition();
        let read_only = definition
            .annotations
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        let writes = capabilities.contains(ToolCapabilities::WRITES_DATA);
        if read_only == writes {
            disagree.push(format!(
                "{name} (readOnlyHint={read_only}, WRITES_DATA={writes})"
            ));
        }
    }
    disagree.sort();
    assert!(
        disagree.is_empty(),
        "readOnlyHint must be the negation of WRITES_DATA: {disagree:?}"
    );
}

/// The tools that read public or open-ended data, with the service each one
/// reaches. Everything else is bounded to Dravr's own store or the athlete's
/// own linked account and must advertise `openWorldHint: false`, the `OpenAI`
/// Apps SDK meaning both directories accept.
const OPEN_WORLD_TOOLS: [(&str, &str); 7] = [
    ("analyze_weather_impact", "weather service"),
    ("analyze_meal_nutrition", "USDA FoodData Central"),
    ("discover_routes", "Overpass public map data"),
    ("get_food_details", "USDA FoodData Central"),
    ("get_weather_forecast", "weather service and geocoder"),
    ("search_food", "USDA FoodData Central"),
    ("validate_recipe", "USDA FoodData Central"),
];

#[test]
fn only_tools_reaching_public_data_are_open_world() {
    // A tool that reads or writes the athlete's own provider account is
    // bounded even though that account lives elsewhere, so a provider fetch
    // alone never makes a tool open world.
    let registry = builtin_registry();
    let mut open_world = Vec::new();
    let mut expected = Vec::new();
    for (name, _, _, _) in registry.all_tool_metadata() {
        let definition = registry.get(&name).expect("listed tool").definition();
        if definition.annotations.and_then(|a| a.open_world_hint) == Some(true) {
            open_world.push(name.clone());
        }
        if OPEN_WORLD_TOOLS.iter().any(|(tool, _)| *tool == name) {
            expected.push(name);
        }
    }
    open_world.sort();
    expected.sort();
    assert!(
        !expected.is_empty(),
        "no public-data tool is registered, so this check proves nothing"
    );
    assert_eq!(
        open_world, expected,
        "openWorldHint must be true exactly on the tools that reach public data \
         (Reach::OpenWorld); a linked provider account is Reach::AthleteAccount"
    );
}

#[test]
fn every_input_schema_fits_the_size_budget() {
    let registry = builtin_registry();
    let mut oversized: Vec<(String, usize)> = registry
        .all_schemas()
        .into_iter()
        .map(|tool| {
            let bytes = serde_json::to_string(&tool.input_schema)
                .expect("schema serializes")
                .len();
            (tool.name, bytes)
        })
        .filter(|(_, bytes)| *bytes > MAX_INPUT_SCHEMA_BYTES)
        .collect();
    oversized.sort();
    assert!(
        oversized.is_empty(),
        "input schemas over {MAX_INPUT_SCHEMA_BYTES} bytes: {oversized:?}"
    );
}
