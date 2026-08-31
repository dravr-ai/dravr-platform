// ABOUTME: Onboarding directive must sit below every behavioural block, with only 7g.4 after it
// ABOUTME: Content + structural assertions on the wording and the Stage 7g.3 position in prompt_assembly

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The 2026-07-24 derail was decided by prompt position. The onboarding
//! directive was appended mid-prompt (Stage 7e.1), then buried under the channel
//! response constraints (7g), the tool-discipline block (7g.1 — placed last *by
//! design* for recency) and, on card-capable channels, the structured-output
//! JSON contract (7g.2). A builder coach whose persona says "your first reply in
//! any conversation MUST emit the plan JSON" won that recency contest and
//! emitted a 16-week plan on the athlete's first profile answer.
//!
//! Two invariants keep the fix in place, and both are asserted here:
//!
//! - **Wording** — the directive states its own precedence rather than relying
//!   on position alone, forbids plan work for the turn, and tells the coach to
//!   go deeper rather than re-ask when the answer already landed.
//! - **Position** — Stage 7g.3 (the directive) is appended *after* 7g/7g.1/7g.2.
//!   Only Stage 7g.4, the identity anchor, may follow it before Stage 7h hardens
//!   the prompt: the anchor states who the assistant is and never instructs the
//!   turn, so it does not compete with the directive for behavioural precedence,
//!   and a 48-run live A/B (2026-07-25) measured the prompt tail as the only
//!   position where the anchor suppresses model-identity disclosure. This one is
//!   structural: a full prompt assembly needs a live pipeline context, whereas
//!   the ordering regression this guards against is a source edit — someone
//!   appending a new stage below the directive.

use pierre_chat_pipeline::stages::onboarding::{directive, GuidedTarget, OnboardingTurn};
use pierre_core::models::{CoverageTarget, GuidedFlow, OnboardingState, Pillar};
use std::fs;
use std::path::PathBuf;

fn prompt_assembly_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/stages/prompt_assembly.rs");
    fs::read_to_string(&path).expect("read prompt_assembly.rs")
}

/// Byte offset of a source marker that must appear exactly once.
fn sole_offset(source: &str, marker: &str) -> usize {
    let count = source.matches(marker).count();
    assert_eq!(
        count, 1,
        "expected exactly one occurrence of {marker:?}, found {count}"
    );
    source.find(marker).expect("checked above")
}

#[test]
fn directive_is_appended_after_every_other_prompt_block() {
    let source = prompt_assembly_source();

    let response_constraints = sole_offset(&source, "// Stage 7g:");
    let tool_discipline = sole_offset(&source, "// Stage 7g.1:");
    let structured_output = sole_offset(&source, "// Stage 7g.2:");
    let directive = sole_offset(&source, "// Stage 7g.3:");
    let harden = sole_offset(&source, "// Stage 7h:");

    assert!(
        response_constraints < tool_discipline
            && tool_discipline < structured_output
            && structured_output < directive,
        "the onboarding directive must be appended after the channel constraints, \
         the tool-discipline block and the structured-output contract"
    );
    assert!(
        directive < harden,
        "the onboarding directive must be appended before the prompt is hardened"
    );

    // Only the identity anchor may sit between the directive and the canary.
    //
    // This assertion previously counted the string `raw_system_prompt}` — a
    // `format!` interpolation closing brace — as a proxy for "a block was
    // appended". Stage 7g.4 appends via `close_with_identity_anchor(&raw_..)`
    // and emits no such literal, so it slipped in below the directive while
    // this test stayed green. Count real rebindings instead: every block that
    // mutates the prompt must rebind `raw_system_prompt`, whatever syntax it
    // uses to build the new value.
    let tail = &source[directive..harden];
    assert!(
        tail.contains("super::onboarding::directive(turn)"),
        "Stage 7g.3 should be the block that appends the directive"
    );
    let anchor = sole_offset(&source, "// Stage 7g.4:");
    assert!(
        directive < anchor && anchor < harden,
        "the identity anchor must sit between the onboarding directive and the canary"
    );
    let language = sole_offset(&source, "// Stage 7g.3b:");
    assert!(
        directive < language && language < anchor,
        "the turn-language block sits between the directive and the identity anchor"
    );
    let rebinds = tail.matches("let raw_system_prompt =").count();
    assert_eq!(
        rebinds, 3,
        "exactly three prompt rebindings may sit between Stage 7g.3 and Stage 7h — the \
         directive, the turn-language block and the identity anchor — found {rebinds}; a \
         new block was added below the directive"
    );
}

