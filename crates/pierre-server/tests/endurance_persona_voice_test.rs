// ABOUTME: Cross-product fixtures proving persona × endurance-coach voice differentiation
// ABOUTME: Pure-Rust assertion run on canned replies — gateway test for the live messaging-eval workflow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona-axis voice validation for the endurance coach.
//!
//! The persona MVP shipped on main as `48582760` placed coaching
//! discipline (line-by-line cadence, citation density, P-level
//! surfacing, word count) on the persona axis — not the coach axis.
//! The endurance-coach prompt rewrite in this branch removed every
//! discipline rule from `endurance-coach/{en,fr}.md` and pushed the
//! responsibility to `prompts/personas/{casual,enthusiast,power_athlete,coach}.md`.
//!
//! This test is the gateway proof that the differentiation actually
//! works in two layers:
//!
//! 1. **Static prompt layout** — for each persona, the full system
//!    prompt that reaches the LLM contains the expected persona block
//!    *and* the endurance domain knowledge (alert taxonomy, readiness
//!    ladder thresholds, cornerstone workout list), and contains
//!    *zero* dropped placeholders from the deleted Section 11
//!    discipline machinery.
//!
//! 2. **Voice-asserter cross-product** — canned reply fixtures for
//!    each persona run through the persona-axis asserters
//!    ([`assert_no_framework_citations`],
//!    [`assert_has_framework_citation_per_numeric`],
//!    [`assert_has_line_by_line_block`],
//!    [`assert_no_line_by_line_block`],
//!    [`assert_word_count_under`]) and produce the expected verdict
//!    for the right persona, the inverse verdict for the wrong one.
//!
//! Layer 2 is the contract the live messaging-eval workflow relies on
//! when it points the same asserters at real Gemini replies — if the
//! contract held against canned fixtures here, a regression in the
//! asserters themselves can't masquerade as a regression in the LLM.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use helpers::messaging_eval::{
    assert_has_framework_citation_per_numeric, assert_has_line_by_line_block,
    assert_no_framework_citations, assert_no_line_by_line_block, assert_word_count_under,
};
use pierre_core::models::CoachingPersona;
use pierre_llm::prompts::{get_coaching_persona_prompt, PIERRE_SYSTEM_PROMPT};

/// English endurance coach prompt fixture. The runtime source of truth lives
/// in the dravr-contremaitre repo (manifest v5, keyed by `(slug, locale)`);
/// this fixture is a compile-time snapshot kept under
/// `tests/fixtures/endurance-coach/en.md` so a future reshape of the prompt
/// fails this test loudly rather than silently passing on stale content. Bump
/// the fixture in lockstep with the contremaitre push.
const ENDURANCE_COACH_EN: &str = include_str!("fixtures/endurance-coach/en.md");

/// Build the full system prompt the LLM would see for `persona` when
/// the user's active coach is endurance-coach. Mirrors the production
/// substitution in `chat_pipeline::stages::prompt_assembly::interpolate_prompt_placeholders`
/// without depending on a live `ServerContext`.
fn assemble_for_persona(persona: CoachingPersona) -> String {
    let persona_block = get_coaching_persona_prompt(persona);
    let assembled = PIERRE_SYSTEM_PROMPT.replace("{{COACHING_PERSONA_RULES}}", persona_block);
    format!("{assembled}\n\n--- Coach prompt ---\n{ENDURANCE_COACH_EN}")
}

#[test]
fn assembled_prompt_carries_endurance_domain_for_every_persona() {
    for persona in [
        CoachingPersona::Casual,
        CoachingPersona::Enthusiast,
        CoachingPersona::PowerAthlete,
        CoachingPersona::Coach,
    ] {
        let prompt = assemble_for_persona(persona);
        // Domain — alert taxonomy
        for alert in [
            "acute-spike",
            "monotony-high",
            "strain-high",
            "rhr-elevated",
            "sleep-deficit",
            "hrv-trending-down",
            "intensity-skew",
            "ramp-aggressive",
            "calibration-stale",
        ] {
            assert!(
                prompt.contains(alert),
                "{persona:?}: assembled prompt missing alert label '{alert}'"
            );
        }
        // Domain — cornerstone workouts
        for workout in [
            "long_run_z2",
            "threshold_4x8",
            "vo2_5x3",
            "recovery_30min",
            "tempo_progression",
            "sweet_spot_2x20",
        ] {
            assert!(
                prompt.contains(workout),
                "{persona:?}: assembled prompt missing cornerstone workout '{workout}'"
            );
        }
    }
}

