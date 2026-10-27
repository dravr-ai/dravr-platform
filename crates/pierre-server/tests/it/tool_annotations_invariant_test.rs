// ABOUTME: Every registered tool advertises all four MCP behaviour hints, consistent with what it declares elsewhere
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

#[test]
fn a_task_capable_tool_is_open_world() {
    // `task_capable` is declared only on a tool whose worst path is an
    // uncached provider fetch, so it reaches past this server by definition.
    let registry = builtin_registry();
    let mut closed = Vec::new();
    for (name, _, _, _) in registry.all_tool_metadata() {
        if registry.task_support(&name).is_none() {
            continue;
        }
        let definition = registry.get(&name).expect("listed tool").definition();
        if definition.annotations.and_then(|a| a.open_world_hint) != Some(true) {
            closed.push(name);
        }
    }
    closed.sort();
    assert!(
        closed.is_empty(),
        "task-capable tools must declare Reach::OpenWorld: {closed:?}"
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
