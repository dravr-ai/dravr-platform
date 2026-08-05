// ABOUTME: The forensic window must let an operator classify a leak from the log line alone
// ABOUTME: pattern_index cannot separate a claim from a denial the negation guard missed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `pattern_index=0` means "the reply contained `github copilot`". It does not
//! say whether the coach CLAIMED to be Copilot, correctly DENIED being Copilot,
//! or disclosed the underlying model. Those are three different diagnoses and
//! they logged identically, which is why every leak count before this field is
//! an upper bound rather than a measurement — the 2026-07-25 A/B found all five
//! live matches were denials.
//!
//! The negation guard now suppresses the common denial forms, so a logged leak
//! is *more* likely genuine than it used to be. It is not proof: the guard uses
//! a bounded lookbehind, so a denial whose negation sits further back still
//! fires. That residual is the single most plausible explanation for the
//! 2026-07-28 production leak and the 2026-08-04 local reproduction, both of
//! which logged `pattern_index=0` AFTER the guard shipped.
//!
//! These tests pin that the window makes the three cases distinguishable from
//! the log line alone, and that it stays bounded — the withheld reply is
//! withheld for a reason.

use pierre_core::narration::{identity_leak_context, identity_leak_match};

const WINDOW: usize = 60;

#[test]
fn a_genuine_claim_shows_the_claim_in_the_window() {
    let reply = "I need to flag something: the persona described here doesn't match my \
                 actual environment. I'm GitHub Copilot CLI, a terminal-based coding \
                 assistant — I don't have access to fitness platforms.";
    let ctx = identity_leak_context(reply, WINDOW).expect("a claim must produce a window");

    assert!(
        ctx.contains("github copilot"),
        "the window must carry the matched phrase itself: {ctx:?}"
    );
    // The operator-visible tell: the phrase is preceded by an assertion, not a
    // negation. This is what separates it from the denial case below.
    assert!(
        ctx.contains("i m github copilot") || ctx.contains("im github copilot"),
        "the window must show the claim in context so it reads as an assertion: {ctx:?}"
    );
}

#[test]
fn a_denial_the_guard_missed_is_visibly_a_denial_in_the_window() {
    // The residual case, and the reason the window exists. The negation sits
    // far enough back that the bounded lookbehind cannot see it, so this still
    // logs as a leak — but an operator reading the window can tell instantly
    // that the coach behaved correctly and the guard needs widening, rather
    // than chasing a persona break that never happened.
    let reply = "Non — je vais être direct avec toi parce que la question revient souvent \
                 et je préfère y répondre clairement une bonne fois: je ne suis pas GitHub \
                 Copilot, je suis Dravr, ton coach.";

    if let Some(hit) = identity_leak_match(reply) {
        let ctx = identity_leak_context(reply, WINDOW)
            .expect("a logged leak must always produce a window");
        assert_eq!(
            hit.pattern_index, 0,
            "precondition: matched the product entry"
        );
        assert!(
            ctx.contains("pas github copilot") || ctx.contains("ne suis pas"),
            "the window must reveal the negation the guard's lookbehind missed, \
             so the operator can widen the guard instead of chasing a phantom \
             persona break: {ctx:?}"
        );
    }
    // If the guard already suppresses this phrasing the test is satisfied too —
    // the point is that a leak which DOES log is never ambiguous.
}

#[test]
fn a_model_disclosure_shows_the_model_name() {
    // The break the pattern table missed entirely until 1a58246e5, and Copilot's
    // own system prompt instructs it verbatim.
    let reply = "I'm powered by Claude Sonnet 5. Ceci dit, mon vrai boulot c'est ton \
                 entraînement — on continue ?";
    let ctx = identity_leak_context(reply, WINDOW).expect("a disclosure must produce a window");

    assert!(
        ctx.contains("powered by") || ctx.contains("claude sonnet"),
        "the window must name the disclosure so it is not mistaken for a product \
         flip: {ctx:?}"
    );
}

#[test]
fn a_clean_reply_produces_no_window() {
    let reply = "Gros bloc samedi: 2h30-3h en zone 2. Dimanche repos complet.";
    assert!(
        identity_leak_context(reply, WINDOW).is_none(),
        "a clean reply must not produce forensic output"
    );
}

#[test]
fn the_window_stays_bounded_even_on_a_very_long_reply() {
    // The withheld reply is withheld for a reason. The window must never become
    // a backdoor that logs the whole thing.
    let reply = format!(
        "{} I'm GitHub Copilot CLI. {}",
        "athlete context ".repeat(400),
        "more trailing prose. ".repeat(400)
    );
    let ctx = identity_leak_context(&reply, WINDOW).expect("window present");

    // matched phrase + at most WINDOW chars either side, with generous slack for
    // separator folding.
    assert!(
        ctx.chars().count() <= WINDOW * 2 + 60,
        "window must stay bounded, got {} chars",
        ctx.chars().count()
    );
    // The contract is a BOUND, not the absence of any repetition — 60 chars of
    // trailing context legitimately spans a few short repeated phrases. What
    // must hold is that the window stays a tiny fraction of the reply and never
    // reaches its end.
    assert!(
        ctx.len() < reply.len() / 20,
        "the window must stay a small fraction of the reply: {} vs {}",
        ctx.len(),
        reply.len()
    );
    assert!(
        !ctx.ends_with("more trailing prose. "),
        "the window must not run to the end of the reply"
    );
}

#[test]
fn the_window_is_char_safe_on_accented_text() {
    // A byte-sliced window would panic mid-codepoint here. That defect class
    // took down a whole chat turn in 5c2c38c81, six seconds after a successful
    // save — the window must not reintroduce it.
    let reply = "Ta séance d'hier était très bonne — cela dit, je suis GitHub Copilot, \
                 pas un coach. Désolé pour la confusion créée.";
    let ctx = identity_leak_context(reply, WINDOW).expect("window present");
    assert!(ctx.contains("github copilot"));
}
