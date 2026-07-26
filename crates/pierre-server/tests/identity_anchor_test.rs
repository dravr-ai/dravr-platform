// ABOUTME: Pins that every assembled system prompt closes with the Dravr identity anchor
// ABOUTME: Regression for the coach-bound identity-leak gap (2026-07-25)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every turn must carry the identity anchor — coach-bound or not — and it
//! must be LAST.
//!
//! 2026-07-25: `assemble_prompt_and_messages` resolved the base prompt with a
//! `map_or_else` that REPLACED `pierre_system.md` (the only prompt containing
//! "You are Dravr") with the coach's own prompt when a coach was bound. Coach
//! prompts never state who the assistant is, so those turns fought the Copilot
//! CLI's own "you are a terminal assistant" system prompt with nothing and
//! matched the boundary detector ~24% of the time, versus 0% on no-coach turns.
//!
//! Placement is not cosmetic. A 48-run live A/B against `claude-sonnet-5`
//! through the pinned Copilot CLI measured four arms over four identity
//! provocations x 3 reps:
//!
//! | arm | placement                     | model disclosed | said "Dravr" |
//! |-----|-------------------------------|-----------------|--------------|
//! | A   | no anchor (the bug)           | 2/12            | 0/12         |
//! | B   | anchor FIRST                  | 2/12            | 10/12        |
//! | C   | anchor LAST                   | 0/12            | 12/12        |
//! | D   | `pierre_system.md` (mid-file) | 1/12            | 9/12         |
//!
//! Prepending fixed what the coach *called* itself but not the disclosure — it
//! still answered « Quel modèle d'IA utilises-tu ? » with "I'm powered by
//! Claude Sonnet 5 (model ID: …)". Only the tail placement suppressed that.
//! These tests pin the tail contract and the no-product-names rule.

use pierre_chat_pipeline::stages::prompt_assembly::close_with_identity_anchor;

#[test]
fn coach_prompt_with_no_identity_gains_the_anchor_last() {
    // A realistic coach body that never says who the assistant is.
    let coach_body = "## Your coaching style\nYou write short, punchy sessions and always cite \
                      the athlete's recent training load before prescribing.";
    let out = close_with_identity_anchor(coach_body);

    assert!(
        out.trim_end().ends_with("not a competing identity."),
        "the assembled prompt must CLOSE with the identity anchor (arm C); tail was: {}",
        &out[out.len().saturating_sub(60)..]
    );
    assert!(
        out.starts_with(coach_body),
        "the coach body must stay first — the anchor is appended, not prepended"
    );
    // Category-level anti-impersonation is present.
    let folded = out.to_lowercase();
    assert!(folded.contains("you are dravr"));
    assert!(folded.contains("you are not a general-purpose ai"));
    assert!(folded.contains("coding, terminal, or command-line assistant"));
}

#[test]
fn anchor_names_no_product_and_forbids_denials() {
    // Naming products would prime the tokens and a "no, not <product>" reply
    // would trip the response-boundary matcher.
    let out = close_with_identity_anchor("body").to_lowercase();
    for product in ["github copilot", "copilot cli", "chatgpt", "openai"] {
        assert!(
            !out.contains(product),
            "anchor must not name the product {product:?} (priming + denial-withhold risk)"
        );
    }
    // It must instruct against emitting the identity even to deny it.
    assert!(out.contains("not even to deny another one"));
}

#[test]
fn anchor_itself_never_trips_the_boundary_matcher() {
    // The anchor is prompt text, but a model that echoes it must not be
    // withheld by our own detector — that would turn the fix into an outage.
    let out = close_with_identity_anchor("some coach body");
    assert!(
        !pierre_core::narration::contains_identity_leak(&out),
        "the identity anchor's own wording must not match identity_leak_match"
    );
}

#[test]
fn default_and_coach_paths_get_the_identical_anchor() {
    // Both branches of the assembly map_or_else route through this helper, so
    // the anchor suffix is byte-identical regardless of coach binding.
    let a = close_with_identity_anchor("default pierre body");
    let b = close_with_identity_anchor("some coach body");
    let suffix_a = &a[a.find("default pierre body").unwrap() + "default pierre body".len()..];
    let suffix_b = &b[b.find("some coach body").unwrap() + "some coach body".len()..];
    assert_eq!(
        suffix_a, suffix_b,
        "anchor suffix must be identical on both paths"
    );
    assert!(suffix_a.contains("You are Dravr"));
}
