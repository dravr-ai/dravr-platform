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

/// The write rule must say WHOSE profile it writes.
///
/// `set_physiology` takes no subject: it writes the profile of the athlete
/// whose turn is executing. In a DM that is unambiguous, but the directive is
/// appended to group-room turns too, and a group prompt carries other members'
/// messages — so "if the athlete states a physiological value" pointed at
/// several people at once while the tool could only ever mean one of them. A
/// coach that took Phil's FTP out of the transcript and saved it would have
/// written it onto whoever was speaking, and the wrong athlete's zones are
/// worse than no zones: they are wrong with the same confidence as right ones.
///
/// Nothing pinned this. The verifier called it "probably safe", which is not a
/// test (registre#260).
#[test]
fn the_write_rule_names_whose_profile_it_touches() {
    assert!(
        TURN_DIRECTIVE.contains("the athlete addressing you"),
        "in a group room 'the athlete' is several people; the rule must point \
         at the one whose turn this is: {TURN_DIRECTIVE}"
    );

    // Said this way on purpose. The first draft read "the athlete you are
    // answering", which trips `the_turn_directive_asserts_no_identity`: "you
    // are" is banned outright in this block, because identity text here was
    // present in every leaking run and adding more of it never closed the leak.
    // The scoping is a task, and has to be phrased as one.
    assert!(
        !TURN_DIRECTIVE.to_lowercase().contains("you are"),
        "scoping the rule must not smuggle an identity assertion into the \
         turn directive: {TURN_DIRECTIVE}"
    );
    assert!(
        TURN_DIRECTIVE.contains("someone else"),
        "and it must refuse the other direction explicitly — a value read out \
         of another member's message is not this athlete's: {TURN_DIRECTIVE}"
    );
}

/// The tool really does take no subject, which is what makes the wording load
/// bearing rather than decorative.
///
/// If a future schema grows an athlete argument, this fails and the directive
/// should be revisited rather than left describing a constraint that no longer
/// holds.
#[test]
fn set_physiology_writes_the_calling_athlete_and_takes_no_subject() {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let schema = registry
        .chat_callable_schemas()
        .into_iter()
        .find(|t| t.name == "set_physiology")
        .expect("set_physiology is chat-callable");

    let properties = schema
        .input_schema
        .properties
        .as_ref()
        .expect("the tool declares an object schema with properties");

    for subject in ["user_id", "athlete", "athlete_id", "member", "member_id"] {
        assert!(
            !properties.contains_key(subject),
            "set_physiology exposes `{subject}`, so it CAN write another \
             athlete's profile — the directive's wording is then not enough on \
             its own and the tool needs the guard: {:?}",
            properties.keys().collect::<Vec<_>>()
        );
    }
}
