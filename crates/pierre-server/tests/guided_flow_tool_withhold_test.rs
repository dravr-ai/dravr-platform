// ABOUTME: Guided-flow tool withhold — prose "Available Tools" list and native declarations in lockstep
// ABOUTME: Content-asserting: save_training_plan absent from BOTH surfaces mid-walk, present otherwise
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! While a guided conversational flow owns the turn — today the `/pillars`
//! profile walk — the plan-writing tool is withheld from the model. Two
//! advertisement surfaces have to move together: the prose list generated at
//! Stage 7a.2 (`build_tools_section`) and the native function declarations
//! handed to the provider at Stage 13 (`build_mcp_tools`). Advertising a tool in
//! prose that the function-calling surface does not expose is exactly the drift
//! the generated tools section exists to prevent, so each assertion below is
//! made against both surfaces at once.
//!
//! Enforcement — the server-side refusal that also covers the native-MCP path —
//! is covered in `training_plan_tools_test.rs`.

use pierre_chat_pipeline::stages::prompt_builder::build_tools_section;
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

    let prose = build_tools_section(&registry, false);
    let declared = declared_names(&registry, false);

    assert!(
        !GUIDED_FLOW_WITHHELD_TOOLS.is_empty(),
        "the withhold list must not be empty, or these tests prove nothing"
    );
    for name in GUIDED_FLOW_WITHHELD_TOOLS {
        assert!(
            prose.contains(&format!("`{name}`")),
            "{name} must be in the prose tool list on a normal turn"
        );
        assert!(
            declared.iter().any(|d| d == name),
            "{name} must be natively declared on a normal turn"
        );
    }
}

#[test]
fn withheld_tools_vanish_from_both_surfaces_during_a_guided_flow() {
    let registry = full_registry();

    let prose = build_tools_section(&registry, true);
    let declared = declared_names(&registry, true);

    for name in GUIDED_FLOW_WITHHELD_TOOLS {
        assert!(
            !prose.contains(&format!("`{name}`")),
            "{name} must be absent from the prose tool list during a guided flow"
        );
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
        let prose = build_tools_section(&registry, true);
        assert!(
            prose.contains(&format!("`{kept}`")),
            "{kept} must stay in the prose list during a guided flow"
        );
    }
}
