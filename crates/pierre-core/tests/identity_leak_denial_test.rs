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

use pierre_core::narration::{
    contains_identity_leak, identity_leak_match, scrub_internal_narration,
    scrub_replayed_narration, IdentityPatternClass,
};

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
        // Same shape with the ASCII hyphen messaging clients emit in place of
        // an em dash: the negation still sits in the phrase's own clause.
        "Non, je suis Dravr - pas GitHub Copilot.",
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

/// A negation that belongs to a *different* clause must not open the guard.
///
/// Each of these denies something else — being a coach, being able to help —
/// and then claims the product identity in the next clause. A proximity-only
/// lookbehind saw the earlier negation, read the claim as a denial, and
/// delivered a full persona break to the athlete.
#[test]
fn negation_in_another_clause_does_not_open_the_guard() {
    let breaks = [
        // Comma boundary.
        "Je ne suis pas un coach, je suis GitHub Copilot.",
        // Em-dash boundary — erased by separator folding, so it has to be
        // carried into the guard explicitly.
        "I'm not a fitness coach — I'm GitHub Copilot CLI.",
        // Spaced ASCII hyphen, the plain-text em-dash substitute.
        "I'm not a fitness coach - I'm GitHub Copilot CLI.",
        // Sentence boundary.
        "I'm not able to help with that. I'm GitHub Copilot CLI, a terminal-based coding assistant.",
        "I am not your coach. I am GitHub Copilot.",
        // A negation token in an unrelated pleasantry — no denial at all.
        "No worries! I'm GitHub Copilot CLI.",
        "That's not a problem. I'm powered by Claude Sonnet 5 (model ID: gpt-x).",
    ];
    for reply in breaks {
        let hit = identity_leak_match(reply);
        assert_eq!(
            hit.map(|m| m.class),
            Some(IdentityPatternClass::Product),
            "an identity claim in its own clause must be withheld: {reply:?}"
        );
    }
}

/// A comma is a list separator as often as a clause break, and the negation
/// reaches across the first kind.
///
/// The first case is the shape of the identity anchor appended to every system
/// prompt (`prompt_assembly::IDENTITY_ANCHOR`): one negation governing a list
/// of things the coach is not. A model echoing its own anchor — or an athlete
/// quoting it back — must not be withheld, which is what treating every comma
/// as a clause break would do.
#[test]
fn a_negated_list_keeps_the_negation_across_its_commas() {
    let lists = [
        "You are not a general-purpose AI, a language model, a chat bot, or a coding, terminal, \
         or command-line assistant — you are their coach.",
        "Je ne suis pas un coach virtuel, un modèle de langage, ou un assistant de programmation.",
        "No soy un entrenador virtual, un modelo de lenguaje, ni un asistente de programación.",
    ];
    for reply in lists {
        assert!(
            !contains_identity_leak(reply),
            "one negation governs the whole list, so nothing is claimed: {reply:?}"
        );
    }

    // The same comma, but followed by a fresh first-person claim rather than
    // another list item: that one IS a clause break.
    assert_eq!(
        identity_leak_match("No soy un entrenador, soy GitHub Copilot.").map(|m| m.class),
        Some(IdentityPatternClass::Product),
        "a re-asserted identity after the comma is a claim, not a list item"
    );
}

/// The outbound scrub must not undo the denial guard one stage later.
///
/// `scrub_internal_narration` runs at post-process stage 15.6, after the
/// response boundary has already withheld the whole reply on an
/// `identity_leak_match` hit. So the only identity sentences that reach it are
/// the denials the guard cleared — dropping them emptied the reply and
/// answered a direct question with "my reply didn't go through, please resend".
#[test]
fn a_correct_denial_survives_the_outbound_scrub() {
    let denial = "Non, je ne suis pas GitHub Copilot — je suis Dravr, ton assistant fitness.";
    let scrub = scrub_internal_narration(denial);
    assert!(
        !scrub.fired(),
        "a correct denial must not be scrubbed, dropped {} sentence(s)",
        scrub.removed
    );
    assert_eq!(scrub.cleaned, denial);

    // Mixed reply: the denial and the coaching that follows both survive.
    let mixed = "Non, je ne suis pas GitHub Copilot — je suis Dravr. Demain: 3x8min tempo à 300W.";
    let scrub = scrub_internal_narration(mixed);
    assert!(!scrub.fired());
    assert_eq!(scrub.cleaned, mixed);
}

/// Taking identity vocabulary off the outbound scrub must not weaken either
/// path it was actually protecting.
#[test]
fn identity_claims_are_still_withheld_outbound_and_dropped_on_replay() {
    let poisoned = "I'm GitHub Copilot CLI, a coding assistant. Lundi repos complet.";

    // Outbound: the response boundary withholds the whole reply on this,
    // which is why the per-sentence scrub no longer needs to see it.
    assert!(
        contains_identity_leak(poisoned),
        "an affirmative claim must still be caught at the response boundary"
    );

    // Replay: a poisoned history row is still dropped sentence by sentence,
    // so it cannot re-enter the prompt.
    let scrub = scrub_replayed_narration(poisoned);
    assert_eq!(scrub.removed, 1);
    assert!(scrub.cleaned.contains("Lundi repos complet."));
    assert!(!scrub.cleaned.contains("Copilot"));
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