/// Stage 7g.3b names the turn's language, and does so on every turn.
///
/// It joins the identity anchor as the second block permitted below the
/// directive, on the same grounds: it says how to write, never what to
/// produce, so it does not enter the recency contest the 2026-07-24 derail was
/// decided by. That is exactly why it is appended unconditionally rather than
/// from an arm of the directive's `match` — a builder coach under the
/// structured-output contract still writes prose inside its JSON, and the
/// English-answer incident (carnet#159) reached a surface that was carrying no
/// language rule at all.
#[test]
fn the_turn_language_block_is_appended_on_every_turn() {
    let source = prompt_assembly_source();
    let language = sole_offset(&source, "// Stage 7g.3b:");
    let anchor = sole_offset(&source, "// Stage 7g.4:");
    let stage = &source[language..anchor];

    assert!(
        stage.contains("KEY_TURN_LANGUAGE"),
        "the block must render the turn-language string from the messaging registry"
    );
    assert!(
        stage.contains("&profile.locale"),
        "the language must come from the turn's resolved locale, not be re-inferred"
    );
    assert_eq!(
        stage.matches("let raw_system_prompt =").count(),
        1,
        "the turn-language block is one unconditional rebinding"
    );
    for gate in ["match ", "if ", "None =>"] {
        assert!(
            !stage.contains(gate),
            "Stage 7g.3b must be unconditional — found a {gate:?} gating it; every surface \
             and every coach shape needs the language named"
        );
    }
}

/// The directive that retracts the interview's rules occupies the interview
/// directive's own slot, not a new block below it.
///
/// It has to sit exactly where the block it retracts sat: a retraction that
/// lands above the channel constraints, the tool-discipline block and the
/// structured-output contract loses the same recency contest the 2026-07-24
/// derail was decided by. Sharing the one `match` also keeps the rebinding
/// count above at two — and the two arms are mutually exclusive by
/// construction, since a conversation is either inside a flow or out of one.
#[test]
fn the_release_directive_shares_the_interview_directives_slot() {
    let source = prompt_assembly_source();
    let directive = sole_offset(&source, "// Stage 7g.3:");
    // Bounded by Stage 7g.3b rather than the identity anchor: the rebinding
    // count below is about the interview directive and its retraction sharing
    // one slot, and every later block would inflate it without saying anything
    // about that.
    let language = sole_offset(&source, "// Stage 7g.3b:");
    let stage = &source[directive..language];

    assert!(
        stage.contains("super::onboarding::release_directive()"),
        "the post-interview release directive must be appended from Stage 7g.3"
    );
    assert!(
        stage.contains("OnboardingState::just_completed"),
        "the release arm must be gated on the retired-interview marker, so it fires \
         on the turn after the wrap-up and not on every ordinary turn"
    );
    assert_eq!(
        stage.matches("let raw_system_prompt =").count(),
        1,
        "the guided directive and its retraction are alternatives in one rebinding, \
         never two stacked blocks — a turn must never carry both"
    );
}

#[test]
fn structured_output_contract_is_suppressed_during_the_walk() {
    let source = prompt_assembly_source();
    let structured_output = sole_offset(&source, "// Stage 7g.2:");
    let directive = sole_offset(&source, "// Stage 7g.3:");
    let stage = &source[structured_output..directive];

    assert!(
        stage.contains("onboarding.is_none()"),
        "Stage 7g.2 must be gated on onboarding being inactive — a JSON-plan output \
         contract contradicts the profile-building directive it would otherwise precede"
    );
}

/// Stage 7a.2 states the tool boundary and lists nothing.
///
/// It used to generate a prose tool list that had to be kept in lockstep with
/// the native declarations, and this test asserted it received the guided-flow
/// flag so the two stayed aligned. The list is deleted — the declarations are
/// the single advertisement surface — so what matters now is that the stage
/// appends the boundary statement and does not reach for the registry again.
#[test]
fn the_tool_stage_states_the_boundary_without_listing_tools() {
    let source = prompt_assembly_source();
    assert!(
        source.contains("{base_prompt}\\n\\n{TOOL_BOUNDARY}"),
        "Stage 7a.2 must append the tool-boundary statement"
    );
    assert!(
        !source.contains("build_tools_section"),
        "the generated prose tool list must stay deleted — a second list built \
         from a different schema set is what it was drifting against"
    );
    // Comments are stripped: the Stage 7a.2 comment names the old schema set
    // deliberately, to record which drift the deletion closed.
    let code: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("user_visible_schemas"),
        "prompt assembly must not read the registry for advertisement: the \
         declarations at Stage 13 are the one surface, and they use \
         chat_callable_schemas"
    );
}

/// The directive for a given pillars-walk topic, built the way Stage 7g.3
/// builds it.
fn directive_for(target: CoverageTarget) -> String {
    let turn = OnboardingTurn {
        target: GuidedTarget::Coverage(target),
        state: OnboardingState::start("2026-07-25T00:00:00Z".to_owned(), GuidedFlow::Pillars),
    };
    directive(&turn)
}

#[test]
fn directive_claims_precedence_over_a_coach_turn_one_protocol() {
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
