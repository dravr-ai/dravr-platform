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
use pierre_tool_runtime::implementations::guided_flow::{
    is_withheld_during_guided_flow, GUIDED_FLOW_WITHHELD_TOOLS,
};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_execution::build_mcp_tools;
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

/// The boundary statement itself must name no tool.
///
/// Stage 7a.2 generated a prose list for months: a line per tool, 11,763
/// characters beside embacle's 16,127-character catalogue of the same ~58
/// tools. Worse than the size, it was built from `user_visible_schemas()` while
/// the declarations came from `chat_callable_schemas()`, so it advertised
/// categories the coach could not call on any path.
///
/// A names-only index generated from the declarations' own source is back
/// alongside the boundary — see
/// `the_tool_index_is_exactly_the_declared_set`, which asserts the property
/// that makes that safe. This test guards the other half: the boundary PROSE
/// stays tool-free. Prose is what cannot be regenerated, so prose is what must
/// never name a tool.
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

/// The surviving advertisement surface must be populated.
///
/// `TOOL_BOUNDARY` tells the coach to look its tools up before claiming it has
/// none. Deleting the prose list made the declarations the only place there is
/// to look: if they ever come back empty, the coach is told to check a surface
/// that holds nothing, and every capability question resolves to a denial. The
/// duplication used to mask that; it does not any more.
///
/// The rendering and injection downstream of here are covered in
/// `chat_tool_loop_test` (`test_generate_tool_catalog_has_tools`,
/// `test_inject_tool_catalog_appends_to_system`). What was untested is the
/// input to that path — that `build_mcp_tools` yields real tools at all — which
/// is the half that goes silent if a filter or a category list is wrong.
///
/// Counterpart to `the_prompt_carries_no_second_tool_list`: that one says the
/// platform must not list tools, this says something else must.
/// The boundary must not put the athlete's own connected platforms outside it.
///
/// Live incident 2026-08-26 (Telegram, two athletes): the coach answered «je
/// n'ai pas d'outil qui écrit vers intervals.icu» with zero tool calls, about
/// `prescribe_workout` — shipped the day before, chat-callable, and doing
/// exactly that. It was not hallucinating. `TOOL_BOUNDARY` said it could not
/// "use third-party services", and Intervals.icu is one; it obeyed.
///
/// The same model searches out `save_training_plan` unprompted — that name
/// appears in no prompt in dravr-contremaitre — so discovery was never the
/// defect. Being told the athlete's connected accounts belong to someone else
/// was. These two assertions pin the repair: no blanket ban that swallows a
/// connected platform, and an explicit instruction to look before denying.
#[test]
fn the_boundary_does_not_disown_the_athletes_connected_platforms() {
    use pierre_chat_pipeline::stages::prompt_builder::TOOL_BOUNDARY;

    let lower = TOOL_BOUNDARY.to_lowercase();
    assert!(
        !lower.contains("use third-party services"),
        "TOOL_BOUNDARY bans 'third-party services' outright. The athlete's \
         connected provider IS a third-party service, so this sentence tells \
         the coach its own calendar-write tool is off-limits — the 2026-08-26 \
         denial. Scope the prohibition to services the athlete has NOT connected."
    );
    assert!(
        lower.contains("connected"),
        "TOOL_BOUNDARY must say the athlete's connected accounts are inside the \
         boundary; without it, the open-internet prohibition reads as covering \
         them"
    );
    assert!(
        lower.contains("look") || lower.contains("check"),
        "TOOL_BOUNDARY must tell the coach to look its tools up before claiming \
         it has none — under MCP tool calling the catalogue is not in the \
         prompt, so an unchecked denial is the default failure"
    );
}

#[test]
fn the_surviving_advertisement_surface_is_populated() {
    let registry = full_registry();
    let declared = declared_names(&registry, false);

    assert!(
        declared.len() > 20,
        "the declarations are now the ONLY advertisement surface, and only {} \
         tools reached it — an empty or near-empty set means the coach is told \
         nothing about its tools while TOOL_BOUNDARY claims they are described",
        declared.len()
    );

    for expected in ["get_activities", "get_athlete", "save_training_plan"] {
        assert!(
            declared.iter().any(|d| d == expected),
            "{expected} must be advertised — with the prose list deleted, a tool \
             missing from the declarations is invisible to the coach"
        );
    }
}

/// The index and the declarations must be the same set, not two aligned sets.
///
/// The deleted list's defect was never that it existed — it was that it read
/// `user_visible_schemas()` while the declarations read
/// `chat_callable_schemas()`. Two sources drift, and discipline does not stop
/// it; this file exists because it did not.
///
/// So the index is generated from the declarations' own source, and this
/// asserts the consequence: every declared tool appears, and the block carries
/// names only. A capability question is then answered by reading the prompt
/// rather than guessing — which is what the coach failed to do on 2026-08-26,
/// telling two athletes it had no tool to write to Intervals.icu, with zero
/// tool calls, about a tool it had.
///
/// Anthropic's own tool-search API rejects a fully deferred catalogue outright
/// (`All tools have defer_loading set`) — at least one tool must stay visible.
/// This index is that floor.
#[test]
fn the_tool_index_is_exactly_the_declared_set() {
    use pierre_chat_pipeline::stages::prompt_builder::render_tool_index;

    let registry = full_registry();
    let declared = declared_names(&registry, false);
    assert!(
        !declared.is_empty(),
        "the registry declares no chat-callable tools — the index below would be \
         empty and every capability question would resolve to a denial"
    );

    let index = render_tool_index(&declared);
    assert!(
        !index.is_empty(),
        "a non-empty declaration set must render a non-empty index"
    );

    for name in &declared {
        assert!(
            index.contains(name.as_str()),
            "{name} is declared to the model but missing from the index — the two \
             have drifted, which is the exact defect that deleted the last list"
        );
    }

    // Names only. A description leaking in would make this the prose list again,
    // and put the schema text on a second maintenance path.
    for decl in build_mcp_tools(&registry).function_declarations {
        if decl.description.len() > 40 {
            assert!(
                !index.contains(decl.description.as_str()),
                "the index carries {}'s description — it must carry names only",
                decl.name
            );
        }
    }

    // Sorted, so the block is byte-stable turn to turn: this sits in the prompt
    // prefix, and a prefix that reorders cannot be cached.
    // The names line is the last non-empty one; the prose above it also contains
    // commas, so match on position rather than punctuation.
    let names_line = index
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim();
    let rendered: Vec<&str> = names_line.split(", ").collect();
    let mut expected = rendered.clone();
    expected.sort_unstable();
    assert_eq!(
        rendered, expected,
        "the index must be sorted, or the prompt prefix changes between turns"
    );
}
