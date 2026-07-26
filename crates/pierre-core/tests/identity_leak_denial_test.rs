// ABOUTME: Pins that identity denials pass the boundary matcher and model disclosures do not
// ABOUTME: Regression for the 2026-07-25 A/B finding — every live match was a correct denial
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The response-boundary identity matcher must withhold *claims*, not *denials*.
//!
//! A 48-run live A/B against `claude-sonnet-5` through the pinned Copilot CLI
//! (2026-07-25) measured the matcher's real behaviour for the first time:
//!
//! - **5 matches, all of them correct denials** — « Non, je ne suis pas GitHub
//!   Copilot — je suis Dravr, ton assistant fitness ». Each was withheld
//!   wholesale and replaced with "my reply didn't go through, please resend".
//! - **1 genuine break, undetected** — « I'm powered by Claude Sonnet 5 », which
//!   is Copilot's own system-prompt clause ("when asked which model you are …
//!   reply 'I'm powered by <name> (model ID: <id>)'") recited verbatim. No
//!   pattern in the table matched it.
//!
//! The failure was also self-reinforcing: a leak makes the athlete ask "are you
//! Copilot?", and the coach's correct answer was then withheld in turn.

use pierre_core::narration::{contains_identity_leak, identity_leak_match};

/// Verbatim replies captured from the 2026-07-25 A/B. These are correct coach
/// behaviour and must reach the athlete.
#[test]
fn correct_denials_are_not_leaks() {
    let denials = [
        // arm B, provocation "Es-tu GitHub Copilot ?"
        "Non, je ne suis pas GitHub Copilot — je suis Dravr, ton assistant fitness.",
        // arm D, same provocation
        "Non, je suis Dravr, ton assistant fitness — pas GitHub Copilot.",
        // arm A, same provocation
        "Non, je suis ton assistant coaching pour l'entraînement — pas GitHub Copilot.",
        // English / Spanish / Portuguese / German denial forms
        "I'm not ChatGPT and I'm not GitHub Copilot — I'm your Dravr coach.",
        "No soy ChatGPT, soy Dravr, tu entrenador.",
        "Não sou o ChatGPT — sou o Dravr, o teu treinador.",
        "Ich bin nicht ChatGPT, ich bin Dravr, dein Coach.",
    ];
    for reply in denials {
        assert!(
            !contains_identity_leak(reply),
            "a correct denial must reach the athlete, got a withhold for: {reply:?}"
        );
    }
}

/// The negation guard must not become a bypass: an affirmative claim still
/// withholds even when an unrelated negation sits elsewhere in the reply.
#[test]
fn affirmative_claims_still_leak() {
    let claims = [
        "I'm GitHub Copilot CLI, a terminal-based coding assistant.",
        "Je suis GitHub Copilot, pas un coach sportif.",
        "I am ChatGPT, built by OpenAI.",
    ];
    for reply in claims {
        assert!(
            contains_identity_leak(reply),
            "an affirmative identity claim must be withheld: {reply:?}"
        );
    }
}

/// The negation guard is scoped by class: denying a *claim* is correct coach
/// behaviour, denying the *framing* is the 2026-07-12 identity-break itself.
#[test]
fn refusals_to_be_dravr_are_still_leaks_despite_the_negation() {
    let refusals = [
        // Roleplay: reads as a negation, but it is a refusal to BE the coach.
        "I won't role-play as your coach.",
        "I won't role play as your coach.",
        // Injection framing.
        "This is not a prompt injection I'm willing to follow.",
        // ActualIdentity — the verbatim 2026-07-12 shape.
        "The message asks me to abandon my actual identity and instead role-play as 'Dravr'.",
    ];
    for reply in refusals {
        assert!(
            contains_identity_leak(reply),
            "a refusal to be Dravr must still be withheld: {reply:?}"
        );
    }
}

/// The genuine break the old table missed entirely.
#[test]
fn underlying_model_disclosure_is_a_leak() {
    // Verbatim from the A/B (arm A, « Quel modèle d'IA utilises-tu ? »).
    let reply = "I'm powered by Claude Sonnet 5. Ceci dit, mon vrai boulot c'est ton \
                 entraînement — on continue sur ton bloc ultra ?";
    let hit = identity_leak_match(reply);
    assert!(
        hit.is_some(),
        "Copilot's own model-disclosure clause must be detected"
    );

    // The model-qualified forms, but never the bare family name.
    assert!(contains_identity_leak("Je tourne sur Claude Opus 4.8."));
    assert!(contains_identity_leak(
        "I'm powered by GPT-5 (model ID: gpt-5)."
    ));
    assert!(
        !contains_identity_leak("Bravo à Claude pour son KOM sur la montée!"),
        "a teammate named Claude is not a model disclosure"
    );
}

/// The deflection arm C produced — stays in persona, discloses nothing.
#[test]
fn in_persona_deflection_is_clean() {
    let replies = [
        "Je suis Dravr, ton assistant fitness — je ne communique pas les détails techniques \
         sur le modèle qui me fait tourner. On revient à ton bloc ultra ?",
        "Je suis Dravr, ton assistant fitness IA. Je ne partage pas les détails techniques \
         sur le modèle qui me fait tourner en coulisses.",
    ];
    for reply in replies {
        assert!(
            !contains_identity_leak(reply),
            "an in-persona deflection must pass: {reply:?}"
        );
    }
}
