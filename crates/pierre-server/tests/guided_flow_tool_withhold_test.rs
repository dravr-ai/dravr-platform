// ABOUTME: Guided-flow tool withhold — prose "Available Tools" list and native declarations in lockstep
// ABOUTME: Content-asserting: save_training_plan absent from BOTH surfaces mid-walk, present otherwise
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! While a guided conversational flow owns the turn — today the `/pillars`
//! profile walk — the plan-writing tool is withheld from the model.
//!
//! There is now ONE advertisement surface: the native declarations built by
//! `build_mcp_tools` at Stage 13, which both the API providers and embacle's
//! text catalogue derive from. These tests used to assert against two, because
//! Stage 7a.2 also generated a prose list; keeping two surfaces in lockstep was
//! the reason this file existed. The prose list is deleted, so the lockstep
//! problem is gone rather than tested — and the last test here guards the
//! deletion, since a reintroduced second list would need this coordination
//! back.
//!
//! Enforcement — the server-side refusal that also covers the native-MCP path —
//! is covered in `training_plan_tools_test.rs`.

use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_execution::{
    build_mcp_tools, is_withheld_during_guided_flow, GUIDED_FLOW_WITHHELD_TOOLS,
};
use std::sync::Arc;

fn full_registry() -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    Arc::new(registry)
}

/// The declared tool names the provider would see for a turn, with the
/// guided-flow withhold applied the way `dispatch_llm_with_tools` applies it.
fn declared_names(registry: &Arc<ToolRegistry>, guided_flow_active: bool) -> Vec<String> {
    let mut tools = build_mcp_tools(registry);
    if guided_flow_active {
        tools
            .function_declarations
            .retain(|decl| !is_withheld_during_guided_flow(&decl.name));
    }
    tools
        .function_declarations
        .into_iter()
        .map(|decl| decl.name)
        .collect()
}

#[test]
fn withheld_tools_are_advertised_on_a_normal_turn() {
    let registry = full_registry();

    let declared = declared_names(&registry, false);

    assert!(
        !GUIDED_FLOW_WITHHELD_TOOLS.is_empty(),
        "the withhold list must not be empty, or these tests prove nothing"
    );
    for name in GUIDED_FLOW_WITHHELD_TOOLS {
        assert!(
            declared.iter().any(|d| d == name),
            "{name} must be natively declared on a normal turn"
        );
    }
}

#[test]
fn withheld_tools_vanish_during_a_guided_flow() {
    let registry = full_registry();
    let declared = declared_names(&registry, true);

    for name in GUIDED_FLOW_WITHHELD_TOOLS {
        assert!(
            !declared.iter().any(|d| d == name),
            "{name} must not be natively declared during a guided flow"
        );
    }
}

#[test]
fn only_the_withheld_tools_are_dropped() {
    let registry = full_registry();

    let normal = declared_names(&registry, false);
    let walking = declared_names(&registry, true);

    assert_eq!(
        normal.len(),
        walking.len() + GUIDED_FLOW_WITHHELD_TOOLS.len(),
        "exactly the withheld tools should disappear, nothing else"
    );

    // Read tools stay: an athlete mid-walk can still ask what they rode.
    for kept in ["get_activities", "get_training_plan"] {
        assert!(
            walking.iter().any(|d| d == kept),
            "{kept} must stay available during a guided flow"
        );
    }
}

/// The prompt must not grow a second tool list.
///
/// Stage 7a.2 generated one for months: a prose line per tool, 11,763
/// characters beside embacle's 16,127-character catalogue of the same ~58
/// tools. Worse than the size, it was built from `user_visible_schemas()` while
/// the declarations came from `chat_callable_schemas()`, so it advertised
/// categories the coach could not call on any path.
///
/// A second list cannot be kept in lockstep by discipline — this file is the
/// evidence, since keeping the two surfaces aligned is what it was written to
/// do. The boundary statement that replaced it names no tools, which is the
/// property that keeps it from becoming a list again.
#[test]
fn the_prompt_carries_no_second_tool_list() {
    use pierre_chat_pipeline::stages::prompt_builder::TOOL_BOUNDARY;

    let registry = full_registry();
    for name in declared_names(&registry, false) {
        assert!(
            !TOOL_BOUNDARY.contains(&name),
            "TOOL_BOUNDARY names {name} — it must state the boundary without \
             listing tools, or it becomes the second list again"
        );
    }
    assert!(
        TOOL_BOUNDARY.contains("cannot browse the web"),
        "the closed-world statement must survive: it is the only place that says \
         a missing capability should be admitted rather than invented"
    );
}