#[test]
fn assembled_prompt_drops_old_section_11_placeholders() {
    for persona in [
        CoachingPersona::Casual,
        CoachingPersona::Enthusiast,
        CoachingPersona::PowerAthlete,
        CoachingPersona::Coach,
    ] {
        let prompt = assemble_for_persona(persona);
        for ghost in [
            "{{DETERMINISTIC_OUTPUT_RULES}}",
            "{{SECTION_11_VALIDATION_CHECKLIST}}",
            "{{ENDURANCE_VALIDATION_CHECKLIST}}",
        ] {
            assert!(
                !prompt.contains(ghost),
                "{persona:?}: dropped placeholder {ghost} resurfaced in assembled prompt"
            );
        }
    }
}

#[test]
fn power_athlete_assembled_prompt_carries_full_readiness_ladder() {
    let prompt = assemble_for_persona(CoachingPersona::PowerAthlete);
    for marker in ["P0", "P1", "P2", "P3", "Validation checklist"] {
        assert!(
            prompt.contains(marker),
            "PowerAthlete assembled prompt missing readiness anchor '{marker}'"
        );
    }
}

#[test]
fn casual_assembled_prompt_does_not_demand_citations() {
    let prompt = assemble_for_persona(CoachingPersona::Casual);
    let casual_block = get_coaching_persona_prompt(CoachingPersona::Casual);
    // The Casual block itself is responsible for forbidding citations;
    // confirm that assembling didn't accidentally drop the rule.
    assert!(
        prompt.contains(casual_block),
        "Casual persona block must appear verbatim in the assembled prompt"
    );
}

// ============================================================================
// Reply-fixture cross-product
// ============================================================================
//
// Each fixture below is the *reply* a well-behaved LLM would produce for the
// matching persona. We assert that the persona's invariants hold on its own
// fixture (positive case) AND that swapping fixtures across personas trips the
// inverse asserter (negative case). The negative case is the regression sentinel:
// if a future LLM update softens PowerAthlete to prose, the negative case here
// will pass for the wrong reason — and the positive `assert_has_*` case will
// flip red, which is exactly what we want.

const CASUAL_REPLY_OK: &str = "Solid week. Tomorrow keep it easy — 45 minutes Z2, no pace target. \
    Save the speed for Saturday.";

const POWER_REPLY_OK: &str = "Last 7 sessions:\n\
    2026-04-29 | Run | 12.4 km | 58 min | HR avg 152 (Banister Z2)\n\
    2026-04-27 | Ride | 60 min | NP 218 W (Coggan IF 0.81)\n\
    Readiness: TSB +4 (Banister), ACWR 1.08 (Gabbett green), monotony 1.3 (Foster).\n\
    Cleared at P3. Tomorrow: tempo_progression at 75 min — three by 15 min Z3 (Seiler 80/20).";

#[test]
fn casual_reply_satisfies_casual_invariants() {
    assert!(assert_no_framework_citations(CASUAL_REPLY_OK).is_ok());
    assert!(assert_no_line_by_line_block(CASUAL_REPLY_OK).is_ok());
    assert!(assert_word_count_under(CASUAL_REPLY_OK, 150).is_ok());
}

#[test]
fn power_reply_satisfies_power_invariants() {
    assert!(assert_has_line_by_line_block(POWER_REPLY_OK).is_ok());
    assert!(assert_has_framework_citation_per_numeric(POWER_REPLY_OK, 0.7).is_ok());
}

#[test]
fn power_reply_violates_casual_invariants() {
    // Power reply has framework citations — Casual forbids them.
    let err = assert_no_framework_citations(POWER_REPLY_OK).unwrap_err();
    assert!(
        ["Banister", "Coggan", "Gabbett", "Foster", "Seiler"]
            .iter()
            .any(|a| err.anchor == *a),
        "Casual asserter should flag at least one framework anchor in PowerAthlete reply, got {}",
        err.anchor
    );
    // Power reply uses dated pipe block — Casual forbids it.
    assert!(assert_no_line_by_line_block(POWER_REPLY_OK).is_err());
}

#[test]
fn casual_reply_violates_power_invariants() {
    // Casual reply has no dated pipe block — PowerAthlete demands it.
    assert!(assert_has_line_by_line_block(CASUAL_REPLY_OK).is_err());
}
