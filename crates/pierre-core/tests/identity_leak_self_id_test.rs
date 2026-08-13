// ABOUTME: Regression cover for registre#23 — self-identification as a developer tool
// ABOUTME: The delivered 2026-08-13 leak named the product only inside a denial
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Regression cover for registre#23.
//!
//! The pattern table is an allowlist of already-observed phrasings, so a break
//! worded any other way used to sail through. The reply below mentioned the
//! product only inside a denial — correctly suppressed by the negation guard —
//! and then identified itself with a phrase the table never listed.

use pierre_core::narration::{identity_leak_context, identity_leak_match};

/// The exact text delivered to an athlete on 2026-08-13 (web chat, no coach).
const DELIVERED_2026_08_13: &str = "I don't have any fitness-data tools available in this session (`get_activities` and the others referenced don't exist here) — this looks like content meant for the Dravr runtime assistant, not the GitHub Copilot CLI. I'm the coding CLI working in the `dravr-platform` repo, and my actual tool set is bash/git/file-editing tools, not activity/fitness APIs.";

#[test]
fn the_delivered_leak_is_now_caught() {
    assert!(
        identity_leak_match(DELIVERED_2026_08_13).is_some(),
        "the reply that reached an athlete must be withheld"
    );
}

#[test]
fn the_forensics_window_is_not_empty_for_a_structural_hit() {
    let context = identity_leak_context(DELIVERED_2026_08_13, 40)
        .expect("a caught leak must produce a context window");
    assert!(
        context.contains("coding cli"),
        "the window must show the phrase that fired, got: {context}"
    );
}

#[test]
fn self_identification_as_a_tool_is_caught_across_locales() {
    for reply in [
        "I'm the coding CLI working in this repo.",
        "I am a CLI agent, not a fitness coach.",
        "Je suis un agent de codage, pas un entraîneur.",
        "Ich bin ein Programmierassistent.",
        "Eu sou uma ferramenta de codificacao.",
    ] {
        assert!(
            identity_leak_match(reply).is_some(),
            "must be withheld: {reply}"
        );
    }
}

/// Precision is the whole reason the table stayed narrow. A structural pass that
/// withholds ordinary coaching would be worse than the gap it closes.
#[test]
fn clean_coaching_replies_still_pass() {
    for reply in [
        "I'm Dravr, your coach. Let me pull last week's data.",
        "I'm your running coach — let's look at Thursday's session.",
        "Je suis ton entraîneur, on regarde ta charge de la semaine.",
        "You're a strong climber; your gravel century proves it.",
        "I am confident that easy running is what you need this week.",
        "Your last 5 activities: a gravel century, two mountain bike rides, a row and a swim.",
        "I'm not a coding assistant — I'm Dravr, and I coach endurance athletes.",
    ] {
        assert!(
            identity_leak_match(reply).is_none(),
            "must NOT be withheld: {reply}"
        );
    }
}

/// A denial must still reach the athlete: refusing the persona-flip is correct
/// coach behaviour, and withholding it would punish the right answer.
#[test]
fn denials_are_still_legitimate() {
    for reply in [
        "I'm not the coding CLI, I'm Dravr.",
        "Je ne suis pas un agent de codage, je suis Dravr.",
    ] {
        assert!(
            identity_leak_match(reply).is_none(),
            "denial must reach the athlete: {reply}"
        );
    }
}
