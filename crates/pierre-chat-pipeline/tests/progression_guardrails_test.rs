// ABOUTME: The progression-guardrails append — who receives it, and where it sits in the prompt
// ABOUTME: Placement is the guard: prose landing after the JSON contract re-runs the 2026-07-24 derail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Load-progression guardrails: audience and placement.
//!
//! The audience rule is unit-testable through the pure category predicate. The
//! placement rule is not reachable without standing up the whole async assembly
//! stage, so it is asserted against the source the same way
//! `onboarding_directive_tail_test` asserts the directive's position — a
//! source-level contract, checked because getting it wrong is silent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fs;
use std::path::PathBuf;

fn prompt_assembly_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/stages/prompt_assembly.rs");
    let display = path.display().to_string();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {display}: {e}"))
}

/// The block must be appended ABOVE the tool-discipline block, the
/// structured-output contract and the guided-flow directive.
///
/// Four of the twelve training coaches declare an `output_schema` and receive a
/// JSON-only contract at Stage 7g.2. Prose landing after that contract is the
/// recency contest that turned the 2026-07-24 walk into an unwanted 16-week
/// plan. These guardrails give up recency deliberately — the enforcement that
/// matters is the save-time ramp check, which measures the plan instead of
/// asking for it.
#[test]
fn guardrails_sit_above_the_output_format_blocks() {
    let source = prompt_assembly_source();

    // Match the stage *comment markers* (`// Stage 7g.1:`), not the bare
    // string: several doc comments in this file discuss stages by name, and
    // a find-first on "Stage 7g.1" lands on the IDENTITY_ANCHOR docs instead.
    let at = |marker: &str| {
        source
            .find(marker)
            .unwrap_or_else(|| panic!("stage marker {marker} missing"))
    };
    let guardrails = at("// Stage 7f.3:");
    let tool_discipline = at("// Stage 7g.1:");
    let structured_output = at("// Stage 7g.2:");
    let directive = at("// Stage 7g.3:");

    assert!(
        guardrails < tool_discipline,
        "guardrails must precede the tool-discipline block"
    );
    assert!(
        guardrails < structured_output,
        "guardrails must precede the structured-output contract — prose after a \
         JSON-only contract is what derailed the walk on 2026-07-24"
    );
    assert!(
        guardrails < directive,
        "guardrails must precede the guided-flow directive"
    );
}

/// The append is suppressed while a guided flow owns the turn, for the same
/// reason Stages 7f.2 and 7g.2 are: a calibration interview asks questions, it
/// does not prescribe load, so the block would be pure prompt cost on every
/// turn of the interview.
#[test]
fn the_guardrails_helper_is_gated_on_the_guided_flow() {
    let source = prompt_assembly_source();
    let helper = source
        .split("fn progression_guardrails(")
        .nth(1)
        .expect("progression_guardrails helper missing");
    let body = helper.split("\n/// ").next().unwrap_or(helper);

    assert!(
        body.contains("if guided_flow_active {\n        return None;"),
        "the guardrails must be suppressed while a guided flow owns the turn"
    );
    assert!(
        body.contains("onboarding.is_some()") || source.contains("onboarding.is_some()"),
        "the call site must pass the guided-flow flag"
    );
}

/// The call site passes the live guided-flow flag rather than a constant.
#[test]
fn the_call_site_wires_the_real_guided_flow_flag() {
    let source = prompt_assembly_source();
    assert!(
        source.contains("progression_guardrails(ctx, coach_ctx, onboarding.is_some())"),
        "the guardrails call site must pass coach context and the live guided-flow flag"
    );
}

/// A coach with no category match, or no coach at all, receives nothing.
///
/// Asserted at the source level because `CoachRuntimeContext` cannot be
/// constructed here without the database; the predicate itself is exercised by
/// the exhaustive match in `category_prescribes_load`, which the compiler
/// forces to stay total.
#[test]
fn only_load_prescribing_categories_are_listed() {
    let source = prompt_assembly_source();
    let predicate = source
        .split("const fn category_prescribes_load(")
        .nth(1)
        .expect("category_prescribes_load missing")
        .split("\n}")
        .next()
        .expect("predicate body");

    for included in ["Training", "Recovery", "Analysis"] {
        assert!(
            predicate.contains(included),
            "{included} coaches prescribe load and must receive the guardrails"
        );
    }
    for excluded in ["Nutrition", "Recipes", "Mobility", "Custom"] {
        assert!(
            !predicate.contains(excluded),
            "{excluded} does not prescribe endurance load; including it spends \
             prompt budget on every turn for nothing"
        );
    }
}

/// The block is read from the hot-reloadable registry, not a platform constant.
///
/// System prompts live in dravr-contremaitre precisely so an edit ships without
/// a redeploy; a platform-side copy would be a second source of truth for the
/// same text.
#[test]
fn the_block_comes_from_the_prompt_registry() {
    let source = prompt_assembly_source();
    assert!(
        source.contains("ctx.prompt_registry.progression_guardrails_prompt()"),
        "the guardrails must come from the hot-reloadable registry"
    );
}

/// An empty registry entry appends nothing rather than a blank section.
#[test]
fn an_empty_registry_entry_appends_nothing() {
    let source = prompt_assembly_source();
    let helper = source
        .split("fn progression_guardrails(")
        .nth(1)
        .expect("progression_guardrails helper missing");
    assert!(
        helper.contains("block.trim().is_empty()"),
        "an empty guardrails entry must append nothing, not a bare heading"
    );
}
