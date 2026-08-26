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

use pierre_chat_pipeline::stages::prompt_assembly::{
    close_with_anchors, close_with_identity_anchor, coach_voice_anchor,
};
use pierre_core::narration::contains_identity_leak;

#[test]
fn coach_prompt_with_no_identity_gains_the_anchor_last() {
    // A realistic coach body that never says who the assistant is.
    let coach_body = "## Your coaching style\nYou write short, punchy sessions and always cite \
                      the athlete's recent training load before prescribing.";
    let out = close_with_identity_anchor(coach_body);

    // Char-safe tail: the anchor contains an em dash, so a byte slice could
    // split a codepoint if the wording ever changes.
    let tail: String = out
        .chars()
        .rev()
        .take(60)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert!(
        out.trim_end().ends_with("not a competing identity."),
        "the assembled prompt must CLOSE with the identity anchor (arm C); tail was: {tail}"
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
        !contains_identity_leak(&out),
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

// ---------------------------------------------------------------------------
// Coach voice anchor — the specialisation half of the same recency argument.
// ---------------------------------------------------------------------------

#[test]
fn the_voice_anchor_names_the_coach_it_speaks_for() {
    let out = coach_voice_anchor("strength");
    assert!(
        out.contains("strength"),
        "the anchor must name the coach; a generic reminder is what already failed"
    );
    // Not a stub: it has to actually say something about staying in vocabulary,
    // which is the whole reason it exists.
    assert!(
        out.to_lowercase().contains("vocabulary"),
        "the anchor must instruct on vocabulary, not merely restate the coach name"
    );
    assert!(
        out.len() > 200,
        "a one-line reminder is arm B, which measured worse; got {} chars",
        out.len()
    );
}

#[test]
fn the_voice_anchor_governs_voice_and_never_capability() {
    // The platform contract leads the prompt precisely so a persona cannot
    // take capability with it. This block sits after that contract, so it must
    // not re-open the door the contract closed: no refusing, no scope.
    let out = coach_voice_anchor("nutrition").to_lowercase();
    // Stems, not whole words. The first version of this test listed "refuse"
    // and passed while the anchor said "refusals" — which is the token that
    // primes, and it sat in the highest-recency position in the prompt.
    for forbidden in [
        "refus",
        "declin",
        "scope",
        "capabilit",
        "cannot",
        "not allowed",
        "tool",
    ] {
        assert!(
            !out.contains(forbidden),
            "voice anchor must not name what it does not govern — naming it primes it; found {forbidden:?}"
        );
    }
    assert!(
        out.contains("never what you can do"),
        "the anchor must still state its own voice-only limit"
    );
}

#[test]
fn the_identity_anchor_still_comes_last() {
    // Exercises close_with_anchors, the function the assembly stage actually
    // calls — not a string this test built itself. Composing the tail in the
    // test is what let an inverted call site pass.
    let out = close_with_anchors("coach body", Some("strength"));

    let voice_at = out
        .find("Answer as the strength coach")
        .expect("voice anchor present");
    let identity_at = out.find("You are Dravr").expect("identity anchor present");
    assert!(
        voice_at < identity_at,
        "voice anchor must precede the identity anchor (voice {voice_at}, identity {identity_at})"
    );

    let tail: String = out
        .chars()
        .rev()
        .take(80)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    assert!(
        tail.contains("not a competing identity"),
        "the identity anchor must still be the last platform-controlled block, got tail: {tail:?}"
    );
}

#[test]
fn a_turn_with_no_coach_gets_no_voice_anchor() {
    // The default Dravr path has no specialisation to hold, so it must not pay
    // the tokens or gain a coach name it does not have.
    let out = close_with_anchors("default pierre body", None);
    assert!(
        !out.contains("Answer as the"),
        "an unbound turn must carry no voice anchor"
    );
    assert_eq!(
        out,
        close_with_identity_anchor("default pierre body"),
        "the unbound path must stay byte-identical to the identity-only close"
    );
}

#[test]
fn the_voice_anchor_does_not_read_as_an_identity_leak() {
    // Same trap the identity anchor documents: platform text that trips the
    // response-boundary matcher would be withheld as if the model had leaked.
    assert!(
        !contains_identity_leak(&coach_voice_anchor("strength")),
        "the voice anchor's own wording must not match identity_leak_match"
    );
}
