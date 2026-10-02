// ABOUTME: The guided-walk directive states its own precedence, forbids plan work and prefers going deeper
// ABOUTME: Wording only — where the directive lands in an assembled prompt is pinned on real turns in pierre-server

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The 2026-07-24 derail was decided by prompt position. The onboarding
//! directive was appended mid-prompt (Stage 7e.1), then buried under the channel
//! response constraints (7g), the tool-discipline block (7g.1 — placed last *by
//! design* for recency) and, on card-capable channels, the structured-output
//! JSON contract (7g.2). A builder agent whose persona says "your first reply in
//! any conversation MUST emit the plan JSON" won that recency contest and
//! emitted a 16-week plan on the athlete's first profile answer.
//!
//! Two invariants keep the fix in place:
//!
//! - **Wording** — the directive states its own precedence rather than relying
//!   on position alone, forbids plan work for the turn, and tells the agent to
//!   go deeper rather than re-ask when the answer already landed. Asserted
//!   here, on the text the directive function returns.
//! - **Position** — the directive is appended after the channel constraints,
//!   the tool-discipline block and the visual contract, and only the turn's
//!   language and the identity anchor follow it before the prompt is hardened.
//!   That is a property of an assembled prompt, so it is asserted on the
//!   prompt a real turn sends, in
//!   `crates/pierre-server/tests/prompt_assembly_wire_test.rs` — along with
//!   the release directive sharing the slot, the visual contract's suppression
//!   during a walk, and the tool index.

use pierre_chat_pipeline::stages::onboarding::{directive, GuidedTarget, OnboardingTurn};
use pierre_core::models::{CoverageTarget, GuidedFlow, OnboardingState, Pillar};

/// The directive for a given pillars-walk topic, built the way Stage 7g.3
/// builds it.
fn directive_for(target: CoverageTarget) -> String {
    let turn = OnboardingTurn {
        target: Some(GuidedTarget::Coverage(target)),
        state: OnboardingState::start("2026-07-25T00:00:00Z".to_owned(), GuidedFlow::Pillars),
    };
    directive(&turn)
}

#[test]
fn directive_claims_precedence_over_an_agent_turn_one_protocol() {
    let text = directive_for(CoverageTarget::NorthStar);

    // Position alone is not enough against a persona block that calls itself
    // NON-NÉGOCIABLE, so the directive says out loud what it overrides.
    assert!(
        text.contains("overrides every other instruction"),
        "directive must announce its precedence, got: {text}"
    );
    for phrase in [
        "supersedes any startup instruction",
        "first-turn protocol",
        "output-format contract",
        "non-negotiable",
    ] {
        assert!(
            text.to_lowercase().contains(&phrase.to_lowercase()),
            "directive must neutralize {phrase:?}, got: {text}"
        );
    }
}

#[test]
fn directive_forbids_plan_work_and_prefers_going_deeper() {
    let text = directive_for(CoverageTarget::Pillar(Pillar::TrainingAndMovement));

    // The exact failure to prevent: a plan, or a "Hypothèses :" assumption block
    // standing in for one, instead of the next pillar question.
    assert!(
        text.contains("Do not build, propose, or save a training plan on this turn"),
        "directive must forbid plan work for the turn, got: {text}"
    );
    assert!(
        text.contains("do not list assumptions in place of one"),
        "directive must also close the assumption-block substitute, got: {text}"
    );

    // Softens the known re-probe edge: when extraction lagged and the topic is
    // asked a second time, the athlete should not be re-asked verbatim.
    assert!(
        text.contains("go deeper on the same topic instead of asking the same question again"),
        "directive must prefer going deeper over re-asking, got: {text}"
    );

    // The topic itself is still named, so the turn has a subject.
    assert!(
        text.contains(Pillar::TrainingAndMovement.display_label()),
        "directive must name the probed topic, got: {text}"
    );
}
