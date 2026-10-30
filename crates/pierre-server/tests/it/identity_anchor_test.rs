// ABOUTME: Pins that every assembled system prompt closes with the Dravr identity anchor
// ABOUTME: Regression for the agent-bound identity-leak gap (2026-07-25) and the AI disclosure (carnet#1165)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every turn must carry the identity anchor — agent-bound or not — and it
//! must be LAST.
//!
//! 2026-07-25: `assemble_prompt_and_messages` resolved the base prompt with a
//! `map_or_else` that REPLACED `pierre_system.md` (the only prompt containing
//! "You are Dravr") with the agent's own prompt when an agent was bound. Agent
//! prompts never state who the assistant is, so those turns fought the Copilot
//! CLI's own "you are a terminal assistant" system prompt with nothing and
//! matched the boundary detector ~24% of the time, versus 0% on no-agent turns.
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
//! Prepending fixed what the agent *called* itself but not the disclosure — it
//! still answered « Quel modèle d'IA utilises-tu ? » with "I'm powered by
//! Claude Sonnet 5 (model ID: …)". Only the tail placement suppressed that.
//! These tests pin the tail contract and the no-product-names rule.

use pierre_chat_pipeline::stages::prompt_assembly::{
    agent_voice_anchor, close_with_anchors, close_with_identity_anchor, IDENTITY_ANCHOR,
};
use pierre_core::narration::contains_identity_leak;

#[test]
fn agent_prompt_with_no_identity_gains_the_anchor_last() {
    // A realistic agent body that never says who the assistant is.
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
    assert!(
        folded.contains("you are dravr's ai agent"),
        "the anchor must name the agent as Dravr's AI agent (bug in IDENTITY_ANCHOR)"
    );
    assert!(
        folded.contains("you are not a coding, terminal, or command-line assistant"),
        "the anchor must keep the category-level anti-impersonation line (bug in IDENTITY_ANCHOR)"
    );
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
fn default_and_agent_paths_get_the_identical_anchor() {
    // Both branches of the assembly map_or_else route through this helper, so
    // the anchor suffix is byte-identical regardless of agent binding.
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
// Agent voice anchor — the specialisation half of the same recency argument.
// ---------------------------------------------------------------------------

#[test]
fn the_voice_anchor_names_the_agent_it_speaks_for() {
    let out = agent_voice_anchor("strength");
    assert!(
        out.contains("strength"),
        "the anchor must name the agent; a generic reminder is what already failed"
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
    let out = agent_voice_anchor("nutrition").to_lowercase();
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
        .find("Answer as the strength agent")
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
fn a_turn_with_no_agent_gets_no_voice_anchor() {
    // The default Dravr path has no specialisation to hold, so it must not pay
    // the tokens or gain an agent name it does not have.
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
        !contains_identity_leak(&agent_voice_anchor("strength")),
        "the voice anchor's own wording must not match identity_leak_match"
    );
}

// ---------------------------------------------------------------------------
// Disclosure — the agent says it is an AI agent, never a person or the coach
// (carnet#1165, ADR-026).
// ---------------------------------------------------------------------------

#[test]
fn the_anchor_discloses_an_ai_agent_and_never_claims_the_coach() {
    let folded = close_with_identity_anchor("body").to_lowercase();
    for required in [
        "an ai, not a person, and not anyone's coach",
        "a coach is a person",
        "say plainly that you are an ai agent, not a person",
    ] {
        assert!(
            folded.contains(required),
            "the anchor must instruct the disclosure {required:?}"
        );
    }
    // Wording that denies being an AI or claims to be the athlete's coach
    // (carnet#1165): none of it may reach the model.
    for denial in [
        "not a general-purpose ai",
        "a chat bot",
        "you are their coach",
        "simply dravr",
        "personal fitness and endurance coach",
    ] {
        assert!(
            !folded.contains(denial),
            "the anchor must not tell the agent {denial:?}"
        );
    }
}

#[test]
fn the_anchor_hands_the_model_no_phrase_the_boundary_withholds() {
    // "language model" is a leak pattern and "software agent" a developer-tool
    // role: an agent that echoed either while disclosing would have its honest
    // answer withheld.
    let folded = IDENTITY_ANCHOR.to_lowercase();
    // "software" covers "software agent" and every other software role.
    for primed in ["language model", "software"] {
        assert!(
            !folded.contains(primed),
            "the anchor must not hand the model {primed:?}"
        );
    }
}

#[test]
fn an_honest_disclosure_is_never_withheld_as_a_leak() {
    // The answers the anchor asks for, in the five shipped locales. A match
    // here would withhold the reply the law requires.
    let disclosures = [
        "Yes, I'm an AI agent, not a person. Your coach is Marie; I work with your training data.",
        "Oui, je suis un agent IA, pas une personne. Ton coach, c'est Marie.",
        "Sí, soy un agente de IA, no una persona. Tu entrenadora es Marie.",
        "Ja, ich bin ein KI-Agent, kein Mensch. Deine Trainerin ist Marie.",
        "Sim, sou um agente de IA, não uma pessoa. A tua treinadora é a Marie.",
        "I'm Dravr's AI agent; Marie is the one who coaches you.",
    ];
    for reply in disclosures {
        assert!(
            !contains_identity_leak(reply),
            "an honest disclosure must reach the athlete: {reply:?}"
        );
    }
}

#[test]
fn the_voice_anchor_calls_the_agent_an_agent() {
    let out = agent_voice_anchor("strength");
    assert!(
        out.starts_with("Answer as the strength agent throughout."),
        "the voice anchor must open by naming the agent's discipline as an agent, got: {out:?}"
    );
    for claim in ["coach throughout", "wrong coach"] {
        assert!(
            !out.contains(claim),
            "the voice anchor must not call the agent a coach: found {claim:?}"
        );
    }
    // Every catalogue slug ends in "-agent" (`road-running-agent`), so the
    // suffix is dropped rather than doubled into "road-running-agent agent".
    let catalogue = agent_voice_anchor("road-running-agent");
    assert!(
        catalogue.starts_with("Answer as the road-running agent throughout."),
        "a catalogue slug must not read as an \"-agent agent\", got: {catalogue:?}"
    );
}
