// ABOUTME: Smoke tests for CoachingPersona → persona-block prompt resolution
// ABOUTME: Asserts each persona variant maps to a non-empty, discriminating block
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the coaching-persona prompt block resolver.
//!
//! Covers the contract between [`CoachingPersona`] enum variants and the
//! `personas/*.md` files they map to via
//! [`get_coaching_persona_prompt`]: every variant must resolve to a
//! distinct, non-empty block, and `pierre_system.md` must keep the
//! `{{COACHING_PERSONA_RULES}}` placeholder so substitution does not
//! silently no-op.

use pierre_core::models::CoachingPersona;
use pierre_llm::prompts::{get_coaching_persona_prompt, PIERRE_SYSTEM_PROMPT};

/// Each persona variant must resolve to a non-empty block whose first
/// line names the persona by convention (`**Active persona: <Name>.**`).
/// This catches the failure mode where a new variant lands on the enum
/// without a matching `include_str!()` constant or `match` arm.
#[test]
fn each_persona_variant_resolves_to_discriminating_block() {
    let cases = [
        (CoachingPersona::Casual, "Active persona: Casual"),
        (CoachingPersona::Enthusiast, "Active persona: Enthusiast"),
        (
            CoachingPersona::PowerAthlete,
            "Active persona: Power-athlete",
        ),
        (CoachingPersona::Coach, "Active persona: Coach"),
    ];
    for (persona, expected_marker) in cases {
        let block = get_coaching_persona_prompt(persona);
        assert!(
            !block.is_empty(),
            "persona {persona:?} resolved to empty block"
        );
        assert!(
            block.contains(expected_marker),
            "persona {persona:?} block missing expected marker '{expected_marker}'; \
             got first 200 chars: {first}",
            first = &block[..block.len().min(200)],
        );
    }
}

/// Default persona must be `Casual` so an unmigrated user (NULL → DEFAULT)
/// or a lookup failure inside `resolve_user_persona` collapses to the
/// least-restrictive output style — never to `PowerAthlete` or `Coach`,
/// which carry stricter discipline appropriate only for opted-in users.
#[test]
fn default_persona_is_casual() {
    assert_eq!(CoachingPersona::default(), CoachingPersona::Casual);
}

/// Persona-block content must be distinct across all four variants —
/// otherwise the substitution provides no behavioral differentiation.
#[test]
fn persona_blocks_are_distinct() {
    let blocks = [
        get_coaching_persona_prompt(CoachingPersona::Casual),
        get_coaching_persona_prompt(CoachingPersona::Enthusiast),
        get_coaching_persona_prompt(CoachingPersona::PowerAthlete),
        get_coaching_persona_prompt(CoachingPersona::Coach),
    ];
    for i in 0..blocks.len() {
        for j in (i + 1)..blocks.len() {
            assert_ne!(
                blocks[i], blocks[j],
                "persona blocks at index {i} and {j} are byte-identical"
            );
        }
    }
}

/// The `PowerAthlete` persona must surface the Section 11 discipline anchors —
/// P0–P3 readiness ladder and the 10-point validation checklist — because
/// those are what differentiate it from Enthusiast/Casual. If they vanish
/// from `power_athlete.md`, the persona collapses to plain prose.
#[test]
fn power_athlete_carries_section_11_anchors() {
    let block = get_coaching_persona_prompt(CoachingPersona::PowerAthlete);
    assert!(
        block.contains("P0")
            && block.contains("P1")
            && block.contains("P2")
            && block.contains("P3"),
        "power_athlete block missing P0–P3 readiness ladder"
    );
    assert!(
        block.contains("Validation checklist"),
        "power_athlete block missing 10-point validation checklist anchor"
    );
}

/// `pierre_system.md` must carry the `{{COACHING_PERSONA_RULES}}` placeholder.
/// If a future prompt edit removes it, persona substitution silently no-ops
/// and every user gets the un-personalized default — this test fails loudly.
#[test]
fn pierre_system_prompt_contains_persona_placeholder() {
    let prompt = PIERRE_SYSTEM_PROMPT;
    assert!(
        prompt.contains("{{COACHING_PERSONA_RULES}}"),
        "pierre_system.md is missing the {{{{COACHING_PERSONA_RULES}}}} placeholder \
         — persona substitution will silently no-op"
    );
}
