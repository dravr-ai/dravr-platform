// ABOUTME: Every built-in write declares what it stores, free text a later prompt reads or a structured value
// ABOUTME: The member-read write refusals decide a write by it, and may_read_others follows each tool's declarations

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::read_subject::may_read_others;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_data::WriteKind;

fn builtin() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry
}

/// carnet#1511 call 16: every write declares what it stores, free text a
/// later prompt reads or a structured own-account value, since the
/// member-read refusals decide the write by it; and only a write declares
/// one. Removing `.writes(..)` from any write's `reads:` fails here.
#[test]
fn every_registered_write_declares_what_it_stores() {
    let registry = builtin();
    let mut undeclared = Vec::new();
    let mut not_a_write = Vec::new();
    for name in registry.tool_names() {
        let writes = registry
            .get(name)
            .is_some_and(|tool| tool.capabilities().contains(ToolCapabilities::WRITES_DATA));
        let kind = registry.write_kind(name);
        if writes && kind.is_none() {
            undeclared.push(name.to_owned());
        }
        if !writes && kind.is_some() {
            not_a_write.push(name.to_owned());
        }
    }
    assert!(
        undeclared.is_empty(),
        "a registered write declares nothing it stores, so the member-read refusal takes it \
         as free text; declare `.writes(..)` on its `reads:`: {undeclared:?}"
    );
    assert!(
        not_a_write.is_empty(),
        "a tool that writes no data declares a write kind: {not_a_write:?}"
    );
}

/// carnet#1511 call 16: the writes that store free text a later prompt
/// reads, refused once a turn drew on another member's data, and the
/// structured own-account writes it names.
#[test]
fn the_free_text_writes_are_the_ones_call_16_names() {
    let registry = builtin();
    for named in [
        "remember_fact",
        "create_agent",
        "update_agent",
        "commitment_create",
        "save_recipe",
        "set_goal",
        // Round 13: it files the model's free-text folder name.
        "save_workout_to_library",
    ] {
        assert_eq!(
            registry.write_kind(named),
            Some(WriteKind::FreeText),
            "call 16 names {named} as filing free text"
        );
    }
    for structured in ["set_physiology", "set_fitness_config"] {
        assert_eq!(
            registry.write_kind(structured),
            Some(WriteKind::Structured),
            "call 16 names {structured} a structured own-account write"
        );
    }
}

/// carnet#1511 design A9: whether a tool may read anyone but its caller is
/// derived from what it declares: a training or wellness read, a tool
/// taking `athlete`, a group tool, a coach tool, or one no longer registered
/// may; an own-data tool does not.
#[test]
fn whether_a_tool_reads_others_follows_its_declarations() {
    let registry = builtin();
    for name in [
        "get_activities",
        "get_sleep_sessions",
        "a_tool_since_removed",
    ] {
        assert!(may_read_others(&registry, name), "{name} may read others");
    }
    for name in registry.tools_in_category("groups") {
        assert!(may_read_others(&registry, name), "{name} may read others");
    }
    for name in ["get_connection_status", "remember_fact", "set_physiology"] {
        if registry.get(name).is_some() {
            assert!(
                !may_read_others(&registry, name),
                "{name} reads its caller alone"
            );
        }
    }
}
