// ABOUTME: Pins the turn directive's one write rule — physiology the athlete states gets saved
// ABOUTME: Regression for 2026-09-02, where the coach saw an FTP, used it once, and stored nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `set_physiology` has been chat-callable since registre#39, and the category
//! comment that admits it names this exact scenario — *"Physiology arrives
//! mid-conversation — 'my FTP is 285' — so the coach must be able to save it on
//! the turn it is said."* The capability was there. Nothing asked the coach to
//! use it.
//!
//! Live 2026-09-02: the athlete asked *"As tu acces a mes power zones?"*, was
//! told no, supplied 380 W, and the coach acknowledged seeing it («tu l'as
//! mentionné à 380W plus tôt»), hand-computed a threshold in prose, and stored
//! nothing. Zero tool calls across all fifteen turns. The next session starts
//! from the same flat "I don't have your zones" (registre#250).
//!
//! The directive names the tool literally, which is the exception to this
//! codebase's usual rule — "the physiology tool" is precisely the vagueness
//! that produced the miss. This file is what keeps the literal honest: the name
//! is read back out of the directive and checked against the registry, so it is
//! derived from one source rather than pinned by a second hand-kept list.

use std::collections::HashSet;

use pierre_chat_pipeline::stages::prompt_assembly::TURN_DIRECTIVE;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;

/// Every `` `backticked` `` token in the directive — the tool names it claims
/// are callable.
fn backticked_names(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn chat_callable() -> HashSet<String> {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry
        .chat_callable_schemas()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[test]
fn the_turn_directive_names_a_tool_the_registry_serves() {
    let named = backticked_names(TURN_DIRECTIVE);
    assert!(
        !named.is_empty(),
        "the directive must still name the physiology tool; if the rule was \
         removed, remove this test in the same change: {TURN_DIRECTIVE}"
    );

    let surface = chat_callable();
    for name in &named {
        assert!(
            surface.contains(name),
            "the turn directive tells the coach to call `{name}`, which the \
             chat surface does not serve — a rename left the instruction \
             pointing at nothing"
        );
    }
}

#[test]
fn the_directive_asks_for_the_save_on_the_same_turn() {
    assert!(
        TURN_DIRECTIVE.contains("set_physiology"),
        "the rule must name the tool, not describe it: {TURN_DIRECTIVE}"
    );
    assert!(
        TURN_DIRECTIVE.contains("same turn"),
        "the timing is the whole point — a value acknowledged now and saved \
         'later' is what was lost: {TURN_DIRECTIVE}"
    );
}

/// The values the athlete actually says out loud. FTP is the one from the
/// incident; the others share the same failure mode and the same tool.
#[test]
fn the_rule_covers_the_values_an_athlete_volunteers() {
    for term in ["FTP", "heart rate", "weight", "VO2max"] {
        assert!(
            TURN_DIRECTIVE.contains(term),
            "an athlete stating their {term} must reach the same rule: \
             {TURN_DIRECTIVE}"
        );
    }
}

/// The capability itself, pinned separately: the directive is worthless if the
/// tool is not on the chat surface at all.
#[test]
fn set_physiology_is_reachable_from_a_chat_turn() {
    assert!(
        chat_callable().contains("set_physiology"),
        "physiology arrives mid-conversation; a coach that cannot save it \
         there cannot save it at all"
    );
}
