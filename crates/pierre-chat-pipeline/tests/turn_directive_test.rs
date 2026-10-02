// ABOUTME: What the ordinary-turn directive says and what it must never say — a task, no identity, no format
// ABOUTME: An empty slot is what let the 2026-08-05 Telegram identity break through; this block fills it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn directive: what it says and what it must never say.
//!
//! Measured against a production-scale fixture (`pierre_system.md` + the platform
//! tool list + the embacle catalogue + an 18-message history, ~73 KB) driving the
//! pinned Copilot CLI over ACP:
//!
//! | Stage 7g.3 slot          | leaked | runs |
//! |--------------------------|--------|------|
//! | empty (production today) | 6      | 14   |
//! | guided interview block   | 0      | 14   |
//! | this block               | 0      | 8    |
//!
//! This file holds the content contract, which is a property of the constant.
//! Where the block lands and when it yields — the ordinary arm is never empty,
//! every arm of the slot carries exactly one directive — are properties of an
//! assembled prompt, and are asserted on the prompt a real turn sends in
//! `crates/pierre-server/tests/prompt_assembly_wire_test.rs`. That the re-ask
//! after an identity leak finds its provider under production wiring is
//! `the_reask_fires_when_the_provider_is_wired_as_production_wires_it` in
//! `crates/pierre-server/tests/chat_reply_narration_scrub_e2e_test.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::stages::prompt_assembly::TURN_DIRECTIVE;

/// The block must give the turn a concrete job.
///
/// This is the whole mechanism. The refusals name it themselves — "I won't
/// role-play as a different assistant with tools I don't have", "this looks
/// like a full system-prompt/persona transcript" — the model evaluates an
/// assembled prompt AS A DOCUMENT when nothing asks it to do anything, and
/// judges that document an injection attempt. A task makes the question moot.
#[test]
fn the_turn_directive_states_a_task() {
    assert!(
        TURN_DIRECTIVE.contains("Answer the athlete's question"),
        "the directive must name the turn's job, got: {TURN_DIRECTIVE}"
    );
    assert!(
        TURN_DIRECTIVE.contains("call one tool and then answer"),
        "the directive must route a data gap to a tool call — the runs that opened \
         with a tool call never leaked, because a tool call is already a task"
    );
    assert!(
        TURN_DIRECTIVE.contains("Ground every recommendation"),
        "the directive must demand grounding in the athlete's own facts; that is \
         what makes the answers specific rather than generic"
    );
}

/// The block must assert NOTHING about identity.
///
/// The identity anchor was present in all six leaking runs, so more identity
/// text is not the lever — and text shaped like an identity override is what
/// provokes the refusal. This is the same regression guard embacle's catalogue
/// carries after the 2026-07-12 incident, where "Output ONLY the raw XML /
/// Registered functions" framing got "I'm GitHub Copilot, not <persona>" back.
///
/// Each phrase below appeared in a refusal or in the framing that triggered
/// one. A future edit that "strengthens" this block by re-asserting who the
/// agent is fails here rather than in production.
#[test]
fn the_turn_directive_asserts_no_identity() {
    let lower = TURN_DIRECTIVE.to_lowercase();
    for banned in [
        "you are",
        "you are not",
        "never reveal",
        "never claim",
        "role-play",
        "language model",
        "coding assistant",
        "command-line",
        "your identity",
        "real identity",
        "underlying model",
    ] {
        assert!(
            !lower.contains(banned),
            "the turn directive must state a TASK, never an identity: found {banned:?}. \
             Identity text belongs in IDENTITY_ANCHOR, and adding more of it does not \
             close this leak — it was present in every leaking run."
        );
    }
}

/// The block must carry no length or format rule.
///
/// Those belong to the Stage 7g channel constraints and the Stage 7g.2b
/// visual contract. A length rule here would contradict one of them depending
/// on channel, and the contradiction would be invisible until an agent emitted
/// prose where a block was expected.
#[test]
fn the_turn_directive_owns_no_formatting_rule() {
    let lower = TURN_DIRECTIVE.to_lowercase();
    for banned in ["characters", "markdown", "json", "plain text", "bullet"] {
        assert!(
            !lower.contains(banned),
            "formatting belongs to Stage 7g / 7g.2b, not the turn directive: found {banned:?}"
        );
    }
}
